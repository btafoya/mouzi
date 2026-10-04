//! TUI state and key handling. `update` is DB-free: it returns an `Effect` for `exec` to run.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mouzi_core::db::{ActionLog, Rule, WatchedFolder};
use mouzi_core::operations::{OperationResult, Preview, PreviewEntry};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    Status,
    Folders,
    Review,
    Rules,
}
pub const SCREENS: [Screen; 4] = [
    Screen::Status,
    Screen::Folders,
    Screen::Review,
    Screen::Rules,
];

#[derive(PartialEq, Debug)]
pub enum Mode {
    Normal,
    AddFolder(String),
    ConfirmUndoAll,
}

pub struct Review {
    pub id: String,
    pub entries: Vec<PreviewEntry>,
    pub checked: Vec<bool>,
}

pub struct Model {
    pub screen: Screen,
    pub mode: Mode,
    pub quit: bool,
    pub busy: bool,
    pub daemon_active: Option<bool>,
    pub logs: Vec<ActionLog>,
    pub folders: Vec<WatchedFolder>,
    pub rules: Vec<Rule>,
    pub review: Option<Review>,
    pub sel: [usize; 4],
    pub message: String,
}

pub enum Msg {
    Key(KeyEvent),
    Tick,
    Daemon(Option<bool>),
    Previewed(Result<Preview, String>),
    Applied(Result<Vec<OperationResult>, String>),
    Undone(Vec<Result<bool, String>>),
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    AddFolder(String),
    RemoveFolder(i64),
    CycleMode(i64, String),
    ToggleOnlyNew(i64, bool),
    Preview,
    Apply { id: String, selected: Vec<String> },
    Discard(String),
    Undo(Vec<i64>),
    MoveRule(usize, i32),
    EditRules,
}

impl Model {
    pub fn new() -> Self {
        Self {
            screen: Screen::Status,
            mode: Mode::Normal,
            quit: false,
            busy: false,
            daemon_active: None,
            logs: Vec::new(),
            folders: Vec::new(),
            rules: Vec::new(),
            review: None,
            sel: [0; 4],
            message: String::new(),
        }
    }

    pub fn idx(&self) -> usize {
        SCREENS.iter().position(|s| *s == self.screen).unwrap()
    }

    fn len(&self) -> usize {
        match self.screen {
            Screen::Status => self.logs.len(),
            Screen::Folders => self.folders.len(),
            Screen::Review => self.review.as_ref().map_or(0, |r| r.entries.len()),
            Screen::Rules => self.rules.len(),
        }
    }

    pub fn selected(&self) -> usize {
        self.sel[self.idx()].min(self.len().saturating_sub(1))
    }

    pub fn clamp(&mut self) {
        for (i, s) in SCREENS.iter().enumerate() {
            let len = match s {
                Screen::Status => self.logs.len(),
                Screen::Folders => self.folders.len(),
                Screen::Review => self.review.as_ref().map_or(0, |r| r.entries.len()),
                Screen::Rules => self.rules.len(),
            };
            self.sel[i] = self.sel[i].min(len.saturating_sub(1));
        }
    }
}

pub fn update(m: &mut Model, msg: Msg) -> Option<Effect> {
    match msg {
        Msg::Key(k) => key(m, k),
        Msg::Tick => None,
        Msg::Daemon(a) => {
            m.daemon_active = a;
            None
        }
        Msg::Previewed(r) => {
            m.busy = false;
            match r {
                Ok(p) => {
                    let checked = p
                        .entries
                        .iter()
                        .map(|e| e.error.is_none() && !e.warning)
                        .collect();
                    m.message = format!("{} file(s) planned", p.entries.len());
                    m.review = Some(Review {
                        id: p.id,
                        entries: p.entries,
                        checked,
                    });
                    m.sel[2] = 0;
                }
                Err(e) => m.message = format!("preview failed: {e}"),
            }
            None
        }
        Msg::Applied(r) => {
            m.busy = false;
            m.review = None;
            m.message = match r {
                Ok(res) => {
                    let failed: Vec<_> = res.iter().filter(|x| !x.success).collect();
                    match failed.first() {
                        None => format!("moved {} file(s)", res.len()),
                        Some(f) => format!(
                            "{} ok, {} failed: {} ({})",
                            res.len() - failed.len(),
                            failed.len(),
                            f.source,
                            f.error.clone().unwrap_or_default()
                        ),
                    }
                }
                Err(e) => format!("apply failed: {e}"),
            };
            None
        }
        Msg::Undone(rs) => {
            m.busy = false;
            let failed: Vec<_> = rs.iter().filter_map(|r| r.as_ref().err()).collect();
            m.message = match failed.first() {
                None => format!("undid {} file(s)", rs.len()),
                Some(e) => format!(
                    "{} undone, {} failed: {e}",
                    rs.len() - failed.len(),
                    failed.len()
                ),
            };
            None
        }
    }
}

