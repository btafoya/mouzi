mod app;
mod form;
mod picker;
mod ui;

use crate::common;
use app::{reorder, update, Effect, Model, Msg};
use crossterm::event::{self, Event, KeyEventKind};
use mouzi_core::db;
use mouzi_core::{operations, rules};
use ratatui::DefaultTerminal;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

pub fn run() -> Result<(), String> {
    let dir = common::bootstrap()?;
    let mut terminal = ratatui::init(); // also installs a panic hook that restores the terminal
    let result = event_loop(&mut terminal, &dir);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, dir: &Path) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let mut m = Model::new();
    refresh(&mut m);
    spawn_daemon_check(&tx);
    let mut last_tick = Instant::now();
    let mut ticks = 0u32;
    while !m.quit {
        terminal
            .draw(|f| ui::view(&m, f))
            .map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(200)).map_err(|e| e.to_string())? {
            if let Event::Key(k) = event::read().map_err(|e| e.to_string())? {
                if k.kind == KeyEventKind::Press {
                    let effect = update(&mut m, Msg::Key(k));
                    if let Some(effect) = effect {
                        if matches!(effect, Effect::EditRules) {
                            edit_rules(terminal, &mut m, dir)?;
                        } else {
                            exec(effect, &mut m, &tx);
                        }
                        refresh(&mut m);
                    }
                }
            }
        }
        while let Ok(msg) = rx.try_recv() {
            update(&mut m, msg);
            refresh(&mut m);
        }
        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = Instant::now();
            ticks += 1;
            update(&mut m, Msg::Tick);
            refresh(&mut m);
            if ticks.is_multiple_of(3) {
                spawn_daemon_check(&tx);
            }
        }
    }
    Ok(())
}

fn refresh(m: &mut Model) {
    m.logs = db::get_recent_logs(200).unwrap_or_default();
    m.folders = db::get_watched_folders().unwrap_or_default();
    m.rules = db::get_rules().unwrap_or_default();
    if let Ok(s) = db::get_settings() {
        m.load_settings(s);
    }
    m.clamp();
}

fn spawn_daemon_check(tx: &Sender<Msg>) {
    let tx = tx.clone();
    std::thread::spawn(move || {
        let active = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", "mouzi"])
            .status()
            .ok()
            .map(|s| s.success());
        let _ = tx.send(Msg::Daemon(active));
    });
}

