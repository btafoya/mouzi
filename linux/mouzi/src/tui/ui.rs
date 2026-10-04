use super::app::{Mode, Model, Purpose, Screen};
use super::form::{shadowed, Form, Kind};
use super::picker::Want;
use mouzi_core::ignore::IgnoreLine;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};
use ratatui::Frame;

pub const MIN: (u16, u16) = (80, 24);
const NAMES: [&str; 5] = ["1 Status", "2 Folders", "3 Review", "4 Rules", "5 Settings"];

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
    match (&m.mode, m.screen) {
        (Mode::Rule(e), _) => {
            let title = match e.base.id {
                None => "New rule (added last)".to_string(),
                Some(_) => format!("Edit rule: {}", e.base.name),
            };
            form_view(f, body, &e.form, &title);
        }
        (Mode::Pick(p), _) => {
            let items = p
                .picker
                .entries
                .iter()
                .map(|e| match (e.name.as_str(), e.dir) {
                    ("..", _) => ListItem::new("../  (up)"),
                    (n, true) => ListItem::new(format!("{n}/")),
                    (n, false) => ListItem::new(n.to_string()),
                })
                .collect();
            let what = match p.purpose {
                Purpose::AddFolder => "folder to watch",
                Purpose::Destination => "destination folder",
                Purpose::ImportRules => "rules file (.json)",
                Purpose::IgnorePattern => "entry to ignore",
            };
            list(
                f,
                body,
                &format!("Choose {what}: {}", p.picker.dir.display()),
                items,
                p.picker.sel,
            );
        }
        (Mode::Ignore(ig), _) => {
            let items = if ig.lines.is_empty() {
                vec![ListItem::new("(no patterns)")]
            } else {
                ig.lines
                    .iter()
                    .map(|l| match l {
                        IgnoreLine::Comment { text } => {
                            ListItem::new(text.clone()).style(Style::new().fg(Color::DarkGray))
                        }
                        IgnoreLine::Pattern { pattern, .. } => {
                            ListItem::new(pattern.clone())
                        }
                    })
                    .collect()
            };
            list(
                f,
                body,
                &format!("Ignore patterns: {}/.mouziignore", ig.folder),
                items,
                ig.sel,
            );
        }
        (_, Screen::Status) => status(m, f, body),
        (_, Screen::Folders) => folders(m, f, body),
        (_, Screen::Review) => review(m, f, body),
        (_, Screen::Rules) => rules(m, f, body),
        (_, Screen::Settings) => match &m.settings {
            Some(form) => form_view(f, body, form, "Settings (saved to the shared database)"),
            None => f.render_widget(Paragraph::new("loading…"), body),
        },
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
    const FORM: &str =
        "Tab/↓ next  Shift-Tab/↑ back  type to edit  ←/→/space change  Ctrl-O browse (Destination)  Ctrl-S save  Esc cancel";
    match (&m.mode, m.screen) {
        (Mode::Pick(p), _) => match (&p.picker.goto, &p.picker.error) {
            (Some(buf), _) => format!("go to: {buf}█   (Enter go, Esc back)"),
            (None, Some(e)) => format!("✗ {e}"),
            (None, None) => match p.picker.want {
                Want::Dir => "↑↓ move  Enter open  ← up  Space choose this folder  g type path  . hidden  ~ home  Esc cancel".into(),
                Want::JsonFile => "↑↓ move  Enter open/choose  ← up  g type path  . hidden  ~ home  Esc cancel".into(),
                Want::Entry => "↑↓ move  Enter/Space choose  . hidden  Esc cancel".into(),
            },
        },
        (Mode::Confirm(_, prompt), _) => format!("{prompt}   (y confirms, any other key cancels)"),
        (Mode::Rule(_), _) => FORM.into(),
        (Mode::Ignore(ig), _) => match &ig.input {
            Some((_, buf)) => format!("pattern: {buf}█   (Enter save, Esc cancel)"),
            None => "n type  b browse  Enter edit  d delete  ↑↓ select  Esc close".into(),
        },
        (_, Screen::Status) => "↑↓ select  u undo  A undo all  Tab switch  q quit".into(),
        (_, Screen::Folders) => "a add (browse)  d remove  m mode  n only-new  i ignore list  q quit".into(),
        (_, Screen::Review) => "r scan  space toggle  a all  Enter apply  x discard  q quit".into(),
        (_, Screen::Rules) => {
            "n new  Enter edit  c copy  d delete  space on/off  J/K move  i import  e JSON  q quit".into()
        }
        (_, Screen::Settings) => {
            "Tab next field  type to edit  ←/→/space change  Ctrl-S save  Esc leave".into()
        }
    }
}

