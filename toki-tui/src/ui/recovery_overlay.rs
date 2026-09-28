use crate::recovery::RecoverySnapshot;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub enum RecoveryOverlay {
    Checking,
    Review {
        snapshot: RecoverySnapshot,
        scroll: u16,
        confirm: bool,
    },
    Error(String),
    Cleared,
}

impl RecoveryOverlay {
    pub fn scroll(&mut self, down: bool) {
        if let Self::Review { scroll, .. } = self {
            *scroll = if down {
                scroll.saturating_add(1)
            } else {
                scroll.saturating_sub(1)
            };
        }
    }
}

pub fn render(frame: &mut Frame, overlay: &RecoveryOverlay) {
    let root = frame.area();
    let area = Rect::new(
        root.x + root.width.saturating_sub(100) / 2,
        root.y + root.height.saturating_sub(32) / 2,
        root.width.min(100),
        root.height.min(32),
    );
    frame.render_widget(Clear, area);
    let (body, scroll, controls) = match overlay {
        RecoveryOverlay::Checking => (
            "Checking this account's server timer and recent entries...\nNo write or retry will be sent.".to_string(),
            0,
            "Esc: close  Q: quit",
        ),
        RecoveryOverlay::Review {
            snapshot,
            scroll,
            confirm,
        } => {
            let controls = if *confirm {
                "After web verification: Y clears LOCAL guard  N: cancel  Q: quit"
            } else {
                "Check web app  C: request local clear  Up/Down: scroll  Esc: close  Q: quit"
            };
            (snapshot.lines.join("\n"), *scroll, controls)
        }
        RecoveryOverlay::Error(message) => (
            format!("Could not inspect server state: {message}\nThe guard is unchanged."),
            0,
            "Esc: close  Q: quit",
        ),
        RecoveryOverlay::Cleared => (
            "Only the local recovery guard was cleared. No entry was created, retried or deleted.\nQuit and relaunch the TUI to reload the current server timer.".to_string(),
            0,
            "Q: quit",
        ),
    };
    let block = Block::default()
        .title(" Save recovery — server is authoritative ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(body)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(controls)
            .style(Style::default().fg(Color::Yellow))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending_save::{PendingSave, SaveMode};
    use ratatui::{backend::TestBackend, Terminal};
    use time::OffsetDateTime;

    #[test]
    fn confirmation_controls_remain_visible_even_when_history_exceeds_screen() {
        let snapshot = RecoverySnapshot {
            pending: PendingSave {
                user_id: 1,
                timer_started_at: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
                attempted_at: OffsetDateTime::from_unix_timestamp(1_700_000_060).unwrap(),
                mode: SaveMode::Stop,
            },
            lines: (0..30).map(|i| format!("history entry {i}")).collect(),
        };
        let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    &RecoveryOverlay::Review {
                        snapshot,
                        scroll: 0,
                        confirm: true,
                    },
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("history entry 0"));
        assert!(text.contains("After web verification: Y clears LOCAL guard"));
    }
}
