use std::fs;
use std::path::Path;

/// One line of a .mouziignore file. Hand-written comments and blank lines are
/// stored verbatim, and untouched pattern lines keep their raw text, so the
/// editors can write the file back without losing anything but the lines they changed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum IgnoreLine {
    /// A full-line comment (with the leading `#`) or a blank line, verbatim.
    Comment { text: String },
    /// An ignore pattern, with any inline comment stripped and `#` unescaped.
    /// `raw` is the original line, preserved on write; a freshly typed line is
    /// escaped instead of being written raw.
    Pattern { pattern: String, raw: Option<String> },
}

impl IgnoreLine {
    /// The pattern a line ignores, or `None` for a comment or blank line.
    pub fn pattern(&self) -> Option<&str> {
        match self {
            IgnoreLine::Pattern { pattern, .. } => Some(pattern),
            IgnoreLine::Comment { .. } => None,
        }
    }
}

/// Load .mouziignore patterns from a folder. Returns Vec of non-empty, non-comment lines.
pub fn load_mouziignore(folder_path: &str) -> Vec<String> {
    load_mouziignore_lines(folder_path)
        .into_iter()
        .filter_map(|l| l.pattern().map(|p| p.to_string()))
        .collect()
}

/// Load .mouziignore as typed lines, keeping comments and raw pattern lines
/// so an editor can write the file back unchanged apart from its edits.
pub fn load_mouziignore_lines(folder_path: &str) -> Vec<IgnoreLine> {
    let path = Path::new(folder_path).join(".mouziignore");
    match fs::read_to_string(&path) {
        Ok(content) => content.lines().map(parse_line).collect(),
        Err(_) => Vec::new(),
    }
}

fn parse_line(line: &str) -> IgnoreLine {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return IgnoreLine::Comment {
            text: line.to_string(),
        };
    }
    let mut escaped = false;
    let mut end = line.len();
    for (i, c) in line.char_indices() {
        match c {
            '\\' => escaped = !escaped,
            '#' if !escaped => {
                end = i;
                break;
            }
            _ => escaped = false,
        }
    }
    // The pattern text on screen stays unescaped; the file text is written escaped below.
    IgnoreLine::Pattern {
        pattern: line[..end].trim().replace(r"\#", "#"),
        raw: Some(line.to_string()),
    }
}