fn exec(effect: Effect, m: &mut Model, tx: &Sender<Msg>) {
    let tx = tx.clone();
    let result: Result<String, String> = match effect {
        Effect::AddFolder(path) => add_folder(&path).map(|_| "folder added".into()),
        // appended, never replacing: a picked file must not wipe the existing rules
        Effect::ImportRules(path) => import_rules_file(&path, false)
            .map(|n| format!("imported {n} rule(s) (added to existing)")),
        Effect::RemoveFolder(id) => {
            let _guard = operations::OPERATION_LOCK.lock().unwrap();
            db::remove_watched_folder(id)
                .map(|_| "folder removed".into())
                .map_err(|e| e.to_string())
        }
        Effect::CycleMode(id, mode) => {
            let _guard = operations::OPERATION_LOCK.lock().unwrap();
            db::update_folder_mode(id, &mode)
                .map(|_| format!("mode: {mode}"))
                .map_err(|e| e.to_string())
        }
        Effect::ToggleOnlyNew(id, on) => {
            db::set_only_new(id, on).map(|_| format!("only new files: {on}"))
        }
        Effect::MoveRule(idx, delta) => {
            let _guard = operations::OPERATION_LOCK.lock().unwrap();
            reorder(&m.rules, idx, delta)
                .iter()
                .try_for_each(|r| db::update_rule(r).map_err(|e| e.to_string()))
                .map(|_| {
                    m.sel[3] = (idx as i32 + delta) as usize;
                    "rule moved".into()
                })
        }
        Effect::SaveRule(rule) => {
            let _guard = operations::OPERATION_LOCK.lock().unwrap();
            let new = rule.id.is_none();
            let saved = if new {
                db::add_rule(&rule).map(|_| ())
            } else {
                db::update_rule(&rule)
            };
            saved
                .map(|_| {
                    format!(
                        "rule {}: {}",
                        if new { "added" } else { "saved" },
                        rule.name
                    )
                })
                .map_err(|e| e.to_string())
        }
        Effect::DeleteRule(id) => {
            let _guard = operations::OPERATION_LOCK.lock().unwrap();
            db::delete_rule(id)
                .map(|_| "rule deleted".into())
                .map_err(|e| e.to_string())
        }
        Effect::SaveSettings(s) => db::update_settings(&s)
            .map(|_| "settings saved".into())
            .map_err(|e| e.to_string()),
        Effect::OpenIgnore(folder) => {
            let patterns = mouzi_core::ignore::load_mouziignore(&folder);
            m.mode = app::Mode::Ignore(app::Ignore {
                folder,
                patterns,
                sel: 0,
                input: None,
            });
            Ok(String::new())
        }
        Effect::SaveIgnore { folder, patterns } => {
            mouzi_core::ignore::save_mouziignore(&folder, &patterns)
                .map(|_| "ignore list saved".into())
        }
        Effect::Discard(id) => {
            operations::discard(&id);
            Ok("discarded".into())
        }
        Effect::Preview => {
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Previewed(operations::preview(None)));
            });
            return;
        }
        Effect::Apply { id, selected } => {
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Applied(operations::apply(&id, &selected)));
            });
            return;
        }
        Effect::Undo(ids) => {
            std::thread::spawn(move || {
                let results = ids
                    .into_iter()
                    .map(|id| {
                        let _guard = operations::OPERATION_LOCK.lock().unwrap();
                        operations::undo_one(id)
                    })
                    .collect();
                let _ = tx.send(Msg::Undone(results));
            });
            return;
        }
        Effect::EditRules => unreachable!("handled by the event loop"),
    };
    m.message = result.unwrap_or_else(|e| format!("error: {e}"));
}

