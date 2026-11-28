use crate::app::{App, StatusKind};
use json_tree_editor::{EditKind, JsonTree, Theme};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};

pub const HELP_EDIT: &str = "Enter confirm · Shift+Enter newline · Esc cancel · ←/→/Home/End cursor · Ctrl+S confirm & save";
pub const HELP_NORMAL: &str = "↑/↓/j/k move · Space/←/→ fold · Enter/e edit · r rename · a add · o/O sibling · d del · Ctrl+S save · Ctrl+R reload · q quit";

pub fn draw(app: &mut App, frame: &mut Frame<'_>) {
    let status_lines = match app.status.kind {
        StatusKind::Error => 5,
        StatusKind::Warn => 4,
        _ => 3,
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(status_lines),
            Constraint::Length(2),
        ])
        .split(frame.area());

    let mut header = vec![
        Span::styled(
            " nixa-json ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  {} ", app.tree)),
    ];
    if app.editor.is_dirty() {
        header.push(Span::styled(
            "[unsaved] ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        header.push(Span::styled("[saved] ", Style::default().fg(Color::Green)));
    }
    if let Some(kind) = app.editor.edit_kind() {
        let label = match kind {
            EditKind::Key => "[EDIT KEY]",
            EditKind::Value => "[EDIT VALUE]",
        };
        header.push(Span::styled(
            label,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(header)), chunks[0]);

    let tree = JsonTree::new()
        .theme(Theme::default())
        .block(Block::bordered().title(format!(" {}.json (saves to nixa-brain) ", app.tree)));
    frame.render_stateful_widget(tree, chunks[1], &mut app.editor);

    let status_style = match app.status.kind {
        StatusKind::Info => Style::default().fg(Color::Gray),
        StatusKind::Ok => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        StatusKind::Warn => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        StatusKind::Error => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    };
    frame.render_widget(
        Paragraph::new(app.status.text.as_str())
            .style(status_style)
            .wrap(Wrap { trim: true })
            .block(Block::bordered()),
        chunks[2],
    );

    let help = if app.editor.is_editing() {
        HELP_EDIT
    } else {
        HELP_NORMAL
    };
    frame.render_widget(
        Paragraph::new(help)
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: true }),
        chunks[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Status, new_editor};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    fn screen(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
        terminal.draw(|frame| draw(app, frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let width = buffer.area.width as usize;
        let cells: Vec<&str> = buffer.content.iter().map(|cell| cell.symbol()).collect();
        cells
            .chunks(width)
            .map(|row| row.concat())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn draws_header_tree_status_and_help() {
        let mut app = App {
            tree: "watch-agents".to_string(),
            addr: "127.0.0.1:1".to_string(),
            data_dir: std::env::temp_dir().join("mj-no-token-here"),
            editor: new_editor(
                json!({"version": 1, "title": "Watch", "boxes": {"r1": {"text": "Start"}}}),
            ),
            status: Status {
                kind: StatusKind::Info,
                text: "Loaded watch-agents.".to_string(),
            },
            confirm_quit: false,
            confirm_reload: false,
            should_quit: false,
        };
        let text = screen(&mut app);
        assert!(text.contains(" nixa-json   watch-agents [saved]"), "{text}");
        assert!(
            text.contains("watch-agents.json (saves to nixa-brain)"),
            "{text}"
        );
        assert!(text.contains("r1"), "{text}");
        assert!(text.contains("Loaded watch-agents."), "{text}");
        assert!(text.contains("Ctrl+R reload · q quit"), "{text}");
        let idx = app
            .editor
            .rows()
            .iter()
            .position(|row| row.key() == Some("title"))
            .unwrap();
        app.editor.select(idx);
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        let text = screen(&mut app);
        assert!(text.contains("[EDIT VALUE]"), "{text}");
        assert!(text.contains("Shift+Enter newline"), "{text}");
    }
}
