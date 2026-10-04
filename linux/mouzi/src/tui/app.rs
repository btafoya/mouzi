//! TUI state and key handling. `update` is DB-free: it returns an `Effect` for `exec` to run.
use super::form::{
    blank_rule, build_rule, build_settings, relative_destination, rule_form, settings_form, Form,
    Out,
};
use super::picker::{self, Picker, Want};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mouzi_core::db::{ActionLog, AppSettings, Rule, WatchedFolder};
use mouzi_core::ignore::IgnoreLine;
use mouzi_core::operations::{OperationResult, Preview, PreviewEntry};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    Status,
    Folders,
    Review,
    Rules,
    Settings,
}
pub const SCREENS: [Screen; 5] = [
    Screen::Status,
    Screen::Folders,
    Screen::Review,
    Screen::Rules,
    Screen::Settings,
];

#[derive(Debug)]
pub struct RuleEdit {
    pub form: Form,
    pub base: Rule,
}

#[derive(Debug)]
pub struct Ignore {
    pub folder: String,
    /// The file as typed lines: comments ride along untouched, patterns are
    /// the rows the keys act on.
    pub lines: Vec<IgnoreLine>,
    pub sel: usize,
    /// (index being edited or None for new, text)
    pub input: Option<(Option<usize>, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Purpose {
    AddFolder,
    Destination,
    ImportRules,
    IgnorePattern,
}

/// A picker open on top of `back` (the form or list it was opened from).
#[derive(Debug)]
pub struct Pick {
    pub picker: Picker,
    pub purpose: Purpose,
    pub back: Mode,
}

#[derive(Debug)]
pub enum Mode {
    Normal,
    Pick(Box<Pick>),
    Confirm(Box<Effect>, String),
    Rule(Box<RuleEdit>),
    Ignore(Ignore),
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
    pub app: Option<AppSettings>,
    pub settings: Option<Form>,
    pub sel: [usize; 5],
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

#[derive(Debug)]
pub enum Effect {
    AddFolder(String),
    ImportRules(std::path::PathBuf),
    RemoveFolder(i64),
    CycleMode(i64, String),
    ToggleOnlyNew(i64, bool),
    Preview,
    Apply {
        id: String,
        selected: Vec<String>,
    },
    Discard(String),
    Undo(Vec<i64>),
    MoveRule(usize, i32),
    EditRules,
    SaveRule(Rule),
    DeleteRule(i64),
    SaveSettings(AppSettings),
    OpenIgnore(String),
    SaveIgnore {
        folder: String,
        lines: Vec<IgnoreLine>,
    },
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
            app: None,
            settings: None,
            sel: [0; 5],
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
            Screen::Settings => 0,
        }
    }

    /// Adopt settings from the DB unless the user is mid-edit.
    pub fn load_settings(&mut self, s: AppSettings) {
        if self.settings.as_ref().is_none_or(|f| !f.dirty) {
            let focus = self.settings.as_ref().map_or(0, |f| f.focus);
            let mut form = settings_form(&s);
            form.focus = focus;
            self.settings = Some(form);
        }
        self.app = Some(s);
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
                Screen::Settings => 0,
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
    match std::mem::replace(&mut m.mode, Mode::Normal) {
        Mode::Normal => {}
        Mode::Pick(p) => return pick_key(m, *p, k),
        Mode::Confirm(effect, _) => {
            if k.code != KeyCode::Char('y') {
                m.message = "cancelled".into();
                return None;
            }
            m.busy = matches!(*effect, Effect::Undo(_));
            return Some(*effect);
        }
        Mode::Rule(edit) => return rule_form_key(m, *edit, k),
        Mode::Ignore(ig) => return ignore_key(m, ig, k),
    }
    if m.busy {
        return None;
    }
    if m.screen == Screen::Settings && m.settings.is_some() {
        return settings_key(m, k);
    }
    let sel = m.selected();
    let i = m.idx();
    let n = SCREENS.len();
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Char('c'), KeyModifiers::CONTROL) => m.quit = true,
        (KeyCode::Char(c @ '1'..='5'), _) => m.screen = SCREENS[c as usize - '1' as usize],
        (KeyCode::Tab, _) => m.screen = SCREENS[(i + 1) % n],
        (KeyCode::BackTab, _) => m.screen = SCREENS[(i + n - 1) % n],
        (KeyCode::Down | KeyCode::Char('j'), _) => {
            m.sel[i] = (sel + 1).min(m.len().saturating_sub(1))
        }
        (KeyCode::Up | KeyCode::Char('k'), _) => m.sel[i] = sel.saturating_sub(1),
        _ => return screen_key(m, k, sel),
    }
    None
}

