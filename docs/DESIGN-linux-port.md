# Design: Linux TUI + systemd user daemon

Companion to `IMPLEMENTATION_PLAN.md`. Minimal design: signatures only, no implementation.

## 1. Constraints

- Linux only, Rust only, no webview. Runs as the user, never root.
- Same SQLite DB and rule semantics as the existing app (`ProjectDirs("cc","mouzi","mouzi")/mouzi.db`).
- Tauri GUI keeps building from the same core.
- No desktop-environment dependency.

## 2. Components

```
 systemd --user: mouzi.service → `mouzi daemon`
   ├─ FolderWatcher (notify/inotify)      ──► mpsc<Event> ──► notify-send
   ├─ Scheduler (60 s tick)               ──► mpsc<Event>
   └─ 500 ms loop: PRAGMA data_version changed? → watcher.refresh()
                      │
                 SQLite (WAL) ◄── flock: op.lock ──► `mouzi` (TUI, same binary)
```

- **One binary**, `mouzi`: no args = TUI, `mouzi daemon` = service. Same version on both sides, so no protocol and no version skew.
- **No IPC.** The TUI writes the DB; the daemon notices via `data_version` (changes only on commits from *other* connections, which is exactly the TUI). 
- Crates: `mouzi-core` (lib, no Tauri) + `mouzi-linux` (bin).

## 3. Code movement

| Today | Destination | Change |
|---|---|---|
| `db.rs`, `rules.rs`, `operations.rs`, `ignore.rs`, `archive.rs` | `mouzi-core` | `OPERATION_LOCK` → `OpLock`; `init_db` adds WAL + `busy_timeout` |
| `watcher.rs`, `scheduler.rs` | `mouzi-core` | `AppHandle` → `mpsc::Sender<Event>`; notification text built in core |
| `i18n.rs` | `mouzi-core` | unchanged; reused for notification text |
| `commands.rs`, `lib.rs`, `tray.rs`, `integration.rs` | `src-tauri` | thin wrappers over core; the Tauri side drains the channel into `emit` + tray tooltip |

`ignored_files` stays a watcher-internal debounce. Cross-process it is unnecessary: `undo_one` sets `processed_files.restored=1`, and `process_file_in_run` skips restored files with an unchanged fingerprint.

## 4. Interfaces

```rust
pub enum Event {
    FileDetected  { folder: String, file: String },
    FileWarning   { path: String },
    Organized     { count: usize, message: String, dest_folder: Option<String> },
    ScheduledDone { organized: usize },
    PendingCount  (usize),
}
impl FolderWatcher { pub fn new(tx: Sender<Event>) -> Self; /* watch_folders, refresh, flush_manual_to_pending: no AppHandle */ }
impl Scheduler     { pub fn start(&self, tx: Sender<Event>); }

// cross-process; same non-reentrant semantics as the Mutex it replaces
pub struct OpLock;
impl OpLock { pub fn acquire() -> Result<OpGuard, String>; }   // flock on $XDG_RUNTIME_DIR/mouzi/op.lock
```

Daemon drains `Event::Organized` to `notify-send` (best effort, ignore failure) and everything else to `eprintln!`.

## 5. Data ownership

| State | Owner |
|---|---|
| rules, folders, settings, history, `processed_files`, `folder_baseline` | SQLite, both processes |
| grace-period queue | daemon memory, rebuilt by the initial folder scan on start |
| preview plans (`PLANS`) | TUI memory, 30 min TTL; `apply()` revalidates rule + fingerprint + mtime |
| manual-folder "pending" list in the TUI | derived: `operations::preview` over manual folders |
| daemon alive? | `systemctl --user is-active mouzi` |

## 6. Sequences

- **File arrives (silent folder):** inotify → filters → queue (+grace) → tick → re-check mode → `OpLock` → `process_file_in_run("automatic")` → log row → `Event::Organized` → `notify-send`.
- **Rule/folder edit in TUI:** validate → write DB → daemon sees `data_version` change within 500 ms → `refresh()`. Daemon down: persists, applies at next start.
- **Manual organize:** worker thread: `preview` (holds `OpLock`, hashes) → select → `apply` (holds `OpLock`, revalidates) → rows with `trigger="approved"`.
- **Undo:** TUI → `undo_one` under `OpLock`. No daemon involvement.

