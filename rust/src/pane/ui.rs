//! The overlay rendering: a microVM table, details, help and confirm popups.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row as TableRow, Table};
use ratatui::Frame;

use crate::pane::app::App;
use crate::pane::model::status_text;

/// Renders the whole overlay.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " kata.microvm — microVMs ",
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        )])),
        chunks[0],
    );

    render_table(frame, app, chunks[1]);

    let status = app.status.as_deref().unwrap_or("");
    frame.render_widget(Paragraph::new(status), chunks[2]);

    frame.render_widget(
        Paragraph::new(
            "q quit · j/k move · enter details · g jump · v verify · x stop · p prune · a clean all · r refresh · ? help",
        ),
        chunks[3],
    );

    if app.help {
        render_popup(frame, area, "Keys", &help_lines(), 52, 16);
    } else if let Some(pending) = &app.confirm {
        render_popup(
            frame,
            area,
            "Confirm",
            &[format!("{}  [y]es / [n]o", pending.prompt)],
            60,
            5,
        );
    } else if app.detail {
        if let Some(row) = app.selected_row() {
            let mapping = &row.mapping;
            render_popup(
                frame,
                area,
                "Details",
                &[
                    format!("pane:     {}", mapping.pane_id),
                    format!("microVM:  {}", mapping.microvm_id),
                    format!("agent:    {}", mapping.agent),
                    format!("profile:  {}", mapping.profile),
                    format!("worktree: {}", mapping.worktree),
                    format!("state:    {}", status_text(row)),
                    format!(
                        "runtime:   {}",
                        row.runtime.as_deref().unwrap_or("not running")
                    ),
                ],
                56,
                11,
            );
        }
    }
}

fn render_table(frame: &mut Frame, app: &App, area: Rect) {
    let header = TableRow::new(vec!["state", "pane", "microVM", "agent", "runtime"])
        .style(Style::new().add_modifier(Modifier::BOLD));
    let widths = [
        Constraint::Length(6),
        Constraint::Length(14),
        Constraint::Length(26),
        Constraint::Length(8),
        Constraint::Min(10),
    ];
    if app.rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No microVMs tracked. Start an agent with the start-agent action.")
                .block(Block::default().borders(Borders::ALL).title(" microVMs ")),
            area,
        );
        return;
    }
    let body: Vec<TableRow> = app
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let style = if index == app.selected {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new()
            };
            TableRow::new(vec![
                Cell::from(status_text(row)),
                Cell::from(row.mapping.pane_id.as_str()),
                Cell::from(row.mapping.microvm_id.as_str()),
                Cell::from(row.mapping.agent.as_str()),
                Cell::from(row.runtime.as_deref().unwrap_or("-")),
            ])
            .style(style)
        })
        .collect();
    let table = Table::new(body, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" microVMs "));
    frame.render_widget(table, area);
}

fn help_lines() -> Vec<String> {
    vec![
        "j/k, ↑/↓   move the selection".to_string(),
        "enter       toggle the details panel".to_string(),
        "g           jump to the selected pane".to_string(),
        "v           verify the selected microVM".to_string(),
        "x           stop the selected microVM (asks first)".to_string(),
        "p           prune mappings whose pane is gone".to_string(),
        "a           stop everything and forget all (asks first)".to_string(),
        "r           refresh the list".to_string(),
        "?           this help".to_string(),
        "q, ctrl-c   quit".to_string(),
    ]
}

/// A centered popup; content lines are clipped to the box.
fn render_popup(frame: &mut Frame, area: Rect, title: &str, lines: &[String], w: u16, h: u16) {
    let popup = centered(area, w.min(area.width), h.min(area.height));
    frame.render_widget(Clear, popup);
    let text: Vec<Line> = lines.iter().map(|line| Line::from(line.as_str())).collect();
    frame.render_widget(
        Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} ")),
        ),
        popup,
    );
}

/// A centered rectangle of the given size.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    Rect::new(x, y, w.min(area.width), h.min(area.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::app::{App, Key};
    use crate::pane::model::Row;
    use crate::state::Mapping;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn row(pane_id: &str, live: bool) -> Row {
        Row {
            mapping: Mapping {
                pane_id: pane_id.to_string(),
                microvm_id: format!("kata-{pane_id}-abc123"),
                agent: "codex".to_string(),
                profile: "prof".to_string(),
                worktree: "/w".to_string(),
                created_at_secs: 0,
            },
            live,
            runtime: live.then(|| "io.containerd.kata.v2".to_string()),
        }
    }

    fn screen(app: &App) -> String {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| render(frame, app))
            .expect("draw works");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn the_table_lists_every_tracked_microvm() {
        let app = App::new(vec![row("pane1", true), row("pane2", false)]);
        let text = screen(&app);
        assert!(text.contains("pane1"), "shows the first pane");
        assert!(
            text.contains("kata-pane2-abc123"),
            "shows the second microVM"
        );
        assert!(text.contains("up"), "shows the live state");
        assert!(text.contains("down"), "shows the stopped state");
        assert!(text.contains("microVMs"), "has a title");
    }

    #[test]
    fn an_empty_list_says_so() {
        let app = App::new(vec![]);
        let text = screen(&app);
        assert!(text.contains("No microVMs tracked"));
    }

    #[test]
    fn the_details_panel_shows_the_mapping() {
        let mut app = App::new(vec![row("pane1", true)]);
        app.detail = true;
        let text = screen(&app);
        assert!(text.contains("Details"));
        assert!(text.contains("codex"));
        assert!(text.contains("/w"));
        assert!(text.contains("io.containerd.kata.v2"));
    }

    #[test]
    fn the_help_panel_lists_the_keys() {
        let mut app = App::new(vec![]);
        app.help = true;
        let text = screen(&app);
        assert!(text.contains("Keys"));
        assert!(text.contains("jump to the selected pane"));
    }

    #[test]
    fn the_confirm_panel_asks_yes_or_no() {
        use crate::herdr::HerdrClient;
        use crate::kata::CtrBackend;
        use crate::pane::app::Ops;
        // The 'x' key on a live row only builds the question; the backend is
        // never touched here.
        let backend = CtrBackend {
            bin: "/definitely/not/ctr".to_string(),
            socket: "/nonexistent.sock".to_string(),
            namespace: "herdr-kata".to_string(),
            runtime: "io.containerd.kata.v2".to_string(),
        };
        let herdr = HerdrClient::new("/definitely/not/herdr".to_string());
        let dir = std::path::PathBuf::from("/definitely/not/a/dir");
        let mut ops = Ops {
            backend: &backend,
            herdr: &herdr,
            state_dir: &dir,
            plugin_root: &dir,
        };
        let mut app = App::new(vec![row("pane1", true)]);
        app.on_key(Key::Char('x'), &mut ops);
        let text = screen(&app);
        assert!(text.contains("Confirm"));
        assert!(text.contains("[y]es / [n]o"));
    }
}
