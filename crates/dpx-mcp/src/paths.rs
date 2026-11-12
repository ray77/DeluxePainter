//! The per-user DeluxePainter config directory (ADR-33, ADR-43), shared by the app's settings
//! file, the MCP endpoint files and the stdio bridge, so all of them look in the same place.

use std::ffi::OsString;
use std::path::PathBuf;

/// The folder name below the platform's config directory.
pub const APP_DIR: &str = "DeluxePainter";

/// The platform rules [`config_dir_with`] follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    /// Linux and the other Unix systems (XDG).
    Other,
}

impl Platform {
    /// The platform this build runs on.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Other
        }
    }
}

/// The config directory: `DPX_CONFIG_DIR` if set, else the platform's config directory
/// (macOS `~/Library/Application Support`, Windows `%APPDATA%`, elsewhere `$XDG_CONFIG_HOME`
/// or `~/.config`) plus `DeluxePainter`. `None` if no home directory is known.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_with(|k| std::env::var_os(k), Platform::current())
}

/// [`config_dir`] with the environment and the platform given (for tests). Empty variables
/// count as unset.
pub fn config_dir_with(
    env: impl Fn(&str) -> Option<OsString>,
    platform: Platform,
) -> Option<PathBuf> {
    let env = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(d) = env("DPX_CONFIG_DIR") {
        return Some(d);
    }
    let base = match platform {
        Platform::MacOs => env("HOME")?.join("Library/Application Support"),
        Platform::Windows => env("APPDATA")?,
        Platform::Other => {
            env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))?
        }
    };
    Some(base.join(APP_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn dir(vars: &[(&str, &str)], platform: Platform) -> Option<PathBuf> {
        let map: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        config_dir_with(|k| map.get(k).cloned(), platform)
    }

    #[test]
    fn override_wins_everywhere() {
        for p in [Platform::MacOs, Platform::Windows, Platform::Other] {
            let d = dir(&[("DPX_CONFIG_DIR", "/x/y"), ("HOME", "/h")], p);
            assert_eq!(d, Some(PathBuf::from("/x/y")), "{p:?}");
        }
        // An empty override is ignored.
        let d = dir(&[("DPX_CONFIG_DIR", ""), ("HOME", "/h")], Platform::Other);
        assert_eq!(d, Some(PathBuf::from("/h/.config/DeluxePainter")));
    }

    #[test]
    fn platform_directories() {
        assert_eq!(
            dir(&[("HOME", "/Users/a")], Platform::MacOs),
            Some(PathBuf::from(
                "/Users/a/Library/Application Support/DeluxePainter"
            ))
        );
        assert_eq!(
            dir(
                &[("APPDATA", "C:/Users/a/AppData/Roaming")],
                Platform::Windows
            ),
            Some(PathBuf::from("C:/Users/a/AppData/Roaming").join("DeluxePainter"))
        );
        assert_eq!(dir(&[("HOME", "/h")], Platform::Windows), None);
        assert_eq!(
            dir(
                &[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/h")],
                Platform::Other
            ),
            Some(PathBuf::from("/xdg/DeluxePainter"))
        );
        assert_eq!(dir(&[], Platform::Other), None);
    }
}
