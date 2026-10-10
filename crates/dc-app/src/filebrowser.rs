//! The file list of the Open / Save dialogs: what is in a folder, folders first, only `.dcad`
//! documents and folders. Logic only, no egui.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// The folder a typed path means: the path itself if it is a folder (or ends in a separator),
/// else its parent; the current folder when that is empty.
pub fn folder_of(text: &str) -> PathBuf {
    let t = text.trim();
    let p = Path::new(t);
    let dir = if t.is_empty() {
        PathBuf::from(".")
    } else if t.ends_with(std::path::MAIN_SEPARATOR) || t.ends_with('/') || p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent()
            .filter(|d| !d.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    };
    dir
}

/// Folders (no hidden ones) then `.dcad` files, each sorted by name; at most 500.
pub fn list(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<Entry> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = e.path().is_dir();
            (is_dir || name.ends_with(".dcad")).then_some(Entry { name, is_dir })
        })
        .collect();
    v.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    v.truncate(500);
    v
}

/// What the text becomes when `entry` of `dir` is clicked: a folder is entered, a file chosen.
pub fn pick(dir: &Path, entry: &Entry) -> String {
    let p = if dir == Path::new(".") {
        PathBuf::from(&entry.name)
    } else {
        dir.join(&entry.name)
    };
    let s = p.display().to_string();
    if entry.is_dir {
        format!("{s}{}", std::path::MAIN_SEPARATOR)
    } else {
        s
    }
}

/// The parent folder as text, ending in a separator (what the "up" button sets).
pub fn up(dir: &Path) -> String {
    let abs = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    match abs.parent() {
        Some(p) => format!("{}{}", p.display(), std::path::MAIN_SEPARATOR),
        None => format!("{}", abs.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_first_then_documents_and_hides_the_rest() {
        let d = std::env::temp_dir().join(format!("dc-fb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("zeta")).unwrap();
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        for f in ["b.dcad", "a.dcad", "notes.txt"] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        let names: Vec<(String, bool)> = list(&d).into_iter().map(|e| (e.name, e.is_dir)).collect();
        assert_eq!(
            names,
            [
                ("zeta".into(), true),
                ("a.dcad".into(), false),
                ("b.dcad".into(), false)
            ]
        );
        // Typed text: a file in the folder means the folder; the folder's own name too.
        assert_eq!(folder_of(&format!("{}/a.dcad", d.display())), d);
        assert_eq!(folder_of(&d.display().to_string()), d);
        assert_eq!(folder_of(""), PathBuf::from("."));
        assert_eq!(folder_of("part.dcad"), PathBuf::from("."));
        // Clicking.
        let z = Entry {
            name: "zeta".into(),
            is_dir: true,
        };
        assert!(pick(&d, &z).ends_with(&format!("zeta{}", std::path::MAIN_SEPARATOR)));
        let a = Entry {
            name: "a.dcad".into(),
            is_dir: false,
        };
        assert_eq!(pick(&d, &a), d.join("a.dcad").display().to_string());
        assert_eq!(pick(Path::new("."), &a), "a.dcad");
        assert!(up(&d).ends_with(std::path::MAIN_SEPARATOR));
        assert!(list(&d.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
