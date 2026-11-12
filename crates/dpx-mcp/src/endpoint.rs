//! Endpoint discovery (ADR-43). A running HTTP server writes `<config dir>/mcp-endpoint.json`
//! (`{version, port, token, pid, app_path, url}`, atomically, owner-only on Unix) and removes
//! it when it stops; the bridge reads it on every (re)connect, so the user never types a port
//! or a token. The token in it is made for that start of the server only, so a file left
//! behind by a crash names no token that still works.
//!
//! While it serves, the server holds an exclusive lock on `mcp-endpoint.lock`; the bridge
//! trusts an endpoint file only while that lock is held ([`read_live_endpoint`]) and removes
//! one nobody holds, so it never sends a request to whatever listens on a dead server's
//! port. The OS releases a lock when its process ends, so a crash leaves no stale lock and a
//! reused process id fools nothing. A running GUI holds `mcp-app.lock` the same way
//! ([`claim_app`], [`app_running`]).
//!
//! `<config dir>/mcp-app.json` (`{version, app_path, args, enabled}`) stays when the app
//! stops. It tells the bridge how to start the app, and `enabled` whether it may: the GUI
//! writes `false` when the user turns MCP control off, so the bridge never starts the app
//! (or a second one) against that choice.
//!
//! The GUI keeps its bearer token in `<config dir>/mcp-token` (owner-only on Unix), so a
//! client configured once still connects after a restart of the app; revoking access
//! writes a new one ([`rotate_token`]).

use std::fs::{File, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const ENDPOINT_FILE: &str = "mcp-endpoint.json";
pub const APP_FILE: &str = "mcp-app.json";
pub const TOKEN_FILE: &str = "mcp-token";
/// Held by the server that wrote the endpoint file, while it serves.
pub const ENDPOINT_LOCK: &str = "mcp-endpoint.lock";
/// Held by a running GUI.
pub const APP_LOCK: &str = "mcp-app.lock";
pub const VERSION: u32 = 1;
/// The arguments the bridge starts the app with (GUI with the MCP server on).
pub const LAUNCH_ARGS: &[&str] = &["--mcp"];

/// A running server's address and token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub version: u32,
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub app_path: Option<String>,
    pub url: String,
}

impl Endpoint {
    pub fn new(port: u16, token: String) -> Self {
        Self {
            version: VERSION,
            port,
            token,
            pid: std::process::id(),
            app_path: current_exe(),
            url: url_for(port),
        }
    }
}

/// How the bridge starts the app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLaunch {
    pub version: u32,
    pub app_path: String,
    #[serde(default = "default_args")]
    pub args: Vec<String>,
    /// MCP control is on (as the app last wrote it): the bridge may start the app. A record
    /// without it counts as off.
    #[serde(default)]
    pub enabled: bool,
}

fn default_args() -> Vec<String> {
    LAUNCH_ARGS.iter().map(|s| s.to_string()).collect()
}

pub fn url_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

fn current_exe() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string())
}

/// A new random bearer token: 32 bytes from the operating system, as 64 hex digits.
pub fn new_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Writes `bytes` to `path` through a temporary file and a rename, readable and writable by
/// the owner only on Unix (the default per-user permissions elsewhere).
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// A token as read from a file: at least 16 visible ASCII characters, no spaces.
fn valid_token(t: &str) -> bool {
    t.len() >= 16 && t.len() <= 256 && t.bytes().all(|b| b.is_ascii_graphic())
}

pub fn token_path(dir: &Path) -> PathBuf {
    dir.join(TOKEN_FILE)
}

/// The persistent token in `dir`: the one stored there, else a new one written there. A file
/// that others could read (Unix: any group or other permission bit) may have given the
/// token away, so it is replaced by a new owner-only one.
pub fn load_or_create_token(dir: &Path) -> std::io::Result<String> {
    let path = token_path(dir);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let t = text.trim();
        if valid_token(t) && !readable_by_others(&path)? {
            return Ok(t.to_string());
        }
    }
    rotate_token(dir)
}

#[cfg(unix)]
fn readable_by_others(path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    Ok(std::fs::metadata(path)?.permissions().mode() & 0o077 != 0)
}

#[cfg(not(unix))]
fn readable_by_others(_path: &Path) -> std::io::Result<bool> {
    Ok(false)
}

