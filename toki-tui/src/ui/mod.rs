use crate::app::{App, SaveAction, View};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

mod delete_dialog;
mod description_editor;
mod history_panel;
mod history_view;
pub(crate) mod recovery_overlay;
mod save_dialog;
mod selection_views;
mod statistics_view;
mod template_selection_view;
mod timer_view;
pub(super) mod utils;
pub(super) mod widgets;
mod zen_view;

pub fn render_with_recovery(
    frame: &mut Frame,
    app: &mut App,
    recovery: Option<&recovery_overlay::RecoveryOverlay>,
) {
    render(frame, app);
    if let Some(recovery) = recovery {
        recovery_overlay::render(frame, recovery);
    }
}

pub fn render(frame: &mut Frame, app: &mut App) {
    // Zen mode: full-screen, no stats bar, no other UI
    if app.current_view == View::Timer && app.zen_mode {
        zen_view::render_zen_view(frame, app);
        return;
    }

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(frame.area());

    timer_view::render_compact_stats(frame, root[0], app);

    let body = root[1];
    match app.current_view {
        View::Timer => timer_view::render_timer_view(frame, app, body),
        View::History => history_view::render_history_view(frame, app, body),
        View::SelectProject => selection_views::render_project_selection(frame, app, body),
        View::SelectActivity => selection_views::render_activity_selection(frame, app, body),
        View::SelectTemplate => {
            template_selection_view::render_template_selection(frame, app, body)
        }
        View::EditDescription => {
            if app.aven_overlay.is_some() {
                description_editor::render_aven_overlay(frame, app, body);
            } else if app.taskwarrior_overlay.is_some() {
                description_editor::render_taskwarrior_overlay(frame, app, body);
            } else {
                description_editor::render_description_editor(frame, app, body);
            }
        }
        View::SaveAction => save_dialog::render_save_action_dialog(frame, app, body),
        View::Statistics => statistics_view::render_statistics_view(frame, app, body),
        View::ConfirmDelete => delete_dialog::render_delete_confirm_dialog(frame, app, body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_app;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_lines(app: &mut App) -> Vec<String> {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| render(frame, app))
            .expect("render should succeed");

        let backend = terminal.backend();
        let buffer = backend.buffer();

        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn rendered_text(app: &mut App) -> String {
        render_lines(app).join("\n")
    }

    #[test]
    fn header_displays_muted_binary_version() {
        let mut app = test_app();
        let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, 1)].symbol())
            .collect();
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        let x = line.find(&version).expect("version beside title") as u16;
        assert!(line[..x as usize].ends_with("Toki Timer TUI "));
        assert_eq!(buffer[(x, 1)].fg, Color::DarkGray);
    }

    #[test]
    fn note_editor_shows_only_configured_task_shortcut() {
        for (manager, shows_aven, shows_taskwarrior) in [
            (crate::config::TaskManager::None, false, false),
            (crate::config::TaskManager::Aven, true, false),
            (crate::config::TaskManager::Taskwarrior, false, true),
        ] {
            let mut app = test_app();
            app.task_manager = manager;
            app.navigate_to(View::EditDescription);
            let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = (0..buffer.area.height)
                .flat_map(|y| {
                    (0..buffer.area.width).map(move |x| buffer[(x, y)].symbol().to_string())
                })
                .collect();
            assert_eq!(text.contains("Ctrl+A"), shows_aven);
            assert_eq!(text.contains("Ctrl+T"), shows_taskwarrior);
        }
    }

    #[test]
    fn aven_picker_renders_titles_and_loading_without_task_metadata() {
        let mut app = test_app();
        app.navigate_to(View::EditDescription);
        let (request, _) = app.open_aven_overlay();
        assert!(rendered_text(&mut app).contains("Loading Aven tasks"));
        app.finish_aven_load(
            request,
            Ok(vec![crate::aven::AvenTask {
                title: "Fixture title".to_string(),
                reference: Some("CHL-X30D".to_string()),
            }]),
        );
        let text = rendered_text(&mut app);
        assert!(text.contains("Aven Tasks"));
        assert!(text.contains("[CHL-X30D] Fixture title"));
    }

    #[test]
    fn render_status_shows_success_copy() {
        let mut app = test_app();
        app.status_message = Some("Saved 00:15:00 to Project / Activity".to_string());

        let text = rendered_text(&mut app);

        assert!(text.contains("Saved 00:15:00 to Project / Activity"));
    }
}
