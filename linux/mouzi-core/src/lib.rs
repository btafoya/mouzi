//! Tauri-free core. The modules are the very same files the GUI uses
//! (`src-tauri/src`), included by path so there is one source of truth.
#[path = "../../../src-tauri/src/archive.rs"]
pub mod archive;
#[path = "../../../src-tauri/src/db.rs"]
pub mod db;
#[path = "../../../src-tauri/src/i18n.rs"]
pub mod i18n;
#[path = "../../../src-tauri/src/ignore.rs"]
pub mod ignore;
#[path = "../../../src-tauri/src/operations.rs"]
pub mod operations;
#[path = "../../../src-tauri/src/rules.rs"]
pub mod rules;
