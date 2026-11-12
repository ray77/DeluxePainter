//! Finding and driving Claude Code and Claude Desktop for Prefs > AI & MCP.. (ADR-43,
//! part 3).
//!
//! - **Claude Code:** `claude` is looked for at its usual absolute paths first (a GUI app
//!   on macOS does not get the shell's `PATH`), then on `PATH`. The app registers itself
//!   with `claude mcp add --scope user deluxe-painter -- <this executable> mcp-bridge`,
//!   checks a registration with `claude mcp get deluxe-painter` (exit code 0: registered)
//!   and removes it with `claude mcp remove deluxe-painter -s user`. The token never
//!   appears in a command: the bridge reads the endpoint file.
//! - **Claude Desktop:** the extension `deluxepainter-<os>-<arch>.mcpb` lies next to the
//!   executable (or in `Contents/Resources` of a macOS app bundle, or in `target/mcpb/` of
//!   a development build); the OS opens it (`open`, `explorer.exe`, `xdg-open`), so Claude
//!   Desktop shows its own install dialog. Nothing is installed without the user.
//!
//! Every process runs without a shell, off the UI thread ([`spawn`]), from the home
//! directory, at most [`RUN_TIMEOUT`] (its output too: a process that leaves a child holding
//! its pipes is not waited for beyond [`AFTER_EXIT`]). `claude` gets its own directory (and
//! its symlink target's) in front of `PATH`: an npm-installed `claude` is a
//! `#!/usr/bin/env node` script, and a GUI app started from the Finder or Dock has only
//! `/usr/bin:/bin:/usr/sbin:/sbin`. [`System`] is the seam the ui-loop tests replace.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dpx_mcp::bundle;
use dpx_mcp::paths::Platform;
use eframe::egui;

/// Where the "Get Claude Code" button leads.
pub const CLAUDE_CODE_SETUP_URL: &str = "https://code.claude.com/docs/en/setup";
/// Where the "Get Claude" button leads.
pub const CLAUDE_DOWNLOAD_URL: &str = "https://claude.ai/download";
/// A `claude` (or opener) process that has not ended by then is stopped.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the output of a process that has ended is still read: a child it left running
/// may hold its pipes open (an opener that starts the app it opens with).
pub const AFTER_EXIT: Duration = Duration::from_secs(2);

/// How a process ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunOutput {
    /// The exit code (`None`: ended by a signal).
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl RunOutput {
    pub fn success() -> Self {
        Self {
            code: Some(0),
            ..Self::default()
        }
    }

    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// The first line of stderr (else stdout) that says something, for a message.
    pub fn first_line(&self) -> Option<String> {
        [&self.stderr, &self.stdout]
            .into_iter()
            .flat_map(|s| s.lines())
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(str::to_string)
    }
}

/// The computer the connect flows act on.
pub trait System: Send + Sync {
    /// This executable (the command the client launches with `mcp-bridge`).
    fn exe(&self) -> Option<PathBuf>;
    /// The `claude` command of Claude Code.
    fn find_claude(&self) -> Option<PathBuf>;
    /// This platform's Claude Desktop extension.
    fn find_mcpb(&self) -> Option<PathBuf>;
    fn desktop_installed(&self) -> bool;
    /// Runs `program` with `args`, without a shell, waiting at most [`RUN_TIMEOUT`]. Never
    /// called on the UI thread.
    fn run(&self, program: &Path, args: &[String]) -> Result<RunOutput, String>;
    /// Opens a file with the program the OS associates with it.
    fn open_file(&self, path: &Path) -> Result<(), String>;
}

/// The real computer.
#[derive(Debug, Default)]
pub struct OsSystem;

fn env(k: &str) -> Option<OsString> {
    std::env::var_os(k).filter(|v| !v.is_empty())
}

impl System for OsSystem {
    fn exe(&self) -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    fn find_claude(&self) -> Option<PathBuf> {
        claude_candidates(Platform::current(), &env)
            .into_iter()
            .find(|p| p.is_file())
    }

    fn find_mcpb(&self) -> Option<PathBuf> {
        let exe = self.exe()?;
        mcpb_candidates(&exe, &bundle::this_mcpb_file_name())
            .into_iter()
            .find(|p| p.is_file())
    }

    fn desktop_installed(&self) -> bool {
        let platform = Platform::current();
        desktop_candidates(platform, &env)
            .iter()
            .any(|p| p.exists())
            || msix_installed(platform, &env)
    }

    fn run(&self, program: &Path, args: &[String]) -> Result<RunOutput, String> {
        let path = child_path(program, env("PATH"));
        run_with_timeout(program, args, RUN_TIMEOUT, path)
    }

