use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveMode {
    Stop,
    ContinueSame,
    ContinueNew,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSave {
    pub user_id: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub timer_started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub attempted_at: OffsetDateTime,
    pub mode: SaveMode,
}

pub fn path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("Cannot determine config directory")?
        .join("toki-tui")
        .join("pending-save.json"))
}

/// Hold this OS lock for the lifetime of the TUI or a recovery session. The lock
/// file stays on disk; the operating system releases its lock after a crash.
pub fn lock() -> Result<std::fs::File> {
    lock_at(&path()?.with_file_name("tui.lock"))
}

fn lock_at(lock_path: &Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(lock_path.parent().context("Missing lock directory")?)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(lock_path)
        .context("Cannot open TUI lock file")?;
    file.try_lock()
        .context("Another TUI or recovery session is using this config; close it first")?;
    Ok(file)
}

/// The file is created exclusively and synced *before* a request is sent.
/// An existing or malformed record must never be replaced by a later attempt.
pub fn begin(path: &Path, pending: &PendingSave) -> Result<()> {
    begin_with(path, pending, |file, bytes| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn begin_with(
    path: &Path,
    pending: &PendingSave,
    persist: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
) -> Result<()> {
    let bytes = serde_json::to_vec(pending)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("Cannot create pending save record at {}", path.display()))?;
    let result = (|| -> Result<()> {
        persist(&mut file, &bytes)?;
        #[cfg(unix)]
        std::fs::File::open(path.parent().context("Missing pending save directory")?)?
            .sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        // No request was sent. The exclusive lock prevents another TUI from
        // replacing this file while we remove a partially persisted record.
        drop(file);
        std::fs::remove_file(path).with_context(|| {
            format!("Cannot remove incomplete pending save record after {error}")
        })?;
        return Err(error).context("Could not persist pending save record before request");
    }
    Ok(())
}

pub fn load(path: &Path) -> Result<Option<PendingSave>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Cannot read pending save record"),
    };
    let pending = serde_json::from_slice(&bytes).context("Pending save record is unreadable")?;
    Ok(Some(pending))
}

/// Clear only the attempt that has been explicitly confirmed or reconciled.
pub fn clear(path: &Path, expected: &PendingSave) -> Result<()> {
    if load(path)?.as_ref() != Some(expected) {
        bail!("Pending save record does not match the confirmed attempt");
    }
    std::fs::remove_file(path).context("Cannot clear pending save record")?;
    #[cfg(unix)]
    std::fs::File::open(path.parent().context("Missing pending save directory")?)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_path() -> std::path::PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "toki-pending-save-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn attempt() -> PendingSave {
        PendingSave {
            user_id: 7,
            timer_started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            attempted_at: OffsetDateTime::from_unix_timestamp(1_700_000_060).unwrap(),
            mode: SaveMode::ContinueSame,
        }
    }

    #[test]
    fn records_and_clears_only_the_confirmed_attempt() {
        let path = test_path();
        let pending = attempt();
        begin(&path, &pending).unwrap();
        assert_eq!(load(&path).unwrap(), Some(pending.clone()));
        let mut other = pending.clone();
        other.attempted_at += time::Duration::seconds(1);
        assert!(clear(&path, &other).is_err());
        assert_eq!(load(&path).unwrap(), Some(pending.clone()));
        clear(&path, &pending).unwrap();
        assert_eq!(load(&path).unwrap(), None);
    }

    #[test]
    fn refuses_to_overwrite_an_unresolved_attempt() {
        let path = test_path();
        let pending = attempt();
        begin(&path, &pending).unwrap();
        assert!(begin(&path, &pending).is_err());
        assert_eq!(load(&path).unwrap(), Some(pending.clone()));
        clear(&path, &pending).unwrap();
    }

    #[test]
    fn partial_write_before_request_does_not_strand_a_save() {
        let path = test_path();
        let pending = attempt();
        let error = begin_with(&path, &pending, |file, _| {
            file.write_all(b"{incomplete")?;
            Err(std::io::Error::other("simulated write failure"))
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("simulated write failure"));
        assert!(
            !path.exists(),
            "no request was sent, so the partial guard must be removed"
        );
        begin(&path, &pending).unwrap();
        assert_eq!(load(&path).unwrap(), Some(pending.clone()));
        clear(&path, &pending).unwrap();
    }

    #[test]
    fn corrupt_record_fails_closed() {
        let path = test_path();
        std::fs::write(&path, b"{incomplete").unwrap();
        assert!(load(&path).is_err());
        assert!(begin(&path, &attempt()).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn concurrent_recovery_or_tui_is_rejected_until_lock_is_released() {
        let path = test_path();
        let first = lock_at(&path).unwrap();
        assert!(lock_at(&path).is_err());
        drop(first);
        let second = lock_at(&path).unwrap();
        drop(second);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_record_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = test_path();
        let pending = attempt();
        begin(&path, &pending).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        clear(&path, &pending).unwrap();
    }
}
