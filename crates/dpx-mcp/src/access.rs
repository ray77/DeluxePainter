//! Approved-directory file access (ADR-5, ADR-24). Every path is canonicalised (symlinks
//! followed) and allowed only if it lies inside an approved directory. Without any approved
//! directory every file tool fails with `FILE_ACCESS_OFF`.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::error::ToolError;

#[derive(Clone, Debug, Default)]
pub struct FileAccess {
    dirs: Vec<PathBuf>,
}

impl FileAccess {
    /// No directory approved: all file tools fail.
    pub fn off() -> Self {
        Self::default()
    }

    /// Approves `dirs`; each must exist and be a directory (it is canonicalised here).
    pub fn new(dirs: impl IntoIterator<Item = PathBuf>) -> Result<Self, String> {
        let mut out = Vec::new();
        for d in dirs {
            let c = d
                .canonicalize()
                .map_err(|e| format!("--allow-dir {}: {e}", d.display()))?;
            if !c.is_dir() {
                return Err(format!("--allow-dir {}: not a directory", d.display()));
            }
            if !out.contains(&c) {
                out.push(c);
            }
        }
        Ok(Self { dirs: out })
    }

    pub fn is_on(&self) -> bool {
        !self.dirs.is_empty()
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    fn require_on(&self) -> Result<(), ToolError> {
        if self.is_on() {
            Ok(())
        } else {
            Err(ToolError::new(
                "FILE_ACCESS_OFF",
                "file access is off: start the server with `deluxepainter mcp --allow-dir DIR` to approve a directory",
            ))
        }
    }

    /// Relative paths are taken relative to the first approved directory.
    fn absolute(&self, path: &str) -> Result<PathBuf, ToolError> {
        if path.is_empty() || path.contains('\0') {
            return Err(ToolError::invalid("path is empty or contains a NUL byte"));
        }
        let p = Path::new(path);
        Ok(if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dirs[0].join(p)
        })
    }

    fn inside(&self, canonical: &Path) -> bool {
        self.dirs.iter().any(|d| canonical.starts_with(d))
    }

    /// True if the path, without touching the file system, names a place inside an approved
    /// directory and never uses `..`. Decides whether a missing path may be reported as such
    /// (outside the approved directories nothing is revealed).
    fn lexically_inside(&self, p: &Path) -> bool {
        !p.components().any(|c| matches!(c, Component::ParentDir))
            && self.dirs.iter().any(|d| p.starts_with(d))
    }

    fn not_allowed(&self, path: &str) -> ToolError {
        ToolError::new(
            "PATH_NOT_ALLOWED",
            format!("{path}: outside the approved directories"),
        )
    }