    fn open_file(&self, path: &Path) -> Result<(), String> {
        let platform = Platform::current();
        let (program, args) = open_command(platform, path);
        let out = run_with_timeout(Path::new(&program), &args, RUN_TIMEOUT, None)?;
        // explorer.exe answers 1 even when it opened the file.
        if out.ok() || platform == Platform::Windows {
            Ok(())
        } else {
            Err(out
                .first_line()
                .unwrap_or_else(|| format!("{program} ended with {:?}", out.code)))
        }
    }
}

/// Where `claude` may be, in the order they are tried: the native installer's
/// `~/.local/bin/claude` (`%USERPROFILE%\.local\bin\claude.exe`), Homebrew's and
/// `/usr/local/bin`, then every `PATH` directory.
pub fn claude_candidates(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let exe_name = match platform {
        Platform::Windows => {
            if let Some(home) = env("USERPROFILE") {
                out.push(
                    PathBuf::from(home)
                        .join(".local")
                        .join("bin")
                        .join("claude.exe"),
                );
            }
            "claude.exe"
        }
        Platform::MacOs | Platform::Other => {
            if let Some(home) = env("HOME") {
                out.push(PathBuf::from(home).join(".local/bin/claude"));
            }
            out.push(PathBuf::from("/opt/homebrew/bin/claude"));
            out.push(PathBuf::from("/usr/local/bin/claude"));
            "claude"
        }
    };
    if let Some(path) = env("PATH") {
        out.extend(std::env::split_paths(&path).map(|d| d.join(exe_name)));
    }
    out
}

/// Where an installed Claude Desktop shows: macOS `/Applications/Claude.app` or
/// `~/Applications/Claude.app`; Windows `%LOCALAPPDATA%\AnthropicClaude` (the older
/// installer; unverified); Linux the `claude-desktop` command of the beta (unverified).
pub fn desktop_candidates(
    platform: Platform,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<PathBuf> {
    match platform {
        Platform::MacOs => {
            let mut out = vec![PathBuf::from("/Applications/Claude.app")];
            if let Some(home) = env("HOME") {
                out.push(PathBuf::from(home).join("Applications/Claude.app"));
            }
            out
        }
        Platform::Windows => env("LOCALAPPDATA")
            .map(|d| PathBuf::from(d).join("AnthropicClaude"))
            .into_iter()
            .collect(),
        Platform::Other => {
            let mut out = vec![PathBuf::from("/usr/bin/claude-desktop")];
            if let Some(path) = env("PATH") {
                out.extend(std::env::split_paths(&path).map(|d| d.join("claude-desktop")));
            }
            out
        }
    }
}

/// Windows: the Store (MSIX) package leaves a `Claude_*` folder in
/// `%LOCALAPPDATA%\Packages` (unverified).
fn msix_installed(platform: Platform, env: &dyn Fn(&str) -> Option<OsString>) -> bool {
    if platform != Platform::Windows {
        return false;
    }
    let Some(local) = env("LOCALAPPDATA") else {
        return false;
    };
    std::fs::read_dir(PathBuf::from(local).join("Packages")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
    })
}

/// Where the extension `name` may be for the executable `exe`: next to it, in a macOS app
/// bundle's `Contents/Resources`, and in `target/mcpb/` when `exe` is a build in `target/`
/// (`target/debug`, `target/release`, `target/<triple>/release`).
pub fn mcpb_candidates(exe: &Path, name: &str) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let mut out = vec![dir.join(name)];
    if dir.file_name().is_some_and(|n| n == "MacOS") {
        out.push(dir.with_file_name("Resources").join(name));
    }
    for up in dir.ancestors().take(3) {
        if up.file_name().is_some_and(|n| n == "target") {
            out.push(up.join(bundle::MCPB_DIR).join(name));
        }
    }
    out
}

/// The program and arguments that open `path` with its associated program. `start` is a
/// `cmd.exe` built-in, so Windows uses `explorer.exe` (no shell).
pub fn open_command(platform: Platform, path: &Path) -> (String, Vec<String>) {
    let program = match platform {
        Platform::MacOs => "/usr/bin/open",
        Platform::Windows => "explorer.exe",
        Platform::Other => "xdg-open",
    };
    (program.into(), vec![path.to_string_lossy().into_owned()])
}

/// `claude mcp add --scope user deluxe-painter -- <exe> mcp-bridge`, without `claude`.
pub fn add_args(exe: &Path) -> Vec<String> {
    [
        "mcp",
        "add",
        "--scope",
        "user",
        bundle::CLIENT_NAME,
        "--",
        &exe.to_string_lossy(),
        "mcp-bridge",
    ]
    .map(String::from)
    .to_vec()
}