/// Save lines back to .mouziignore. Comments and untouched pattern lines are
/// written verbatim; edited lines are escaped. An empty list keeps the header.
pub fn save_mouziignore_lines(folder_path: &str, lines: &[IgnoreLine]) -> Result<(), String> {
    let path = Path::new(folder_path).join(".mouziignore");
    let mut content = String::new();
    for line in lines {
        match line {
            IgnoreLine::Comment { text } => content.push_str(text),
            IgnoreLine::Pattern { pattern, raw } => match raw {
                Some(r) => content.push_str(r),
                None => content.push_str(&pattern.replace('#', r"\#")),
            },
        }
        content.push('\n');
    }
    if content.is_empty() {
        content = "# Mouzi ignore rules\n# https://mouzi.cc/docs\n\n".into();
    }
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// Check if a file name matches any of the ignore patterns.
/// Supports: literal match, `*` wildcard (any number of `*`), and `folder/` directory suffix.
/// On Windows, matching is case-insensitive for both literals and wildcards.
pub fn is_ignored(name: &str, patterns: &[String]) -> bool {
    #[cfg(windows)]
    let name = name.to_lowercase();

    for original_pat in patterns {
        #[cfg(windows)]
        let pat = original_pat.to_lowercase();
        #[cfg(not(windows))]
        let pat = original_pat.as_str();

        // Directory pattern: ends with /
        if pat.ends_with('/') {
            let dir_pat = &pat[..pat.len() - 1];
            if name.eq_ignore_ascii_case(dir_pat) {
                return true;
            }
            continue;
        }
        // Wildcard pattern: contains *
        if pat.contains('*') {
            if glob_match(&name, &pat) {
                return true;
            }
            continue;
        }
        // Literal match
        if name.eq_ignore_ascii_case(&pat) {
            return true;
        }
    }
    false
}

/// Simple glob matcher supporting multiple `*` wildcards.
/// On Windows both `name` and `pat` are expected to already be lowercased.
fn glob_match(name: &str, pat: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.is_empty() {
        return true;
    }

    let starts_with_star = pat.starts_with('*');
    let ends_with_star = pat.ends_with('*');
    let mut rest = name;

    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 && !starts_with_star {
            // First non-empty part must match the start of the name.
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
        } else {
            // Subsequent parts must appear somewhere in the remaining name.
            match rest.find(part) {
                Some(pos) => rest = &rest[pos + part.len()..],
                None => return false,
            }
        }
    }

    // If the pattern does not end with `*`, the remaining text must be empty.
    if !ends_with_star && !rest.is_empty() {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_folder() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mouzi-ignore-test-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn loads_inline_comments_and_escaped_hashes() {
        let folder = temporary_folder();
        fs::write(
            folder.join(".mouziignore"),
            "*.tmp # temporary files\nreport\\#final.pdf # literal hash\n# full comment\n",
        )
        .unwrap();

        let patterns = load_mouziignore(folder.to_str().unwrap());
        assert_eq!(patterns, vec!["*.tmp", "report#final.pdf"]);

        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn round_trip_preserves_comments_and_untouched_lines() {
        let folder = temporary_folder();
        fs::write(
            folder.join(".mouziignore"),
            "# keep me\n\n*.tmp # temporary files\nreport\\#final.pdf # literal hash\n",
        )
        .unwrap();

        let lines = load_mouziignore_lines(folder.to_str().unwrap());
        assert_eq!(
            lines,
            vec![
                IgnoreLine::Comment {
                    text: "# keep me".into()
                },
                IgnoreLine::Comment { text: String::new() },
                IgnoreLine::Pattern {
                    pattern: "*.tmp".into(),
                    raw: Some("*.tmp # temporary files".into()),
                },
                IgnoreLine::Pattern {
                    pattern: "report#final.pdf".into(),
                    raw: Some("report\\#final.pdf # literal hash".into()),
                },
            ]
        );

        // Editing one pattern keeps the surrounding comments and the untouched raw line.
        let mut edited = lines.clone();
        edited[2] = IgnoreLine::Pattern {
            pattern: "*.log".into(),
            raw: None,
        };
        save_mouziignore_lines(folder.to_str().unwrap(), &edited).unwrap();

        let saved = fs::read_to_string(folder.join(".mouziignore")).unwrap();
        assert_eq!(saved, "# keep me\n\n*.log\nreport\\#final.pdf # literal hash\n");
        assert_eq!(
            load_mouziignore(folder.to_str().unwrap()),
            vec!["*.log", "report#final.pdf"]
        );

        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn save_escapes_edited_patterns_and_keeps_the_header_when_empty() {
        let folder = temporary_folder();
        save_mouziignore_lines(
            folder.to_str().unwrap(),
            &[IgnoreLine::Pattern {
                pattern: "report#final.pdf".into(),
                raw: None,
            }],
        )
        .unwrap();
        assert!(
            fs::read_to_string(folder.join(".mouziignore"))
                .unwrap()
                .contains("report\\#final.pdf")
        );

        save_mouziignore_lines(folder.to_str().unwrap(), &[]).unwrap();
        let saved = fs::read_to_string(folder.join(".mouziignore")).unwrap();
        assert_eq!(saved, "# Mouzi ignore rules\n# https://mouzi.cc/docs\n\n");
        assert!(load_mouziignore(folder.to_str().unwrap()).is_empty());

        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn wildcard_case_insensitive_on_windows() {
        assert!(is_ignored("FOO.TMP", &["*.tmp".to_string()]));
        assert!(is_ignored("Foo.Tmp", &["*.tmp".to_string()]));
        assert!(is_ignored("BAR.EXE", &["*.exe".to_string()]));
        assert!(is_ignored("prefixSUFFIX.txt", &["prefix*.TXT".to_string()]));
        assert!(is_ignored("README", &["readme".to_string()]));
        assert!(!is_ignored("foo.tmp", &["*.txt".to_string()]));
        assert!(!is_ignored("FOO.TMP", &["*.txt".to_string()]));
    }

    #[test]
    fn multiple_wildcards_and_spaces() {
        assert!(is_ignored(
            "The Chronicle Herald (Metro)_20260612.txt",
            &["*Metro*".to_string()]
        ));
        assert!(is_ignored(
            "The Chronicle Herald (Metro)_20260612.txt",
            &["*Chronicle Herald*".to_string()]
        ));
        assert!(is_ignored(
            "some.Metro.file.txt",
            &["*Metro*.txt".to_string()]
        ));
        assert!(is_ignored("file.name.txt", &["file.*.txt".to_string()]));
        assert!(!is_ignored("foo.txt", &["*metro*".to_string()]));
    }

    #[test]
    #[cfg(not(windows))]
    fn wildcard_case_sensitive_on_non_windows() {
        assert!(is_ignored("foo.tmp", &["*.tmp".to_string()]));
        assert!(!is_ignored("FOO.TMP", &["*.tmp".to_string()]));
    }
}
