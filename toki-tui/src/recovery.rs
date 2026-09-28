use crate::api::ApiClient;
use crate::config::TokiConfig;
use crate::pending_save::{self, PendingSave};
use crate::session_store;
use anyhow::{bail, Context, Result};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::time::Duration;

/// Never decides whether the server committed. The only mutation is removing
/// the local retry guard after a human verifies the outcome in their account.
pub async fn run() -> Result<()> {
    let _lock = pending_save::lock()?;
    let path = pending_save::path()?;
    let Some(pending) = pending_save::load(&path)? else {
        println!("No unresolved save attempt.");
        return Ok(());
    };
    if !io::stdin().is_terminal() {
        bail!("Recovery requires an interactive terminal; the local guard was not changed");
    }
    let cfg = TokiConfig::load()?;
    let session = session_store::load_session()?.context("Log in to the same account first")?;
    let mut client = ApiClient::new(&cfg.api_url, &session)?;
    let snapshot = inspect(&mut client, &pending).await?;
    let mut output = io::stdout().lock();
    for line in &snapshot.lines {
        writeln!(output, "{line}")?;
    }
    confirm_clear(&path, &pending, &mut io::stdin().lock(), &mut output)
}

/// Read-only evidence for CLI and in-app recovery. Absence of a matching entry
/// is never proof that the provider did not commit.
pub(crate) struct RecoverySnapshot {
    pub pending: PendingSave,
    pub lines: Vec<String>,
}

pub(crate) async fn inspect(
    client: &mut ApiClient,
    pending: &PendingSave,
) -> Result<RecoverySnapshot> {
    let me = tokio::time::timeout(Duration::from_secs(15), client.me())
        .await
        .context("Timed out checking the current account")??;
    if me.id != pending.user_id {
        bail!("The recovery record belongs to another account; log in to that account first");
    }
    let timer = tokio::time::timeout(Duration::from_secs(15), client.get_active_timer())
        .await
        .context("Timed out checking the server timer")??;
    let to = time::OffsetDateTime::now_utc().date() + time::Duration::days(1);
    let earliest = pending
        .timer_started_at
        .date()
        .checked_sub(time::Duration::days(1))
        .context("Invalid recorded timer start")?;
    let from = earliest.max(to - time::Duration::days(29));
    if from > to {
        bail!("The recorded timer start is in the future; check the system clock before recovery");
    }
    let mut entries =
        tokio::time::timeout(Duration::from_secs(15), client.get_time_entries(from, to))
            .await
            .context("Timed out checking recent server entries")??;
    entries.sort_by(|a, b| b.date.cmp(&a.date));

    let mut lines = vec![format!(
        "Unresolved save for account {}: timer started {}, attempted {} ({:?}).",
        pending.user_id, pending.timer_started_at, pending.attempted_at, pending.mode
    )];
    lines.push(match timer {
        Some(timer) => format!(
            "Current server timer: started {}, project {}.",
            timer.start_time,
            safe_label(timer.project_name.as_deref().unwrap_or("(none)"))
        ),
        None => "Current server timer: none.".to_string(),
    });
    if from > earliest {
        lines.push("The original timer predates this 30-day history window; check older entries in the web app.".to_string());
    }
    lines.push("Recent server entries (up to 20; not proof of a match):".to_string());
    for entry in entries.iter().take(20) {
        lines.push(format!(
            "  {}  {}h  {} / {}  registration {}  start {}",
            safe_label(&entry.date),
            entry.hours,
            safe_label(&entry.project_name),
            safe_label(&entry.activity_name),
            safe_label(&entry.registration_id),
            entry
                .start_time
                .map(|t| t.to_string())
                .unwrap_or_else(|| "unavailable".to_string())
        ));
    }
    if entries.len() > 20 {
        lines.push(format!(
            "  ... {} more entries. Inspect the web app for the full list.",
            entries.len() - 20
        ));
    }
    lines.push("These reads may be inconclusive. Verify the save in the server/web app and close other TUI instances before clearing. If unsure, leave the guard in place. No save will be retried automatically.".to_string());
    Ok(RecoverySnapshot {
        pending: pending.clone(),
        lines,
    })
}

fn safe_label(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(64).collect()
}

fn confirm_clear(
    path: &Path,
    pending: &PendingSave,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<()> {
    let phrase = format!(
        "CLEAR VERIFIED {} {}",
        pending.user_id,
        pending.attempted_at.unix_timestamp()
    );
    writeln!(output, "Type '{phrase}' ONLY after verifying whether this entry was saved (anything else keeps the guard):")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if answer.trim_end_matches(['\r', '\n']) != phrase {
        writeln!(output, "Recovery record unchanged. No write was sent.")?;
        return Ok(());
    }
    pending_save::clear(path, pending)?;
    writeln!(
        output,
        "Local recovery guard cleared. No time entry was created, retried or deleted."
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{confirm_clear, safe_label};
    use crate::pending_save::{self, PendingSave, SaveMode};
    use std::io::Cursor;
    use time::OffsetDateTime;

    fn attempt() -> PendingSave {
        PendingSave {
            user_id: 17,
            timer_started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            attempted_at: OffsetDateTime::from_unix_timestamp(1_700_000_060).unwrap(),
            mode: SaveMode::Stop,
        }
    }

    #[test]
    fn external_labels_cannot_inject_terminal_control_sequences() {
        assert_eq!(safe_label("Project\u{1b}[2J\r\nOther"), "Project[2JOther");
    }

    #[test]
    fn refusal_or_eof_keeps_the_record() {
        for (i, input) in ["no\n", ""].iter().enumerate() {
            let path = std::env::temp_dir().join(format!(
                "toki-recover-refuse-{}-{}",
                std::process::id(),
                i
            ));
            let pending = attempt();
            pending_save::begin(&path, &pending).unwrap();
            let mut output = Vec::new();
            confirm_clear(
                &path,
                &pending,
                &mut Cursor::new(input.as_bytes()),
                &mut output,
            )
            .unwrap();
            assert_eq!(pending_save::load(&path).unwrap(), Some(pending.clone()));
            pending_save::clear(&path, &pending).unwrap();
        }
    }

    #[test]
    fn exact_review_confirmation_only_clears_the_original_attempt() {
        let path =
            std::env::temp_dir().join(format!("toki-recover-confirm-{}", std::process::id()));
        let pending = attempt();
        pending_save::begin(&path, &pending).unwrap();
        let mut changed = pending.clone();
        changed.attempted_at += time::Duration::seconds(1);
        let mut output = Vec::new();
        let phrase = format!(
            "CLEAR VERIFIED {} {}\n",
            pending.user_id,
            pending.attempted_at.unix_timestamp()
        );
        let changed_phrase = format!(
            "CLEAR VERIFIED {} {}\n",
            changed.user_id,
            changed.attempted_at.unix_timestamp()
        );
        assert!(confirm_clear(
            &path,
            &changed,
            &mut Cursor::new(changed_phrase.as_bytes()),
            &mut output
        )
        .is_err());
        assert_eq!(pending_save::load(&path).unwrap(), Some(pending.clone()));
        confirm_clear(
            &path,
            &pending,
            &mut Cursor::new(phrase.as_bytes()),
            &mut output,
        )
        .unwrap();
        assert_eq!(pending_save::load(&path).unwrap(), None);
    }
}