/// `claude mcp get deluxe-painter`: exit code 0 when Claude Code has the server.
pub fn get_args() -> Vec<String> {
    ["mcp", "get", bundle::CLIENT_NAME]
        .map(String::from)
        .to_vec()
}

/// `claude mcp remove deluxe-painter -s user`.
pub fn remove_args() -> Vec<String> {
    ["mcp", "remove", bundle::CLIENT_NAME, "-s", "user"]
        .map(String::from)
        .to_vec()
}

/// `args` as a line to show or paste into a terminal: `claude` and every argument, quoted
/// where a shell would split or change it (POSIX single quotes; double quotes on Windows).
pub fn command_line(platform: Platform, args: &[String]) -> String {
    std::iter::once("claude".to_string())
        .chain(args.iter().map(|a| quote(platform, a)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(platform: Platform, arg: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c);
    let windows = platform == Platform::Windows;
    if !arg.is_empty() && arg.chars().all(|c| plain(c) || (windows && c == '\\')) {
        return arg.to_string();
    }
    if windows {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// A process run for a connect flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Task {
    /// `claude mcp get deluxe-painter`.
    Check { claude: PathBuf },
    /// `claude mcp add …`.
    Add { claude: PathBuf, exe: PathBuf },
    /// `claude mcp remove …`, then `claude mcp add …` (the add's outcome counts).
    Replace { claude: PathBuf, exe: PathBuf },
    /// `claude mcp remove …`.
    Remove { claude: PathBuf },
    /// Opens the extension with the OS (Claude Desktop's install dialog).
    Open { path: PathBuf },
}

/// Runs `task` on a thread of its own; the answer comes on the receiver and `wake` (the
/// window) is woken.
pub fn spawn(
    system: Arc<dyn System>,
    task: Task,
    wake: Option<egui::Context>,
) -> Receiver<Result<RunOutput, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let out = run_task(system.as_ref(), &task);
        let _ = tx.send(out);
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    });
    rx
}

fn run_task(system: &dyn System, task: &Task) -> Result<RunOutput, String> {
    match task {
        Task::Check { claude } => system.run(claude, &get_args()),
        Task::Add { claude, exe } => system.run(claude, &add_args(exe)),
        Task::Replace { claude, exe } => {
            // A failed removal (nothing to remove) does not stop the add.
            let _ = system.run(claude, &remove_args());
            system.run(claude, &add_args(exe))
        }
        Task::Remove { claude } => system.run(claude, &remove_args()),
        Task::Open { path } => system.open_file(path).map(|()| RunOutput::success()),
    }
}

/// `PATH` for running `program`: its own directory and, for a symlink, its target's, in
/// front of `inherited` (so `#!/usr/bin/env node` finds the `node` installed with it).
pub fn child_path(program: &Path, inherited: Option<OsString>) -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut add = |d: Option<&Path>| {
        if let Some(d) = d.filter(|d| !d.as_os_str().is_empty())
            && !dirs.iter().any(|x| x == d)
        {
            dirs.push(d.to_path_buf());
        }
    };
    add(program.parent());
    if let Ok(target) = std::fs::canonicalize(program) {
        add(target.parent());
    }
    if let Some(p) = &inherited {
        for d in std::env::split_paths(p) {
            add(Some(&d));
        }
    }
    std::env::join_paths(dirs).ok().or(inherited)
}

/// Reads `pipe` to its end on a thread of its own, into a buffer the caller can take
/// whenever it stops waiting; the receiver hears when the end came.
fn read_pipe(pipe: Option<Box<dyn Read + Send>>) -> (Arc<Mutex<Vec<u8>>>, Receiver<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let b = buf.clone();
    std::thread::spawn(move || {
        if let Some(mut p) = pipe {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = p.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                b.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(&chunk[..n]);
            }
        }
        let _ = tx.send(());
    });
    (buf, rx)
}

