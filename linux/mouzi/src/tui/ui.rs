use super::app::{Mode, Model, Screen};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
use ratatui::Frame;

pub const MIN: (u16, u16) = (80, 24);
const NAMES: [&str; 4] = ["1 Status", "2 Folders", "3 Review", "4 Rules"];

pub fn view(m: &Model, f: &mut Frame) {
    let area = f.area();
    if area.width < MIN.0 || area.height < MIN.1 {
        f.render_widget(
            Paragraph::new(format!("terminal too small ({}x{} needed)", MIN.0, MIN.1)),
            area,
        );
        return;
    }
    let [tabs, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(area);
    f.render_widget(
        Tabs::new(NAMES)
            .select(m.idx())
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED)),
        tabs,
    );
    match m.screen {
        Screen::Status => status(m, f, body),
        Screen::Folders => folders(m, f, body),
        Screen::Review => review(m, f, body),
        Screen::Rules => rules(m, f, body),
    }
    f.render_widget(
        Paragraph::new(vec![
            Line::from(m.message.clone()),
            Line::from(hint(m)).style(Style::new().fg(Color::DarkGray)),
        ]),
        foot,
    );
}

fn hint(m: &Model) -> String {
    match (&m.mode, m.screen) {
        (Mode::AddFolder(buf), _) => format!("add folder path: {buf}█   (Enter add, Esc cancel)"),
        (Mode::ConfirmUndoAll, _) => "y confirm, any other key cancels".into(),
        (_, Screen::Status) => "↑↓ select  u undo  A undo all  Tab switch  q quit".into(),
        (_, Screen::Folders) => "a add  d remove  m mode  n only-new  q quit".into(),
        (_, Screen::Review) => "r scan  space toggle  a all  Enter apply  x discard  q quit".into(),
        (_, Screen::Rules) => "K/J move up/down  e edit as JSON in $EDITOR  q quit".into(),
    }
}

fn list(f: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>, sel: usize) {
    let mut state = ListState::default().with_selected(Some(sel));
    let l = List::new(items)
        .block(Block::new().borders(Borders::ALL).title(title.to_string()))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(l, area, &mut state);
}

fn status(m: &Model, f: &mut Frame, area: Rect) {
    let [head, rest] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
    let (text, color) = match m.daemon_active {
        Some(true) => ("daemon: running".to_string(), Color::Green),
        Some(false) => (
            "daemon: stopped (systemctl --user enable --now mouzi)".to_string(),
            Color::Yellow,
        ),
        None => ("daemon: unknown".to_string(), Color::DarkGray),
    };
    f.render_widget(
        Paragraph::new(text)
            .style(Style::new().fg(color))
            .block(Block::new().borders(Borders::ALL).title("Mouzi")),
        head,
    );
    let items = m
        .logs
        .iter()
        .map(|l| {
            let dest = l.destination_path.clone().unwrap_or_default();
            let style = if l.undone {
                Style::new().fg(Color::DarkGray)
            } else {
                Style::new()
            };
            ListItem::new(format!(
                "{}  {:<6} {}  [{}] → {}{}",
                l.timestamp
                    .with_timezone(&chrono::Local)
                    .format("%m-%d %H:%M"),
                l.action,
                l.file_name,
                l.file_type,
                dest,
                if l.undone { "  (undone)" } else { "" }
            ))
            .style(style)
        })
        .collect();
    list(f, rest, "History", items, m.selected());
}

fn folders(m: &Model, f: &mut Frame, area: Rect) {
    let items = m
        .folders
        .iter()
        .map(|d| {
            ListItem::new(format!(
                "{:<8} {}{}",
                d.mode,
                d.path,
                if d.only_new {
                    "   (only new files)"
                } else {
                    ""
                }
            ))
        })
        .collect();
    list(
        f,
        area,
        "Watched folders (silent = auto, manual = review only, paused = off)",
        items,
        m.selected(),
    );
}

fn review(m: &Model, f: &mut Frame, area: Rect) {
    let Some(r) = &m.review else {
        f.render_widget(
            Paragraph::new("Press r to scan watched folders for files to organize.")
                .block(Block::new().borders(Borders::ALL).title("Review")),
            area,
        );
        return;
    };
    let items = r
        .entries
        .iter()
        .zip(&r.checked)
        .map(|(e, c)| {
            let flag = match (&e.error, e.warning) {
                (Some(err), _) => format!("  ✗ {err}"),
                (None, true) => "  ! suspicious name".into(),
                _ => String::new(),
            };
            let line = Line::from(vec![
                Span::raw(format!(
                    "[{}] {:<10} {}",
                    if *c { "x" } else { " " },
                    e.rule,
                    e.source
                )),
                Span::raw(format!(
                    " → {}",
                    e.destination.clone().unwrap_or_else(|| e.action.clone())
                )),
                Span::styled(flag, Style::new().fg(Color::Yellow)),
            ]);
            ListItem::new(line)
        })
        .collect();
    list(f, area, "Review", items, m.selected());
}

fn rules(m: &Model, f: &mut Frame, area: Rect) {
    let items = m
        .rules
        .iter()
        .map(|r| {
            ListItem::new(format!(
                "{:>3} {}{:<16} {:<28} {} → {}",
                r.priority,
                if r.enabled { " " } else { "✗" },
                r.name,
                r.extensions.join(","),
                r.action,
                r.destination
            ))
        })
        .collect();
    list(f, area, "Rules (first match wins)", items, m.selected());
}

#[cfg(test)]
mod tests {
    use super::*;
    use mouzi_core::operations::{Preview, PreviewEntry};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(m: &Model, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| view(m, f)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn small_terminal_shows_notice() {
        assert!(render(&Model::new(), 40, 10).contains("terminal too small"));
    }

    #[test]
    fn status_shows_daemon_state_and_tabs() {
        let mut m = Model::new();
        m.daemon_active = Some(false);
        let out = render(&m, 100, 30);
        assert!(out.contains("1 Status") && out.contains("4 Rules"));
        assert!(out.contains("daemon: stopped"));
    }

    #[test]
    fn review_lists_entries_with_flags() {
        let mut m = Model::new();
        m.screen = Screen::Review;
        let e = |id: &str, err: Option<&str>, w| PreviewEntry {
            id: id.into(),
            source: format!("/in/{id}.pdf"),
            destination: Some(format!("/out/{id}.pdf")),
            rule: "Docs".into(),
            action: "move".into(),
            size: 1,
            warning: w,
            error: err.map(Into::into),
        };
        super::super::app::update(
            &mut m,
            super::super::app::Msg::Previewed(Ok(Preview {
                id: "p".into(),
                entries: vec![
                    e("a", None, false),
                    e("b", None, true),
                    e("c", Some("preview.locked"), false),
                ],
            })),
        );
        let out = render(&m, 110, 30);
        assert!(
            out.contains("[x] Docs")
                && out.contains("suspicious name")
                && out.contains("preview.locked")
        );
    }
}