fn form_view(f: &mut Frame, area: Rect, form: &Form, title: &str) {
    let mut lines: Vec<Line> = form
        .fields
        .iter()
        .enumerate()
        .map(|(i, fl)| {
            let focused = i == form.focus;
            let value = match &fl.kind {
                Kind::Bool => {
                    if fl.value == "true" {
                        "[x]".to_string()
                    } else {
                        "[ ]".to_string()
                    }
                }
                Kind::Choice(_) => format!("◀ {} ▶", fl.value),
                Kind::Text => format!("{}{}", fl.value, if focused { "█" } else { "" }),
            };
            let line = Line::from(format!("{:<30} {value}", fl.label));
            if focused {
                line.style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                line
            }
        })
        .collect();
    if let Some(e) = &form.error {
        lines.push(Line::from(format!("✗ {e}")).style(Style::new().fg(Color::Red)));
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::new().borders(Borders::ALL).title(title.to_string())),
        area,
    );
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
    let shadow = shadowed(&m.rules);
    let items = m
        .rules
        .iter()
        .zip(shadow)
        .map(|(r, shadowed)| {
            ListItem::new(format!(
                "{:>3} {}{:<16} {:<28} {} → {}{}",
                r.priority,
                if r.enabled { " " } else { "✗" },
                r.name,
                r.extensions.join(","),
                r.action,
                r.destination,
                if shadowed {
                    "   (shadowed: an earlier rule takes these files)"
                } else {
                    ""
                }
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

    fn key(m: &mut Model, code: crossterm::event::KeyCode) {
        use crossterm::event::{KeyEvent, KeyEventKind, KeyModifiers};
        let k = KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press);
        super::super::app::update(m, super::super::app::Msg::Key(k));
    }

    #[test]
    fn rule_form_settings_and_shadow_marker_render() {
        use crossterm::event::KeyCode;
        let mut m = Model::new();
        let rule = |id: i64| {
            let mut r = super::super::form::blank_rule(id as i32);
            r.id = Some(id);
            r.name = format!("rule{id}");
            r.extensions = vec!["pdf".into()];
            r.destination = "Docs".into();
            r
        };
        m.rules = vec![rule(1), rule(2)];
        m.screen = Screen::Rules;
        assert!(render(&m, 110, 30).contains("(shadowed"));
        key(&mut m, KeyCode::Char('n'));
        let out = render(&m, 110, 30);
        assert!(out.contains("New rule") && out.contains("Extensions") && out.contains("Ctrl-S"));
        key(&mut m, KeyCode::Char('x'));
        key(&mut m, KeyCode::Esc);
        key(&mut m, KeyCode::Esc);
        m.load_settings(super::super::form::tests_settings());
        m.screen = Screen::Settings;
        let out = render(&m, 110, 30);
        assert!(
            out.contains("Grace period") && out.contains("◀ en ▶") && out.contains("5 Settings")
        );
    }

    #[test]
    fn picker_renders_title_entries_and_hint() {
        use crossterm::event::KeyCode;
        let root = std::env::temp_dir().join(format!("mouzi-ui-pick-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Pictures")).unwrap();
        let mut m = Model::new();
        m.screen = Screen::Folders;
        key(&mut m, KeyCode::Char('a'));
        key(&mut m, KeyCode::Char('g'));
        for c in root.to_str().unwrap().chars() {
            key(&mut m, KeyCode::Char(c));
        }
        key(&mut m, KeyCode::Enter);
        let out = render(&m, 140, 30);
        assert!(
            out.contains("Choose folder to watch")
                && out.contains("Pictures/")
                && out.contains("../  (up)")
        );
        assert!(out.contains("Space choose this folder"));
    }

    #[test]
    fn ignore_list_and_confirm_prompt_render() {
        let mut m = Model::new();
        m.mode = Mode::Ignore(super::super::app::Ignore {
            folder: "/dl".into(),
            lines: vec![
                IgnoreLine::Comment {
                    text: "# temp files".into(),
                },
                IgnoreLine::Pattern {
                    pattern: "*.tmp".into(),
                    raw: None,
                },
            ],
            sel: 1,
            input: None,
        });
        let out = render(&m, 110, 30);
        assert!(out.contains("/dl/.mouziignore") && out.contains("*.tmp"));
        m.mode = Mode::Confirm(
            Box::new(super::super::app::Effect::DeleteRule(1)),
            "delete rule 'x'? y/n".into(),
        );
        assert!(render(&m, 110, 30).contains("y confirms"));
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
