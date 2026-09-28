use crate::api::{ApiClient, SaveTimerRequest};
use crate::app::{self, App};
use crate::pending_save::{self, PendingSave, SaveMode};
use crate::types;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::action_queue::{Action, ActionTx};

/// Apply an active timer fetched from the server into App state.
pub(crate) fn restore_active_timer(app: &mut App, timer: crate::types::ActiveTimerState) {
    let elapsed_secs = (timer.hours * 3600 + timer.minutes * 60 + timer.seconds) as u64;
    app.absolute_start = Some(timer.start_time);
    app.local_start = Some(Instant::now() - Duration::from_secs(elapsed_secs));
    app.timer_state = app::TimerState::Running;
    if app.auto_resize_timer {
        app.timer_size = app::TimerSize::Large;
    }
    if let (Some(id), Some(name)) = (timer.project_id, timer.project_name) {
        app.selected_project = Some(crate::types::Project { id, name });
    }
    if let (Some(id), Some(name)) = (timer.activity_id, timer.activity_name) {
        app.selected_activity = Some(crate::types::Activity {
            id,
            name,
            project_id: app
                .selected_project
                .as_ref()
                .map(|p| p.id.clone())
                .unwrap_or_default(),
        });
    }
    if !timer.note.is_empty() {
        app.set_note_from_raw(&timer.note);
        app.description_is_default = false;
    }
}

pub(super) async fn run_action(
    action: Action,
    app: &mut App,
    client: &mut ApiClient,
) -> Result<()> {
    match action {
        Action::ApplyProjectSelection {
            had_edit_state,
            saved_selected_project,
            saved_selected_activity,
        } => {
            handle_project_selection_enter(
                app,
                client,
                had_edit_state,
                saved_selected_project,
                saved_selected_activity,
            )
            .await;
        }
        Action::ApplyActivitySelection {
            was_in_edit_mode,
            saved_selected_project,
            saved_selected_activity,
        } => {
            handle_activity_selection_enter(
                app,
                client,
                was_in_edit_mode,
                saved_selected_project,
                saved_selected_activity,
            )
            .await;
        }
        Action::OpenEditActivityPicker { project_id } => {
            app.pending_edit_selection_restore.get_or_insert_with(|| {
                (app.selected_project.clone(), app.selected_activity.clone())
            });

            let project_name = app
                .projects
                .iter()
                .find_map(|project| (project.id == project_id).then(|| project.name.clone()))
                .unwrap_or_default();
            app.selected_project = Some(types::Project {
                id: project_id.clone(),
                name: project_name,
            });
            app.selected_activity = None;

            ensure_activities_for_project(app, client, &project_id).await;
            app.navigate_to(app::View::SelectActivity);
        }
        Action::StartTimer => {
            handle_start_timer(app, client).await?;
        }
        Action::SaveTimer => {
            app.set_status("Save must be scheduled by the UI event loop".to_string());
        }
        Action::SyncRunningTimerNote { note } => {
            sync_running_timer_note(note, app, client).await;
        }
        Action::SaveHistoryEdit => {
            handle_history_edit_save(app, client).await?;
        }
        Action::SaveThisWeekEdit => {
            handle_this_week_edit_save(app, client).await?;
        }
        Action::LoadHistoryAndOpen => {
            load_history_and_open(app, client).await;
        }
        Action::ConfirmDelete => {
            handle_confirm_delete(app, client).await;
        }
        Action::StopServerTimerAndClear => {
            stop_server_timer_and_clear(app, client).await;
        }
        Action::RefreshHistoryBackground => {
            refresh_history_background(app, client).await;
        }
        Action::ResumeEntry(entry) => {
            resume_entry(entry, app, client).await;
        }
        Action::ApplyTemplate { template } => {
            handle_apply_template(template, app, client).await?;
        }
        Action::OpenLogNote => {
            if let Err(e) = handle_open_log_note(app, client).await {
                app.set_status(format!("Log note error: {}", e));
            }
        }
        Action::OpenEntryLogNote(id) => {
            handle_open_entry_log_note(&id, app).await;
        }
    }
    Ok(())
}

pub(super) async fn handle_start_timer(app: &mut App, client: &mut ApiClient) -> Result<()> {
    match app.timer_state {
        app::TimerState::Stopped => {
            let project_id = app.selected_project.as_ref().map(|p| p.id.clone());
            let project_name = app.selected_project.as_ref().map(|p| p.name.clone());
            let activity_id = app.selected_activity.as_ref().map(|a| a.id.clone());
            let activity_name = app.selected_activity.as_ref().map(|a| a.name.clone());
            let note = {
                let full = app.full_note_value();
                if full.is_empty() {
                    None
                } else {
                    Some(full)
                }
            };
            // Optimistic: start the timer locally immediately, roll back on error.
            let auto_resize = app.auto_resize_timer;
            app.start_timer(auto_resize);
            if let Err(e) = client
                .start_timer(project_id, project_name, activity_id, activity_name, note)
                .await
            {
                app.stop_timer(auto_resize);
                app.set_status(format!("Error starting timer: {}", e));
                return Ok(());
            }
            app.clear_status();
        }
        app::TimerState::Running => {
            app.set_status("Timer already running (Ctrl+S to save)".to_string());
        }
    }
    Ok(())
}

async fn handle_project_selection_enter(
    app: &mut App,
    client: &mut ApiClient,
    had_edit_state: bool,
    saved_selected_project: Option<types::Project>,
    saved_selected_activity: Option<types::Activity>,
) {
    if let Some(project_id) = app
        .selected_project
        .as_ref()
        .map(|project| project.id.clone())
    {
        ensure_activities_for_project(app, client, &project_id).await;
    }

    if had_edit_state {
        if let Some(project) = app.selected_project.clone() {
            app.update_edit_state_project(project.id.clone(), project.name.clone());
        }
        app.pending_edit_selection_restore =
            Some((saved_selected_project, saved_selected_activity));
    }

    app.navigate_to(app::View::SelectActivity);
}