/// Mirrors the GUI's add_folder_cmd checks.
fn add_folder(path: &str) -> Result<(), String> {
    let path = std::fs::canonicalize(path.trim()).map_err(|e| e.to_string())?;
    if !path.is_dir() {
        return Err("not an existing folder".into());
    }
    let folders = db::get_watched_folders().map_err(|e| e.to_string())?;
    if folders
        .iter()
        .any(|f| std::fs::canonicalize(&f.path).ok().as_ref() == Some(&path))
    {
        return Err("this folder is already watched".into());
    }
    db::add_watched_folder(&path.to_string_lossy(), db::FOLDER_MODE_SILENT)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Export rules to JSON, edit in $VISUAL/$EDITOR, validate and import (replace).
/// Any error leaves the existing rules untouched.
fn edit_rules(terminal: &mut DefaultTerminal, m: &mut Model, dir: &Path) -> Result<(), String> {
    let file = dir.join("rules-edit.json");
    let json = serde_json::to_string_pretty(&db::get_rules().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::write(&file, json).map_err(|e| e.to_string())?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    ratatui::restore();
    let ran = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&file)
        .status();
    *terminal = ratatui::init();
    m.message = match ran {
        Ok(s) if s.success() => import_rules_file(&file, true),
        Ok(s) => Err(format!("editor exited with {s}")),
        Err(e) => Err(format!("cannot run {editor}: {e}")),
    }
    .map(|n| format!("imported {n} rule(s)"))
    .unwrap_or_else(|e| format!("rules unchanged: {e}"));
    let _ = std::fs::remove_file(&file);
    Ok(())
}

fn import_rules_file(file: &Path, replace: bool) -> Result<usize, String> {
    let data = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let new: Vec<db::Rule> = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    new.iter().try_for_each(rules::validate_rule)?;
    let _guard = operations::OPERATION_LOCK.lock().unwrap();
    db::import_rules(&new, replace)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The core DB is process-global, so every DB-touching check lives in one test.
    #[test]
    fn rules_edit_roundtrip_and_preview_apply_undo() {
        let root = std::env::temp_dir().join(format!("mouzi-tui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let watched = root.join("watched");
        std::fs::create_dir_all(&watched).unwrap();
        db::init_db(root.clone()).unwrap();
        operations::enable_multi_process(&root).unwrap();
        let mut s = db::get_settings().unwrap();
        s.lock_check_enabled = false;
        db::update_settings(&s).unwrap();
        db::insert_default_rules("").unwrap();

        // rules: edited JSON is imported; bad JSON and invalid rules leave rules unchanged
        let file = root.join("rules.json");
        let before = db::get_rules().unwrap();
        let mut edited = before.clone();
        edited[0].name = "Pictures".into();
        std::fs::write(&file, serde_json::to_string(&edited).unwrap()).unwrap();
        assert_eq!(import_rules_file(&file, true).unwrap(), before.len());
        assert_eq!(db::get_rules().unwrap()[0].name, "Pictures");
        std::fs::write(&file, "not json").unwrap();
        assert!(import_rules_file(&file, true).is_err());
        edited[0].name = " ".into();
        std::fs::write(&file, serde_json::to_string(&edited).unwrap()).unwrap();
        assert!(import_rules_file(&file, true).is_err());
        assert_eq!(db::get_rules().unwrap()[0].name, "Pictures");

        // manual folder: preview -> apply -> undo
        db::add_watched_folder(watched.to_str().unwrap(), db::FOLDER_MODE_MANUAL).unwrap();
        std::fs::write(watched.join("a.pdf"), b"%PDF-1.4").unwrap();
        let preview = operations::preview(None).unwrap();
        assert_eq!(preview.entries.len(), 1);
        let ids: Vec<String> = preview.entries.iter().map(|e| e.id.clone()).collect();
        let res = operations::apply(&preview.id, &ids).unwrap();
        assert!(res[0].success, "{:?}", res[0].error);
        assert!(watched.join("Documents/a.pdf").exists() && !watched.join("a.pdf").exists());
        let log = db::get_recent_logs(1).unwrap().remove(0);
        operations::undo_one(log.id.unwrap()).unwrap();
        assert!(watched.join("a.pdf").exists());

        // editor effects write through to the DB
        let (tx, _rx) = mpsc::channel();
        let mut m = Model::new();
        let mut r = form::blank_rule(99);
        r.name = "Ebooks".into();
        r.extensions = vec!["epub".into()];
        r.destination = "Books".into();
        exec(Effect::SaveRule(r), &mut m, &tx);
        assert!(m.message.starts_with("rule added"), "{}", m.message);
        let mut saved = db::get_rules()
            .unwrap()
            .into_iter()
            .find(|x| x.name == "Ebooks")
            .unwrap();
        assert_eq!(saved.priority, 99);
        saved.enabled = false;
        exec(Effect::SaveRule(saved.clone()), &mut m, &tx);
        assert!(
            !db::get_rules()
                .unwrap()
                .iter()
                .find(|x| x.name == "Ebooks")
                .unwrap()
                .enabled
        );
        exec(Effect::DeleteRule(saved.id.unwrap()), &mut m, &tx);
        assert!(db::get_rules().unwrap().iter().all(|x| x.name != "Ebooks"));
        let mut s = db::get_settings().unwrap();
        s.grace_period_seconds = 42;
        exec(Effect::SaveSettings(s), &mut m, &tx);
        assert_eq!(db::get_settings().unwrap().grace_period_seconds, 42);
        let folder = watched.to_string_lossy().to_string();
        let patterns = vec!["*.tmp".to_string(), "node_modules/".to_string()];
        exec(
            Effect::SaveIgnore {
                folder: folder.clone(),
                patterns: patterns.clone(),
            },
            &mut m,
            &tx,
        );
        exec(Effect::OpenIgnore(folder), &mut m, &tx);
        assert!(matches!(&m.mode, app::Mode::Ignore(ig) if ig.patterns == patterns));

        // add_folder mirrors the GUI checks
        assert!(add_folder(watched.to_str().unwrap()).is_err());
        assert!(add_folder("/definitely/not/here").is_err());
    }
}
