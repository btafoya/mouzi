use directories::{ProjectDirs, UserDirs};
use mouzi_core::{db, operations};
use std::path::PathBuf;

pub fn data_dir() -> Result<PathBuf, String> {
    ProjectDirs::from("cc", "mouzi", "mouzi")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or_else(|| "cannot determine data directory".into())
}

/// Open the shared DB (same file as the GUI), make it multi-process safe and
/// apply first-run defaults. Safe to call from the daemon and the TUI.
pub fn bootstrap() -> Result<PathBuf, String> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    db::init_db(dir.clone()).map_err(|e| e.to_string())?;
    operations::enable_multi_process(&dir)?;
    let settings = db::get_settings().map_err(|e| e.to_string())?;
    if settings.first_run {
        let downloads = UserDirs::new()
            .and_then(|d| d.download_dir().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| "Downloads".into());
        let _ = db::add_watched_folder(&downloads, db::FOLDER_MODE_SILENT);
        let _ = db::insert_default_rules(&downloads);
        let mut s = settings;
        s.first_run = false;
        let _ = db::update_settings(&s);
    }
    Ok(dir)
}