fn pick_key(m: &mut Model, mut pick: Pick, k: KeyEvent) -> Option<Effect> {
    match pick.picker.key(k) {
        picker::Out::Idle => {
            m.mode = Mode::Pick(Box::new(pick));
            None
        }
        picker::Out::Cancel => {
            m.mode = pick.back;
            None
        }
        picker::Out::Picked(path) => {
            m.mode = pick.back;
            match (pick.purpose, &mut m.mode) {
                (Purpose::AddFolder, _) => Some(Effect::AddFolder(path.to_string_lossy().into())),
                (Purpose::ImportRules, _) => Some(Effect::ImportRules(path)),
                (Purpose::Destination, Mode::Rule(edit)) => {
                    let watched: Vec<String> = m.folders.iter().map(|f| f.path.clone()).collect();
                    edit.form
                        .set("Destination", relative_destination(&path, &watched));
                    edit.form.error = build_rule(&edit.form, &edit.base, &m.folders).err();
                    None
                }
                (Purpose::IgnorePattern, Mode::Ignore(ig)) => {
                    let name = path.file_name()?.to_string_lossy();
                    let pattern = if path.is_dir() {
                        format!("{name}/")
                    } else {
                        name.to_string()
                    };
                    if ig.lines.iter().any(|l| l.pattern() == Some(pattern.as_str())) {
                        m.message = format!("{pattern} is already ignored");
                        return None;
                    }
                    ig.sel = ig.lines.len();
                    ig.lines.push(IgnoreLine::Pattern {
                        pattern,
                        raw: None,
                    });
                    Some(Effect::SaveIgnore {
                        folder: ig.folder.clone(),
                        lines: ig.lines.clone(),
                    })
                }
                _ => None,
            }
        }
    }
}

fn rule_form_key(m: &mut Model, mut edit: RuleEdit, k: KeyEvent) -> Option<Effect> {
    if k.code == KeyCode::Char('o')
        && k.modifiers.contains(KeyModifiers::CONTROL)
        && edit.form.fields[edit.form.focus].label == "Destination"
    {
        let typed = std::path::PathBuf::from(edit.form.get("Destination"));
        let scope = std::path::PathBuf::from(edit.form.get("Scope"));
        let start = [
            typed,
            scope,
            m.folders
                .first()
                .map(|f| f.path.clone().into())
                .unwrap_or_default(),
        ]
        .into_iter()
        .find(|p| p.is_absolute() && p.is_dir())
        .unwrap_or_else(picker::home);
        m.mode = Mode::Pick(Box::new(Pick {
            picker: Picker::new(&start, Want::Dir),
            purpose: Purpose::Destination,
            back: Mode::Rule(Box::new(edit)),
        }));
        return None;
    }
    match edit.form.key(k) {
        Out::Cancel => return None,
        Out::Armed => m.message = "unsaved changes: press Esc again to discard".into(),
        Out::Edited => edit.form.error = build_rule(&edit.form, &edit.base, &m.folders).err(),
        Out::Save => match build_rule(&edit.form, &edit.base, &m.folders) {
            Ok(rule) => return Some(Effect::SaveRule(rule)),
            Err(e) => edit.form.error = Some(e),
        },
        Out::Idle => {}
    }
    m.mode = Mode::Rule(Box::new(edit));
    None
}