    /// An existing regular file inside an approved directory.
    pub fn resolve_read(&self, path: &str) -> Result<PathBuf, ToolError> {
        self.require_on()?;
        let abs = self.absolute(path)?;
        let canon = match abs.canonicalize() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.lexically_inside(&abs) => {
                return Err(ToolError::new(
                    "FILE_NOT_FOUND",
                    format!("{path}: not found"),
                ));
            }
            Err(_) => return Err(self.not_allowed(path)),
        };
        if !self.inside(&canon) {
            return Err(self.not_allowed(path));
        }
        let meta = std::fs::metadata(&canon)
            .map_err(|e| ToolError::new("IO_ERROR", format!("{path}: {e}")))?;
        if !meta.is_file() {
            return Err(ToolError::new(
                "NOT_A_REGULAR_FILE",
                format!("{path}: not a regular file"),
            ));
        }
        Ok(canon)
    }

    /// The final path for writing `path`: its parent is canonicalised and must be inside an
    /// approved directory; an existing target must be a regular file (never a symlink) and is
    /// refused unless `overwrite`.
    pub fn resolve_write(&self, path: &str, overwrite: bool) -> Result<PathBuf, ToolError> {
        self.require_on()?;
        let abs = self.absolute(path)?;
        let (Some(parent), Some(name)) = (abs.parent(), abs.file_name()) else {
            return Err(ToolError::invalid(format!("{path}: not a file name")));
        };
        let canon_parent = match parent.canonicalize() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.lexically_inside(parent) => {
                return Err(ToolError::new(
                    "DIR_NOT_FOUND",
                    format!("{path}: the directory does not exist"),
                ));
            }
            Err(_) => return Err(self.not_allowed(path)),
        };
        if !self.inside(&canon_parent) {
            return Err(self.not_allowed(path));
        }
        let target = canon_parent.join(name);
        match std::fs::symlink_metadata(&target) {
            Ok(m) if m.file_type().is_symlink() => Err(ToolError::new(
                "NOT_A_REGULAR_FILE",
                format!("{path}: is a symbolic link; refusing to write through it"),
            )),
            Ok(m) if !m.is_file() => Err(ToolError::new(
                "NOT_A_REGULAR_FILE",
                format!("{path}: not a regular file"),
            )),
            Ok(_) if !overwrite => Err(ToolError::new(
                "FILE_EXISTS",
                format!("{path}: already exists (pass overwrite=true to replace it)"),
            )),
            Ok(_) => Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(target),
            Err(e) => Err(ToolError::new("IO_ERROR", format!("{path}: {e}"))),
        }
    }

    /// Writes `bytes` to a path from [`Self::resolve_write`]. A new file is created
    /// exclusively; replacing goes through a temporary file in the same directory and a
    /// rename, so a failed save never leaves a half-written picture.
    pub fn write_file(target: &Path, bytes: &[u8], overwrite: bool) -> Result<(), ToolError> {
        let io =
            |e: std::io::Error| ToolError::new("IO_ERROR", format!("{}: {e}", target.display()));
        if !overwrite {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|e| {
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        ToolError::new(
                            "FILE_EXISTS",
                            format!("{}: already exists", target.display()),
                        )
                    } else {
                        io(e)
                    }
                })?;
            return f.write_all(bytes).map_err(io);
        }
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = target.with_file_name(format!(".{name}.dpx-tmp-{}", std::process::id()));
        let result = (|| {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            std::fs::rename(&tmp, target)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.map_err(io)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh scratch directory (canonical) under the system temp dir.
    pub(crate) fn scratch(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "dpx-mcp-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn code(r: Result<PathBuf, ToolError>) -> String {
        r.unwrap_err().code
    }

    #[test]
    fn off_without_dirs() {
        let a = FileAccess::off();
        assert_eq!(code(a.resolve_read("/etc/hosts")), "FILE_ACCESS_OFF");
        assert_eq!(code(a.resolve_write("x.iff", false)), "FILE_ACCESS_OFF");
    }

    #[test]
    fn missing_allow_dir_is_a_startup_error() {
        let d = scratch("missing");
        assert!(FileAccess::new([d.join("nope")]).is_err());
        std::fs::write(d.join("f"), b"x").unwrap();
        assert!(FileAccess::new([d.join("f")]).is_err());
        assert!(FileAccess::new([d.clone()]).is_ok());
    }

    #[test]
    fn reads_inside_and_refuses_outside() {
        let allowed = scratch("in");
        let outside = scratch("out");
        std::fs::write(allowed.join("a.iff"), b"x").unwrap();
        std::fs::write(outside.join("b.iff"), b"x").unwrap();
        let a = FileAccess::new([allowed.clone()]).unwrap();
        assert!(a.resolve_read("a.iff").is_ok(), "relative to first dir");
        assert!(
            a.resolve_read(allowed.join("a.iff").to_str().unwrap())
                .is_ok()
        );
        assert_eq!(
            code(a.resolve_read(outside.join("b.iff").to_str().unwrap())),
            "PATH_NOT_ALLOWED"
        );
        assert_eq!(code(a.resolve_read("missing.iff")), "FILE_NOT_FOUND");
        // A missing file outside reveals nothing.
        assert_eq!(
            code(a.resolve_read(outside.join("missing").to_str().unwrap())),
            "PATH_NOT_ALLOWED"
        );
        std::fs::create_dir(allowed.join("sub")).unwrap();
        assert_eq!(code(a.resolve_read("sub")), "NOT_A_REGULAR_FILE");
    }

    #[test]
    fn dot_dot_traversal_is_refused() {
        let allowed = scratch("dd");
        let outside = scratch("dd-out");
        std::fs::write(outside.join("b.iff"), b"x").unwrap();
        let a = FileAccess::new([allowed.clone()]).unwrap();
        let name = outside.file_name().unwrap().to_str().unwrap();
        // `outside` is a sibling of `allowed` in the temp dir.
        let sneaky = format!("../{name}/b.iff");
        assert_eq!(code(a.resolve_read(&sneaky)), "PATH_NOT_ALLOWED");
        let abs = format!("{}/../{name}/b.iff", allowed.display());
        assert_eq!(code(a.resolve_read(&abs)), "PATH_NOT_ALLOWED");
        assert_eq!(code(a.resolve_write(&sneaky, true)), "PATH_NOT_ALLOWED");
        // `..` that stays inside is fine.
        std::fs::create_dir(allowed.join("s")).unwrap();
        std::fs::write(allowed.join("a.iff"), b"x").unwrap();
        assert!(a.resolve_read("s/../a.iff").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_refused() {
        use std::os::unix::fs::symlink;
        let allowed = scratch("sl");
        let outside = scratch("sl-out");
        std::fs::write(outside.join("secret.iff"), b"x").unwrap();
        symlink(outside.join("secret.iff"), allowed.join("link.iff")).unwrap();
        symlink(&outside, allowed.join("linkdir")).unwrap();
        let a = FileAccess::new([allowed.clone()]).unwrap();
        assert_eq!(code(a.resolve_read("link.iff")), "PATH_NOT_ALLOWED");
        assert_eq!(
            code(a.resolve_read("linkdir/secret.iff")),
            "PATH_NOT_ALLOWED"
        );
        assert_eq!(
            code(a.resolve_write("linkdir/new.iff", false)),
            "PATH_NOT_ALLOWED"
        );
        // A symlink inside the approved dir to a file inside it may be read, but is never
        // written through.
        std::fs::write(allowed.join("real.iff"), b"x").unwrap();
        symlink(allowed.join("real.iff"), allowed.join("ok.iff")).unwrap();
        assert!(a.resolve_read("ok.iff").is_ok());
        assert_eq!(code(a.resolve_write("ok.iff", true)), "NOT_A_REGULAR_FILE");
    }

    #[test]
    fn write_rules() {
        let allowed = scratch("w");
        let a = FileAccess::new([allowed.clone()]).unwrap();
        let t = a.resolve_write("new.iff", false).unwrap();
        FileAccess::write_file(&t, b"one", false).unwrap();
        assert_eq!(code(a.resolve_write("new.iff", false)), "FILE_EXISTS");
        let t = a.resolve_write("new.iff", true).unwrap();
        FileAccess::write_file(&t, b"two", true).unwrap();
        assert_eq!(std::fs::read(allowed.join("new.iff")).unwrap(), b"two");
        assert_eq!(code(a.resolve_write("nodir/x.iff", false)), "DIR_NOT_FOUND");
        std::fs::create_dir(allowed.join("d")).unwrap();
        assert_eq!(code(a.resolve_write("d", true)), "NOT_A_REGULAR_FILE");
        assert_eq!(code(a.resolve_write("..", true)), "INVALID_ARGUMENT");
        // No temp files are left behind (new.iff and d only).
        assert_eq!(std::fs::read_dir(&allowed).unwrap().count(), 2);
    }
}
