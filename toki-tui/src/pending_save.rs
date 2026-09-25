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

/// The file is created exclusively and synced *before* a request is sent.
/// An existing or malformed record must never be replaced by a later attempt.
pub fn begin(path: &Path, pending: &PendingSave) -> Result<()> {
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
    file.write_all(&bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    std::fs::File::open(path.parent().context("Missing pending save directory")?)?.sync_all()?;
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
    fn corrupt_record_fails_closed() {
        let path = test_path();
        std::fs::write(&path, b"{incomplete").unwrap();
        assert!(load(&path).is_err());
        assert!(begin(&path, &attempt()).is_err());
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