async fn ensure_activities_for_project(app: &mut App, client: &mut ApiClient, project_id: &str) {
    if !app.activity_cache.contains_key(project_id) {
        app.is_loading = true;
        let fetch_result = client.get_activities(project_id).await;
        app.is_loading = false;

        match fetch_result {
            Ok(activities) => {
                app.activity_cache
                    .insert(project_id.to_string(), activities);
            }
            Err(e) => {
                app.set_status(format!("Failed to load activities: {}", e));
            }
        }
    }

    if let Some(cached) = app.activity_cache.get(project_id) {
        app.activities = cached.clone();
    } else {
        app.activities.clear();
    }
    app.filtered_activities.clear();
    app.filtered_activity_index = 0;
}

async fn handle_activity_selection_enter(
    app: &mut App,
    client: &mut ApiClient,
    was_in_edit_mode: bool,
    saved_selected_project: Option<types::Project>,
    saved_selected_activity: Option<types::Activity>,
) {
    if was_in_edit_mode {
        if let Some(activity) = app.selected_activity.clone() {
            app.update_edit_state_activity(activity.id.clone(), activity.name.clone());
        }
        let (restore_project, restore_activity) = app
            .pending_edit_selection_restore
            .take()
            .unwrap_or((saved_selected_project, saved_selected_activity));
        app.selected_project = restore_project;
        app.selected_activity = restore_activity;
        let return_view = app.get_return_view_from_edit();
        app.navigate_to(return_view);
        app.focused_box = app::FocusedBox::Today;
        app.entry_edit_set_focused_field(app::EntryEditField::Activity);
        return;
    }

    app.pending_edit_selection_restore = None;

    if app.timer_state == app::TimerState::Running {
        let project_id = app.selected_project.as_ref().map(|p| p.id.clone());
        let project_name = app.selected_project.as_ref().map(|p| p.name.clone());
        let activity_id = app.selected_activity.as_ref().map(|a| a.id.clone());
        let activity_name = app.selected_activity.as_ref().map(|a| a.name.clone());
        if let Err(e) = client
            .update_active_timer(
                project_id,
                project_name,
                activity_id,
                activity_name,
                None,
                None,
            )
            .await
        {
            app.set_status(format!("Warning: Could not sync project to server: {}", e));
        }
    }
}

pub(super) fn apply_recent_history(app: &mut App, entries: Vec<types::TimeEntry>) {
    app.update_history(entries);
    app.rebuild_history_list();
}

pub(super) async fn fetch_recent_history(client: &mut ApiClient) -> Result<Vec<types::TimeEntry>> {
    let today = time::OffsetDateTime::now_utc().date();
    let month_ago = today - time::Duration::days(30);
    client.get_time_entries(month_ago, today).await
}

async fn sync_running_timer_note(note: String, app: &mut App, client: &mut ApiClient) {
    if app.timer_state != app::TimerState::Running {
        return;
    }

    if let Err(e) = client
        .update_active_timer(None, None, None, None, Some(note), None)
        .await
    {
        app.set_status(format!("Warning: Could not sync note to server: {}", e));
    }
}

async fn handle_apply_template(
    template: crate::config::TemplateConfig,
    app: &mut App,
    client: &mut ApiClient,
) -> Result<()> {
    // Find project by name (case-insensitive)
    let project = app
        .projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(&template.project))
        .cloned();

    let Some(project) = project else {
        // Project not found — tell the user and navigate back
        app.set_status(format!(
            "Project '{}' not found — template not applied",
            template.project
        ));
        app.navigate_to(app::View::Timer);
        return Ok(());
    };

    app.selected_project = Some(project.clone());

    // Ensure activities are loaded for this project
    ensure_activities_for_project(app, client, &project.id).await;

    // Find activity by name (case-insensitive)
    let activity = app.activity_cache.get(&project.id).and_then(|acts| {
        acts.iter()
            .find(|a| a.name.eq_ignore_ascii_case(&template.activity))
            .cloned()
    });

    if let Some(activity) = activity {
        app.selected_activity = Some(activity);
    } else {
        app.set_status(format!(
            "Activity '{}' not found — skipped",
            template.activity
        ));
    }

    // Set note
    app.description_input = app::TextInput::from_str(&template.note);
    app.description_is_default = template.note.is_empty();

    // Navigate back to timer
    app.navigate_to(app::View::Timer);

    // If timer is running, sync to server
    if app.timer_state == app::TimerState::Running {
        let note = app.full_note_value();
        let project_id = app.selected_project.as_ref().map(|p| p.id.clone());
        let project_name = app.selected_project.as_ref().map(|p| p.name.clone());
        let activity_id = app.selected_activity.as_ref().map(|a| a.id.clone());
        let activity_name = app.selected_activity.as_ref().map(|a| a.name.clone());
        if let Err(e) = client
            .update_active_timer(
                project_id,
                project_name,
                activity_id,
                activity_name,
                Some(note),
                None,
            )
            .await
        {
            app.set_status(format!("Warning: Could not sync template to server: {}", e));
        }
    }

    Ok(())
}

async fn load_history_and_open(app: &mut App, client: &mut ApiClient) {
    match fetch_recent_history(client).await {
        Ok(entries) => {
            apply_recent_history(app, entries);
            app.navigate_to(app::View::History);
        }
        Err(e) => {
            app.set_status(format!("Error loading history: {}", e));
        }
    }
}

async fn handle_confirm_delete(app: &mut App, client: &mut ApiClient) {
    if let Some(ctx) = app.delete_context.take() {
        let origin = ctx.origin;

        // Optimistic: remove from local state immediately, restore on error.
        let removed = app
            .time_entries
            .iter()
            .position(|e| e.registration_id == ctx.registration_id)
            .map(|idx| app.time_entries.remove(idx));
        app.rebuild_history_list();

        match origin {
            app::DeleteOrigin::Timer => app.navigate_to(app::View::Timer),
            app::DeleteOrigin::History => app.navigate_to(app::View::History),
        }

        match client.delete_time_entry(&ctx.registration_id).await {
            Ok(()) => {
                app.set_status("Entry deleted".to_string());
            }
            Err(e) => {
                // Roll back: re-insert the entry and rebuild the list
                if let Some(entry) = removed {
                    app.time_entries.push(entry);
                    app.time_entries.sort_by(|a, b| b.date.cmp(&a.date));
                    app.rebuild_history_list();
                }
                app.set_status(format!("Delete failed: {}", e));
            }
        }
    }
}