/// Writes a new random token to `dir` and returns it; the old one stops working wherever
/// the server checks against the file's token (revoking every configured client).
pub fn rotate_token(dir: &Path) -> std::io::Result<String> {
    let token = new_token()?;
    write_private(&token_path(dir), token.as_bytes())?;
    Ok(token)
}

pub fn endpoint_path(dir: &Path) -> PathBuf {
    dir.join(ENDPOINT_FILE)
}

pub fn app_path(dir: &Path) -> PathBuf {
    dir.join(APP_FILE)
}

pub fn write_endpoint(dir: &Path, e: &Endpoint) -> std::io::Result<PathBuf> {
    let path = endpoint_path(dir);
    let json = serde_json::to_vec_pretty(e).expect("serialisable");
    write_private(&path, &json)?;
    Ok(path)
}

pub fn read_endpoint(dir: &Path) -> Option<Endpoint> {
    let bytes = std::fs::read(endpoint_path(dir)).ok()?;
    serde_json::from_slice::<Endpoint>(&bytes)
        .ok()
        .filter(|e| e.version == VERSION && e.port != 0 && !e.token.is_empty())
}

/// The endpoint file, if the server that wrote it still serves (it holds
/// [`ENDPOINT_LOCK`]). A file nobody serves was left by a crashed or killed server: it is
/// removed (under the lock, so a server starting now cannot lose its new file) and `None`
/// returned, so nothing is sent to whatever listens on its port now.
pub fn read_live_endpoint(dir: &Path) -> Option<Endpoint> {
    let e = read_endpoint(dir)?;
    let lock = dir.join(ENDPOINT_LOCK);
    if held(&lock) {
        return Some(e);
    }
    if let Ok(Some(_claim)) = try_claim(&lock)
        && read_endpoint(dir).as_ref() == Some(&e)
    {
        let _ = std::fs::remove_file(endpoint_path(dir));
    }
    None
}

/// Removes the endpoint file if it still names this process (another instance may have
/// replaced it).
pub fn remove_endpoint(dir: &Path, port: u16) {
    if let Some(e) = read_endpoint(dir)
        && e.pid == std::process::id()
        && e.port == port
    {
        let _ = std::fs::remove_file(endpoint_path(dir));
    }
}

/// Records how to start the app and whether the bridge may (the GUI writes it when MCP
/// control is turned on or off and when it quits).
pub fn write_app_file(dir: &Path, launch: &AppLaunch) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(launch).expect("serialisable");
    write_private(&app_path(dir), &json)
}

/// The app's own launch record: this executable with [`LAUNCH_ARGS`], MCP control
/// `enabled` or not.
pub fn this_app(enabled: bool) -> Option<AppLaunch> {
    Some(AppLaunch {
        version: VERSION,
        app_path: current_exe()?,
        args: default_args(),
        enabled,
    })
}

pub fn read_app_file(dir: &Path) -> Option<AppLaunch> {
    let bytes = std::fs::read(app_path(dir)).ok()?;
    serde_json::from_slice::<AppLaunch>(&bytes)
        .ok()
        .filter(|a| a.version == VERSION && !a.app_path.is_empty())
}

/// An exclusive lock on a file of the config directory, held until it is dropped. The OS
/// releases it when the process ends, so a crash leaves no stale lock behind.
#[derive(Debug)]
pub struct Claim {
    _file: File,
}

