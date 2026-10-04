# Linux TUI + systemd user daemon port

**Goal**: Rust-only Linux front-end (ratatui TUI) and `mouzi daemon` run as a systemd *user* service. No webview, no tray. Tauri GUI stays buildable (Windows). Design: `docs/DESIGN-linux-port.md`.

## Findings (CodeGraph + source)
- `db.rs`, `rules.rs`, `operations.rs`, `ignore.rs`, `archive.rs`: **zero** `tauri` references. Reusable as-is.
- Tauri coupling is limited to `commands.rs`, `lib.rs`, `tray.rs`, `watcher.rs` (7), `integration.rs` (2), `scheduler.rs` (1).
- `db.rs` sets no `journal_mode`/`busy_timeout`: unsafe for two processes.
- `OPERATION_LOCK` is an in-process `Mutex`: daemon and TUI could race on the same file.
- `FolderWatcher` / `PendingFile` have no tests (CodeGraph).

## Architecture
```
Cargo workspace
 ├─ mouzi-core    db, rules, operations, ignore, archive, watcher, scheduler (no tauri)
 ├─ mouzi-linux   one binary: `mouzi` (TUI), `mouzi daemon`
 └─ src-tauri     existing GUI, depends on mouzi-core
```
- Shared SQLite (WAL + `busy_timeout`) at the existing `ProjectDirs` path. No IPC: daemon polls `PRAGMA data_version` and refreshes watchers when the TUI commits.
- `flock` `OpLock` replaces `OPERATION_LOCK`; second flock file gives single-instance.
- Events: `mpsc::Sender<Event>` instead of `tauri::AppHandle`. Daemon: `notify-send` + `eprintln!`.
- Service: packaged `mouzi.service` (`Restart=on-failure`), no install subcommand.

## Stage 1: Core extraction
**Goal**: `mouzi-core` with no Tauri dependency; GUI still builds and behaves the same.
**Success Criteria**: `cargo test -p mouzi-core` green; `cargo build` for `src-tauri` green; watcher/scheduler use `Sender<Event>`; no nested `OPERATION_LOCK` acquisition found (or fixed).
**Tests**: existing unit + `beta_tests` moved and passing; two-connection test (concurrent writes, no `database is locked`, `data_version` changes across connections under WAL); second `OpLock` holder blocks; watcher test with temp dir + channel receiver (closes the coverage gap).
**Status**: Not Started

## Stage 2: `mouzi daemon` + unit file
**Goal**: Headless daemon organizes silent folders and runs the schedule under systemd.
**Success Criteria**: `systemctl --user enable --now mouzi` organizes a dropped file after the grace period; a rule edit in the DB is picked up within ~1 s without restart; second daemon exits; `SIGTERM` exits cleanly after any in-flight move.
**Tests**: integration test with temp XDG dirs: spawn daemon, drop file, assert moved + `action_logs` row; change a rule via a second connection, assert applied; `kill -9` restart checked manually.
**Status**: Not Started

## Stage 3: TUI
**Goal**: Four screens over the shared DB: Status+History (undo), Folders, Review (preview/apply), Rules (list, reorder, `$EDITOR` JSON round-trip via `import_rules`).
**Success Criteria**: full flow in 80×24; daemon-stopped shown, not a crash; terminal restored on panic; bad rule JSON shows `validate_rule` error and leaves rules unchanged.
**Tests**: `TestBackend` snapshots per screen; `update()` unit tests for key → state; preview/apply/undo against a temp dir; rules export → edit → import round-trip.
**Status**: Not Started

## Stage 4: Packaging and docs
**Goal**: Installable and documented.
**Scope**: `cargo-deb` (+ rpm) shipping `mouzi.service` to `/usr/lib/systemd/user/`; README Linux section (incl. `loginctl enable-linger`, inotify limit hint, editing `.mouziignore` / settings); update `docs/ARCHITECTURE.md`; verify `trash` on freedesktop and cross-device moves (ext4 → tmpfs/btrfs).
**Success Criteria**: clean install on Debian/Ubuntu and Fedora; service starts on login.
**Tests**: smoke script.
**Status**: Not Started

## Risks
| Risk | Level | Mitigation |
|---|---|---|
| Moving code out of `src-tauri` breaks GUI | moderate | Stage 1 keeps GUI building at every step; small commits |
| `preview()` holds `OpLock` while hashing; stalls daemon | moderate | accepted v1; hash outside lock if observed |
| `data_version` semantics differ from assumption | moderate | proven in Stage 1 test before relying on it |
| inotify limits / network mounts | moderate | explicit error with hint |
| `$EDITOR` unset or fails | low | fall back to `vi`; leave rules untouched on error |

## Cut for now (add on demand)
Control socket/IPC, TUI i18n, settings/schedule/ignore screens, directory picker, `service install`, `notify-rust`, `env_logger`, rules form widget.

## Decisions assumed (say if wrong)
1. Same DB path as the GUI.
2. Tauri app stays and depends on `mouzi-core`; same repo.
3. Self-update dropped; distribution via packages.
4. Single binary `mouzi`.