async fn stop_server_timer_and_clear(app: &mut App, client: &mut ApiClient) {
    if app.timer_state == app::TimerState::Running {
        if let Err(e) = client.stop_timer().await {
            app.set_status(format!("Warning: Could not stop server timer: {}", e));
        }
    }
    app.clear_timer();
}

async fn refresh_history_background(app: &mut App, client: &mut ApiClient) {
    match fetch_recent_history(client).await {
        Ok(entries) => {
            apply_recent_history(app, entries);
        }
        Err(e) => {
            app.set_status(format!("Error refreshing history: {}", e));
        }
    }
}

async fn resume_entry(entry: types::TimeEntry, app: &mut App, client: &mut ApiClient) {
    if app.timer_state == app::TimerState::Running {
        // Timer already running — copy fields and sync to server (yank behaviour)
        app.copy_entry_fields(&entry);

        let project_id = app.selected_project.as_ref().map(|p| p.id.clone());
        let project_name = app.selected_project.as_ref().map(|p| p.name.clone());
        let activity_id = app.selected_activity.as_ref().map(|a| a.id.clone());
        let activity_name = app.selected_activity.as_ref().map(|a| a.name.clone());
        let note = {
            let full = app.full_note_value();
            if full.is_empty() {
                None
            } else {
                Some(full)
            }
        };
        if let Err(e) = client
            .update_active_timer(
                project_id,
                project_name,
                activity_id,
                activity_name,
                note,
                None,
            )
            .await
        {
            app.set_status(format!(
                "Warning: Could not sync copied entry to server: {}",
                e
            ));
        } else {
            app.set_status(format!(
                "Copied: {}: {}",
                entry.project_name, entry.activity_name
            ));
        }
        return;
    }

    // Build server arguments from the entry directly (not from app state)
    let project_id = Some(entry.project_id.clone());
    let project_name = Some(entry.project_name.clone());
    let activity_id = Some(entry.activity_id.clone());
    let activity_name = Some(entry.activity_name.clone());
    let note = entry.note.clone().filter(|n| !n.is_empty());

    // Optimistic: copy fields and start the timer locally immediately, roll back on error.
    app.copy_entry_fields(&entry);
    let auto_resize = app.auto_resize_timer;
    app.start_timer(auto_resize);
    match client
        .start_timer(project_id, project_name, activity_id, activity_name, note)
        .await
    {
        Ok(()) => {
            app.set_status(format!(
                "Resumed: {}: {}",
                entry.project_name, entry.activity_name
            ));
        }
        Err(e) => {
            app.stop_timer(auto_resize);
            app.set_status(format!("Error resuming entry: {}", e));
        }
    }
}

pub(super) struct SaveAttempt {
    path: PathBuf,
    pending: PendingSave,
    note: Option<String>,
    start_args: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    duration_str: String,
    project_display: String,
    activity_display: String,
}

pub(super) enum SaveOutcome {
    Uncertain(String),
    Confirmed { restart: Option<Result<(), String>> },
}

pub(super) fn prepare_save(app: &mut App) -> Option<SaveAttempt> {
    if app.selected_save_action == app::SaveAction::Cancel {
        app.navigate_to(app::View::Timer);
        return None;
    }
    let path = match pending_save::path() {
        Ok(path) => path,
        Err(e) => {
            app.navigate_to(app::View::Timer);
            app.set_status(format!("Cannot save without a recovery record: {}", e));
            return None;
        }
    };
    prepare_save_at(app, &path)
}

fn prepare_save_at(app: &mut App, path: &Path) -> Option<SaveAttempt> {
    if app.selected_save_action == app::SaveAction::Cancel {
        app.navigate_to(app::View::Timer);
        return None;
    }
    let Some(timer_started_at) = app
        .absolute_start
        .filter(|_| app.timer_state == app::TimerState::Running)
    else {
        app.navigate_to(app::View::Timer);
        app.set_status("No running timer to save".to_string());
        return None;
    };
    let duration = app.elapsed_duration();
    if duration < Duration::from_secs(60) {
        app.navigate_to(app::View::Timer);
        app.set_status(format!(
            "Save requires at least one minute. Wait {} more seconds; timer is still running.",
            60 - duration.as_secs()
        ));
        return None;
    }
    let mode = match app.selected_save_action {
        app::SaveAction::SaveAndStop => SaveMode::Stop,
        app::SaveAction::ContinueSameProject => SaveMode::ContinueSame,
        app::SaveAction::ContinueNewProject => SaveMode::ContinueNew,
        app::SaveAction::Cancel => unreachable!(),
    };
    let pending = PendingSave {
        user_id: app.user_id,
        timer_started_at,
        attempted_at: time::OffsetDateTime::now_utc(),
        mode,
    };
    if let Err(e) = pending_save::begin(path, &pending) {
        app.navigate_to(app::View::Timer);
        app.set_status(format!(
            "Cannot save: previous attempt unresolved or recovery record unavailable ({})",
            e
        ));
        return None;
    }

    let full_note = app.full_note_value();
    let note = (!full_note.is_empty()).then_some(full_note);
    let start_args = if mode == SaveMode::ContinueSame {
        (
            app.selected_project.as_ref().map(|p| p.id.clone()),
            app.selected_project.as_ref().map(|p| p.name.clone()),
            app.selected_activity.as_ref().map(|a| a.id.clone()),
            app.selected_activity.as_ref().map(|a| a.name.clone()),
        )
    } else {
        (None, None, None, None)
    };
    app.navigate_to(app::View::Timer);
    app.set_status("Saving... press q to quit safely; verify the result on restart".to_string());
    Some(SaveAttempt {
        path: path.to_owned(),
        pending,
        note,
        start_args,
        duration_str: format!(
            "{:02}:{:02}:{:02}",
            duration.as_secs() / 3600,
            (duration.as_secs() % 3600) / 60,
            duration.as_secs() % 60
        ),
        project_display: app.current_project_name(),
        activity_display: app.current_activity_name(),
    })
}

