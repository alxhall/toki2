use crate::api::ApiClient;
use crate::pending_save;
use crate::recovery::{self, RecoverySnapshot};
use crate::ui::recovery_overlay::RecoveryOverlay;
use crossterm::event::KeyCode;
use std::path::PathBuf;

type InspectionTask = tokio::task::JoinHandle<Result<RecoverySnapshot, String>>;

#[derive(Default)]
pub(super) struct RecoveryFlow {
    pub overlay: Option<RecoveryOverlay>,
    task: Option<InspectionTask>,
    guard_path: Option<PathBuf>,
    /// After clearing, the current App may not match the authoritative server timer.
    /// Never allow further timer changes until the app is relaunched.
    pub must_relaunch: bool,
}

impl RecoveryFlow {
    pub fn start(&mut self, user_id: i32, client: &ApiClient) {
        let result = pending_save::path().and_then(|path| {
            let pending = pending_save::load(&path)?;
            Ok((path, pending))
        });
        match result {
            Ok((path, Some(pending))) if pending.user_id == user_id => {
                self.guard_path = Some(path);
                self.overlay = Some(RecoveryOverlay::Checking);
                let mut worker_client = client.clone();
                self.task = Some(tokio::spawn(async move {
                    recovery::inspect(&mut worker_client, &pending)
                        .await
                        .map_err(|e| e.to_string())
                }));
            }
            Ok((_, Some(_))) => {
                self.overlay = Some(RecoveryOverlay::Error(
                    "This guard belongs to another account; it was not changed.".to_string(),
                ));
            }
            Ok((_, None)) => {
                self.overlay = Some(RecoveryOverlay::Error(
                    "No recovery record found. Quit and relaunch before changing a timer."
                        .to_string(),
                ));
            }
            Err(error) => self.overlay = Some(RecoveryOverlay::Error(error.to_string())),
        }
    }

    pub async fn poll(&mut self) -> bool {
        if !self.task.as_ref().is_some_and(|task| task.is_finished()) {
            return false;
        }
        self.overlay = Some(match self.task.take().unwrap().await {
            Ok(Ok(snapshot)) => RecoveryOverlay::Review {
                snapshot,
                scroll: 0,
                confirm: false,
            },
            Ok(Err(error)) => RecoveryOverlay::Error(error),
            Err(error) => RecoveryOverlay::Error(format!("Review worker failed: {error}")),
        });
        true
    }

    /// Called only when the normal TUI/recovery exclusive lock is held.
    pub fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc if !matches!(self.overlay, Some(RecoveryOverlay::Cleared)) => {
                if let Some(task) = self.task.take() {
                    task.abort();
                }
                self.overlay = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(overlay) = &mut self.overlay {
                    overlay.scroll(false);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(overlay) = &mut self.overlay {
                    overlay.scroll(true);
                }
            }
            KeyCode::Char('c' | 'C') => {
                if let Some(RecoveryOverlay::Review { confirm, .. }) = &mut self.overlay {
                    *confirm = true;
                }
            }
            KeyCode::Char('n' | 'N') => {
                if let Some(RecoveryOverlay::Review { confirm, .. }) = &mut self.overlay {
                    *confirm = false;
                }
            }
            KeyCode::Char('y' | 'Y') => {
                if let Some(RecoveryOverlay::Review {
                    snapshot,
                    confirm: true,
                    ..
                }) = &self.overlay
                {
                    let result = self
                        .guard_path
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Missing recovery record path"))
                        .and_then(|path| pending_save::clear(path, &snapshot.pending));
                    match result {
                        Ok(()) => {
                            self.must_relaunch = true;
                            self.overlay = Some(RecoveryOverlay::Cleared);
                        }
                        Err(error) => {
                            self.overlay = Some(RecoveryOverlay::Error(error.to_string()))
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending_save::{PendingSave, SaveMode};
    use time::OffsetDateTime;

    #[test]
    fn only_reviewed_two_step_confirmation_clears_matching_local_guard() {
        let path =
            std::env::temp_dir().join(format!("toki-inline-recovery-{}", std::process::id()));
        let pending = PendingSave {
            user_id: 17,
            timer_started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            attempted_at: OffsetDateTime::from_unix_timestamp(1_700_000_060).unwrap(),
            mode: SaveMode::Stop,
        };
        pending_save::begin(&path, &pending).unwrap();
        let mut flow = RecoveryFlow {
            overlay: Some(RecoveryOverlay::Review {
                snapshot: RecoverySnapshot {
                    pending: pending.clone(),
                    lines: vec!["Server timer: none".to_string()],
                },
                scroll: 0,
                confirm: false,
            }),
            guard_path: Some(path.clone()),
            ..Default::default()
        };

        flow.handle_key(KeyCode::Char('y'));
        assert_eq!(pending_save::load(&path).unwrap(), Some(pending.clone()));
        flow.handle_key(KeyCode::Char('c'));
        flow.handle_key(KeyCode::Char('n'));
        flow.handle_key(KeyCode::Char('y'));
        assert_eq!(pending_save::load(&path).unwrap(), Some(pending.clone()));
        flow.handle_key(KeyCode::Char('c'));
        flow.handle_key(KeyCode::Char('y'));
        assert_eq!(pending_save::load(&path).unwrap(), None);
        assert!(flow.must_relaunch);
        assert!(matches!(flow.overlay, Some(RecoveryOverlay::Cleared)));
    }
}
