use crate::api::ApiClient;
use crate::app::{App, TimerState, View};
use crate::pending_save;
use crate::types::TimeEntry;
use crate::ui;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::time::{Duration, Instant};

use super::action_queue::{channel, Action};
use super::actions::{
    apply_recent_history, fetch_recent_history, finish_save, perform_save, prepare_save,
    run_action, SaveAttempt, SaveOutcome,
};
use super::views::handle_view_key;

pub async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &mut ApiClient,
) -> Result<()> {
    // Show throbber for at least 3 seconds on startup.
    app.is_loading = true;
    let loading_until = Instant::now() + Duration::from_secs(3);

    // Background polling: refresh time entries every 60 seconds.
    let mut last_history_refresh = Instant::now();
    const HISTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

    let (action_tx, mut action_rx) = channel();
    let mut save_task: Option<tokio::task::JoinHandle<(SaveAttempt, SaveOutcome)>> = None;
    let mut unresolved = has_unresolved_save();
    let mut history_task: Option<tokio::task::JoinHandle<Result<Vec<TimeEntry>, String>>> = None;
    let mut history_requested_during_save = false;
    let mut redraw = true;
    let mut last_elapsed_second = None;

    loop {
        // A stopped, unchanged screen should emit no ANSI output. The running
        // clock only needs one frame per second; input and worker results redraw
        // immediately. Whether this also bounds Windows Terminal memory still
        // needs a native before/after measurement.
        let elapsed_second =
            (app.timer_state == TimerState::Running).then(|| app.elapsed_duration().as_secs());
        if redraw
            || app.needs_full_redraw
            || app.is_loading
            || elapsed_second != last_elapsed_second
        {
            // Clear before drawing when returning from an external editor or focus loss.
            if app.needs_full_redraw {
                terminal.clear()?;
                app.needs_full_redraw = false;
            }
            terminal.draw(|f| ui::render(f, app))?;
            redraw = false;
            last_elapsed_second = elapsed_second;
        }

        if app.is_loading {
            app.throbber_state.calc_next();
            if Instant::now() >= loading_until {
                app.is_loading = false;
                redraw = true;
            }
        }

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if save_task.is_some() || unresolved || has_unresolved_save() {
                        if matches!(key.code, KeyCode::Char('q' | 'Q')) {
                            app.quit();
                        } else if matches!(key.code, KeyCode::Char('h' | 'H')) {
                            let _ = action_tx.send(Action::LoadHistoryAndOpen);
                        } else if key.code == KeyCode::Esc && app.current_view == View::History {
                            app.navigate_to(View::Timer);
                        } else {
                            app.set_status(
                                "Save pending or unresolved. Use h for history or q to quit."
                                    .to_string(),
                            );
                        }
                    } else {
                        handle_view_key(key, app, &action_tx);
                    }
                    redraw = true;
                }
                // Force a full redraw when the terminal regains focus (e.g. after sleep/wake)
                Event::FocusGained | Event::Resize(_, _) => {
                    app.needs_full_redraw = true;
                    redraw = true;
                }
                _ => {}
            }
        }

        if save_task.as_ref().is_some_and(|task| task.is_finished()) {
            redraw = true;
            match save_task.take().unwrap().await {
                Ok((attempt, outcome)) => {
                    let uncertain = matches!(
                        &outcome,
                        SaveOutcome::Uncertain(_)
                            | SaveOutcome::Confirmed {
                                restart: Some(Err(_))
                            }
                    );
                    let confirmed = finish_save(app, attempt, outcome);
                    if confirmed || history_requested_during_save {
                        if let Some(task) = history_task.take() {
                            task.abort();
                        }
                        history_task = Some(spawn_history_refresh(client));
                        last_history_refresh = Instant::now();
                    }
                    history_requested_during_save = false;
                    unresolved = uncertain || has_unresolved_save();
                }
                Err(e) => {
                    unresolved = true;
                    app.set_status(format!(
                        "Save outcome unknown (worker failed: {}). Do not retry.",
                        e
                    ));
                    if history_requested_during_save {
                        history_task = Some(spawn_history_refresh(client));
                        history_requested_during_save = false;
                    }
                }
            }
        }
        if history_task.as_ref().is_some_and(|task| task.is_finished()) {
            redraw = true;
            match history_task.take().unwrap().await {
                Ok(Ok(entries)) => apply_recent_history(app, entries),
                Ok(Err(error)) => app.set_status(format!("Error refreshing history: {}", error)),
                Err(error) => app.set_status(format!("History refresh failed: {}", error)),
            }
        }

        if last_history_refresh.elapsed() >= HISTORY_REFRESH_INTERVAL && !app.is_in_edit_mode() {
            let _ = action_tx.send(Action::RefreshHistoryBackground);
            last_history_refresh = Instant::now();
        }

        while app.running {
            let Ok(action) = action_rx.try_recv() else {
                break;
            };
            redraw = true;
            match action {
                Action::SaveTimer => {
                    if unresolved {
                        app.set_status(
                            "Prior save unresolved; do not retry until verified".to_string(),
                        );
                    } else if save_task.is_some() {
                        app.set_status(
                            "Save already in progress; no second request sent".to_string(),
                        );
                    } else if let Some(attempt) = prepare_save(app) {
                        if let Some(task) = history_task.take() {
                            task.abort();
                        }
                        let mut worker_client = client.clone();
                        save_task = Some(tokio::spawn(async move {
                            let outcome = perform_save(&mut worker_client, &attempt).await;
                            (attempt, outcome)
                        }));
                    }
                }
                Action::RefreshHistoryBackground => {
                    if save_task.is_none() && history_task.is_none() {
                        history_task = Some(spawn_history_refresh(client));
                    }
                }
                Action::LoadHistoryAndOpen => {
                    app.navigate_to(View::History);
                    if save_task.is_some() {
                        history_requested_during_save = true;
                    } else if history_task.is_none() {
                        history_task = Some(spawn_history_refresh(client));
                    }
                }
                _other if save_task.is_some() || unresolved || has_unresolved_save() => {
                    // Nothing that could write or replace the active timer may run during an uncertain save.
                    app.set_status(
                        "Save pending or unresolved; timer changes are blocked".to_string(),
                    );
                }
                other => {
                    // An older read must not undo a successful edit or deletion.
                    if matches!(
                        other,
                        Action::SaveHistoryEdit | Action::SaveThisWeekEdit | Action::ConfirmDelete
                    ) {
                        if let Some(task) = history_task.take() {
                            task.abort();
                        }
                    }
                    run_action(other, app, client).await?;
                }
            }
        }

        if !app.running {
            break;
        }
    }

    Ok(())
}

fn has_unresolved_save() -> bool {
    pending_save::path()
        .and_then(|path| pending_save::load(&path))
        .map(|pending| pending.is_some())
        .unwrap_or(true)
}

fn spawn_history_refresh(
    client: &ApiClient,
) -> tokio::task::JoinHandle<Result<Vec<TimeEntry>, String>> {
    let mut worker_client = client.clone();
    tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(30),
            fetch_recent_history(&mut worker_client),
        )
        .await
        .map_err(|_| "history request timed out".to_string())?
        .map_err(|e| e.to_string())
    })
}