pub(super) async fn perform_save(client: &mut ApiClient, attempt: &SaveAttempt) -> SaveOutcome {
    // A deadline can stop waiting, but cannot prove the server did not commit.
    match tokio::time::timeout(
        Duration::from_secs(30),
        client.save_timer(SaveTimerRequest {
            user_note: attempt.note.clone(),
        }),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return SaveOutcome::Uncertain(e.to_string()),
        Err(_) => return SaveOutcome::Uncertain("save request timed out".to_string()),
    }
    let restart = if attempt.pending.mode == SaveMode::Stop {
        None
    } else {
        let (project_id, project_name, activity_id, activity_name) = attempt.start_args.clone();
        Some(
            match tokio::time::timeout(
                Duration::from_secs(30),
                client.start_timer(project_id, project_name, activity_id, activity_name, None),
            )
            .await
            {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => Err("restart request timed out".to_string()),
            },
        )
    };
    SaveOutcome::Confirmed { restart }
}

/// Apply only the API-confirmed result. Returns true if history should be refreshed.
pub(super) fn finish_save(app: &mut App, attempt: SaveAttempt, outcome: SaveOutcome) -> bool {
    let SaveOutcome::Confirmed { restart } = outcome else {
        if let SaveOutcome::Uncertain(error) = outcome {
            app.set_status(format!(
                "Save outcome unknown ({}). Press r to review; do not retry.",
                error
            ));
        }
        return false;
    };

    app.stop_timer(app.auto_resize_timer);
    if let Some(Err(error)) = &restart {
        // The entry was saved, but the separate restart may have committed even
        // if its response was lost. Keep the guard until the server timer and
        // history have been checked manually; never start another timer here.
        app.set_status(format!(
            "Entry saved; restart outcome unknown ({}). Press r to review; do not retry.",
            error
        ));
        return true;
    }
    if let Err(e) = pending_save::clear(&attempt.path, &attempt.pending) {
        app.set_status(format!("Saved, but could not clear recovery record: {}", e));
        return true;
    }
    if attempt.pending.mode != SaveMode::Stop {
        app.description_input.clear();
        app.description_is_default = true;
        if attempt.pending.mode == SaveMode::ContinueNew {
            app.selected_project = None;
            app.selected_activity = None;
        }
    }
    match restart {
        Some(Ok(())) => {
            app.start_timer(app.auto_resize_timer);
            app.set_status(format!(
                "Saved {} to {} / {}",
                attempt.duration_str, attempt.project_display, attempt.activity_display
            ));
        }
        Some(Err(e)) => app.set_status(format!("Saved, but could not confirm restart: {}", e)),
        None => app.set_status(format!(
            "Saved {} to {} / {}",
            attempt.duration_str, attempt.project_display, attempt.activity_display
        )),
    }
    true
}

#[cfg(test)]
async fn handle_save_timer_with_action_at(
    app: &mut App,
    client: &mut ApiClient,
    path: &Path,
) -> Result<()> {
    if let Some(attempt) = prepare_save_at(app, path) {
        let outcome = perform_save(client, &attempt).await;
        if finish_save(app, attempt, outcome) {
            refresh_history_background(app, client).await;
        }
    }
    Ok(())
}

// Helper functions for edit mode

enum EditEnterAction {
    ProjectPicker,
    ActivityPicker { project_id: String },
    NoteEditor { note: String },
}

/// Handle Enter key in edit mode - open modal for Project/Activity/Note or move to next field.
pub(super) fn handle_entry_edit_enter(app: &mut App, action_tx: &ActionTx) {
    // Extract the data we need first to avoid borrow conflicts
    let action = if let Some(state) = app.current_edit_state() {
        match state.focused_field {
            app::EntryEditField::Project => Some(EditEnterAction::ProjectPicker),
            app::EntryEditField::Activity => {
                if let Some(project_id) = state.project_id.clone() {
                    Some(EditEnterAction::ActivityPicker { project_id })
                } else {
                    app.set_status("Please select a project first".to_string());
                    None
                }
            }
            app::EntryEditField::Note => {
                let note = state.note.value.clone();
                Some(EditEnterAction::NoteEditor { note })
            }
            app::EntryEditField::StartTime | app::EntryEditField::EndTime => {
                // Move to next field (like Tab)
                app.entry_edit_next_field();
                None
            }
        }
    } else {
        None
    };

    let Some(action) = action else {
        return;
    };

    // Now perform actions that don't require the borrow.
    match action {
        EditEnterAction::ProjectPicker => {
            app.navigate_to(app::View::SelectProject);
        }
        EditEnterAction::ActivityPicker { project_id } => {
            let _ = action_tx.send(Action::OpenEditActivityPicker { project_id });
        }
        EditEnterAction::NoteEditor { note } => {
            // Save running timer's full note (including any log tag) before overwriting
            // with the entry's note. On return, this will be restored to description_input
            // and navigate_to(EditDescription) will re-strip the tag if present.
            app.saved_timer_note = Some(app.full_note_value());
            // Load the entry's note, stripping any embedded log tag into description_log_id.
            // This prevents the running timer's log from leaking into the entry's Notes view.
            app.set_note_from_raw(&note);
            // Open description editor
            app.navigate_to(app::View::EditDescription);
        }
    }
}

/// Save changes from This Week edit mode to database
pub(super) async fn handle_this_week_edit_save(
    app: &mut App,
    client: &mut ApiClient,
) -> Result<()> {
    // Running timer edits don't touch the DB
    if app
        .this_week_edit_state
        .as_ref()
        .map(|s| s.registration_id.is_empty())
        == Some(true)
    {
        handle_running_timer_edit_save(app, client).await;
        return Ok(());
    }

    let Some(state) = app.this_week_edit_state.take() else {
        return Ok(());
    };
    app.exit_this_week_edit_mode();
    if let Err(e) = handle_saved_entry_edit_save(state, app, client).await {
        app.set_status(format!("Error saving entry: {}", e));
    }
    Ok(())
}

