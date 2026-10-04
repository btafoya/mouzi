//! Generic field form plus the rule/settings forms built on it. No DB access: building a
//! form or reading one back is a pure function, validated with the core's own validators.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mouzi_core::db::{normalize_extensions, AppSettings, Rule, RuleOptions, WatchedFolder};
use mouzi_core::rules::validate_rule;

pub const LANGS: [&str; 11] = [
    "en", "de", "es", "fr", "it", "ja", "pl", "ru", "uk", "vi", "zh-CN",
];
const ALL_FOLDERS: &str = "all folders";

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Text,
    Bool,
    Choice(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub label: &'static str,
    pub value: String, // Bool is "true"/"false"
    pub kind: Kind,
}

#[derive(Debug, PartialEq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub focus: usize,
    pub dirty: bool,
    armed: bool,
    pub error: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Out {
    Idle,
    Edited,
    Save,
    Cancel,
    /// Esc on a dirty form: press Esc again to discard.
    Armed,
}

fn text(label: &'static str, value: impl Into<String>) -> Field {
    Field {
        label,
        value: value.into(),
        kind: Kind::Text,
    }
}
fn flag(label: &'static str, on: bool) -> Field {
    Field {
        label,
        value: on.to_string(),
        kind: Kind::Bool,
    }
}
fn choice(label: &'static str, value: impl Into<String>, options: Vec<String>) -> Field {
    Field {
        label,
        value: value.into(),
        kind: Kind::Choice(options),
    }
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Self {
        Self {
            fields,
            focus: 0,
            dirty: false,
            armed: false,
            error: None,
        }
    }

    pub fn get(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map_or("", |f| f.value.as_str())
    }

    pub fn set(&mut self, label: &str, value: String) {
        if let Some(field) = self.fields.iter_mut().find(|f| f.label == label) {
            field.value = value;
            self.dirty = true;
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> Out {
        let was_armed = std::mem::take(&mut self.armed);
        let n = self.fields.len();
        match (k.code, k.modifiers) {
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => return Out::Save,
            (KeyCode::Esc, _) if self.dirty && !was_armed => {
                self.armed = true;
                return Out::Armed;
            }
            (KeyCode::Esc, _) => return Out::Cancel,
            (KeyCode::Tab | KeyCode::Down, _) => self.focus = (self.focus + 1) % n,
            (KeyCode::BackTab | KeyCode::Up, _) => self.focus = (self.focus + n - 1) % n,
            _ => return self.edit(k),
        }
        Out::Idle
    }

    fn edit(&mut self, k: KeyEvent) -> Out {
        let field = &mut self.fields[self.focus];
        let step = match k.code {
            KeyCode::Left => -1,
            KeyCode::Char(' ') | KeyCode::Right | KeyCode::Enter => 1,
            _ => 0,
        };
        let changed = match &field.kind {
            Kind::Text => match k.code {
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    field.value.push(c);
                    true
                }
                KeyCode::Backspace => field.value.pop().is_some(),
                _ => false,
            },
            Kind::Bool if step != 0 => {
                field.value = (field.value != "true").to_string();
                true
            }
            Kind::Choice(opts) if step != 0 => {
                let at = opts.iter().position(|o| *o == field.value).unwrap_or(0) as i32;
                field.value = opts[(at + step).rem_euclid(opts.len() as i32) as usize].clone();
                true
            }
            _ => false,
        };
        if changed {
            self.dirty = true;
            Out::Edited
        } else {
            Out::Idle
        }
    }
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

pub fn blank_rule(priority: i32) -> Rule {
    Rule {
        id: None,
        name: String::new(),
        priority,
        enabled: true,
        extensions: Vec::new(),
        pattern: None,
        destination: String::new(),
        action: "move".into(),
        folder_id: 0,
        notification_message: None,
        normalize_extensions: false,
        extension_mappings: "jpeg:jpg".into(),
        options: RuleOptions::default(),
    }
}

fn scope_value(r: &Rule, folders: &[WatchedFolder]) -> String {
    if r.folder_id == 0 {
        return ALL_FOLDERS.into();
    }
    folders
        .iter()
        .find(|f| f.id == Some(r.folder_id))
        .map_or_else(
            || format!("folder #{} (missing)", r.folder_id),
            |f| f.path.clone(),
        )
}

pub fn rule_form(r: &Rule, folders: &[WatchedFolder]) -> Form {
    let scope = scope_value(r, folders);
    let mut options: Vec<String> = std::iter::once(ALL_FOLDERS.to_string())
        .chain(folders.iter().map(|f| f.path.clone()))
        .collect();
    if !options.contains(&scope) {
        options.push(scope.clone());
    }
    let o = &r.options;
    Form::new(vec![
        text("Name", r.name.clone()),
        flag("Enabled", r.enabled),
        choice("Scope", scope, options),
        text("Extensions", r.extensions.join(", ")),
        text("Pattern (regex)", r.pattern.clone().unwrap_or_default()),
        choice(
            "Action",
            r.action.clone(),
            ["move", "rename", "delete", "ignore"]
                .map(String::from)
                .to_vec(),
        ),
        text("Destination", r.destination.clone()),
        text("Rename template", o.rename_template.clone()),
        text(
            "Min size (bytes)",
            o.min_size.map(|v| v.to_string()).unwrap_or_default(),
        ),
        text(
            "Max size (bytes)",
            o.max_size.map(|v| v.to_string()).unwrap_or_default(),
        ),
        text(
            "Modified after (YYYY-MM-DD)",
            o.modified_after.clone().unwrap_or_default(),
        ),
        text(
            "Modified before (YYYY-MM-DD)",
            o.modified_before.clone().unwrap_or_default(),
        ),
        flag("Normalize extensions", r.normalize_extensions),
        text("Extension mappings", r.extension_mappings.clone()),
        text(
            "Notification message",
            r.notification_message.clone().unwrap_or_default(),
        ),
    ])
}

/// Destination value for a picked folder: relative to the deepest watched folder that
/// contains it (rules resolve relative destinations per watched folder), else absolute.
pub fn relative_destination(picked: &std::path::Path, watched: &[String]) -> String {
    watched
        .iter()
        .map(std::path::Path::new)
        .filter(|w| picked.starts_with(w))
        .max_by_key(|w| w.components().count())
        .map_or_else(
            || picked.to_string_lossy().to_string(),
            |w| match picked.strip_prefix(w).unwrap().to_string_lossy() {
                rest if rest.is_empty() => ".".to_string(),
                rest => rest.to_string(),
            },
        )
}

fn opt(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn number(label: &str, s: &str) -> Result<Option<u64>, String> {
    opt(s).map_or(Ok(None), |v| {
        v.parse()
            .map(Some)
            .map_err(|_| format!("{label} must be a whole number"))
    })
}

fn explain(code: &str) -> String {
    match code {
        "validation.name" => "name is required",
        "validation.extensions" => "extensions: letters, digits, _ or - only (or * for all)",
        "validation.pattern" => "pattern is not a valid regex",
        "validation.action" => "unknown action",
        "validation.destination" => "destination is required for move and must not contain ..",
        "validation.size" => "min size is larger than max size",
        "validation.date" => {
            "dates must be YYYY-MM-DD, and 'after' must not be later than 'before'"
        }
        "validation.template" => {
            "rename template may only use {stem} {filename} {extension} {year} {month} {day}"
        }
        other => other,
    }
    .into()
}

pub fn build_rule(f: &Form, base: &Rule, folders: &[WatchedFolder]) -> Result<Rule, String> {
    let scope = f.get("Scope");
    let folder_id = if scope == ALL_FOLDERS {
        0
    } else {
        folders
            .iter()
            .find(|d| d.path == scope)
            .and_then(|d| d.id)
            .unwrap_or(base.folder_id) // "(missing)" option keeps the old scope
    };
    let rule = Rule {
        id: base.id,
        name: f.get("Name").trim().to_string(),
        priority: base.priority,
        enabled: f.get("Enabled") == "true",
        extensions: f
            .get("Extensions")
            .split([',', ';', ' '])
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        pattern: opt(f.get("Pattern (regex)")),
        destination: f.get("Destination").trim().to_string(),
        action: f.get("Action").to_string(),
        folder_id,
        notification_message: opt(f.get("Notification message")),
        normalize_extensions: f.get("Normalize extensions") == "true",
        extension_mappings: f.get("Extension mappings").to_string(),
        options: RuleOptions {
            min_size: number("min size", f.get("Min size (bytes)"))?,
            max_size: number("max size", f.get("Max size (bytes)"))?,
            modified_after: opt(f.get("Modified after (YYYY-MM-DD)")),
            modified_before: opt(f.get("Modified before (YYYY-MM-DD)")),
            rename_template: f.get("Rename template").to_string(),
        },
    };
    validate_rule(&rule).map_err(|c| explain(&c))?;
    Ok(rule)
}

/// Flags rules that can never match because an earlier enabled rule already takes every
/// file they would. Conservative: only plain extension rules (no pattern/size/date) shadow.
pub fn shadowed(rules: &[Rule]) -> Vec<bool> {
    let plain = |r: &Rule| {
        let o = &r.options;
        r.enabled
            && r.pattern.as_deref().is_none_or(str::is_empty)
            && o.min_size.is_none()
            && o.max_size.is_none()
            && o.modified_after.is_none()
            && o.modified_before.is_none()
    };
    rules
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mine = normalize_extensions(&r.extensions);
            r.enabled
                && rules[..i].iter().any(|e| {
                    let theirs = normalize_extensions(&e.extensions);
                    plain(e)
                        && (e.folder_id == 0 || e.folder_id == r.folder_id)
                        && (theirs.iter().any(|x| x == "*")
                            || mine.iter().all(|x| theirs.contains(x)))
                })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// App settings
// ---------------------------------------------------------------------------

pub fn settings_form(s: &AppSettings) -> Form {
    let t = |v: &Option<String>| v.clone().unwrap_or_default();
    Form::new(vec![
        text("Grace period (seconds)", s.grace_period_seconds.to_string()),
        flag("Lock check", s.lock_check_enabled),
        choice(
            "Language",
            s.language.clone(),
            LANGS.map(String::from).to_vec(),
        ),
        flag("Schedule enabled", s.schedule_enabled),
        text("Times per day (1-4)", s.schedule_times_per_day.to_string()),
        text("Time 1 (HH:MM)", t(&s.schedule_time_1)),
        text("Time 2 (HH:MM)", t(&s.schedule_time_2)),
        text("Time 3 (HH:MM)", t(&s.schedule_time_3)),
        text("Time 4 (HH:MM)", t(&s.schedule_time_4)),
    ])
}

pub fn build_settings(f: &Form, base: &AppSettings) -> Result<AppSettings, String> {
    let grace: i64 = f
        .get("Grace period (seconds)")
        .trim()
        .parse()
        .ok()
        .filter(|v| *v >= 0)
        .ok_or("grace period must be a whole number of seconds, 0 or more")?;
    let per_day: i64 = f
        .get("Times per day (1-4)")
        .trim()
        .parse()
        .ok()
        .filter(|v| (1..=4).contains(v))
        .ok_or("times per day must be 1 to 4")?;
    let language = f.get("Language");
    if !LANGS.contains(&language) {
        return Err(format!("unsupported language {language}"));
    }
    let schedule_enabled = f.get("Schedule enabled") == "true";
    let mut times = Vec::new();
    for (i, label) in [
        "Time 1 (HH:MM)",
        "Time 2 (HH:MM)",
        "Time 3 (HH:MM)",
        "Time 4 (HH:MM)",
    ]
    .into_iter()
    .enumerate()
    {
        let v = opt(f.get(label));
        match &v {
            Some(t) if chrono::NaiveTime::parse_from_str(t, "%H:%M").is_err() => {
                return Err(format!("time {} must be HH:MM (24 hour)", i + 1));
            }
            None if schedule_enabled && (i as i64) < per_day => {
                return Err(format!(
                    "time {} is required while the schedule is on",
                    i + 1
                ));
            }
            _ => {}
        }
        times.push(v);
    }
    let mut it = times.into_iter();
    Ok(AppSettings {
        grace_period_seconds: grace,
        lock_check_enabled: f.get("Lock check") == "true",
        language: language.to_string(),
        schedule_enabled,
        schedule_times_per_day: per_day,
        schedule_time_1: it.next().flatten(),
        schedule_time_2: it.next().flatten(),
        schedule_time_3: it.next().flatten(),
        schedule_time_4: it.next().flatten(),
        ..base.clone()
    })
}

#[cfg(test)]
pub fn tests_settings() -> AppSettings {
    tests::settings()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
    }
    fn folder(id: i64, path: &str) -> WatchedFolder {
        WatchedFolder {
            id: Some(id),
            path: path.into(),
            enabled: true,
            mode: "silent".into(),
            only_new: false,
        }
    }
    fn valid() -> Rule {
        let mut r = blank_rule(3);
        r.name = "Docs".into();
        r.extensions = vec!["pdf".into(), "docx".into()];
        r.destination = "Documents/{year}".into();
        r
    }
    fn set(f: &mut Form, label: &str, v: &str) {
        f.fields
            .iter_mut()
            .find(|x| x.label == label)
            .unwrap()
            .value = v.into();
    }
    pub fn settings() -> AppSettings {
        AppSettings {
            id: Some(1),
            language: "en".into(),
            theme: "dark".into(),
            telemetry_enabled: false,
            first_run: false,
            autostart: true,
            grace_period_seconds: 300,
            lock_check_enabled: true,
            auto_update_enabled: true,
            schedule_enabled: false,
            schedule_times_per_day: 1,
            schedule_time_1: None,
            schedule_time_2: None,
            schedule_time_3: None,
            schedule_time_4: None,
        }
    }

    #[test]
    fn rule_roundtrips_through_the_form() {
        let folders = [folder(7, "/dl")];
        let mut r = valid();
        r.id = Some(9);
        r.folder_id = 7;
        r.pattern = Some("^inv".into());
        r.options.min_size = Some(10);
        r.options.modified_after = Some("2026-01-01".into());
        r.notification_message = Some("{file}".into());
        let back = build_rule(&rule_form(&r, &folders), &r, &folders).unwrap();
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            serde_json::to_string(&r).unwrap()
        );
    }

    #[test]
    fn invalid_rules_give_readable_errors_and_never_build() {
        let folders = [];
        let base = valid();
        let try_with = |label: &str, v: &str| {
            let mut f = rule_form(&base, &folders);
            set(&mut f, label, v);
            build_rule(&f, &base, &folders).unwrap_err()
        };
        assert!(try_with("Name", "  ").contains("name"));
        assert!(try_with("Extensions", "").contains("extensions"));
        assert!(try_with("Extensions", "p!f").contains("extensions"));
        assert!(try_with("Pattern (regex)", "(").contains("regex"));
        assert!(try_with("Destination", "../x").contains("destination"));
        assert!(try_with("Min size (bytes)", "abc").contains("min size"));
        let mut f = rule_form(&base, &folders);
        set(&mut f, "Min size (bytes)", "10");
        set(&mut f, "Max size (bytes)", "5");
        assert!(build_rule(&f, &base, &folders)
            .unwrap_err()
            .contains("larger"));
        assert!(try_with("Modified after (YYYY-MM-DD)", "01/02/2026").contains("YYYY-MM-DD"));
        let mut f = rule_form(&base, &folders);
        set(&mut f, "Action", "rename");
        set(&mut f, "Rename template", "{nope}");
        assert!(build_rule(&f, &base, &folders)
            .unwrap_err()
            .contains("template"));
    }

    #[test]
    fn missing_scope_folder_is_kept_not_silently_widened() {
        let mut r = valid();
        r.folder_id = 42;
        let f = rule_form(&r, &[]);
        assert_eq!(f.get("Scope"), "folder #42 (missing)");
        assert_eq!(build_rule(&f, &r, &[]).unwrap().folder_id, 42);
    }

    #[test]
    fn shadowing_needs_an_earlier_plain_rule_covering_everything() {
        let mk = |id: i64, exts: &[&str], folder_id: i64| {
            let mut r = valid();
            r.id = Some(id);
            r.folder_id = folder_id;
            r.extensions = exts.iter().map(|s| s.to_string()).collect();
            r
        };
        let rules = [
            mk(1, &["pdf", "txt"], 0),
            mk(2, &["pdf"], 0),
            mk(3, &["pdf", "zip"], 0),
            mk(4, &["txt"], 5),
        ];
        assert_eq!(shadowed(&rules), vec![false, true, false, true]);
        let mut with_pattern = rules.clone();
        with_pattern[0].pattern = Some("^a".into());
        assert_eq!(shadowed(&with_pattern), vec![false, false, false, false]);
        let mut star = rules.clone();
        star[0].extensions = vec!["*".into()];
        assert!(shadowed(&star)[1..].iter().all(|s| *s));
        let mut off = rules.clone();
        off[0].enabled = false;
        assert!(!shadowed(&off)[1]);
    }

    #[test]
    fn form_navigation_editing_and_discard_guard() {
        let mut f = rule_form(&valid(), &[]);
        assert_eq!(f.key(key(KeyCode::BackTab)), Out::Idle);
        assert_eq!(f.focus, f.fields.len() - 1);
        f.key(key(KeyCode::Tab));
        assert_eq!(f.focus, 0);
        assert_eq!(f.key(key(KeyCode::Char('x'))), Out::Edited);
        assert!(f.get("Name").ends_with('x') && f.dirty);
        f.key(key(KeyCode::Backspace));
        f.key(key(KeyCode::Tab)); // Enabled (bool)
        f.key(key(KeyCode::Char(' ')));
        assert_eq!(f.get("Enabled"), "false");
        f.key(key(KeyCode::Tab)); // Scope (choice, single option)
        assert_eq!(f.key(key(KeyCode::Right)), Out::Edited);
        assert_eq!(f.get("Scope"), "all folders");
        assert_eq!(f.key(key(KeyCode::Esc)), Out::Armed);
        assert_eq!(f.key(key(KeyCode::Esc)), Out::Cancel);
        let mut clean = rule_form(&valid(), &[]);
        assert_eq!(clean.key(key(KeyCode::Esc)), Out::Cancel);
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(clean.key(ctrl_s), Out::Save);
    }

    #[test]
    fn destination_is_relative_to_the_deepest_watched_folder() {
        use std::path::Path;
        let w = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let rel = |p: &str, v: &[&str]| relative_destination(Path::new(p), &w(v));
        assert_eq!(
            rel("/home/u/downloads/images", &["/home/u/downloads"]),
            "images"
        );
        assert_eq!(
            rel("/home/u/Downloads/images", &["/home/u/downloads"]),
            "/home/u/Downloads/images"
        );
        assert_eq!(
            rel("/downloads-old/images", &["/downloads"]),
            "/downloads-old/images"
        );
        assert_eq!(
            rel("/downloads/work/images", &["/downloads", "/downloads/work"]),
            "images"
        );
        assert_eq!(rel("/downloads", &["/downloads"]), ".");
        assert_eq!(rel("/images", &["/"]), "images");
        assert_eq!(rel("/elsewhere", &[]), "/elsewhere");
    }

    #[test]
    fn choice_cycles_through_actions_both_ways() {
        let mut f = rule_form(&valid(), &[]);
        f.focus = 5;
        f.key(key(KeyCode::Left));
        assert_eq!(f.get("Action"), "ignore");
        f.key(key(KeyCode::Right));
        assert_eq!(f.get("Action"), "move");
    }

    #[test]
    fn settings_roundtrip_and_validation() {
        let base = settings();
        let f = settings_form(&base);
        assert_eq!(build_settings(&f, &base).unwrap().grace_period_seconds, 300);
        let bad = |label: &str, v: &str| {
            let mut f = settings_form(&base);
            set(&mut f, label, v);
            build_settings(&f, &base).unwrap_err()
        };
        assert!(bad("Grace period (seconds)", "-1").contains("grace"));
        assert!(bad("Grace period (seconds)", "x").contains("grace"));
        assert!(bad("Times per day (1-4)", "0").contains("1 to 4"));
        assert!(bad("Times per day (1-4)", "5").contains("1 to 4"));
        assert!(bad("Language", "xx").contains("language"));
        assert!(bad("Time 1 (HH:MM)", "25:99").contains("HH:MM"));
        let mut f = settings_form(&base);
        set(&mut f, "Schedule enabled", "true");
        assert!(build_settings(&f, &base).unwrap_err().contains("required"));
        set(&mut f, "Time 1 (HH:MM)", "09:30");
        set(&mut f, "Language", "pl");
        let s = build_settings(&f, &base).unwrap();
        assert_eq!(
            (
                s.schedule_time_1.as_deref(),
                s.language.as_str(),
                s.theme.as_str()
            ),
            (Some("09:30"), "pl", "dark")
        );
    }
}
