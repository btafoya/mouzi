//! `mouzi daemon`: watches silent folders, organizes after the grace period, runs the
//! schedule, and re-reads the DB when another process (the TUI) commits a change.
use crate::common;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime};
use mouzi_core::db::{self, AppSettings};
use mouzi_core::{i18n::TrayI18n, operations, rules};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::fs::{self, TryLockError};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

pub struct Organized {
    pub file: String,
    pub rule: String,
    pub message: Option<String>,
}

struct Daemon {
    tx: Sender<PathBuf>,
    watchers: Vec<RecommendedWatcher>,
    pending: HashMap<PathBuf, Instant>,
    silent: HashSet<String>,
}

pub fn run() -> Result<(), String> {
    let dir = common::bootstrap()?;
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("daemon.lock"))
        .map_err(|e| e.to_string())?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err("mouzi daemon is already running".into()),
        Err(TryLockError::Error(e)) => return Err(e.to_string()),
    }

    let stop = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(sig, stop.clone()).map_err(|e| e.to_string())?;
    }

    let (tx, rx) = mpsc::channel();
    let mut daemon = Daemon {
        tx,
        watchers: Vec::new(),
        pending: HashMap::new(),
        silent: HashSet::new(),
    };
    daemon.reload();
    eprintln!(
        "[daemon] started, watching {} folder(s)",
        daemon.silent.len()
    );

    let mut version = operations::db_version()?;
    let mut last_slots = HashMap::new();
    let mut last_sched = Instant::now() - StdDuration::from_secs(60);
    while !stop.load(Ordering::Relaxed) {
        daemon.pump(&rx, StdDuration::from_millis(500));
        match operations::db_version() {
            // ponytail: any commit by another process reloads; narrow to rules/folders/settings if it matters
            Ok(v) if v != version => {
                version = v;
                eprintln!("[daemon] database changed, reloading");
                daemon.reload();
            }
            Ok(_) => {}
            Err(e) => eprintln!("[daemon] database: {e}"),
        }
        daemon.process_due();
        if last_sched.elapsed() >= StdDuration::from_secs(10) {
            last_sched = Instant::now();
            if let Ok(settings) = db::get_settings() {
                if !due_slots(&settings, Local::now(), &mut last_slots).is_empty() {
                    scheduled_clean();
                }
            }
        }
    }
    eprintln!("[daemon] stopping");
    Ok(())
}

impl Daemon {
    fn reload(&mut self) {
        self.watchers.clear();
        self.pending.clear();
        self.silent.clear();
        for folder in db::get_watched_folders().unwrap_or_default() {
            if !folder.enabled || !db::is_folder_auto_mode(&folder.mode) {
                continue;
            }
            if !Path::new(&folder.path).is_dir() {
                eprintln!("[daemon] skipping missing folder: {}", folder.path);
                continue;
            }
            let tx = self.tx.clone();
            let watcher = RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    if let Ok(ev) = res {
                        if matches!(
                            ev.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        ) {
                            for p in ev.paths {
                                let _ = tx.send(p);
                            }
                        }
                    }
                },
                Config::default(),
            )
            .and_then(|mut w| {
                w.watch(Path::new(&folder.path), RecursiveMode::NonRecursive)
                    .map(|_| w)
            });
            match watcher {
                Ok(w) => self.watchers.push(w),
                // inotify limit hit: fs.inotify.max_user_watches
                Err(e) => {
                    eprintln!("[daemon] cannot watch {}: {e}", folder.path);
                    continue;
                }
            }
            self.silent.insert(folder.path.clone());
            // Files already present are organized too (after the grace period).
            if let Ok(entries) = fs::read_dir(&folder.path) {
                for entry in entries.flatten() {
                    self.enqueue(entry.path());
                }
            }
        }
    }

    fn pump(&mut self, rx: &Receiver<PathBuf>, wait: StdDuration) {
        if let Ok(path) = rx.recv_timeout(wait) {
            self.enqueue(path);
            while let Ok(path) = rx.try_recv() {
                self.enqueue(path);
            }
        }
    }

    fn enqueue(&mut self, path: PathBuf) {
        db::forget_removed_baseline(&path);
        let parent = path.parent().map(|p| p.to_string_lossy().to_string());
        if !path.is_file()
            || rules::should_ignore_file(&path)
            || rules::is_file_ignored_by_mouziignore(&path)
            || db::is_baseline_file(&path)
            || !parent.is_some_and(|p| self.silent.contains(&p))
        {
            return;
        }
        let grace = db::get_settings()
            .map(|s| s.grace_period_seconds.max(0) as u64)
            .unwrap_or(300);
        self.pending
            .insert(path, Instant::now() + StdDuration::from_secs(grace));
    }

    fn process_due(&mut self) {
        let now = Instant::now();
        let due: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, at)| now >= **at)
            .map(|(p, _)| p.clone())
            .collect();
        if due.is_empty() {
            return;
        }
        let run_id = operations::new_run_id();
        let mut done = Vec::new();
        for path in due {
            self.pending.remove(&path);
            if !path.is_file() {
                continue;
            }
            if operations::suspicious_name(&path) {
                eprintln!(
                    "[daemon] suspicious name, left for review: {}",
                    path.display()
                );
                continue;
            }
            match rules::process_file_in_run(&path, true, &run_id, "automatic") {
                Ok(Some((rule, dest))) => {
                    let file = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let destination = if rule.action == "delete" {
                        "Recycle Bin".to_string()
                    } else {
                        dest.unwrap_or_default()
                    };
                    eprintln!("[daemon] {file} -> {destination} ({})", rule.name);
                    let message = rule
                        .notification_message
                        .as_deref()
                        .filter(|m| !m.trim().is_empty())
                        .map(|m| render_message(m, &file, &rule.name, &destination));
                    done.push(Organized {
                        file,
                        rule: rule.name,
                        message,
                    });
                }
                Ok(None) => {}
                Err(e) if e == "No matching rule" => {}
                Err(e) => eprintln!("[daemon] {}: {e}", path.display()),
            }
        }
        if !done.is_empty() {
            let lang = db::get_settings()
                .map(|s| s.language)
                .unwrap_or_else(|_| "en".into());
            // best effort: no notification daemon, no notification
            let _ = Command::new("notify-send")
                .args([
                    "--app-name=Mouzi",
                    "--", // file names and rule messages are untrusted: never parsed as options
                    "Mouzi",
                    &notification_body(&done, &lang),
                ])
                .status();
        }
    }
}