## 7. Storage and concurrency

- `init_db`: `PRAGMA journal_mode=WAL; busy_timeout=5000`. One connection per process. WAL needs a local filesystem (`$HOME` on NFS unsupported).
- Migrations run inside `OpLock`.
- Single instance: daemon takes `daemon.lock` (flock, same dir); a second `mouzi daemon` exits.
- `preview()` holds `OpLock` while hashing; may stall the daemon on a large folder. Accepted for v1.
- `data_version` is DB-wide, so the daemon also refreshes after TUI undo/apply commits. Harmless: the queue de-duplicates. `// ponytail: coarse refresh, narrow to rules/folders/settings changes if it matters`.

## 8. systemd

Ship `mouzi.service` in the package (deb/rpm install it to `/usr/lib/systemd/user/`; `cargo install` users copy it to `~/.config/systemd/user/`). No `install` subcommand.

```ini
[Unit]
Description=Mouzi file organizer

[Service]
ExecStart=/usr/bin/mouzi daemon
Restart=on-failure

[Install]
WantedBy=default.target
```

Logs: stderr → `journalctl --user -u mouzi`. `SIGTERM`/`SIGINT` (`signal-hook`): stop the loop, drop watchers, let any in-flight move finish (it holds `OpLock`), exit 0. Without `loginctl enable-linger` the service stops at logout (documented).

## 9. TUI

Elm-style `Msg` + pure `update`/`view` so transitions are testable with `TestBackend`; blocking work (`preview`, hashing, DB) runs on spawned threads that send a `Msg` back over `mpsc`.

```rust
struct Model { screen: Screen, /* per-screen state */ }
enum Msg { Key(KeyEvent), Tick, Resize, Loaded(..), OpDone(..) }
fn update(m: &mut Model, msg: Msg, tx: &Sender<Msg>);
fn view(m: &Model, f: &mut Frame);
```

| Screen | Does |
|---|---|
| Status + History | daemon active?, recent log, filter, undo one/selected/all |
| Folders | add (typed path), remove, mode silent/manual/paused, only-new |
| Review | preview → select → apply; warnings/errors |
| Rules | list, reorder priority, `e` = `$EDITOR` on exported JSON → `import_rules(replace)` (`validate_rule` rejects bad input) |

Settings, schedule, and `.mouziignore` are edited as files or via flags; no screens. English only. Keys: `Tab`/`1–4`, `?`, `q`. Terminal restored by a drop guard + panic hook; below 80×24 show "terminal too small".

New deps: `ratatui`, `crossterm`, `signal-hook`, `fs4` (or `rustix`) for flock. Notifications via `notify-send`; logging via `eprintln!`.

## 10. Failure behavior

| Failure | Behavior |
|---|---|
| daemon down | TUI shows "stopped"; DB screens work; edits apply at next start |
| `notify-send` missing | ignored |
| DB locked > 5 s | TUI shows `database busy`; daemon retries next tick |
| inotify limit | `watch_folders` returns the OS error; logged with the `fs.inotify.max_user_watches` hint; other folders keep working |
| folder missing/unmounted | skipped (existing behavior), retried on refresh |
| move fails mid-copy | unchanged: source stays, no log row |

## 11. Decisions and rejected alternatives

| Decision | Rejected | Why |
|---|---|---|
| `data_version` poll | control socket + JSON protocol | no socket, stale-socket handling, or version negotiation |
| mpsc channel | `EventSink` trait + 3 impls | one consumer per front-end |
| `$EDITOR` for rules | form widget + dir picker | largest risk item; export/import already exists |
| flock `OpLock` | rely on `apply()` revalidation | revalidation misses two writers moving the same file |
| Packaged unit file | `service install` subcommand | packages install units natively |
| English-only v1 | TUI i18n loader | avoids the 11-locale parity CI cost |

## 12. Open points

- Whether any core function re-takes `OPERATION_LOCK` while holding it (would deadlock `OpLock`, as it would today's Mutex). None observed in `process_file_in_run`, `preview`, `apply`, `undo_action_cmd`; verify during Stage 1.
- `data_version` behavior with WAL across processes: confirm in the Stage 1 two-connection test.