fn open_lock(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Owner-only: another user who could open it could hold it and fake a running app.
        opts.mode(0o600);
    }
    let file = opts.open(path)?;
    #[cfg(unix)]
    if readable_by_others(path)? {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// Takes the lock at `path`; `Ok(None)` while another process (or another open handle of
/// this one) holds it.
pub fn try_claim(path: &Path) -> std::io::Result<Option<Claim>> {
    let file = open_lock(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Claim { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// [`try_claim`], tried again for up to `wait`: [`held`] holds a lock for a moment.
pub fn claim_within(path: &Path, wait: Duration) -> std::io::Result<Option<Claim>> {
    let until = Instant::now() + wait;
    loop {
        if let Some(c) = try_claim(path)? {
            return Ok(Some(c));
        }
        if Instant::now() >= until {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Somebody holds the lock at `path` (a missing file: nobody). The probe takes a shared
/// lock and releases it at once.
pub fn held(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock))
}

/// How long a starting app or server waits for a lock a probe holds.
pub const CLAIM_WAIT: Duration = Duration::from_secs(1);

/// The running GUI's claim on `dir` (`Ok(None)`: another DeluxePainter runs).
pub fn claim_app(dir: &Path) -> std::io::Result<Option<Claim>> {
    claim_within(&dir.join(APP_LOCK), CLAIM_WAIT)
}

/// A DeluxePainter GUI runs with this config directory.
pub fn app_running(dir: &Path) -> bool {
    held(&dir.join(APP_LOCK))
}

/// The claim of the server that writes the endpoint file (`Ok(None)`: another one serves).
pub fn claim_endpoint(dir: &Path) -> std::io::Result<Option<Claim>> {
    claim_within(&dir.join(ENDPOINT_LOCK), CLAIM_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_hex() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn endpoint_round_trip_private_and_removed_only_by_its_owner() {
        let dir = crate::access::tests::scratch("endpoint");
        assert_eq!(read_endpoint(&dir), None);
        let e = Endpoint::new(8765, "t0k3n".into());
        let path = write_endpoint(&dir, &e).unwrap();
        assert_eq!(read_endpoint(&dir), Some(e.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Another port (another server of this process) does not remove it.
        remove_endpoint(&dir, 9999);
        assert!(path.exists());
        // Another process's file is kept.
        let other = Endpoint {
            pid: e.pid + 1,
            ..e.clone()
        };
        write_endpoint(&dir, &other).unwrap();
        remove_endpoint(&dir, 8765);
        assert!(path.exists());
        write_endpoint(&dir, &e).unwrap();
        remove_endpoint(&dir, 8765);
        assert!(!path.exists());
        // No temporary files are left.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn tokens_persist_private_and_rotate() {
        let dir = crate::access::tests::scratch("token");
        let a = load_or_create_token(&dir).unwrap();
        assert_eq!(a.len(), 64);
        assert_eq!(
            load_or_create_token(&dir).unwrap(),
            a,
            "kept across restarts"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = token_path(&dir);
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            // A file others could read may have given the token away: a new private one.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            let fresh = load_or_create_token(&dir).unwrap();
            assert_ne!(fresh, a, "the exposed token is replaced");
            assert_eq!(fresh.len(), 64);
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(load_or_create_token(&dir).unwrap(), fresh, "and kept");
        }
        let b = rotate_token(&dir).unwrap();
        assert_ne!(a, b);
        assert_eq!(load_or_create_token(&dir).unwrap(), b);
        // A damaged file gets a new token.
        std::fs::write(token_path(&dir), "short").unwrap();
        let c = load_or_create_token(&dir).unwrap();
        assert!(c != b && c.len() == 64);
    }

    #[test]
    fn app_file_defaults_to_the_mcp_flag_and_off() {
        let dir = crate::access::tests::scratch("appfile");
        std::fs::write(
            app_path(&dir),
            br#"{"version":1,"app_path":"/x/deluxepainter"}"#,
        )
        .unwrap();
        let a = read_app_file(&dir).unwrap();
        assert_eq!(a.args, vec!["--mcp".to_string()]);
        assert!(
            !a.enabled,
            "a record that does not say so does not allow a start"
        );
        write_app_file(&dir, &this_app(true).unwrap()).unwrap();
        let a = read_app_file(&dir).unwrap();
        assert!(a.app_path.len() > 1 && a.enabled);
    }

    #[test]
    fn locks_are_exclusive_probed_without_taking_them_and_released_on_drop() {
        let dir = crate::access::tests::scratch("locks");
        assert!(!app_running(&dir), "no lock file: nobody");
        let a = claim_app(&dir).unwrap().expect("the first claim");
        assert!(app_running(&dir));
        assert!(
            try_claim(&dir.join(APP_LOCK)).unwrap().is_none(),
            "exclusive"
        );
        assert!(app_running(&dir), "a probe does not take it");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(APP_LOCK))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(a);
        assert!(!app_running(&dir));
        assert!(claim_app(&dir).unwrap().is_some());
    }

    #[test]
    fn an_endpoint_nobody_serves_is_ignored_and_removed() {
        let dir = crate::access::tests::scratch("live");
        let e = Endpoint::new(8765, "t".repeat(64));
        write_endpoint(&dir, &e).unwrap();
        let claim = claim_endpoint(&dir).unwrap().unwrap();
        assert_eq!(read_live_endpoint(&dir), Some(e.clone()), "served");
        assert!(endpoint_path(&dir).exists());
        drop(claim);
        assert_eq!(read_live_endpoint(&dir), None, "the server is gone");
        assert!(!endpoint_path(&dir).exists(), "the stale file is removed");
    }
}