fn key(m: &mut Model, k: KeyEvent) -> Option<Effect> {
    if let Mode::AddFolder(buf) = &mut m.mode {
        match k.code {
            KeyCode::Esc => m.mode = Mode::Normal,
            KeyCode::Enter => {
                let path = std::mem::take(buf);
                m.mode = Mode::Normal;
                return Some(Effect::AddFolder(path));
            }
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            _ => {}
        }
        return None;
    }
    if m.mode == Mode::ConfirmUndoAll {
        m.mode = Mode::Normal;
        if k.code == KeyCode::Char('y') {
            let ids = m
                .logs
                .iter()
                .filter(|l| !l.undone && ["move", "rename"].contains(&l.action.as_str()))
                .filter_map(|l| l.id)
                .collect();
            m.busy = true;
            return Some(Effect::Undo(ids));
        }
        return None;
    }
    if m.busy {
        return None;
    }
    let sel = m.selected();
    let i = m.idx();
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => m.quit = true,
        (KeyCode::Char(c @ '1'..='4'), _) => m.screen = SCREENS[c as usize - '1' as usize],
        (KeyCode::Tab, _) => m.screen = SCREENS[(i + 1) % 4],
        (KeyCode::BackTab, _) => m.screen = SCREENS[(i + 3) % 4],
        (KeyCode::Down | KeyCode::Char('j'), _) => {
            m.sel[i] = (sel + 1).min(m.len().saturating_sub(1))
        }
        (KeyCode::Up | KeyCode::Char('k'), _) => m.sel[i] = sel.saturating_sub(1),
        _ => return screen_key(m, k, sel),
    }
    None
}

fn screen_key(m: &mut Model, k: KeyEvent, sel: usize) -> Option<Effect> {
    match (m.screen, k.code) {
        (Screen::Status, KeyCode::Char('u')) => {
            let log = m.logs.get(sel)?;
            if log.undone || !["move", "rename"].contains(&log.action.as_str()) {
                m.message = "nothing to undo for this entry".into();
                return None;
            }
            m.busy = true;
            Some(Effect::Undo(vec![log.id?]))
        }
        (Screen::Status, KeyCode::Char('A')) => {
            m.mode = Mode::ConfirmUndoAll;
            m.message = "undo ALL recorded moves? y/n".into();
            None
        }
        (Screen::Folders, KeyCode::Char('a')) => {
            m.mode = Mode::AddFolder(String::new());
            None
        }
        (Screen::Folders, KeyCode::Char('d')) => {
            Some(Effect::RemoveFolder(m.folders.get(sel)?.id?))
        }
        (Screen::Folders, KeyCode::Char('m')) => {
            let f = m.folders.get(sel)?;
            let next = match f.mode.as_str() {
                "silent" => "manual",
                "manual" => "paused",
                _ => "silent",
            };
            Some(Effect::CycleMode(f.id?, next.into()))
        }
        (Screen::Folders, KeyCode::Char('n')) => {
            let f = m.folders.get(sel)?;
            Some(Effect::ToggleOnlyNew(f.id?, !f.only_new))
        }
        (Screen::Review, KeyCode::Char('r')) => {
            m.busy = true;
            m.message = "scanning…".into();
            Some(Effect::Preview)
        }
        (Screen::Review, KeyCode::Char(' ')) => {
            let r = m.review.as_mut()?;
            if r.entries.get(sel)?.error.is_none() {
                r.checked[sel] = !r.checked[sel];
            }
            None
        }
        (Screen::Review, KeyCode::Char('a')) => {
            let r = m.review.as_mut()?;
            let all = r
                .checked
                .iter()
                .zip(&r.entries)
                .all(|(c, e)| *c || e.error.is_some());
            for (c, e) in r.checked.iter_mut().zip(&r.entries) {
                *c = !all && e.error.is_none();
            }
            None
        }
        (Screen::Review, KeyCode::Char('x')) => m.review.take().map(|r| Effect::Discard(r.id)),
        (Screen::Review, KeyCode::Enter) => {
            let r = m.review.as_ref()?;
            let selected: Vec<String> = r
                .entries
                .iter()
                .zip(&r.checked)
                .filter(|(_, c)| **c)
                .map(|(e, _)| e.id.clone())
                .collect();
            if selected.is_empty() {
                m.message = "nothing selected".into();
                return None;
            }
            m.busy = true;
            Some(Effect::Apply {
                id: r.id.clone(),
                selected,
            })
        }
        (Screen::Rules, KeyCode::Char('K')) if sel > 0 => Some(Effect::MoveRule(sel, -1)),
        (Screen::Rules, KeyCode::Char('J')) if sel + 1 < m.rules.len() => {
            Some(Effect::MoveRule(sel, 1))
        }
        (Screen::Rules, KeyCode::Char('e')) => Some(Effect::EditRules),
        _ => None,
    }
}