/// Apply edits from This Week edit mode back to the live running timer (no DB write).
/// Called when registration_id is empty (sentinel for the running timer).
async fn handle_running_timer_edit_save(app: &mut App, client: &mut ApiClient) {
    let Some(state) = app.this_week_edit_state.take() else {
        return;
    };

    // Parse start time input
    let start_parts: Vec<&str> = state.start_time_input.split(':').collect();
    if start_parts.len() != 2 {
        app.set_status("Error: Invalid time format".to_string());
        return;
    }
    let Ok(start_hours) = start_parts[0].parse::<u8>() else {
        app.set_status("Error: Invalid start hour".to_string());
        return;
    };
    let Ok(start_mins) = start_parts[1].parse::<u8>() else {
        app.set_status("Error: Invalid start minute".to_string());
        return;
    };

    // Build new absolute_start: today's local date + typed HH:MM, converted to UTC
    let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let today = time::OffsetDateTime::now_utc()
        .to_offset(local_offset)
        .date();
    let Ok(new_time) = time::Time::from_hms(start_hours, start_mins, 0) else {
        app.set_status("Error: Invalid start time".to_string());
        return;
    };
    let new_start = time::OffsetDateTime::new_in_offset(today, new_time, local_offset);

    // Reject if new start is in the future
    if new_start > time::OffsetDateTime::now_utc() {
        app.set_status("Error: Start time cannot be in the future".to_string());
        // Restore edit state so the user can correct it
        app.this_week_edit_state = Some(state);
        return;
    }

    // Write back to App fields
    app.absolute_start = Some(new_start.to_offset(time::UtcOffset::UTC));

    // Recalculate local_start so elapsed_duration() reflects the new start time
    let now_utc = time::OffsetDateTime::now_utc();
    let elapsed_secs = (now_utc - new_start.to_offset(time::UtcOffset::UTC))
        .whole_seconds()
        .max(0) as u64;
    app.local_start =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(elapsed_secs));

    app.selected_project = state
        .project_id
        .zip(state.project_name)
        .map(|(id, name)| types::Project { id, name });
    app.selected_activity = state
        .activity_id
        .zip(state.activity_name)
        .map(|(id, name)| types::Activity {
            id,
            name,
            project_id: String::new(),
        });
    app.set_note_from_raw(&state.note.value);

    app.set_status("Running timer updated".to_string());

    // Sync updated start time / project / activity / note to server
    let project_id = app.selected_project.as_ref().map(|p| p.id.clone());
    let project_name = app.selected_project.as_ref().map(|p| p.name.clone());
    let activity_id = app.selected_activity.as_ref().map(|a| a.id.clone());
    let activity_name = app.selected_activity.as_ref().map(|a| a.name.clone());
    let full_note = app.full_note_value();
    let note = if full_note.is_empty() {
        None
    } else {
        Some(full_note)
    };
    if let Err(e) = client
        .update_active_timer(
            project_id,
            project_name,
            activity_id,
            activity_name,
            note,
            app.absolute_start,
        )
        .await
    {
        app.set_status(format!("Warning: Could not sync timer to server: {}", e));
    }
}

/// Save changes from History edit mode to database
pub(super) async fn handle_history_edit_save(app: &mut App, client: &mut ApiClient) -> Result<()> {
    let Some(state) = app.history_edit_state.take() else {
        return Ok(());
    };
    app.exit_history_edit_mode();
    if let Err(e) = handle_saved_entry_edit_save(state, app, client).await {
        app.set_status(format!("Error saving entry: {}", e));
    }
    Ok(())
}

/// Shared helper: save edits to a completed (non-running) timer history entry via the API.
async fn handle_saved_entry_edit_save(
    state: app::EntryEditState,
    app: &mut App,
    client: &mut ApiClient,
) -> Result<()> {
    // Look up the original entry from history
    let entry = match app
        .time_entries
        .iter()
        .find(|e| e.registration_id == state.registration_id)
    {
        Some(e) => e.clone(),
        None => {
            app.set_status("Error: Entry not found in history".to_string());
            return Ok(());
        }
    };

    // registration_id is always present on TimeEntry
    let registration_id = entry.registration_id.clone();

    // Parse start / end times (HH:MM) on the entry's original local date
    let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);

    // Parse entry.date ("YYYY-MM-DD") to get the calendar date
    let entry_date = app::parse_date_str(&entry.date)
        .ok_or_else(|| anyhow::anyhow!("Unexpected date format: {}", entry.date))?;

    let parse_hhmm = |s: &str| -> Result<time::Time> {
        let parts: Vec<&str> = s.split(':').collect();
        anyhow::ensure!(parts.len() == 2, "Expected HH:MM format, got {:?}", s);
        let h: u8 = parts[0].parse().context("Invalid hour")?;
        let m: u8 = parts[1].parse().context("Invalid minute")?;
        time::Time::from_hms(h, m, 0).map_err(|e| anyhow::anyhow!("Invalid time: {}", e))
    };

    let start_local = time::OffsetDateTime::new_in_offset(
        entry_date,
        parse_hhmm(&state.start_time_input)?,
        local_offset,
    );
    let end_local = time::OffsetDateTime::new_in_offset(
        entry_date,
        parse_hhmm(&state.end_time_input)?,
        local_offset,
    );

    anyhow::ensure!(end_local > start_local, "End time must be after start time");

    let project_id = state.project_id.as_deref().unwrap_or("");
    let project_name = state.project_name.as_deref().unwrap_or("");
    let activity_id = state.activity_id.as_deref().unwrap_or("");
    let activity_name = state.activity_name.as_deref().unwrap_or("");
    let user_note = &state.note.value;

    // Optimistic: apply the edit to the in-memory entry immediately, roll back on error.
    // Keep a snapshot for rollback.
    let snapshot = entry.clone();
    let start_utc = start_local.to_offset(time::UtcOffset::UTC);
    let end_utc = end_local.to_offset(time::UtcOffset::UTC);
    let optimistic_hours = (end_utc - start_utc).as_seconds_f64() / 3600.0;

    if let Some(e) = app
        .time_entries
        .iter_mut()
        .find(|e| e.registration_id == registration_id)
    {
        e.project_id = project_id.to_string();
        e.project_name = project_name.to_string();
        e.activity_id = activity_id.to_string();
        e.activity_name = activity_name.to_string();
        e.note = if user_note.is_empty() {
            None
        } else {
            Some(user_note.to_string())
        };
        e.start_time = Some(start_utc);
        e.end_time = Some(end_utc);
        e.hours = optimistic_hours;
    }
    app.rebuild_history_list();

    if let Err(e) = client
        .edit_time_entry(
            &registration_id,
            project_id,
            project_name,
            activity_id,
            activity_name,
            start_utc,
            end_utc,
            user_note,
        )
        .await
    {
        // Roll back to the original entry
        if let Some(entry) = app
            .time_entries
            .iter_mut()
            .find(|e| e.registration_id == registration_id)
        {
            *entry = snapshot;
        }
        app.rebuild_history_list();
        return Err(e);
    }

    app.set_status("Entry updated".to_string());
    Ok(())
}

