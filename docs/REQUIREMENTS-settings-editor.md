# Requirements: TUI settings editor

Scope decided with the user: **everything**: rules, app settings, folders, `.mouziignore`. Rule editor is a **single form**. New rules append at the **bottom**. Replaces the "edit everything as JSON" path as the primary UI; JSON edit stays as an escape hatch.

## Goals
- Create, read, update, delete every user-editable entity without leaving the TUI or touching JSON.
- Every field is validated as typed, using the core validators, so nothing invalid reaches the DB.
- Priority order is visible and safe to change (first matching rule wins).

## Entities and CRUD

| Entity | Create | Read | Update | Delete | Extra |
|---|---|---|---|---|---|
| **Rule** | form (appends last) | list + detail | form | confirm | duplicate, enable/disable toggle, reorder (J/K), JSON escape hatch (`e`) |
| **Folder** | add path (exists today) | list | mode, only-new (exists today); edit scope notes below | confirm | |
| **App settings** | n/a (singleton) | Settings screen | form | n/a | reset field to default |
| **Ignore pattern** (per folder) | add line | list per folder | edit line | delete line | folder picker, comment lines preserved |

## Screen and key flow (order)

1. **Rules list** → `n` new · `Enter` edit · `c` duplicate · `d` delete (y/n) · `Space` enable/disable · `J`/`K` reorder · `e` JSON.
2. **Rule form**, fixed field order, `Tab`/`Shift-Tab` move, `Ctrl-S` save, `Esc` cancel (confirm if dirty):
   1. Name
   2. Enabled
   3. Scope (all folders, or one watched folder)
   4. Extensions (comma list; `*` allowed)
   5. Pattern (regex, optional)
   6. Action (move / rename / delete / ignore; cycle with ←/→)
   7. Destination (template: `{year}` `{month}` `{day}` `{extension}` `{filename}`; hidden for delete/ignore)
   8. Rename template (shown only for `rename`)
   9. Size min / max, modified after / before (optional)
   10. Extension normalization + mappings (optional)
   11. Notification message (optional; `{file}` `{rule}` `{destination}`)
3. **Folders** → existing keys plus `Enter` = folder detail (mode, only-new, ignore list).
4. **Ignore list** (per folder) → `n` add · `Enter` edit · `d` delete · `Ctrl-S` write `.mouziignore`.
5. **Settings** → fields: grace period (seconds), lock check, language, schedule enabled, times per day (1–4), schedule time 1–4 (`HH:MM`). `Ctrl-S` saves.

Tab order: Status · Folders · Review · Rules · Settings.

## Functional requirements
- **FR1** Form shows per-field validation errors inline and blocks save while any exist.
- **FR2** Rule save reuses `validate_rule` and `validate_template`; settings save validates `HH:MM`, 1–4 slots, non-negative grace period, language ∈ the 11 shipped locales.
- **FR3** New and duplicated rules get the next priority (bottom); duplicates are named `<name> (copy)`.
- **FR4** The rule list marks a rule as **shadowed** when an earlier enabled rule with the same folder scope already covers every one of its extensions.
- **FR5** Destructive actions (delete rule, folder, ignore line) ask for `y` confirmation.
- **FR6** Dirty form + `Esc` asks before discarding.
- **FR7** Writes take `OPERATION_LOCK`, like the GUI commands. The daemon picks changes up through `data_version`; no extra signaling.
- **FR8** `.mouziignore` edits preserve comments and ordering of untouched lines (`save_mouziignore` currently rewrites a header plus patterns; see open questions).
- **FR9** Pressing `Esc`/`q` never leaves the terminal in raw mode; panics restore it (already true).

## Non-functional
- Works at 80×24; form scrolls when fields don't fit.
- Input handling stays DB-free and unit-testable (`update()` → `Effect`), as today.
- English only (unchanged). No new dependencies beyond what a text-field needs (hand-rolled single-line input first).

## Acceptance criteria
- Create a rule through the form → appears last, daemon organizes a matching file within ~1 s of the save plus grace.
- Enter an invalid regex, empty name, or bad destination template → inline error, save blocked, DB unchanged.
- Duplicate, toggle, reorder and delete a rule; priorities stay a contiguous 1..n.
- Change grace period and a schedule time in Settings → persisted; invalid `25:99` rejected.
- Add, edit and delete an ignore pattern → `.mouziignore` on disk matches; ignored file is skipped by the daemon.
- A shadowed rule is flagged in the list.
- `cargo test --workspace` covers: form state machine (field order, dirty/cancel), each validator path, shadow detection, ignore-list CRUD round trip, settings round trip.

## Gaps found in the core (affect scope)
- There is no DB function to edit a folder's path or toggle `enabled`; "paused" mode covers the latter. Editing a path would be remove + add, which orphans rules scoped to that folder (`folder_id`). Proposal: no path edit in v1.
- Deleting a folder leaves rules with a dangling `folder_id` that can never match. The editor should warn or offer to delete or re-scope them.
- `save_mouziignore` writes a generated header plus patterns, so hand-written comments are lost on save (FR8 conflict).

## Open questions
1. Folder deletion with dependent rules: warn only, re-scope them to "all folders", or delete them?
2. Ignore editor: accept losing hand-written comments, or extend the core writer to preserve them?
3. Keep the JSON escape hatch (`e`) permanently, or remove once the form is complete?
4. Should destination get a directory picker later (currently typed text)?

**Next step:** `/sc:design` (form state machine and widgets), then `/sc:workflow` for staging.

## As built
Implemented in `linux/mouzi/src/tui/{form,app,ui,mod}.rs`. Differences from the spec above:
- **Validation display:** one error line under the form (first failing rule) instead of per-field marks; save is blocked while it shows. Messages are readable translations of the core's `validation.*` codes.
- **Conditional fields:** all rule fields are always shown (no hiding destination for delete/ignore or rename template for non-rename).
- **Folders:** no detail screen; `i` opens the ignore list, `m`/`n` change mode/only-new. Deleting a folder asks for `y` and says how many rules are scoped to it (open question 1: warn only).
- **Ignore list:** each add/edit/delete is written to `.mouziignore` immediately (no `Ctrl-S`); deleting a single pattern has no confirmation. Hand-written comments are still lost on save (core writer unchanged).
- **Settings screen:** its form captures all keys (so `q` types); `Esc` leaves, `Esc` twice if there are unsaved edits.
- **JSON escape hatch** (`e`) kept (open question 3).
- **Rule duplicate/toggle** save immediately; **delete** and **undo-all** use a `y` prompt.
- Shadow detection is conservative: only an earlier enabled rule with no pattern/size/date limits counts.