/// Swap rule `idx` with its neighbour and renumber priorities 1..n; returns only changed rules.
pub fn reorder(rules: &[Rule], idx: usize, delta: i32) -> Vec<Rule> {
    let to = idx as i32 + delta;
    if to < 0 || to as usize >= rules.len() {
        return Vec::new();
    }
    let mut order = rules.to_vec();
    order.swap(idx, to as usize);
    order
        .into_iter()
        .enumerate()
        .filter_map(|(i, mut r)| {
            let p = i as i32 + 1;
            (r.priority != p).then(|| {
                r.priority = p;
                r
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;
    use mouzi_core::db::RuleOptions;

    fn k(c: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new_with_kind(
            c,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        ))
    }
    fn rule(id: i64, priority: i32) -> Rule {
        Rule {
            id: Some(id),
            name: format!("r{id}"),
            priority,
            enabled: true,
            extensions: vec!["pdf".into()],
            pattern: None,
            destination: "Docs".into(),
            action: "move".into(),
            folder_id: 0,
            notification_message: None,
            normalize_extensions: false,
            extension_mappings: String::new(),
            options: RuleOptions::default(),
        }
    }
    fn entry(id: &str, error: Option<&str>, warning: bool) -> PreviewEntry {
        PreviewEntry {
            id: id.into(),
            source: format!("/s/{id}"),
            destination: Some(format!("/d/{id}")),
            rule: "r".into(),
            action: "move".into(),
            size: 1,
            warning,
            error: error.map(Into::into),
        }
    }
    fn previewed(entries: Vec<PreviewEntry>) -> Model {
        let mut m = Model::new();
        update(
            &mut m,
            Msg::Previewed(Ok(Preview {
                id: "p1".into(),
                entries,
            })),
        );
        m.screen = Screen::Review;
        m
    }

    #[test]
    fn tab_and_number_keys_switch_screens_and_q_quits() {
        let mut m = Model::new();
        update(&mut m, k(KeyCode::Tab));
        assert_eq!(m.screen, Screen::Folders);
        update(&mut m, k(KeyCode::Char('4')));
        assert_eq!(m.screen, Screen::Rules);
        update(&mut m, k(KeyCode::BackTab));
        assert_eq!(m.screen, Screen::Review);
        update(&mut m, k(KeyCode::Char('q')));
        assert!(m.quit);
    }

    #[test]
    fn add_folder_collects_text_and_emits_effect() {
        let mut m = Model::new();
        m.screen = Screen::Folders;
        update(&mut m, k(KeyCode::Char('a')));
        for c in "/tmp/x".chars() {
            update(&mut m, k(KeyCode::Char(c)));
        }
        update(&mut m, k(KeyCode::Backspace));
        assert_eq!(
            update(&mut m, k(KeyCode::Enter)),
            Some(Effect::AddFolder("/tmp/".into()))
        );
        assert_eq!(m.mode, Mode::Normal);
    }

    #[test]
    fn review_defaults_skip_warnings_and_errors_and_apply_sends_checked_only() {
        let mut m = previewed(vec![
            entry("a", None, false),
            entry("b", None, true),
            entry("c", Some("preview.locked"), false),
        ]);
        assert_eq!(m.review.as_ref().unwrap().checked, vec![true, false, false]);
        update(&mut m, k(KeyCode::Down));
        update(&mut m, k(KeyCode::Char(' ')));
        update(&mut m, k(KeyCode::Down));
        update(&mut m, k(KeyCode::Char(' '))); // errored entry cannot be checked
        assert_eq!(m.review.as_ref().unwrap().checked, vec![true, true, false]);
        assert_eq!(
            update(&mut m, k(KeyCode::Enter)),
            Some(Effect::Apply {
                id: "p1".into(),
                selected: vec!["a".into(), "b".into()]
            })
        );
        assert!(m.busy);
    }

    #[test]
    fn busy_ignores_keys_and_undo_all_needs_confirmation() {
        let mut m = Model::new();
        update(&mut m, k(KeyCode::Char('A')));
        assert_eq!(m.mode, Mode::ConfirmUndoAll);
        assert_eq!(update(&mut m, k(KeyCode::Char('n'))), None);
        assert_eq!(m.mode, Mode::Normal);
        m.busy = true;
        update(&mut m, k(KeyCode::Tab));
        assert_eq!(m.screen, Screen::Status);
    }

    #[test]
    fn reorder_swaps_and_renumbers() {
        let rules = vec![rule(1, 1), rule(2, 2), rule(3, 3)];
        let changed = reorder(&rules, 2, -1);
        let ids: Vec<(i64, i32)> = changed
            .iter()
            .map(|r| (r.id.unwrap(), r.priority))
            .collect();
        assert_eq!(ids, vec![(3, 2), (2, 3)]);
        assert!(reorder(&rules, 0, -1).is_empty());
        // duplicate priorities get normalized
        let dup = vec![rule(1, 5), rule(2, 5)];
        assert_eq!(reorder(&dup, 0, 1).len(), 2);
    }
}