/// What `buf` holds once its reader ended or `until` passed.
fn read_until(buf: &Mutex<Vec<u8>>, done: &Receiver<()>, until: Instant) -> String {
    let _ = done.recv_timeout(until.saturating_duration_since(Instant::now()));
    String::from_utf8_lossy(&buf.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
}

/// Runs `program` without a shell from the home directory, with no input (and `PATH` when
/// given), and stops it after `timeout`; its output is read until it ends, at most
/// [`AFTER_EXIT`] more and never beyond `timeout`. On Windows no console window opens
/// (unverified).
fn run_with_timeout(
    program: &Path,
    args: &[String],
    timeout: Duration,
    path: Option<OsString>,
) -> Result<RunOutput, String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(p) = path {
        cmd.env("PATH", p);
    }
    // `claude mcp get` runs health checks of the servers a working directory's `.mcp.json`
    // names: run it from the home directory, never from wherever the app was started.
    if let Some(home) = env("HOME").or_else(|| env("USERPROFILE")) {
        cmd.current_dir(home);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let name = program.display().to_string();
    let deadline = Instant::now() + timeout;
    let mut child = cmd.spawn().map_err(|e| format!("cannot run {name}: {e}"))?;
    let (out, out_done) = read_pipe(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let (err, err_done) = read_pipe(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{name} did not finish within {} s",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("{name}: {e}")),
        }
    };
    let until = deadline.min(Instant::now() + AFTER_EXIT);
    Ok(RunOutput {
        code: status.code(),
        stdout: read_until(&out, &out_done, until),
        stderr: read_until(&err, &err_done, until),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + use<> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn claude_is_looked_for_at_absolute_paths_first() {
        let path = std::env::join_paths(["/bin", "/usr/bin"]).unwrap();
        let path = path.to_string_lossy().into_owned();
        let env = env_of(&[("HOME", "/Users/me"), ("PATH", &path)]);
        let got = claude_candidates(Platform::MacOs, &env);
        assert_eq!(
            got,
            [
                "/Users/me/.local/bin/claude",
                "/opt/homebrew/bin/claude",
                "/usr/local/bin/claude",
                "/bin/claude",
                "/usr/bin/claude"
            ]
            .map(PathBuf::from)
        );
        let env = env_of(&[("USERPROFILE", r"C:\Users\me")]);
        let got = claude_candidates(Platform::Windows, &env);
        assert_eq!(
            got,
            [PathBuf::from(r"C:\Users\me")
                .join(".local")
                .join("bin")
                .join("claude.exe")]
        );
        // No HOME: only the fixed paths.
        assert_eq!(claude_candidates(Platform::Other, &env_of(&[])).len(), 2);
    }

    #[test]
    fn claude_desktop_places() {
        let env = env_of(&[("HOME", "/Users/me"), ("LOCALAPPDATA", r"C:\L")]);
        assert_eq!(
            desktop_candidates(Platform::MacOs, &env),
            [
                "/Applications/Claude.app",
                "/Users/me/Applications/Claude.app"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            desktop_candidates(Platform::Windows, &env),
            [PathBuf::from(r"C:\L").join("AnthropicClaude")]
        );
        assert_eq!(
            desktop_candidates(Platform::Other, &env_of(&[])),
            [PathBuf::from("/usr/bin/claude-desktop")]
        );
        assert!(!msix_installed(Platform::MacOs, &env));
    }

    #[test]
    fn the_extension_is_next_to_the_app_or_in_target_mcpb() {
        let name = "deluxepainter-macos-aarch64.mcpb";
        assert_eq!(
            mcpb_candidates(Path::new("/w/target/debug/deluxepainter"), name),
            [
                format!("/w/target/debug/{name}"),
                format!("/w/target/mcpb/{name}")
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            mcpb_candidates(
                Path::new("/w/target/aarch64-apple-darwin/release/deluxepainter"),
                name
            )[1],
            PathBuf::from(format!("/w/target/mcpb/{name}"))
        );
        assert_eq!(
            mcpb_candidates(
                Path::new("/Applications/DeluxePainter.app/Contents/MacOS/deluxepainter"),
                name
            ),
            [
                format!("/Applications/DeluxePainter.app/Contents/MacOS/{name}"),
                format!("/Applications/DeluxePainter.app/Contents/Resources/{name}")
            ]
            .map(PathBuf::from)
        );
        assert!(mcpb_candidates(Path::new(""), name).is_empty());
    }

    #[test]
    fn commands_are_exact_and_quoted_for_a_terminal() {
        let exe = Path::new("/Applications/Deluxe Painter.app/Contents/MacOS/deluxepainter");
        let args = add_args(exe);
        assert_eq!(
            args,
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "deluxe-painter",
                "--",
                "/Applications/Deluxe Painter.app/Contents/MacOS/deluxepainter",
                "mcp-bridge"
            ]
        );
        assert_eq!(
            command_line(Platform::MacOs, &args),
            "claude mcp add --scope user deluxe-painter -- \
             '/Applications/Deluxe Painter.app/Contents/MacOS/deluxepainter' mcp-bridge"
        );
        assert_eq!(
            command_line(
                Platform::MacOs,
                &add_args(Path::new("/opt/dp/deluxepainter"))
            ),
            "claude mcp add --scope user deluxe-painter -- /opt/dp/deluxepainter mcp-bridge"
        );
        assert_eq!(
            command_line(
                Platform::Windows,
                &add_args(Path::new(r"C:\Program Files\DP\deluxepainter.exe"))
            ),
            r#"claude mcp add --scope user deluxe-painter -- "C:\Program Files\DP\deluxepainter.exe" mcp-bridge"#
        );
        assert_eq!(quote(Platform::Other, "it's"), r"'it'\''s'");
        assert_eq!(quote(Platform::Other, ""), "''");
        assert_eq!(
            command_line(Platform::MacOs, &get_args()),
            "claude mcp get deluxe-painter"
        );
        assert_eq!(
            command_line(Platform::MacOs, &remove_args()),
            "claude mcp remove deluxe-painter -s user"
        );
        assert_eq!(
            open_command(Platform::MacOs, Path::new("/x/a.mcpb")),
            ("/usr/bin/open".to_string(), vec!["/x/a.mcpb".to_string()])
        );
        assert_eq!(
            open_command(Platform::Windows, Path::new("a")).0,
            "explorer.exe"
        );
        assert_eq!(open_command(Platform::Other, Path::new("a")).0, "xdg-open");
    }

    #[cfg(unix)]
    #[test]
    fn a_process_runs_without_a_shell_and_is_stopped_after_its_time() {
        // `sh` here is the program itself, not a shell around another command line.
        let out = run_with_timeout(
            Path::new("/bin/sh"),
            &["-c".into(), "echo out; echo err >&2; exit 3".into()],
            Duration::from_secs(10),
            None,
        )
        .unwrap();
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout.trim(), "out");
        assert_eq!(out.first_line().as_deref(), Some("err"));
        let started = Instant::now();
        let e = run_with_timeout(
            Path::new("/bin/sleep"),
            &["5".into()],
            Duration::from_millis(200),
            None,
        )
        .unwrap_err();
        assert!(e.contains("did not finish"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(3));
        let e = run_with_timeout(
            Path::new("/no/such/claude"),
            &[],
            Duration::from_secs(1),
            None,
        )
        .unwrap_err();
        assert!(e.starts_with("cannot run /no/such/claude"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn a_child_left_holding_the_pipes_is_not_waited_for() {
        // The process ends at once; the `sleep` it leaves keeps stdout open for 6 s.
        let started = Instant::now();
        let out = run_with_timeout(
            Path::new("/bin/sh"),
            &["-c".into(), "echo early; sleep 6 & exit 0".into()],
            Duration::from_secs(20),
            None,
        )
        .unwrap();
        assert!(out.ok());
        assert_eq!(out.stdout.trim(), "early");
        let took = started.elapsed();
        assert!(took < AFTER_EXIT + Duration::from_secs(2), "{took:?}");
    }

    #[cfg(unix)]
    fn script(path: &Path, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn claude_finds_the_interpreter_installed_next_to_it() {
        // Like npm's `claude` (`#!/usr/bin/env node`, node in the same bin directory), run
        // with the PATH of an app started from the Finder.
        let dir = std::env::temp_dir().join(format!("dpx-connect-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        script(
            &dir.join("bin/dpxnode"),
            "#!/bin/sh\necho \"node ran $2 $3\"\n",
        );
        script(&dir.join("lib/cli.js"), "#!/usr/bin/env dpxnode\n");
        std::os::unix::fs::symlink(dir.join("lib/cli.js"), dir.join("bin/claude")).unwrap();
        let claude = dir.join("bin/claude");
        let finder = Some(OsString::from("/usr/bin:/bin:/usr/sbin:/sbin"));
        let args = ["mcp".to_string(), "get".to_string()];
        let bare =
            run_with_timeout(&claude, &args, Duration::from_secs(10), finder.clone()).unwrap();
        assert!(
            !bare.ok(),
            "without its directory env cannot find the interpreter"
        );
        let path = child_path(&claude, finder);
        let p = path.clone().unwrap();
        let dirs: Vec<PathBuf> = std::env::split_paths(&p).collect();
        assert_eq!(dirs[0], dir.join("bin"));
        assert_eq!(dirs[1], dir.join("lib").canonicalize().unwrap());
        assert_eq!(dirs[2], PathBuf::from("/usr/bin"));
        let out = run_with_timeout(&claude, &args, Duration::from_secs(10), path).unwrap();
        assert!(out.ok(), "{out:?}");
        assert_eq!(out.stdout.trim(), "node ran mcp get");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
