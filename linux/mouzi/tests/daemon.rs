use mouzi_core::{db, operations};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn wait_for(path: &Path) -> bool {
    let end = Instant::now() + Duration::from_secs(15);
    while Instant::now() < end {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

// One DB per test process (core's DB is global), so one test covers the daemon flow.
#[test]
fn daemon_organizes_reloads_and_stops_cleanly() {
    let root = std::env::temp_dir().join(format!("mouzi-daemon-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let data_home = root.join("data");
    let watched = root.join("watched");
    let watched2 = root.join("watched2");
    for d in [&watched, &watched2] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::env::set_var("XDG_DATA_HOME", &data_home);
    // Same location the binary derives from XDG_DATA_HOME.
    let dir = data_home.join("mouzi");
    std::fs::create_dir_all(&dir).unwrap();
    db::init_db(dir.clone()).unwrap();
    operations::enable_multi_process(&dir).unwrap();
    let mut s = db::get_settings().unwrap();
    s.first_run = false;
    s.grace_period_seconds = 0;
    s.lock_check_enabled = false;
    db::update_settings(&s).unwrap();
    db::add_watched_folder(watched.to_str().unwrap(), db::FOLDER_MODE_SILENT).unwrap();
    db::insert_default_rules("").unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_mouzi"))
        .arg("daemon")
        .env("XDG_DATA_HOME", &data_home)
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut child = Kill(child);

    // 1. a dropped file is organized by the default Documents rule
    std::fs::write(watched.join("a.pdf"), b"%PDF-1.4 test").unwrap();
    assert!(
        wait_for(&watched.join("Documents/a.pdf")),
        "file not organized"
    );
    let logs = db::get_recent_logs(10).unwrap();
    assert!(logs
        .iter()
        .any(|l| l.file_name == "a.pdf" && l.trigger == "automatic"));

    // 2. a folder added by another process (the TUI) is picked up without restart
    db::add_watched_folder(watched2.to_str().unwrap(), db::FOLDER_MODE_SILENT).unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    std::fs::write(watched2.join("b.pdf"), b"%PDF-1.4 test").unwrap();
    assert!(
        wait_for(&watched2.join("Documents/b.pdf")),
        "reload not applied"
    );

    // 3. a second daemon refuses to start
    let second = Command::new(env!("CARGO_BIN_EXE_mouzi"))
        .arg("daemon")
        .env("XDG_DATA_HOME", &data_home)
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!second.success());

    // 4. SIGTERM exits cleanly
    Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .unwrap();
    let status = child.0.wait().unwrap();
    assert!(status.success(), "exit: {status:?}");
}
