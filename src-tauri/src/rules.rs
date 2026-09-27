use crate::db::{
    get_rules, get_settings, get_watched_folders, is_folder_auto_mode, log_action, ActionLog, Rule,
};
use crate::ignore::{is_ignored, load_mouziignore};
use chrono::Utc;
use regex::Regex;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub fn validate_rule(rule: &Rule) -> Result<(), String> {
    if rule.name.trim().is_empty() {
        return Err("validation.name".into());
    }
    let extensions = crate::db::normalize_extensions(&rule.extensions);
    if extensions.is_empty()
        || extensions.iter().any(|ext| {
            ext != "*"
                && !ext
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
    {
        return Err("validation.extensions".into());
    }
    if let Some(pattern) = &rule.pattern {
        Regex::new(pattern).map_err(|_| "validation.pattern")?;
    }
    if !["move", "rename", "delete", "ignore"].contains(&rule.action.as_str()) {
        return Err("validation.action".into());
    }
    if rule.action == "move"
        && (rule.destination.trim().is_empty()
            || Path::new(&rule.destination)
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir)))
    {
        return Err("validation.destination".into());
    }
    if rule
        .options
        .min_size
        .zip(rule.options.max_size)
        .is_some_and(|(a, b)| a > b)
    {
        return Err("validation.size".into());
    }
    for date in [&rule.options.modified_after, &rule.options.modified_before]
        .into_iter()
        .flatten()
    {
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "validation.date")?;
    }
    if rule
        .options
        .modified_after
        .as_ref()
        .zip(rule.options.modified_before.as_ref())
        .is_some_and(|(a, b)| a > b)
    {
        return Err("validation.date".into());
    }
    if rule.action == "rename" {
        crate::operations::validate_template(&rule.options.rename_template)?;
    }
    Ok(())
}

/// Check if a file is currently locked by another process.
/// On Windows this tries to open with write access; if another process holds
/// the file without FILE_SHARE_WRITE the open will fail.
pub(crate) fn is_file_locked(path: &Path) -> bool {
    match fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        Err(_) => true,
    }
}

/// Check whether a file has passed its grace period since last modification.
/// Returns true if the file is ready to be moved.
fn check_grace_period(path: &Path, grace_seconds: i64) -> bool {
    if grace_seconds <= 0 {
        return true;
    }
    if let Ok(metadata) = fs::metadata(path) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(elapsed) = modified.elapsed() {
                return elapsed.as_secs() >= grace_seconds as u64;
            }
        }
    }
    true
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub extension: String,
    pub size: u64,
}

fn extension_from_magic_bytes(path: &Path) -> Option<String> {
    let mut header = [0_u8; 16];
    let mut file = fs::File::open(path).ok()?;
    let bytes_read = file.read(&mut header).ok()?;
    let header = &header[..bytes_read];

    if header.starts_with(b"MZ")
        || header.starts_with(b"\x7FELF")
        || header.starts_with(&[0xFE, 0xED, 0xFA, 0xCE])
        || header.starts_with(&[0xFE, 0xED, 0xFA, 0xCF])
        || header.starts_with(&[0xCF, 0xFA, 0xED, 0xFE])
        || header.starts_with(&[0xCE, 0xFA, 0xED, 0xFE])
    {
        return Some("exe".to_string());
    }

    infer::get(header).map(|kind| kind.extension().to_lowercase())
}

pub fn should_ignore_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    let ignored_names = [
        "desktop.ini",
        "thumbs.db",
        "ntuser.dat",
        "ntuser.ini",
        "boot.ini",
        "bootmgr",
        "pagefile.sys",
        "hiberfil.sys",
        "swapfile.sys",
        "autorun.inf",
        "config.sys",
        "io.sys",
        "msdos.sys",
        "command.com",
        "ntldr",
        "bootsect.bak",
    ];
    if ignored_names.contains(&name.as_str()) {
        return true;
    }

    // Browser temporary download files
    let temp_extensions = [".crdownload", ".part", ".download", ".tmp"];
    for ext in &temp_extensions {
        if name.ends_with(ext) {
            return true;
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(metadata) = fs::metadata(path) {
            let attrs = metadata.file_attributes();
            const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
            const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
            if attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
                return true;
            }
        }
    }

    if name.starts_with('.') {
        return true;
    }

    false
}

/// Check whether a file is ignored by the `.mouziignore` in its parent folder.
/// This is the single helper used by both the watcher and the manual Organize Now path
/// so both code paths behave identically.
pub fn is_file_ignored_by_mouziignore(path: &Path) -> bool {
    if let Some(parent) = path.parent() {
        let patterns = load_mouziignore(&parent.to_string_lossy());
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            return is_ignored(name, &patterns);
        }
    }
    false
}