/// Open an existing log file for a history/today entry.
/// Takes a pre-extracted log ID (may be empty if the entry has no log tag).
/// Does NOT create a new log file and does NOT mutate running-timer state.
pub(super) async fn handle_open_entry_log_note(id: &str, app: &mut App) {
    use crate::log_notes;

    if id.is_empty() {
        app.set_status("No log linked to this entry".to_string());
        return;
    }

    let path = match log_notes::log_path(id) {
        Ok(p) => p,
        Err(e) => {
            app.set_status(format!("Log error: {}", e));
            return;
        }
    };

    if !path.exists() {
        app.set_status("Log file not found".to_string());
        return;
    }

    if let Err(e) = crate::editor::open_editor(&path).await {
        app.set_status(format!("Editor error: {}", e));
        return;
    }

    app.needs_full_redraw = true;
}

async fn handle_open_log_note(app: &mut App, client: &mut ApiClient) -> anyhow::Result<()> {
    use crate::log_notes;
    use time::OffsetDateTime;

    // The description editor strips the tag into `description_log_id` so
    // `description_input.value` is always the clean summary.
    let summary = app.description_input.value.clone();

    // Determine or generate the log ID.  Prefer the one already stored on App;
    // fall back to generating a new one (first time Ctrl+L is pressed).
    let id = app
        .description_log_id
        .clone()
        .unwrap_or_else(log_notes::generate_id);

    // Date string
    let today = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let date = format!(
        "{:04}-{:02}-{:02}",
        today.year(),
        today.month() as u8,
        today.day()
    );

    // Create log file if it doesn't exist yet
    let log_path = log_notes::create_log_file(&id, &date)?;

    // Open the editor (suspends TUI)
    crate::editor::open_editor(&log_path).await?;

    // Store the ID on App so subsequent Ctrl+L presses reuse it and the tag
    // survives further editing. Refresh the cache so the render path sees the
    // newly written file immediately.
    app.description_log_id = Some(id.clone());
    app.refresh_log_cache();

    // Build the full note value (summary + tag) to save/sync.
    let new_note = log_notes::append_tag(&summary, &id);

    // description_input stays as the clean summary (tag lives in description_log_id).
    app.description_is_default = false;

    // If in edit mode, also sync the edit state's note field so Enter saves it correctly.
    if app.is_in_edit_mode() {
        app.update_edit_state_note(new_note.clone());
    }

    // Signal the event loop to do a full terminal redraw after the editor exits
    app.needs_full_redraw = true;

    // If timer is running AND we are NOT in edit mode, sync the updated note to the server.
    // (In edit mode the note belongs to a history entry — it will be saved on Enter.)
    if app.timer_state == app::TimerState::Running && !app.is_in_edit_mode() {
        sync_running_timer_note(new_note, app, client).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiClient;
    use crate::app::{DeleteContext, DeleteOrigin, SaveAction, View};
    use crate::test_support::test_app;
    use crate::types::ActiveTimerState;
    use time::macros::datetime;

    fn start_eligible_timer(app: &mut App) {
        app.start_timer(false);
        app.absolute_start = Some(time::OffsetDateTime::now_utc() - time::Duration::seconds(90));
        app.local_start = Some(Instant::now() - Duration::from_secs(90));
    }

    #[test]
    fn restore_active_timer_populates_local_app_state() {
        let mut app = test_app();
        let timer = ActiveTimerState {
            start_time: datetime!(2026-03-06 09:15 UTC),
            project_id: Some("proj-1".to_string()),
            project_name: Some("Project One".to_string()),
            activity_id: Some("act-1".to_string()),
            activity_name: Some("Activity One".to_string()),
            note: "Investigate tests".to_string(),
            hours: 1,
            minutes: 2,
            seconds: 3,
        };

        restore_active_timer(&mut app, timer);

        assert_eq!(app.timer_state, app::TimerState::Running);
        assert_eq!(
            app.selected_project.as_ref().map(|p| p.id.as_str()),
            Some("proj-1")
        );
        assert_eq!(
            app.selected_activity.as_ref().map(|a| a.name.as_str()),
            Some("Activity One")
        );
        assert_eq!(app.description_input.value, "Investigate tests");
        assert!(!app.description_is_default);
        assert_eq!(app.absolute_start, Some(datetime!(2026-03-06 09:15 UTC)));
        assert!(app.local_start.is_some());
    }

    #[tokio::test]
    async fn handle_start_timer_starts_timer_in_dev_mode() {
        let mut app = test_app();
        let mut client = ApiClient::dev().expect("dev client");

        handle_start_timer(&mut app, &mut client)
            .await
            .expect("start timer should succeed");

        assert_eq!(app.timer_state, app::TimerState::Running);
        assert!(app.absolute_start.is_some());
        assert!(app.local_start.is_some());
        assert!(app.status_message.is_none());
    }

    #[test]
    fn under_minute_timer_does_not_create_a_guard_or_send_a_save() {
        let mut app = test_app();
        app.start_timer(false);
        app.description_input = app::TextInput::from_str("Keep this note");
        app.selected_save_action = SaveAction::SaveAndStop;
        let original_start = app.absolute_start;
        let path = std::env::temp_dir().join(format!("toki-too-short-{}", std::process::id()));

        let attempt = prepare_save_at(&mut app, &path);
        if let Some(attempt) = &attempt {
            crate::pending_save::clear(&path, &attempt.pending).unwrap();
        }
        assert!(attempt.is_none());
        assert!(!path.exists());
        assert_eq!(app.timer_state, app::TimerState::Running);
        assert_eq!(app.absolute_start, original_start);
        assert_eq!(app.description_input.value, "Keep this note");
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("at least one minute"));
    }

    #[test]
    fn minute_old_timer_can_create_a_pending_record() {
        let mut app = test_app();
        app.start_timer(false);
        app.absolute_start = Some(time::OffsetDateTime::now_utc() - time::Duration::seconds(60));
        app.local_start = Some(Instant::now() - Duration::from_secs(60));
        app.selected_save_action = SaveAction::SaveAndStop;
        let path = std::env::temp_dir().join(format!("toki-one-minute-{}", std::process::id()));

        let attempt = prepare_save_at(&mut app, &path).unwrap();
        assert_eq!(
            crate::pending_save::load(&path).unwrap(),
            Some(attempt.pending.clone())
        );
        crate::pending_save::clear(&path, &attempt.pending).unwrap();
    }

    #[test]
    fn save_refuses_before_network_if_the_recovery_record_cannot_be_created() {
        let mut app = test_app();
        start_eligible_timer(&mut app);
        app.selected_save_action = SaveAction::SaveAndStop;
        let original_start = app.absolute_start;
        // A directory is not a writable recovery file, on Unix or Windows.
        assert!(prepare_save_at(&mut app, &std::env::temp_dir()).is_none());
        assert_eq!(app.timer_state, app::TimerState::Running);
        assert_eq!(app.absolute_start, original_start);
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("recovery record unavailable"));
    }

    #[tokio::test]
    async fn pending_save_blocks_a_second_write_without_clearing_the_timer() {
        let mut app = test_app();
        let mut client = ApiClient::dev().unwrap();
        start_eligible_timer(&mut app);
        app.description_input = app::TextInput::from_str("Keep this note");
        app.selected_save_action = SaveAction::SaveAndStop;
        let original_start = app.absolute_start.unwrap();
        let path = std::env::temp_dir().join(format!("toki-pending-retry-{}", std::process::id()));
        let pending = crate::pending_save::PendingSave {
            user_id: app.user_id,
            timer_started_at: original_start,
            attempted_at: time::OffsetDateTime::now_utc(),
            mode: crate::pending_save::SaveMode::Stop,
        };
        crate::pending_save::begin(&path, &pending).unwrap();

        handle_save_timer_with_action_at(&mut app, &mut client, &path)
            .await
            .unwrap();

        assert_eq!(app.timer_state, app::TimerState::Running);
        assert_eq!(app.absolute_start, Some(original_start));
        assert_eq!(app.description_input.value, "Keep this note");
        assert_eq!(
            crate::pending_save::load(&path).unwrap(),
            Some(pending.clone())
        );
        crate::pending_save::clear(&path, &pending).unwrap();
    }

    #[tokio::test]
    async fn pending_record_exists_before_the_save_request_and_clears_on_confirmation() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = ApiClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "test",
        )
        .unwrap();
        let mut app = test_app();
        start_eligible_timer(&mut app);
        app.selected_save_action = SaveAction::SaveAndStop;
        let path =
            std::env::temp_dir().join(format!("toki-pending-before-write-{}", std::process::id()));
        let saved_path = path.clone();
        let save_task = tokio::spawn(async move {
            handle_save_timer_with_action_at(&mut app, &mut client, &saved_path)
                .await
                .unwrap();
            app
        });

        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut request = [0; 2048];
        let read = socket.read(&mut request).await.unwrap();
        assert!(String::from_utf8_lossy(&request[..read]).contains("PUT /time-tracking/timer"));
        assert!(crate::pending_save::load(&path).unwrap().is_some());
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{}")
            .await
            .unwrap();
        drop(socket);
        let (mut history, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        history
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n[]")
            .await
            .unwrap();

        let app = save_task.await.unwrap();
        assert_eq!(app.timer_state, app::TimerState::Stopped);
        assert_eq!(crate::pending_save::load(&path).unwrap(), None);
    }

    #[tokio::test]
    async fn failed_save_preserves_the_timer_and_unresolved_record() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = ApiClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "test",
        )
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let received = socket.read(&mut request).await.unwrap();
            assert!(received > 0);
            socket
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let mut app = test_app();
        start_eligible_timer(&mut app);
        app.description_input = app::TextInput::from_str("Keep this note");
        app.selected_save_action = SaveAction::ContinueNewProject;
        let original_start = app.absolute_start;
        let path =
            std::env::temp_dir().join(format!("toki-pending-failure-{}", std::process::id()));

        handle_save_timer_with_action_at(&mut app, &mut client, &path)
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(app.timer_state, app::TimerState::Running);
        assert_eq!(app.absolute_start, original_start);
        assert_eq!(app.description_input.value, "Keep this note");
        let pending = crate::pending_save::load(&path).unwrap().unwrap();
        assert_eq!(pending.mode, crate::pending_save::SaveMode::ContinueNew);
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("Keep this note"));
        handle_save_timer_with_action_at(&mut app, &mut client, &path)
            .await
            .unwrap();
        assert_eq!(
            crate::pending_save::load(&path).unwrap(),
            Some(pending.clone())
        );
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("previous attempt"));
        crate::pending_save::clear(&path, &pending).unwrap();
    }

    #[test]
    fn confirmed_save_and_restart_resumes_only_after_both_responses() {
        let mut app = test_app();
        start_eligible_timer(&mut app);
        app.selected_save_action = SaveAction::ContinueSameProject;
        app.description_input = app::TextInput::from_str("Finished work");
        let path = std::env::temp_dir().join(format!("toki-restart-ok-{}", std::process::id()));
        let attempt = prepare_save_at(&mut app, &path).unwrap();

        assert!(finish_save(
            &mut app,
            attempt,
            SaveOutcome::Confirmed {
                restart: Some(Ok(()))
            }
        ));
        assert_eq!(app.timer_state, app::TimerState::Running);
        assert_eq!(crate::pending_save::load(&path).unwrap(), None);
        assert_eq!(app.description_input.value, "");
        assert!(app.status_message.as_deref().unwrap().starts_with("Saved "));
    }

    #[tokio::test]
    async fn confirmed_save_with_failed_restart_does_not_fabricate_a_running_timer() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = ApiClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "test",
        )
        .unwrap();
        let server = tokio::spawn(async move {
            for response in [
                b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
                b"HTTP/1.1 500 Internal Server Error\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".as_slice(),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                let received = socket.read(&mut request).await.unwrap();
                assert!(received > 0);
                socket.write_all(response).await.unwrap();
            }
        });
        let mut app = test_app();
        start_eligible_timer(&mut app);
        app.selected_save_action = SaveAction::ContinueSameProject;
        let path = std::env::temp_dir().join(format!("toki-restart-error-{}", std::process::id()));
        let attempt = prepare_save_at(&mut app, &path).unwrap();
        let outcome = perform_save(&mut client, &attempt).await;
        assert!(matches!(
            outcome,
            SaveOutcome::Confirmed {
                restart: Some(Err(_))
            }
        ));
        assert!(finish_save(&mut app, attempt, outcome));
        server.await.unwrap();
        assert_eq!(app.timer_state, app::TimerState::Stopped);
        let pending = crate::pending_save::load(&path).unwrap().unwrap();
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("restart outcome unknown"));
        assert!(prepare_save_at(&mut app, &path).is_none());
        crate::pending_save::clear(&path, &pending).unwrap();
    }

    #[tokio::test]
    async fn committed_restart_with_lost_response_keeps_guard() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = ApiClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "test",
        )
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut save, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let read = save.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).contains("PUT /time-tracking/timer"));
            save.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{}")
                .await
                .unwrap();
            drop(save);
            let (mut restart, _) = listener.accept().await.unwrap();
            let read = restart.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).contains("POST /time-tracking/timer"));
            // The new server timer was created, but its response was lost.
        });
        let mut app = test_app();
        start_eligible_timer(&mut app);
        app.selected_save_action = SaveAction::ContinueSameProject;
        let path = std::env::temp_dir().join(format!("toki-restart-lost-{}", std::process::id()));
        let attempt = prepare_save_at(&mut app, &path).unwrap();
        let outcome = perform_save(&mut client, &attempt).await;
        assert!(matches!(
            outcome,
            SaveOutcome::Confirmed {
                restart: Some(Err(_))
            }
        ));
        assert!(finish_save(&mut app, attempt, outcome));
        server.await.unwrap();
        assert_eq!(app.timer_state, app::TimerState::Stopped);
        let pending = crate::pending_save::load(&path).unwrap().unwrap();
        assert_eq!(pending.mode, crate::pending_save::SaveMode::ContinueSame);
        assert!(app
            .status_message
            .as_deref()
            .unwrap()
            .contains("restart outcome unknown"));
        assert!(prepare_save_at(&mut app, &path).is_none());
        crate::pending_save::clear(&path, &pending).unwrap();
    }

    #[tokio::test]
    async fn committed_save_with_lost_response_remains_unresolved() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = ApiClient::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "test",
        )
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let received = socket.read(&mut request).await.unwrap();
            assert!(received > 0);
            // The provider may have committed; the response never reaches the client.
        });
        let mut app = test_app();
        start_eligible_timer(&mut app);
        app.selected_save_action = SaveAction::SaveAndStop;
        app.description_input = app::TextInput::from_str("Keep this note");
        let original_start = app.absolute_start;
        let path = std::env::temp_dir().join(format!("toki-lost-response-{}", std::process::id()));
        let attempt = prepare_save_at(&mut app, &path).unwrap();
        let outcome = perform_save(&mut client, &attempt).await;
        assert!(matches!(outcome, SaveOutcome::Uncertain(_)));
        assert!(!finish_save(&mut app, attempt, outcome));
        server.await.unwrap();
        assert_eq!(app.timer_state, app::TimerState::Running);
        assert_eq!(app.absolute_start, original_start);
        assert_eq!(app.description_input.value, "Keep this note");
        let pending = crate::pending_save::load(&path).unwrap().unwrap();
        assert!(prepare_save_at(&mut app, &path).is_none());
        crate::pending_save::clear(&path, &pending).unwrap();
    }

    #[tokio::test]
    async fn handle_save_timer_cancel_returns_to_timer_without_saving() {
        let mut app = test_app();
        let mut client = ApiClient::dev().expect("dev client");
        app.current_view = View::SaveAction;
        app.selected_save_action = SaveAction::Cancel;
        app.timer_state = app::TimerState::Running;

        let path = std::env::temp_dir().join(format!("toki-cancel-save-{}", std::process::id()));
        handle_save_timer_with_action_at(&mut app, &mut client, &path)
            .await
            .expect("cancel should succeed");

        assert_eq!(app.current_view, View::Timer);
        assert_eq!(app.timer_state, app::TimerState::Running);
    }

    #[tokio::test]
    async fn open_entry_log_note_no_log_linked_sets_status() {
        let mut app = test_app();
        // Empty id → sets "No log linked to this entry"
        handle_open_entry_log_note("", &mut app).await;
        assert!(app
            .status_message
            .as_deref()
            .unwrap_or("")
            .contains("No log"));
    }

    #[tokio::test]
    async fn open_entry_log_note_invalid_id_sets_status() {
        let mut app = test_app();
        // Invalid id (non-hex) → log_path returns Err → sets "Log error: ..."
        handle_open_entry_log_note("ZZZZZZ", &mut app).await;
        assert!(app
            .status_message
            .as_deref()
            .unwrap_or("")
            .contains("Log error"));
    }

    #[tokio::test]
    async fn open_entry_log_note_missing_file_sets_status() {
        let mut app = test_app();
        // Valid hex id but file doesn't exist → sets "Log file not found"
        handle_open_entry_log_note("abcdef", &mut app).await;
        assert_eq!(app.status_message.as_deref(), Some("Log file not found"));
    }

    #[tokio::test]
    async fn handle_confirm_delete_removes_entry_and_returns_to_origin_view() {
        let mut app = test_app();
        let mut client = ApiClient::dev().expect("dev client");
        let today = time::OffsetDateTime::now_utc().date();
        let entries = client
            .get_time_entries(today, today)
            .await
            .expect("history should load");
        let entry = entries.first().expect("seeded history entry").clone();

        app.update_history(entries);
        app.rebuild_history_list();
        app.current_view = View::ConfirmDelete;
        app.delete_context = Some(DeleteContext {
            registration_id: entry.registration_id.clone(),
            display_label: format!("{} / {}", entry.project_name, entry.activity_name),
            display_date: entry.date.clone(),
            display_hours: entry.hours,
            origin: DeleteOrigin::History,
        });

        handle_confirm_delete(&mut app, &mut client).await;

        assert_eq!(app.current_view, View::History);
        assert_eq!(app.status_message.as_deref(), Some("Entry deleted"));
        assert!(app
            .time_entries
            .iter()
            .all(|item| item.registration_id != entry.registration_id));
        assert!(app.delete_context.is_none());
    }
}
