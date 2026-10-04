use mouzi_core::{db, operations};
use std::fs::File;
use std::time::Duration;

// One DB per test process (init_db is global), so everything runs in one test.
#[test]
fn two_handles_share_db_and_oplock() {
    let dir = std::env::temp_dir().join(format!("mouzi-mp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    db::init_db(dir.clone()).unwrap();
    operations::enable_multi_process(&dir).unwrap();

    // WAL is on.
    let mode: String = db::get_db()
        .lock()
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");

    // A second connection's commit bumps data_version on ours (the daemon's reload signal).
    let before = operations::db_version().unwrap();
    let other = rusqlite::Connection::open(dir.join("mouzi.db")).unwrap();
    other.busy_timeout(Duration::from_secs(5)).unwrap();
    db::add_watched_folder("/tmp/mouzi-mp-x", db::FOLDER_MODE_SILENT).ok();
    other
        .execute("UPDATE settings SET language='pl'", [])
        .unwrap();
    assert_ne!(operations::db_version().unwrap(), before);

    // OpLock holds a real file lock: another open file description cannot take it.
    let guard = operations::OPERATION_LOCK.lock().unwrap();
    let probe = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("op.lock"))
        .unwrap();
    assert!(matches!(
        probe.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(guard);
    probe.try_lock().unwrap();
}