pub fn scan_file(path: &Path) -> Option<FileInfo> {
    if !fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    if should_ignore_file(path) {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    let path_extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let extension = if path_extension.is_empty() {
        extension_from_magic_bytes(path).unwrap_or_default()
    } else {
        path_extension
    };
    Some(FileInfo {
        path: path.to_path_buf(),
        name,
        extension,
        size: metadata.len(),
    })
}

pub(crate) fn matches_rule(file: &FileInfo, rule: &Rule) -> bool {
    if !rule.enabled {
        return false;
    }

    let extensions = crate::db::normalize_extensions(&rule.extensions);
    let ext_matches = extensions
        .iter()
        .any(|ext| ext == "*" || (!file.extension.is_empty() && ext == &file.extension));
    if rule.options.min_size.is_some_and(|min| file.size < min)
        || rule.options.max_size.is_some_and(|max| file.size > max)
    {
        return false;
    }
    if rule.options.modified_after.is_some() || rule.options.modified_before.is_some() {
        let Some(day) = fs::metadata(&file.path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|time| chrono::DateTime::<Utc>::from(time).date_naive())
        else {
            return false;
        };
        for (value, after) in [
            (&rule.options.modified_after, true),
            (&rule.options.modified_before, false),
        ] {
            if let Some(value) = value {
                let Ok(bound) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") else {
                    return false;
                };
                if (after && day < bound) || (!after && day > bound) {
                    return false;
                }
            }
        }
    }

    let pattern_matches = if let Some(ref pattern) = rule.pattern {
        if pattern.is_empty() {
            true
        } else {
            Regex::new(pattern)
                .map(|re| re.is_match(&file.name))
                .unwrap_or(false)
        }
    } else {
        true
    };

    ext_matches && pattern_matches
}

pub(crate) fn resolve_destination(destination: &str, file: &FileInfo) -> PathBuf {
    let now = Utc::now();
    let resolved = destination
        .replace("{year}", &now.format("%Y").to_string())
        .replace("{month}", &now.format("%m").to_string())
        .replace("{day}", &now.format("%d").to_string())
        .replace("{extension}", &file.extension)
        .replace("{filename}", &file.name);
    PathBuf::from(resolved)
}

fn normalized_extension(rule: &Rule, extension: &str) -> Option<String> {
    if !rule.normalize_extensions || extension.is_empty() {
        return None;
    }

    rule.extension_mappings
        .split([',', ';', '\n'])
        .filter_map(|entry| {
            let (from, to) = entry
                .trim()
                .split_once("->")
                .or_else(|| entry.trim().split_once(':'))?;
            let from = from.trim().trim_start_matches('.').to_lowercase();
            let to = to.trim().trim_start_matches('.').to_lowercase();
            if from.is_empty()
                || to.is_empty()
                || !to.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return None;
            }
            Some((from, to))
        })
        .find_map(|(from, to)| (from == extension).then_some(to))
}

pub(crate) fn output_file_info(file: &FileInfo, rule: &Rule) -> FileInfo {
    let Some(extension) = normalized_extension(rule, &file.extension) else {
        return file.clone();
    };

    // Magic-byte detection may provide a virtual extension for matching, but
    // normalization only renames a suffix that is actually present.
    if file.path.extension().is_none() {
        return file.clone();
    }

    let stem = file.path.file_stem().unwrap_or_default().to_string_lossy();
    FileInfo {
        path: file.path.clone(),
        name: format!("{stem}.{extension}"),
        extension,
        size: file.size,
    }
}

pub fn find_matching_rule(file: &FileInfo) -> Option<Rule> {
    let rules = get_rules().ok()?;
    let folders = get_watched_folders().ok()?;
    rules.into_iter().find(|rule| {
        (rule.folder_id == 0
            || folders.iter().any(|folder| {
                folder.id == Some(rule.folder_id)
                    && file.path.parent().is_some_and(|parent| {
                        fs::canonicalize(parent).ok() == fs::canonicalize(&folder.path).ok()
                    })
            }))
            && matches_rule(file, rule)
    })
}

pub fn execute_rule(file_info: &FileInfo, rule: &Rule) -> Result<Option<String>, String> {
    let dest =
        crate::operations::destination_for(file_info, rule, &mut std::collections::HashSet::new())?;
    crate::operations::execute_destination(&file_info.path, rule, dest.as_deref())?;
    Ok(dest.map(|p| p.to_string_lossy().to_string()))
}

pub fn process_file(
    path: &Path,
    bypass_grace: bool,
) -> Result<Option<(Rule, Option<String>)>, String> {
    process_file_in_run(
        path,
        bypass_grace,
        &crate::operations::new_run_id(),
        "manual",
    )
}

