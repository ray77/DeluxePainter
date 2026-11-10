//! The recovery slot on disk (ADR-34): one `.dpxproj` plus a small JSON note about it, in the
//! app's own data directory. Nothing here knows about the GUI.

use std::path::{Path, PathBuf};

use crate::files;

/// File names inside the recovery folder.
const PROJECT: &str = "recovery.dpxproj";
const META: &str = "recovery.json";

/// Overrides the platform data directory (used by tests and portable installs).
pub const DATA_DIR_ENV: &str = "DELUXEPAINTER_DATA_DIR";

/// What the note file says about the recovery project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// The picture's path when the recovery was written; `None` for an untitled picture.
    pub picture: Option<PathBuf>,
    /// Seconds since the Unix epoch.
    pub saved_unix: u64,
}

impl Meta {
    /// The picture's file name without the folder, or "Untitled".
    pub fn name(&self) -> String {
        self.picture
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string())
    }
}

/// The recovery folder.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

/// The platform's per-user data directory for DeluxePainter, from std and the environment
/// only (no extra dependency). `None` when it cannot be found.
pub fn data_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var(DATA_DIR_ENV) {
        return Some(dir);
    }
    let base = if cfg!(target_os = "macos") {
        var("HOME")?.join("Library").join("Application Support")
    } else if cfg!(target_os = "windows") {
        var("APPDATA")?
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("share")))?
    };
    Some(base.join("DeluxePainter"))
}

impl Store {
    /// The recovery folder inside the platform data directory.
    pub fn platform() -> Option<Self> {
        data_dir().map(|d| Self::at(&d))
    }

    /// The recovery folder inside `data_dir`.
    pub fn at(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("recovery"),
        }
    }

    pub fn project_path(&self) -> PathBuf {
        self.dir.join(PROJECT)
    }

    fn meta_path(&self) -> PathBuf {
        self.dir.join(META)
    }

    /// Replaces the recovery slot: the project first, then the note (both atomic).
    pub fn write(&self, project: &[u8], meta: &Meta) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        files::write_atomic(&self.project_path(), project)?;
        let note = serde_json::json!({
            "picture": meta.picture.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "saved_unix": meta.saved_unix,
        });
        files::write_atomic(&self.meta_path(), note.to_string().as_bytes())
    }

    /// Whether a recovery project is waiting.
    pub fn exists(&self) -> bool {
        self.project_path().is_file()
    }

    /// The note about the waiting recovery project. A missing or damaged note is not fatal:
    /// the picture is then "Untitled" at time 0.
    pub fn meta(&self) -> Meta {
        let parsed = std::fs::read(self.meta_path())
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
        Meta {
            picture: parsed
                .as_ref()
                .and_then(|v| v["picture"].as_str())
                .map(PathBuf::from),
            saved_unix: parsed
                .as_ref()
                .and_then(|v| v["saved_unix"].as_u64())
                .unwrap_or(0),
        }
    }

    /// Deletes the recovery slot (nothing to do when it is empty).
    pub fn discard(&self) {
        for path in [self.project_path(), self.meta_path()] {
            if let Err(e) = std::fs::remove_file(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("{}: {e}", path.display());
            }
        }
    }
}

/// Seconds since the Unix epoch.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `2026-11-11 10:05 UTC` (UTC: std has no local time zone).
pub fn format_utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let rem = unix % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_utc(1_000_000_000), "2001-09-09 01:46 UTC");
        assert_eq!(format_utc(1_782_036_300), "2026-06-21 10:05 UTC");
        // A leap day.
        assert_eq!(format_utc(1_709_164_800), "2024-02-29 00:00 UTC");
    }

    #[test]
    fn store_round_trip_and_discard() {
        let dir = std::env::temp_dir().join(format!("dpx-recovery-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::at(&dir);
        assert!(!store.exists());
        let meta = Meta {
            picture: Some(PathBuf::from("/pics/Castle.dpxproj")),
            saved_unix: 42,
        };
        store.write(b"abc", &meta).unwrap();
        assert!(store.exists());
        assert_eq!(store.meta(), meta);
        assert_eq!(meta.name(), "Castle");
        store.discard();
        assert!(!store.exists());
        store.discard();
        // A damaged note is not fatal.
        std::fs::create_dir_all(dir.join("recovery")).unwrap();
        std::fs::write(store.project_path(), b"x").unwrap();
        std::fs::write(dir.join("recovery").join(META), b"{{{").unwrap();
        assert_eq!(store.meta().name(), "Untitled");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
