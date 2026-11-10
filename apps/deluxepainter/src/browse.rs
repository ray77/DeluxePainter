//! File system listings for the file requester. The requester itself (dpx-ui) never touches
//! files; this module lists drawers, "disks" (mounted volumes) and "assigns".
//!
//! Modern mapping of the Amiga terms (ADR-8): Disks lists mounted volumes, Assigns lists
//! well-known folders (home, desktop, documents, pictures, downloads, the start folder).
//! Hidden entries (names starting with '.') are not listed.

use std::path::{Path, PathBuf};

use dpx_ui::{Entry, PlaceKind};

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Expands a leading `~` to the home folder.
pub fn expand_home(drawer: &str) -> PathBuf {
    match (drawer.strip_prefix('~'), home()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(drawer),
    }
}

/// Lists a drawer: sub-drawers as places, everything else as files. Returns the canonical
/// drawer path shown in the requester.
pub fn list_drawer(drawer: &str) -> Result<(String, Vec<Entry>), String> {
    let dir = std::fs::canonicalize(expand_home(drawer)).map_err(|e| short_io(&e))?;
    let read = std::fs::read_dir(&dir).map_err(|e| short_io(&e))?;
    let mut entries = Vec::new();
    for item in read.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = item.path();
        // Follows links, so a linked folder opens like a folder.
        if std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
            entries.push(Entry::Place {
                kind: PlaceKind::Dir,
                label: name,
                drawer: path_string(&path),
            });
        } else {
            entries.push(Entry::File(name));
        }
    }
    Ok((path_string(&dir), entries))
}

/// The parent drawer, if there is one.
pub fn parent(drawer: &str) -> Option<String> {
    Path::new(drawer)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(path_string)
}

fn volume(label: String, path: &Path) -> Entry {
    Entry::Place {
        kind: PlaceKind::Volume,
        label,
        drawer: path_string(path),
    }
}

/// Mounted volumes ("Disks").
pub fn disks() -> Vec<Entry> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for letter in 'A'..='Z' {
            let root = PathBuf::from(format!("{letter}:\\"));
            if root.exists() {
                out.push(volume(format!("{letter}:"), &root));
            }
        }
        return out;
    }
    let mut dirs = vec![PathBuf::from("/Volumes"), PathBuf::from("/mnt")];
    if let Ok(user) = std::env::var("USER") {
        dirs.push(Path::new("/media").join(&user));
        dirs.push(Path::new("/run/media").join(&user));
    }
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let path = item.path();
            if !name.starts_with('.') && std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) {
                out.push(volume(name, &path));
            }
        }
    }
    // macOS lists the start-up disk in /Volumes; elsewhere the root needs its own entry.
    if !cfg!(target_os = "macos") || out.is_empty() {
        out.push(volume("Root".into(), Path::new("/")));
    }
    out
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Well-known folders ("Assigns").
pub fn assigns() -> Vec<Entry> {
    let mut out = Vec::new();
    let mut add = |label: &str, path: PathBuf| {
        if path.is_dir() {
            out.push(Entry::Place {
                kind: PlaceKind::Assign,
                label: label.into(),
                drawer: path_string(&path),
            });
        }
    };
    if let Some(home) = home() {
        for sub in ["Desktop", "Documents", "Downloads", "Pictures"] {
            add(sub, home.join(sub));
        }
        add("Home", home);
    }
    if let Ok(cwd) = std::env::current_dir() {
        add("Start", cwd);
    }
    out
}

/// An I/O error without the "(os error N)" suffix, short enough for the requester.
pub fn short_io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "Drawer not found".into(),
        std::io::ErrorKind::PermissionDenied => "Permission denied".into(),
        std::io::ErrorKind::NotADirectory => "Not a drawer".into(),
        _ => {
            let s = e.to_string();
            s.split(" (os error").next().unwrap_or(&s).to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_drawers_files_and_parent() {
        let root = std::env::temp_dir().join(format!("dpx-browse-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Brushes")).unwrap();
        std::fs::write(root.join("Castle.iff"), b"x").unwrap();
        std::fs::write(root.join(".hidden"), b"x").unwrap();
        let (drawer, mut entries) = list_drawer(&path_string(&root)).unwrap();
        entries.sort_by_key(|e| format!("{e:?}"));
        let canonical = std::fs::canonicalize(&root).unwrap();
        assert_eq!(drawer, path_string(&canonical));
        assert_eq!(
            entries,
            vec![
                Entry::File("Castle.iff".into()),
                Entry::Place {
                    kind: PlaceKind::Dir,
                    label: "Brushes".into(),
                    drawer: path_string(&canonical.join("Brushes")),
                },
            ]
        );
        assert_eq!(parent(&drawer), canonical.parent().map(path_string));
        assert_eq!(
            list_drawer("/no/such/drawer").unwrap_err(),
            "Drawer not found"
        );
        let file = path_string(&root.join("Castle.iff"));
        assert_eq!(list_drawer(&file).unwrap_err(), "Not a drawer");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn root_has_no_parent_and_home_expands() {
        assert_eq!(parent("/"), None);
        if let Some(h) = home() {
            assert_eq!(expand_home("~"), h);
            assert_eq!(expand_home("~/Pictures"), h.join("Pictures"));
        }
        assert_eq!(expand_home("~x"), PathBuf::from("~x"));
        assert!(
            assigns()
                .iter()
                .any(|e| matches!(e, Entry::Place { label, .. } if label == "Start"))
        );
        assert!(!disks().is_empty());
    }
}