pub fn process_file_in_run(
    path: &Path,
    bypass_grace: bool,
    run_id: &str,
    trigger: &str,
) -> Result<Option<(Rule, Option<String>)>, String> {
    let _guard = crate::operations::OPERATION_LOCK.lock().unwrap();
    if ["automatic", "scheduled"].contains(&trigger)
        && !get_watched_folders()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|folder| {
                folder.enabled
                    && is_folder_auto_mode(&folder.mode)
                    && path.parent().is_some_and(|parent| {
                        fs::canonicalize(parent).ok() == fs::canonicalize(&folder.path).ok()
                    })
            })
    {
        return Ok(None);
    }
    if crate::db::is_baseline_file(path) {
        return Ok(None);
    }
    let (grace_period, lock_check) = get_settings()
        .map(|s| (s.grace_period_seconds, s.lock_check_enabled))
        .unwrap_or((300, true));

    if !bypass_grace && !check_grace_period(path, grace_period) {
        return Ok(None);
    }
    if lock_check && is_file_locked(path) {
        return Ok(None);
    }

    // Safety net: also check .mouziignore inside process_file.
    if is_file_ignored_by_mouziignore(path) {
        return Ok(None);
    }

    let file_info = scan_file(path).ok_or("Cannot read file metadata")?;
    if crate::operations::already_processed(path)? {
        return Ok(None);
    }
    let rule = find_matching_rule(&file_info).ok_or("No matching rule")?;

    if rule.action == "ignore" {
        return Ok(None);
    }

    if crate::operations::suspicious_name(path) {
        return Ok(None);
    }
    let fingerprint = crate::operations::fingerprint(path)?;
    let dest = execute_rule(&file_info, &rule)?;

    let log = ActionLog {
        id: None,
        timestamp: Utc::now(),
        source_path: file_info.path.to_string_lossy().to_string(),
        destination_path: dest.clone(),
        action: rule.action.clone(),
        file_name: file_info.name.clone(),
        file_type: rule.name.clone(),
        undone: false,
        run_id: Some(run_id.into()),
        trigger: trigger.into(),
        file_extension: file_info.extension.clone(),
        file_size: file_info.size,
        fingerprint: Some(fingerprint),
    };
    log_action(&log)
        .map_err(|e| format!("Operation completed but history could not be saved: {e}"))?;

    Ok(Some((rule, dest)))
}

pub fn manual_scan_folder(folder: &str) -> Result<Vec<(String, String, String)>, String> {
    scan_folder_in_run(folder, &crate::operations::new_run_id(), "manual")
}

pub fn scan_folder_in_run(
    folder: &str,
    run_id: &str,
    trigger: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let mut results = Vec::new();
    let entries = fs::read_dir(folder).map_err(|e| e.to_string())?;

    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if should_ignore_file(&path) {
            eprintln!(
                "[manual_scan] ignoring system/hidden/temp file: {}",
                file_name
            );
            continue;
        }
        if is_file_ignored_by_mouziignore(&path) {
            eprintln!("[manual_scan] ignoring due to .mouziignore: {}", file_name);
            continue;
        }
        match process_file_in_run(&path, true, run_id, trigger) {
            Ok(Some((rule, dest))) => {
                let destination = dest.unwrap_or_default();
                eprintln!(
                    "[manual_scan] organized: {} -> {} ({})",
                    file_name, destination, rule.name
                );
                results.push((file_name, rule.name, destination));
            }
            Ok(None) => {
                eprintln!("[manual_scan] no matching rule or skipped: {}", file_name);
            }
            Err(e) => {
                eprintln!("[manual_scan] error processing {}: {}", file_name, e);
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_move_file_cross_device() {
        let test_root =
            std::env::temp_dir().join(format!("mouzi-move-test-{}", std::process::id()));
        let src_dir = test_root.join("src");
        let dst_dir = test_root.join("dst");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();

        let src = src_dir.join("cross_device_test.txt");
        let dst = dst_dir.join("cross_device_test.txt");

        fs::write(&src, "hello cross-device").unwrap();
        if dst.exists() {
            fs::remove_file(&dst).unwrap();
        }

        crate::operations::move_without_overwrite(&src, &dst).unwrap();

        assert!(
            dst.exists(),
            "destination file should exist after cross-device move"
        );
        assert!(
            !src.exists(),
            "source file should be removed after cross-device move"
        );

        // cleanup
        let _ = fs::remove_file(&dst);
        let _ = fs::remove_file(&src);
        let _ = fs::remove_dir_all(&test_root);
    }

    #[test]
    fn detects_extensionless_png_from_magic_bytes() {
        let test_root =
            std::env::temp_dir().join(format!("mouzi-magic-test-{}", std::process::id()));
        fs::create_dir_all(&test_root).unwrap();
        let path = test_root.join("downloaded-file");
        fs::write(&path, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();

        let info = scan_file(&path).expect("extensionless PNG should be readable");
        assert_eq!(info.extension, "png");

        let _ = fs::remove_dir_all(test_root);
    }

    #[test]
    fn normalizes_existing_extension_only() {
        let file = FileInfo {
            path: PathBuf::from("photo.jpeg"),
            name: "photo.jpeg".to_string(),
            extension: "jpeg".to_string(),
            size: 1,
        };
        let rule = Rule {
            id: None,
            name: "Images".to_string(),
            priority: 1,
            enabled: true,
            extensions: vec!["jpeg".to_string()],
            pattern: None,
            destination: "Images".to_string(),
            action: "move".to_string(),
            folder_id: 0,
            notification_message: None,
            normalize_extensions: true,
            extension_mappings: "jpeg:jpg".to_string(),
            options: Default::default(),
        };

        let output = output_file_info(&file, &rule);
        assert_eq!(output.name, "photo.jpg");
        assert_eq!(output.extension, "jpg");
    }
}
