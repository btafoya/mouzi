//! File/path picker: a directory browser used for add-folder, rule destination,
//! rules import and ignore patterns. Reads the filesystem, nothing else.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Want {
    /// Browse folders; Space picks the folder being viewed.
    Dir,
    /// Browse folders and `.json` files; Enter on a file picks it.
    JsonFile,
    /// Flat list of one folder's entries (files and folders); Enter picks one.
    Entry,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

#[derive(Debug, PartialEq)]
pub enum Out {
    Idle,
    Cancel,
    Picked(PathBuf),
}

#[derive(Debug)]
pub struct Picker {
    pub want: Want,
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub sel: usize,
    pub hidden: bool,
    /// Typed path, while the "go to" prompt is open.
    pub goto: Option<String>,
    pub error: Option<String>,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

impl Picker {
    pub fn new(start: &Path, want: Want) -> Self {
        let dir = if start.is_dir() {
            start.to_path_buf()
        } else {
            home()
        };
        let mut p = Self {
            want,
            dir,
            entries: Vec::new(),
            sel: 0,
            hidden: false,
            goto: None,
            error: None,
        };
        p.reload();
        p
    }

    pub fn reload(&mut self) {
        self.error = None;
        let mut entries = Vec::new();
        if self.want != Want::Entry && self.dir.parent().is_some() {
            entries.push(Entry {
                name: "..".into(),
                dir: true,
            });
        }
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                let mut found: Vec<Entry> = rd
                    .flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        let dir = e.path().is_dir(); // follows symlinks
                        let keep = (self.hidden || !name.starts_with('.'))
                            && match self.want {
                                Want::Dir => dir,
                                Want::JsonFile => dir || name.to_lowercase().ends_with(".json"),
                                Want::Entry => true,
                            };
                        keep.then_some(Entry { name, dir })
                    })
                    .collect();
                found.sort_by_key(|e| (!e.dir, e.name.to_lowercase()));
                entries.extend(found);
            }
            Err(e) => self.error = Some(format!("cannot read {}: {e}", self.dir.display())),
        }
        self.entries = entries;
        self.sel = self.sel.min(self.entries.len().saturating_sub(1));
    }

    fn go(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.sel = 0;
        self.reload();
    }

    pub fn key(&mut self, k: KeyEvent) -> Out {
        if let Some(buf) = &mut self.goto {
            match k.code {
                KeyCode::Esc => self.goto = None,
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => buf.push(c),
                KeyCode::Enter => {
                    let typed = std::mem::take(buf);
                    self.goto = None;
                    return self.jump(&typed);
                }
                _ => {}
            }
            return Out::Idle;
        }
        let up = self.dir.parent().map(Path::to_path_buf);
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => return Out::Cancel,
            KeyCode::Down | KeyCode::Char('j') => {
                self.sel = (self.sel + 1).min(self.entries.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace if self.want != Want::Entry => {
                if let Some(p) = up {
                    self.go(p);
                }
            }
            KeyCode::Char('.') => {
                self.hidden = !self.hidden;
                self.reload();
            }
            KeyCode::Char('~') if self.want != Want::Entry => self.go(home()),
            KeyCode::Char('g') if self.want != Want::Entry => self.goto = Some(String::new()),
            KeyCode::Char(' ' | 's') if self.want == Want::Dir => {
                return Out::Picked(self.dir.clone())
            }
            KeyCode::Char(' ') if self.want == Want::Entry => return self.open(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => return self.open(),
            _ => {}
        }
        Out::Idle
    }

    fn open(&mut self) -> Out {
        let Some(e) = self.entries.get(self.sel).cloned() else {
            return Out::Idle;
        };
        if self.want == Want::Entry {
            return Out::Picked(self.dir.join(e.name));
        }
        let target = if e.name == ".." {
            self.dir.parent().map(Path::to_path_buf)
        } else {
            Some(self.dir.join(&e.name))
        };
        match (target, e.dir) {
            (Some(t), true) => self.go(t),
            (Some(t), false) => return Out::Picked(t),
            _ => {}
        }
        Out::Idle
    }

    fn jump(&mut self, typed: &str) -> Out {
        let typed = typed.trim();
        let path = match typed.strip_prefix('~') {
            Some(rest) => home().join(rest.trim_start_matches('/')),
            None => PathBuf::from(typed),
        };
        if path.is_dir() {
            self.go(path);
        } else if path.is_file() && self.want == Want::JsonFile {
            return Out::Picked(path);
        } else {
            self.error = Some(format!("not a usable path: {typed}"));
        }
        Out::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
    }
    fn names(p: &Picker) -> Vec<&str> {
        p.entries.iter().map(|e| e.name.as_str()).collect()
    }
    fn tree() -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "mouzi-pick-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["Beta", "alpha/inner", ".hidden"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in ["rules.JSON", "notes.txt", ".dot.json"] {
            std::fs::write(root.join(f), "[]").unwrap();
        }
        root
    }

    #[test]
    fn lists_folders_first_sorted_hides_dotfiles_and_filters_by_want() {
        let root = tree();
        let mut p = Picker::new(&root, Want::Dir);
        assert_eq!(names(&p), ["..", "alpha", "Beta"]);
        p.key(key(KeyCode::Char('.')));
        assert_eq!(names(&p), ["..", ".hidden", "alpha", "Beta"]);
        let p = Picker::new(&root, Want::JsonFile);
        assert_eq!(names(&p), ["..", "alpha", "Beta", "rules.JSON"]);
        let p = Picker::new(&root, Want::Entry);
        assert_eq!(names(&p), ["alpha", "Beta", "notes.txt", "rules.JSON"]);
    }

    #[test]
    fn navigates_in_and_out_and_space_picks_the_current_folder() {
        let root = tree();
        let mut p = Picker::new(&root, Want::Dir);
        p.key(key(KeyCode::Down)); // alpha
        p.key(key(KeyCode::Enter));
        assert_eq!(p.dir, root.join("alpha"));
        assert_eq!(names(&p), ["..", "inner"]);
        p.key(key(KeyCode::Left));
        assert_eq!(p.dir, root);
        p.key(key(KeyCode::Down));
        p.key(key(KeyCode::Enter));
        p.key(key(KeyCode::Down));
        p.key(key(KeyCode::Enter)); // into inner
        assert_eq!(
            p.key(key(KeyCode::Char(' '))),
            Out::Picked(root.join("alpha/inner"))
        );
        p.key(key(KeyCode::Enter)); // ".." entry goes up
        assert_eq!(p.dir, root.join("alpha"));
        assert_eq!(p.key(key(KeyCode::Esc)), Out::Cancel);
    }

    #[test]
    fn json_picker_picks_files_and_ignores_space() {
        let root = tree();
        let mut p = Picker::new(&root, Want::JsonFile);
        assert_eq!(p.key(key(KeyCode::Char(' '))), Out::Idle);
        for _ in 0..3 {
            p.key(key(KeyCode::Down));
        }
        assert_eq!(
            p.key(key(KeyCode::Enter)),
            Out::Picked(root.join("rules.JSON"))
        );
    }

    #[test]
    fn entry_picker_is_flat_and_picks_files_or_folders() {
        let root = tree();
        let mut p = Picker::new(&root, Want::Entry);
        assert_eq!(p.key(key(KeyCode::Left)), Out::Idle);
        assert_eq!(p.dir, root, "flat picker cannot leave the folder");
        assert_eq!(p.key(key(KeyCode::Enter)), Out::Picked(root.join("alpha")));
        p.key(key(KeyCode::Down));
        p.key(key(KeyCode::Down));
        assert_eq!(
            p.key(key(KeyCode::Char(' '))),
            Out::Picked(root.join("notes.txt"))
        );
    }

    #[test]
    fn goto_navigates_picks_json_files_and_reports_bad_paths() {
        let root = tree();
        let mut p = Picker::new(&root, Want::JsonFile);
        let type_path = |p: &mut Picker, s: &str| {
            p.key(key(KeyCode::Char('g')));
            for c in s.chars() {
                p.key(key(KeyCode::Char(c)));
            }
            p.key(key(KeyCode::Enter))
        };
        assert_eq!(
            type_path(&mut p, root.join("alpha").to_str().unwrap()),
            Out::Idle
        );
        assert_eq!(p.dir, root.join("alpha"));
        assert_eq!(
            type_path(&mut p, root.join("rules.JSON").to_str().unwrap()),
            Out::Picked(root.join("rules.JSON"))
        );
        assert_eq!(type_path(&mut p, "/definitely/not/here"), Out::Idle);
        assert!(p.error.as_deref().unwrap().contains("not a usable path"));
        p.key(key(KeyCode::Char('g')));
        p.key(key(KeyCode::Esc));
        assert!(p.goto.is_none());
        assert_eq!(
            type_path(
                &mut Picker::new(&root, Want::Dir),
                root.join("rules.JSON").to_str().unwrap()
            ),
            Out::Idle,
            "folder picker refuses files"
        );
    }

    #[test]
    fn unreadable_folder_shows_error_instead_of_panicking() {
        let mut p = Picker::new(&tree(), Want::Dir);
        p.dir = PathBuf::from("/definitely/not/here");
        p.reload();
        assert!(p.error.is_some());
    }
}
