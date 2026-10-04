# Mouzi Architecture

Backend reference for `src-tauri/src/`. Frontend is React 19 + Zustand talking to Rust over Tauri `invoke` commands and events.

## Module map

| Module | Responsibility |
|---|---|
| `lib.rs` | `run()`: plugins, `AppState`, DB init, first-run defaults, tray, watcher + scheduler start, `invoke_handler` registration |
| `commands.rs` | Thin `#[tauri::command]` wrappers (`*_cmd`) over `db`, `rules`, `operations`, `archive`, `ignore`, `integration` |
| `db.rs` | SQLite (rusqlite) schema, migrations, CRUD for rules / folders / logs / settings; folder-mode constants |
| `rules.rs` | File scanning, rule matching, the single-file pipeline `process_file_in_run`, folder scan |
| `operations.rs` | Destination planning, collision-safe move, preview/apply workflow, undo, history queries |
| `watcher.rs` | `notify` watchers per folder, debounce queue, manual-mode collection, notifications |
| `scheduler.rs` | 60 s ticker that runs `scan_folder_in_run(..., "scheduled")` at configured times |
| `ignore.rs` | `.mouziignore` load/save/glob matching |
| `archive.rs` | Google Takeout style zip/tgz extraction into a staging dir (zip-slip safe, flattened, collision-renamed) |
| `tray.rs` / `i18n.rs` | Tray menu, popup/settings window placement, tray-only translations |
| `integration.rs` | Windows Explorer context menu (`HKCU\...\Directory\shell\Mouzi`) and folder CLI args |

## Runtime state

`AppState` (managed by Tauri):

- `watcher: Arc<Mutex<FolderWatcher>>`
- `ignored_files` — path → `Instant`; suppresses re-processing for 5 s (`IGNORE_DURATION_SECS`)
- `pending_open_folder` — destination to open when a notification click re-activates the app
- `scheduler`
- `review_request`, `add_folder_request` — one-shot handoffs from Explorer integration to the UI (`take_*_cmd`)

The DB is a process-wide `OnceCell<Arc<Mutex<Connection>>>` (`db::get_db()`), stored at `ProjectDirs("cc","mouzi","mouzi")/mouzi.db` (`mouzi-beta` for beta builds).

## Data model (`db.rs`)

| Table | Notes |
|---|---|
| `watched_folders` | `path` unique, `enabled`, `mode` (`silent` \| `manual` \| `paused`), `only_new` |
| `rules` | `priority`, `extensions` (normalized), `pattern` (regex), `destination` (template), `action` (`move` \| `rename` \| `delete` \| `ignore`), `folder_id` (0 = all folders), size/date options, `rename_template`, `normalize_extensions` + `extension_mappings` (default `jpeg:jpg`), `notification_message` |
| `action_logs` | History: source, destination, action, `run_id`, `trigger`, `fingerprint` (SHA-256), `undone` |
| `processed_files` | path → fingerprint; prevents re-organizing unchanged files |
| `folder_baseline` | Files present when `only_new` was enabled; never touched |
| `settings` | language, theme, autostart, `grace_period_seconds` (default 300), `lock_check_enabled`, schedule times (1–4/day) |

Migrations are additive and idempotent (`migrate_beta_schema`, `migrate_rules_to_relative`).

## Folder modes

| Mode | Watched | Auto-organizes | Scheduled run |
|---|---|---|---|
| `silent` | yes | yes (after grace period) | yes |
| `manual` | yes | no — files collect in `pending_manual` | no |
| `paused` | no | no | no |

Switching manual → silent calls `FolderWatcher::flush_manual_to_pending`.

## Pipelines

### Automatic (watcher)

```
notify event ──► should_ignore_file / .mouziignore / baseline filter
             ──► manual folder or suspicious_name?  yes ► pending_manual + "file-detected"/"file-warning" event
             └─► pending queue (scheduled = now + grace_period)
500 ms loop ──► due items ──► re-check folder mode ──► process_file_in_run(path, bypass_grace=true, run_id, "automatic")
             ──► toast notification (non-Windows via plugin) + tray tooltip
```

`watch_folders` also does an initial scan of existing files so pre-existing files are not ignored forever.

### `rules::process_file_in_run` (single file)

Holds `OPERATION_LOCK` for the whole call. Early-returns `Ok(None)` when:

1. trigger is `automatic`/`scheduled` and the parent folder is not enabled + silent
2. file is in the `only_new` baseline
3. grace period not elapsed (unless bypassed) or file is locked (`lock_check_enabled`)
4. matched by `.mouziignore`
5. `already_processed` (same path + same fingerprint)
6. matching rule has `action = ignore`, or `suspicious_name` (e.g. `invoice.pdf.exe`)

Otherwise: `find_matching_rule` (first enabled rule by priority whose `folder_id` is 0 or the file's folder, and whose extension/regex/size/date options match) → `fingerprint` → `execute_rule` → `log_action`.

### Preview / apply (manual "Organize Now")

1. `operations::preview(paths?)` — plans every candidate, reserving destination names in a shared set so batch collisions get ` (1)`, ` (2)` suffixes. Caches a `CachedPlan` (max 32, 30 min TTL) holding rule, fingerprint, and mtime per item.
2. UI shows `PreviewEntry` rows (`warning`, `error` codes such as `preview.locked`).
3. `apply(id, selected)` — re-validates each selected item (same rule JSON, same fingerprint, same mtime; else `preview.changed`), executes, logs with `trigger = "approved"`.
4. `discard(id)` drops the plan.

### Move safety (`operations::move_without_overwrite`)

Regular files only. Destination is reserved atomically (no overwrite), cross-volume moves copy then remove the source; a failed copy leaves the source, a failed removal keeps both. `delete` goes to the OS trash via the `trash` crate. Rename templates support `{stem}`, `{filename}`, `{extension}`, `{year}`, `{month}`, `{day}`; unsafe names fail with `validation.template`.

### Undo

`undo_one(id)` only for `move`/`rename` logs that have a fingerprint: errors `history.unverified` (no fingerprint), `history.conflict` (original path occupied), `history.changed` (destination content differs). `undo_all_cmd` / `undo_selected_cmd` build on it; restored files are flagged so they can be re-organized.

### Scheduled

`Scheduler::start` wakes every 60 s; for each configured `HH:MM` reached since the last tick (tracked per slot per local date to avoid double-fire) it runs `scan_folder_in_run(folder, run_id, "scheduled")` over silent folders, then emits `scheduled-clean-done { organized }`.

## `.mouziignore`

Per-folder file, one pattern per line. Supports `#` comments (`\#` escapes), literal names, any number of `*` wildcards, and `dir/` suffix. Case-insensitive on Windows only. `rules::is_file_ignored_by_mouziignore` is the single check shared by watcher, preview, and manual scan.

## Tauri surface

- **Commands** — registered in `lib.rs` (`generate_handler!`), implemented in `commands.rs`. Groups: rules (`get/add/update/delete_rule_cmd`, `export/import_rules_cmd`), folders (`*_folder_cmd`, `update_folder_mode_cmd`, `set_only_new_cmd`, `refresh_watcher_cmd`), organize (`scan_folder_cmd`, `scan_selected_files_cmd`, `preview_cmd`, `apply_preview_cmd`, `discard_preview_cmd`, `import_archive_cmd`), history (`history_cmd`, `get_logs_cmd`, `undo_*_cmd`, `clear_logs_cmd`, `get_stats_cmd`), settings/schedule/autostart, ignore (`load/save_mouziignore_cmd`), windows (`show_popup_cmd`, `show_review_cmd`, `close_*`), integration (`explorer_integration_*`).
- **Events emitted** — `file-detected`, `file-warning`, `scheduled-clean-done`, `open-review`, `add-folder-request`.

## Frontend

`src/App.tsx` picks the window view from `location.hash` (`#/popup` default, settings, review); state lives in `src/store/useAppStore.ts`; strings in `src/i18n/locales/*.json` (11 locales, parity enforced by `tests/*.test.mjs`); `src/utils/rulePath.ts` converts picked destination folders into watched-folder-relative rule values; `ruleInput.ts` handles rule form input.

## Tests

- Rust: unit tests inline in `ignore.rs`, `archive.rs`, `rules.rs`, `db.rs` (migrations); integration-style `beta_tests.rs`. Run `cargo test` in `src-tauri/`.
- JS: `bun run test` → `node --test tests/*.test.mjs` (locale parity etc.).
- CodeGraph flags `FolderWatcher` and `PendingFile` as having no covering tests.