fn scheduled_clean() {
    let run_id = operations::new_run_id();
    let mut total = 0;
    for folder in db::get_watched_folders().unwrap_or_default() {
        if !folder.enabled
            || !db::is_folder_auto_mode(&folder.mode)
            || !Path::new(&folder.path).is_dir()
        {
            continue;
        }
        match rules::scan_folder_in_run(&folder.path, &run_id, "scheduled") {
            Ok(r) => total += r.len(),
            Err(e) => eprintln!("[daemon] scheduled clean of {} failed: {e}", folder.path),
        }
    }
    eprintln!("[daemon] scheduled run organized {total} file(s)");
}

fn render_message(template: &str, file: &str, rule: &str, destination: &str) -> String {
    template
        .replace("{file}", file)
        .replace("{rule}", rule)
        .replace("{destination}", destination)
}

pub fn notification_body(done: &[Organized], lang: &str) -> String {
    match done {
        [one] => one
            .message
            .clone()
            .unwrap_or_else(|| format!("{} → {}", one.file, one.rule)),
        many => TrayI18n::new(lang)
            .get("organized")
            .replace("{}", &many.len().to_string()),
    }
}

/// Schedule slots that fire now (within a minute after their time, once per day).
pub fn due_slots(
    s: &AppSettings,
    now: DateTime<Local>,
    last: &mut HashMap<usize, NaiveDate>,
) -> Vec<usize> {
    if !s.schedule_enabled {
        return Vec::new();
    }
    let times = [
        &s.schedule_time_1,
        &s.schedule_time_2,
        &s.schedule_time_3,
        &s.schedule_time_4,
    ];
    let today = now.date_naive();
    let mut due = Vec::new();
    let count = s.schedule_times_per_day.clamp(1, 4) as usize;
    for (slot, time) in times.iter().enumerate().take(count) {
        let Some(t) = time.as_deref().and_then(parse_time) else {
            continue;
        };
        let Some(at) = today.and_time(t).and_local_timezone(Local).single() else {
            continue;
        };
        let diff = now.signed_duration_since(at);
        if diff >= Duration::zero()
            && diff < Duration::minutes(1)
            && last.get(&slot) != Some(&today)
        {
            last.insert(slot, today);
            due.push(slot);
        }
    }
    due
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s.trim(), "%H:%M:%S"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn org(file: &str, message: Option<&str>) -> Organized {
        Organized {
            file: file.into(),
            rule: "Docs".into(),
            message: message.map(Into::into),
        }
    }

    #[test]
    fn single_file_uses_custom_message_or_default() {
        assert_eq!(
            notification_body(&[org("a.pdf", None)], "en"),
            "a.pdf → Docs"
        );
        assert_eq!(notification_body(&[org("a.pdf", Some("hi"))], "en"), "hi");
        assert_eq!(
            render_message("{file}>{rule}>{destination}", "f", "r", "d"),
            "f>r>d"
        );
    }

    #[test]
    fn many_files_use_count_summary() {
        let body = notification_body(&[org("a", None), org("b", None)], "en");
        assert_eq!(body, "Organized 2 file(s)");
    }

    #[test]
    fn schedule_fires_once_within_the_minute() {
        let mut s = mouzi_settings();
        s.schedule_enabled = true;
        s.schedule_times_per_day = 1;
        s.schedule_time_1 = Some("09:30".into());
        let at = |h, m, sec| Local.with_ymd_and_hms(2026, 10, 4, h, m, sec).unwrap();
        let mut last = HashMap::new();
        assert!(due_slots(&s, at(9, 29, 59), &mut last).is_empty());
        assert_eq!(due_slots(&s, at(9, 30, 5), &mut last), vec![0]);
        assert!(due_slots(&s, at(9, 30, 15), &mut last).is_empty());
        assert!(due_slots(&s, at(9, 31, 0), &mut last).is_empty());
        s.schedule_enabled = false;
        assert!(due_slots(&s, at(9, 30, 5), &mut HashMap::new()).is_empty());
    }

    fn mouzi_settings() -> AppSettings {
        AppSettings {
            id: None,
            language: "en".into(),
            theme: "system".into(),
            telemetry_enabled: false,
            first_run: false,
            autostart: false,
            grace_period_seconds: 0,
            lock_check_enabled: false,
            auto_update_enabled: false,
            schedule_enabled: false,
            schedule_times_per_day: 1,
            schedule_time_1: None,
            schedule_time_2: None,
            schedule_time_3: None,
            schedule_time_4: None,
        }
    }
}