fn settings_key(m: &mut Model, k: KeyEvent) -> Option<Effect> {
    let (form, base) = (m.settings.as_mut()?, m.app.as_ref()?);
    match form.key(k) {
        Out::Cancel => {
            m.settings = None; // rebuilt from the DB, dropping edits
            m.screen = Screen::Status;
        }
        Out::Armed => m.message = "unsaved changes: press Esc again to discard".into(),
        Out::Edited => form.error = build_settings(form, base).err(),
        Out::Save => match build_settings(form, base) {
            Ok(s) => {
                form.dirty = false;
                return Some(Effect::SaveSettings(s));
            }
            Err(e) => form.error = Some(e),
        },
        Out::Idle => {}
    }
    None
}

fn ignore_key(m: &mut Model, mut ig: Ignore, k: KeyEvent) -> Option<Effect> {
    let mut save = false;
    if let Some((at, mut buf)) = ig.input.take() {
        match k.code {
            KeyCode::Esc => {}
            KeyCode::Enter if buf.trim().is_empty() => {
                m.message = "pattern is empty".into();
                ig.input = Some((at, buf));
            }
            KeyCode::Enter => {
                let edited = IgnoreLine::Pattern {
                    pattern: buf.trim().to_string(),
                    raw: None,
                };
                match at {
                    Some(i) => ig.lines[i] = edited,
                    None => {
                        ig.lines.push(edited);
                        ig.sel = ig.lines.len() - 1;
                    }
                }
                save = true;
            }
            KeyCode::Backspace => {
                buf.pop();
                ig.input = Some((at, buf));
            }
            KeyCode::Char(c) => {
                buf.push(c);
                ig.input = Some((at, buf));
            }
            _ => ig.input = Some((at, buf)),
        }
    } else {
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return None,
            KeyCode::Down | KeyCode::Char('j') => {
                ig.sel = (ig.sel + 1).min(ig.lines.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => ig.sel = ig.sel.saturating_sub(1),
            KeyCode::Char('n') => ig.input = Some((None, String::new())),
            KeyCode::Char('b') => {
                m.mode = Mode::Pick(Box::new(Pick {
                    picker: Picker::new(std::path::Path::new(&ig.folder), Want::Entry),
                    purpose: Purpose::IgnorePattern,
                    back: Mode::Ignore(ig),
                }));
                return None;
            }
            KeyCode::Enter => {
                if let Some(pattern) = ig
                    .lines
                    .get(ig.sel)
                    .and_then(IgnoreLine::pattern)
                    .map(str::to_string)
                {
                    ig.input = Some((Some(ig.sel), pattern));
                }
            }
            KeyCode::Char('d') if !ig.lines.is_empty() => {
                ig.lines.remove(ig.sel);
                ig.sel = ig.sel.min(ig.lines.len().saturating_sub(1));
                save = true;
            }
            _ => {}
        }
    }
    let effect = save.then(|| Effect::SaveIgnore {
        folder: ig.folder.clone(),
        lines: ig.lines.clone(),
    });
    m.mode = Mode::Ignore(ig);
    effect
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
            let ids: Vec<i64> = m
                .logs
                .iter()
                .filter(|l| !l.undone && ["move", "rename"].contains(&l.action.as_str()))
                .filter_map(|l| l.id)
                .collect();
            if ids.is_empty() {
                m.message = "nothing to undo".into();
                return None;
            }
            m.mode = Mode::Confirm(
                Box::new(Effect::Undo(ids.clone())),
                format!("undo ALL {} recorded moves? y/n", ids.len()),
            );
            None
        }
        (Screen::Folders, KeyCode::Char('a')) => {
            m.mode = Mode::Pick(Box::new(Pick {
                picker: Picker::new(&picker::home(), Want::Dir),
                purpose: Purpose::AddFolder,
                back: Mode::Normal,
            }));
            None
        }
        (Screen::Folders, KeyCode::Char('d')) => {
            let f = m.folders.get(sel)?;
            let id = f.id?;
            let scoped = m.rules.iter().filter(|r| r.folder_id == id).count();
            let note = if scoped > 0 {
                format!(" {scoped} rule(s) scoped to it will never match.")
            } else {
                String::new()
            };
            m.mode = Mode::Confirm(
                Box::new(Effect::RemoveFolder(id)),
                format!("remove {}?{note} y/n", f.path),
            );
            None
        }
        (Screen::Folders, KeyCode::Char('i')) => {
            Some(Effect::OpenIgnore(m.folders.get(sel)?.path.clone()))
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
        (Screen::Rules, KeyCode::Char('i')) => {
            m.mode = Mode::Pick(Box::new(Pick {
                picker: Picker::new(&picker::home(), Want::JsonFile),
                purpose: Purpose::ImportRules,
                back: Mode::Normal,
            }));
            None
        }
        (Screen::Rules, KeyCode::Char('n')) => {
            let next = m.rules.iter().map(|r| r.priority).max().unwrap_or(0) + 1;
            let mut base = blank_rule(next); // bottom: a new rule never shadows existing ones
            // default to the first watched folder: firing on everything should be a conscious choice
            base.folder_id = m.folders.first().and_then(|f| f.id).unwrap_or(0);
            m.mode = Mode::Rule(Box::new(RuleEdit {
                form: rule_form(&base, &m.folders),
                base,
            }));
            None
        }
        (Screen::Rules, KeyCode::Enter) => {
            let base = m.rules.get(sel)?.clone();
            m.mode = Mode::Rule(Box::new(RuleEdit {
                form: rule_form(&base, &m.folders),
                base,
            }));
            None
        }
        (Screen::Rules, KeyCode::Char('c')) => {
            let mut copy = m.rules.get(sel)?.clone();
            copy.id = None;
            copy.name = format!("{} (copy)", copy.name);
            copy.priority = m.rules.iter().map(|r| r.priority).max().unwrap_or(0) + 1;
            Some(Effect::SaveRule(copy))
        }
        (Screen::Rules, KeyCode::Char(' ')) => {
            let mut r = m.rules.get(sel)?.clone();
            r.enabled = !r.enabled;
            Some(Effect::SaveRule(r))
        }
        (Screen::Rules, KeyCode::Char('d')) => {
            let r = m.rules.get(sel)?;
            m.mode = Mode::Confirm(
                Box::new(Effect::DeleteRule(r.id?)),
                format!("delete rule '{}'? y/n", r.name),
            );
            None
        }
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

    fn temp_tree(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("mouzi-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/sub")).unwrap();
        std::fs::write(root.join("docs/rules.json"), "[]").unwrap();
        std::fs::write(root.join("docs/a.tmp"), "").unwrap();
        root
    }
    fn folder(id: i64, path: &std::path::Path) -> WatchedFolder {
        WatchedFolder {
            id: Some(id),
            path: path.to_string_lossy().into(),
            enabled: true,
            mode: "silent".into(),
            only_new: false,
        }
    }

    #[test]
    fn add_folder_opens_the_picker_and_picks_a_typed_path() {
        let root = temp_tree("add");
        let mut m = Model::new();
        m.screen = Screen::Folders;
        update(&mut m, k(KeyCode::Char('a')));
        assert!(matches!(m.mode, Mode::Pick(_)));
        update(&mut m, k(KeyCode::Char('g')));
        typed(&mut m, root.join("docs").to_str().unwrap());
        update(&mut m, k(KeyCode::Enter)); // jumps into docs
        let eff = update(&mut m, k(KeyCode::Char(' ')));
        assert!(
            matches!(eff, Some(Effect::AddFolder(p)) if p == root.join("docs").to_string_lossy())
        );
        assert!(matches!(m.mode, Mode::Normal));
        update(&mut m, k(KeyCode::Char('a')));
        update(&mut m, k(KeyCode::Esc));
        assert!(
            matches!(m.mode, Mode::Normal),
            "cancel returns without an effect"
        );
    }

    #[test]
    fn destination_picker_fills_a_relative_path_and_returns_to_the_form() {
        let root = temp_tree("dest");
        let mut m = on_rules(1);
        m.folders = vec![folder(1, &root.join("docs"))];
        update(&mut m, k(KeyCode::Char('n')));
        for _ in 0..6 {
            update(&mut m, k(KeyCode::Tab)); // Destination
        }
        // Ctrl-O only opens the picker on the Destination field
        let ctrl_o = || {
            Msg::Key(KeyEvent::new_with_kind(
                KeyCode::Char('o'),
                KeyModifiers::CONTROL,
                KeyEventKind::Press,
            ))
        };
        update(&mut m, k(KeyCode::Tab)); // Rename template
        update(&mut m, ctrl_o());
        assert!(matches!(m.mode, Mode::Rule(_)));
        update(&mut m, k(KeyCode::BackTab));
        update(&mut m, ctrl_o());
        assert!(matches!(m.mode, Mode::Pick(_)));
        update(&mut m, k(KeyCode::Down)); // "sub" (starts in the watched folder; ".." is first)
        update(&mut m, k(KeyCode::Down));
        update(&mut m, k(KeyCode::Enter));
        update(&mut m, k(KeyCode::Char(' ')));
        let Mode::Rule(edit) = &m.mode else {
            panic!("not back in the form")
        };
        assert_eq!(edit.form.get("Destination"), "sub");
        assert!(edit.form.dirty);
    }

    #[test]
    fn ignore_picker_adds_names_with_a_slash_for_folders() {
        let root = temp_tree("ign");
        let docs = root.join("docs");
        let mut m = Model::new();
        m.mode = Mode::Ignore(Ignore {
            folder: docs.to_string_lossy().into(),
            lines: vec![],
            sel: 0,
            input: None,
        });
        update(&mut m, k(KeyCode::Char('b')));
        let Some(Effect::SaveIgnore { lines, .. }) = update(&mut m, k(KeyCode::Enter)) else {
            panic!()
        };
        assert_eq!(lines, [pat("sub/")]);
        assert!(matches!(m.mode, Mode::Ignore(_)));
        update(&mut m, k(KeyCode::Char('b')));
        update(&mut m, k(KeyCode::Down));
        let Some(Effect::SaveIgnore { lines, .. }) = update(&mut m, k(KeyCode::Enter)) else {
            panic!()
        };
        assert_eq!(lines, [pat("sub/"), pat("a.tmp")]);
        update(&mut m, k(KeyCode::Char('b')));
        assert!(
            update(&mut m, k(KeyCode::Enter)).is_none(),
            "duplicate is not added twice"
        );
    }

    #[test]
    fn rules_import_picker_returns_the_chosen_json_file() {
        let root = temp_tree("imp");
        let mut m = on_rules(1);
        update(&mut m, k(KeyCode::Char('i')));
        update(&mut m, k(KeyCode::Char('g')));
        typed(&mut m, root.join("docs").to_str().unwrap());
        update(&mut m, k(KeyCode::Enter));
        update(&mut m, k(KeyCode::Down)); // sub
        update(&mut m, k(KeyCode::Down)); // rules.json
        let eff = update(&mut m, k(KeyCode::Enter));
        assert!(matches!(eff, Some(Effect::ImportRules(p)) if p == root.join("docs/rules.json")));
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
        assert!(matches!(
            update(&mut m, k(KeyCode::Enter)),
            Some(Effect::Apply { id, selected }) if id == "p1" && selected == ["a", "b"]
        ));
        assert!(m.busy);
    }

    #[test]
    fn busy_ignores_keys() {
        let mut m = Model::new();
        m.busy = true;
        update(&mut m, k(KeyCode::Tab));
        assert_eq!(m.screen, Screen::Status);
    }

    fn pat(p: &str) -> IgnoreLine {
        IgnoreLine::Pattern {
            pattern: p.into(),
            raw: None,
        }
    }
    fn cm(t: &str) -> IgnoreLine {
        IgnoreLine::Comment {
            text: t.into(),
        }
    }
    fn typed(m: &mut Model, text: &str) {
        for c in text.chars() {
            update(m, k(KeyCode::Char(c)));
        }
    }
    fn ctrl_s() -> Msg {
        Msg::Key(KeyEvent::new_with_kind(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
            KeyEventKind::Press,
        ))
    }
    fn on_rules(n: i64) -> Model {
        let mut m = Model::new();
        m.screen = Screen::Rules;
        m.rules = (1..=n).map(|i| rule(i, i as i32)).collect();
        m
    }

    #[test]
    fn new_rule_defaults_to_the_first_watched_folder() {
        let mut m = on_rules(1);
        m.folders = vec![WatchedFolder {
            id: Some(5),
            path: "/dl".into(),
            enabled: true,
            mode: "silent".into(),
            only_new: false,
        }];
        m.screen = Screen::Rules;
        update(&mut m, k(KeyCode::Char('n')));
        let Mode::Rule(edit) = &m.mode else {
            panic!("no form")
        };
        assert_eq!(edit.form.get("Scope"), "/dl");
        // with no watched folders, "all folders" is still the scope
        let mut blank = Model::new();
        blank.screen = Screen::Rules;
        update(&mut blank, k(KeyCode::Char('n')));
        if let Mode::Rule(edit) = &blank.mode {
            assert_eq!(edit.form.get("Scope"), "all folders");
        }
    }

    #[test]
    fn new_rule_form_blocks_invalid_save_then_appends_last() {
        let mut m = on_rules(2);
        update(&mut m, k(KeyCode::Char('n')));
        assert!(matches!(m.mode, Mode::Rule(_)));
        assert!(update(&mut m, ctrl_s()).is_none()); // empty name/extensions
        let Mode::Rule(edit) = &m.mode else {
            panic!("form closed")
        };
        assert!(edit.form.error.as_deref().unwrap().contains("name"));
        typed(&mut m, "Pics"); // Name
        update(&mut m, k(KeyCode::Tab)); // Enabled
        update(&mut m, k(KeyCode::Tab)); // Scope
        update(&mut m, k(KeyCode::Tab)); // Extensions
        typed(&mut m, "png, jpg");
        update(&mut m, k(KeyCode::Tab)); // Pattern
        update(&mut m, k(KeyCode::Tab)); // Action
        update(&mut m, k(KeyCode::Tab)); // Destination
        typed(&mut m, "Images");
        let Some(Effect::SaveRule(r)) = update(&mut m, ctrl_s()) else {
            panic!("not saved")
        };
        assert_eq!((r.id, r.priority, r.name.as_str()), (None, 3, "Pics"));
        assert_eq!(r.extensions, vec!["png", "jpg"]);
        assert!(matches!(m.mode, Mode::Normal));
    }

    #[test]
    fn edit_existing_rule_keeps_id_and_cancel_asks_first() {
        let mut m = on_rules(2);
        update(&mut m, k(KeyCode::Down));
        update(&mut m, k(KeyCode::Enter));
        typed(&mut m, "!");
        update(&mut m, k(KeyCode::Esc));
        assert!(
            matches!(m.mode, Mode::Rule(_)),
            "dirty form needs a second Esc"
        );
        update(&mut m, k(KeyCode::Esc));
        assert!(matches!(m.mode, Mode::Normal));
        update(&mut m, k(KeyCode::Enter));
        typed(&mut m, "!");
        let Some(Effect::SaveRule(r)) = update(&mut m, ctrl_s()) else {
            panic!("not saved")
        };
        assert_eq!((r.id, r.name.as_str()), (Some(2), "r2!"));
    }

    #[test]
    fn duplicate_toggle_and_delete_rules() {
        let mut m = on_rules(2);
        let Some(Effect::SaveRule(c)) = update(&mut m, k(KeyCode::Char('c'))) else {
            panic!()
        };
        assert_eq!((c.id, c.priority, c.name.as_str()), (None, 3, "r1 (copy)"));
        let Some(Effect::SaveRule(t)) = update(&mut m, k(KeyCode::Char(' '))) else {
            panic!()
        };
        assert!(!t.enabled && t.id == Some(1));
        assert!(update(&mut m, k(KeyCode::Char('d'))).is_none());
        assert!(
            matches!(&m.mode, Mode::Confirm(e, p) if matches!(**e, Effect::DeleteRule(1)) && p.contains("r1"))
        );
        assert!(
            update(&mut m, k(KeyCode::Char('n'))).is_none(),
            "any key but y cancels"
        );
        update(&mut m, k(KeyCode::Char('d')));
        assert!(matches!(
            update(&mut m, k(KeyCode::Char('y'))),
            Some(Effect::DeleteRule(1))
        ));
    }

    #[test]
    fn folder_delete_warns_about_scoped_rules() {
        let mut m = on_rules(1);
        m.screen = Screen::Folders;
        m.folders = vec![WatchedFolder {
            id: Some(5),
            path: "/dl".into(),
            enabled: true,
            mode: "silent".into(),
            only_new: false,
        }];
        m.rules[0].folder_id = 5;
        update(&mut m, k(KeyCode::Char('d')));
        assert!(
            matches!(&m.mode, Mode::Confirm(e, p) if matches!(**e, Effect::RemoveFolder(5)) && p.contains("1 rule(s)"))
        );
    }

    #[test]
    fn undo_all_needs_confirmation() {
        let mut m = Model::new();
        update(&mut m, k(KeyCode::Char('A')));
        assert!(matches!(m.mode, Mode::Normal), "nothing to undo");
    }

    #[test]
    fn settings_screen_captures_keys_saves_and_esc_leaves() {
        let mut m = Model::new();
        m.load_settings(crate::tui::form::tests_settings());
        update(&mut m, k(KeyCode::Char('5')));
        assert_eq!(m.screen, Screen::Settings);
        update(&mut m, k(KeyCode::Backspace)); // grace "300" -> "30"
        update(&mut m, k(KeyCode::Char('q'))); // types, does not quit
        assert!(!m.quit && m.settings.as_ref().unwrap().error.is_some());
        update(&mut m, k(KeyCode::Backspace));
        let Some(Effect::SaveSettings(s)) = update(&mut m, ctrl_s()) else {
            panic!("not saved")
        };
        assert_eq!(s.grace_period_seconds, 30);
        m.load_settings(crate::tui::form::tests_settings()); // clean form follows the DB
        assert_eq!(
            m.settings.as_ref().unwrap().get("Grace period (seconds)"),
            "300"
        );
        update(&mut m, k(KeyCode::Esc));
        assert_eq!(m.screen, Screen::Status);
    }

    #[test]
    fn dirty_settings_are_not_overwritten_by_refresh() {
        let mut m = Model::new();
        m.load_settings(crate::tui::form::tests_settings());
        m.screen = Screen::Settings;
        update(&mut m, k(KeyCode::Char('9')));
        m.load_settings(crate::tui::form::tests_settings());
        assert!(m
            .settings
            .as_ref()
            .unwrap()
            .get("Grace period (seconds)")
            .ends_with('9'));
    }

    #[test]
    fn ignore_list_add_edit_delete_emit_saves() {
        let mut m = Model::new();
        m.mode = Mode::Ignore(Ignore {
            folder: "/dl".into(),
            lines: vec![cm("# tmp files"), pat("*.tmp")],
            sel: 1,
            input: None,
        });
        update(&mut m, k(KeyCode::Char('n')));
        typed(&mut m, "node_modules/");
        let Some(Effect::SaveIgnore { lines, .. }) = update(&mut m, k(KeyCode::Enter)) else {
            panic!()
        };
        assert_eq!(lines, [cm("# tmp files"), pat("*.tmp"), pat("node_modules/")]);
        // Enter on a comment line is inert: comments are not editable patterns
        update(&mut m, k(KeyCode::Up));
        update(&mut m, k(KeyCode::Up));
        update(&mut m, k(KeyCode::Enter));
        assert!(matches!(
            m.mode,
            Mode::Ignore(Ignore {
                input: None,
                ..
            })
        ));
        update(&mut m, k(KeyCode::Down));
        update(&mut m, k(KeyCode::Enter));
        update(&mut m, k(KeyCode::Backspace));
        let Some(Effect::SaveIgnore { lines, .. }) = update(&mut m, k(KeyCode::Enter)) else {
            panic!()
        };
        assert_eq!(lines[1].pattern(), Some("*.tm"));
        update(&mut m, k(KeyCode::Char('n')));
        assert!(
            update(&mut m, k(KeyCode::Enter)).is_none(),
            "empty pattern rejected"
        );
        update(&mut m, k(KeyCode::Esc));
        let Some(Effect::SaveIgnore { lines, .. }) = update(&mut m, k(KeyCode::Char('d')))
        else {
            panic!()
        };
        assert_eq!(lines, [cm("# tmp files"), pat("node_modules/")]);
        update(&mut m, k(KeyCode::Esc));
        assert!(matches!(m.mode, Mode::Normal));
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
