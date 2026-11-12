//! Prefs > AI & MCP.. (ADR-43, part 3): the requester on [`crate::mcp::Host`]'s state and
//! the connect flows.
//!
//! - **MCP control** On/Off is the `mcp.enabled` setting (Off also ends a `--mcp` session);
//!   the server starts or stops at once. The status line says Off, "Ready on
//!   127.0.0.1:PORT" (with the port it fell back from), "Claude connected on …, last call
//!   N s ago" (a call within [`CONNECTED_WITHIN`]: the server is stateless, so there is no
//!   connection to watch) or the error a start gave.
//! - **Permissions** are the settings' three flags and apply to the next call; changing
//!   needs reading, overwriting needs both (the boxes are ghosted otherwise).
//! - **Connect Claude Code** finds `claude`, asks `claude mcp get deluxe-painter`, then
//!   shows the exact `claude mcp add …` command in the choice requester and runs it only
//!   after Run (or offers to replace an existing entry). Without `claude` the command goes
//!   to the clipboard and a note offers "Get Claude Code".
//! - **Connect Claude Desktop** opens this platform's `.mcpb` with the OS, so Claude
//!   Desktop asks the user to install it; without Claude Desktop a note offers "Get
//!   Claude". A connect that worked turns MCP control on.
//! - **Disconnect** removes the Claude Code entry after a question (`claude mcp remove
//!   deluxe-painter -s user`); without Claude Code it offers to turn MCP control off. A
//!   Claude Desktop extension is removed in Claude Desktop.
//! - **Revoke access** writes a new token (a client that kept the old one gets 401) and
//!   turns the three permissions off; the bridge reads the new token from the endpoint
//!   file, so it is the permissions that stop it.
//!
//! Processes run off the UI thread ([`connect::spawn`]); the requester stays open and
//! MCP calls keep running under it (it holds no copy of the document).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, SystemTime};

use dpx_core::Document;
use dpx_mcp::paths::Platform;
use dpx_mcp::state::{ActivitySnapshot, LogEntry};
use dpx_ui::{ChoiceRequester, McpControl, McpRequester, McpResponse, McpView, ReqEvent};
use eframe::egui;

use crate::connect::{self, OsSystem, RunOutput, System, Task};
use crate::mcp::Status;
use crate::{PainterApp, View};

/// A call within this long makes the status say "connected".
pub const CONNECTED_WITHIN: Duration = Duration::from_secs(300);

/// What the AI & MCP requester's flows are doing.
pub struct PrefsState {
    /// The computer the flows act on (a fake in tests).
    pub system: Arc<dyn System>,
    /// What the last connect action did.
    message: Option<String>,
    task: Option<Running>,
    /// The choice requester shows this question for the requester.
    confirm: Option<Confirm>,
}

impl Default for PrefsState {
    fn default() -> Self {
        Self {
            system: Arc::new(OsSystem),
            message: None,
            task: None,
            confirm: None,
        }
    }
}

struct Running {
    step: Step,
    rx: Receiver<Result<RunOutput, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Check { claude: PathBuf, exe: PathBuf },
    Add,
    Remove,
    Open,
}

impl Step {
    fn doing(&self) -> &'static str {
        match self {
            Step::Check { .. } => "Looking for DeluxePainter in Claude Code...",
            Step::Add => "Running claude mcp add...",
            Step::Remove => "Running claude mcp remove...",
            Step::Open => "Opening the extension in Claude Desktop...",
        }
    }
}

/// The question the choice requester asks for the requester.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Confirm {
    Add { claude: PathBuf, exe: PathBuf },
    Replace { claude: PathBuf, exe: PathBuf },
    Remove { claude: PathBuf },
    TurnOff,
    GetCode,
    GetDesktop,
}

/// "12 s", "3 min", "2 h".
fn age(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h", s / 3600),
    }
}

/// "12s", "3m", "2h" (for the activity lines).
fn short_age(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

/// The status line after "Status: ".
pub fn status_text(
    on: bool,
    status: &Status,
    activity: Option<&ActivitySnapshot>,
    wanted_port: u16,
) -> String {
    match status {
        Status::Failed(e) => format!("Error: {e}"),
        Status::Off if on => "Starting...".into(),
        Status::Off => "Off".into(),
        Status::Running { port, .. } => {
            let addr = format!("127.0.0.1:{port}");
            if let Some(a) = activity
                && let Some(t) = a.last_request
                && a.active_within(CONNECTED_WITHIN)
            {
                let who = dpx_mcp::tools::asker(a.client.as_deref());
                return format!(
                    "{who} connected on {addr}, last call {} ago",
                    age(t.elapsed())
                );
            }
            if wanted_port != 0 && *port != wanted_port {
                format!("Ready on {addr} (port {wanted_port} was in use)")
            } else {
                format!("Ready on {addr}")
            }
        }
    }
}

/// One activity line: "12s draw_rectangle 10,20 20x10 colour 5", "3m export_image
/// FILE_EXISTS".
pub fn activity_line(e: &LogEntry, now: SystemTime) -> String {
    let age = short_age(now.duration_since(e.time).unwrap_or_default());
    match (&e.code, &e.detail) {
        (Some(code), _) => format!("{age} {} {code}", e.tool),
        (None, Some(detail)) => format!("{age} {detail}"),
        (None, None) => format!("{age} {}", e.tool),
    }
}

/// The approved folder as shown: the home directory as `~`, "(missing)" when the server
/// left it out, and how many more there are.
pub fn folder_text(dirs: &[PathBuf], missing: &[PathBuf], home: Option<&Path>) -> String {
    let Some(first) = dirs.first() else {
        return "none (file tools are off)".into();
    };
    let mut s = match home.and_then(|h| first.strip_prefix(h).ok()) {
        Some(rest) if !rest.as_os_str().is_empty() => {
            format!("~/{}", rest.to_string_lossy().replace('\\', "/"))
        }
        _ => first.to_string_lossy().into_owned(),
    };
    if missing.contains(first) {
        s.push_str(" (missing)");
    }
    if dirs.len() > 1 {
        s.push_str(&format!(" and {} more", dirs.len() - 1));
    }
    s
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The Claude Code command line as shown and copied.
fn shown(args: &[String]) -> String {
    connect::command_line(Platform::current(), args)
}

/// "Connect Claude Code": the exact command DeluxePainter runs after Run.
pub fn add_requester(command: &str) -> ChoiceRequester {
    ChoiceRequester::new(
        "Connect Claude Code",
        &[
            "DeluxePainter will run this command:".into(),
            command.into(),
        ],
        ["Run", "Cancel"],
        0,
        1,
    )
}

fn replace_requester(command: &str) -> ChoiceRequester {
    ChoiceRequester::new(
        "Connect Claude Code",
        &[
            "Claude Code already knows DeluxePainter. Replace its entry with this app? \
             DeluxePainter will run:"
                .into(),
            shown(&connect::remove_args()),
            command.into(),
        ],
        ["Replace", "Keep"],
        0,
        1,
    )
}

/// Claude Code is missing: the command is on the clipboard.
pub fn code_missing_requester() -> ChoiceRequester {
    ChoiceRequester::new(
        "Claude Code not found",
        &[
            "DeluxePainter cannot find Claude Code on this computer. The command that \
           connects it was copied to the clipboard: install Claude Code, then paste the \
           command into a terminal."
                .into(),
        ],
        ["Get Claude Code", "Close"],
        1,
        1,
    )
}

/// Claude Desktop is missing.
pub fn desktop_missing_requester() -> ChoiceRequester {
    ChoiceRequester::new(
        "Claude Desktop not found",
        &[
            "Claude Desktop is not installed on this computer. Get it from claude.ai/download, \
           then click Claude Desktop again."
                .into(),
        ],
        ["Get Claude", "Close"],
        1,
        1,
    )
}

const DESKTOP_REMOVE: &str = "Claude Desktop: remove the extension in its Settings > Extensions.";

/// Disconnect with Claude Code: the removal DeluxePainter runs after Remove.
pub fn remove_requester() -> ChoiceRequester {
    ChoiceRequester::new(
        "Disconnect",
        &[
            "Remove DeluxePainter from Claude Code? DeluxePainter will run:".into(),
            shown(&connect::remove_args()),
            DESKTOP_REMOVE.into(),
        ],
        ["Remove", "Cancel"],
        0,
        1,
    )
}

fn turn_off_requester() -> ChoiceRequester {
    ChoiceRequester::new(
        "Disconnect",
        &[
            "Claude Code was not found, so it has no entry to remove.".into(),
            DESKTOP_REMOVE.into(),
            "Turn MCP control off to stop every client now?".into(),
        ],
        ["Turn off", "Cancel"],
        0,
        1,
    )
}

impl PainterApp {
    /// Prefs > AI & MCP..: opens the requester.
    pub(crate) fn open_mcp_prefs(&mut self) {
        self.finish_stroke();
        let view = self.mcp_view();
        self.ui.mcp_requester = Some(McpRequester::new(view, self.view().ui_size()));
    }

    /// What the requester shows now.
    pub(crate) fn mcp_view(&self) -> McpView {
        self.mcp_view_with(&self.mcp.status())
    }

    fn mcp_view_with(&self, status: &Status) -> McpView {
        let s = &self.mcp.settings;
        let on = s.enabled || self.mcp.session;
        let activity = self.mcp.activity();
        let now = SystemTime::now();
        let p = &self.mcp_prefs;
        McpView {
            on,
            status: status_text(on, status, activity.as_ref(), s.port),
            read: s.allow_read,
            edit: s.allow_edit,
            overwrite: s.overwrite_without_asking,
            folder: folder_text(
                &s.approved_dirs,
                self.mcp.missing_dirs(),
                home_dir().as_deref(),
            ),
            activity: self
                .mcp
                .log()
                .iter()
                .map(|e| activity_line(e, now))
                .collect(),
            message: match &p.task {
                Some(t) => Some(t.step.doing().into()),
                None => p.message.clone(),
            },
            busy: p.task.is_some(),
        }
    }

    /// Gives the open requester the current state; redraws when it changed.
    fn mcp_prefs_refresh(&mut self) {
        let view = self.mcp_view();
        let screen = self.view().ui_size();
        if let Some(req) = &mut self.ui.mcp_requester {
            let before = req.size();
            req.set_screen(screen);
            if req.set_view(view) || req.size() != before {
                self.dirty = true;
            }
        }
    }

    /// Pointer and keys while the requester is open (it is modal). It redraws every second
    /// for the ages it shows.
    pub(crate) fn mcp_prefs_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        self.mcp_prefs_refresh();
        let Some(req) = &self.ui.mcp_requester else {
            return;
        };
        let at = req.origin();
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.mcp_requester.is_none() || self.ui.choice_requester.is_some() {
                break;
            }
            self.mcp_prefs_event(event, ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    pub(crate) fn mcp_prefs_event(&mut self, event: ReqEvent, ctx: &egui::Context) {
        let Some(req) = &mut self.ui.mcp_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        if let McpResponse::Pick(c) = response {
            self.mcp_prefs_act(c, ctx);
            self.mcp_prefs_refresh();
        }
    }

    fn mcp_prefs_act(&mut self, c: McpControl, ctx: &egui::Context) {
        let s = &mut self.mcp.settings;
        match c {
            McpControl::On => s.enabled = true,
            McpControl::Off => {
                s.enabled = false;
                self.mcp.session = false;
            }
            McpControl::Read => s.allow_read = !s.allow_read,
            McpControl::Edit => s.allow_edit = !s.allow_edit,
            McpControl::Overwrite => s.overwrite_without_asking = !s.overwrite_without_asking,
            McpControl::Revoke => self.mcp_prefs_revoke(),
            McpControl::ConnectCode => self.connect_claude_code(ctx),
            McpControl::ConnectDesktop => self.connect_claude_desktop(),
            McpControl::Disconnect => self.disconnect_claude(),
            // TODO(wave8-integration): the image generator's "Set up.." button (ADR-45) is
            // handled here; the requester only shows "Image generation: Picture > Generate..".
            McpControl::Close => {
                self.ui.mcp_requester = None;
                return;
            }
        }
        // The server follows at once (not only in the next pass).
        self.mcp_sync();
    }

    fn mcp_prefs_say(&mut self, message: impl Into<String>) {
        self.mcp_prefs.message = Some(message.into());
        self.dirty = true;
    }

    fn mcp_prefs_revoke(&mut self) {
        let s = &mut self.mcp.settings;
        s.allow_read = false;
        s.allow_edit = false;
        s.overwrite_without_asking = false;
        match self.mcp_revoke() {
            Ok(()) => self.mcp_prefs_say("Access revoked: new token, permissions off."),
            Err(e) => self.mcp_prefs_say(format!("Permissions off, but {e}")),
        }
    }

    /// Shows `req` for `confirm` over the AI & MCP requester.
    fn mcp_prefs_ask(&mut self, req: ChoiceRequester, confirm: Confirm) {
        if self.ui.choice_requester.is_some() || self.autosave.pending.is_some() {
            return self.mcp_prefs_say("Another question is open: try again after it.");
        }
        self.ui.choice_requester = Some(req);
        self.mcp_prefs.confirm = Some(confirm);
        self.dirty = true;
    }

    fn mcp_prefs_start(&mut self, step: Step, task: Task) {
        let rx = connect::spawn(self.mcp_prefs.system.clone(), task, self.mcp.wake.clone());
        self.mcp_prefs.message = None;
        self.mcp_prefs.task = Some(Running { step, rx });
        self.dirty = true;
    }

    fn connect_claude_code(&mut self, ctx: &egui::Context) {
        let system = self.mcp_prefs.system.clone();
        let Some(exe) = system.exe() else {
            return self.mcp_prefs_say("Cannot tell where DeluxePainter itself is.");
        };
        match system.find_claude() {
            Some(claude) => {
                let task = Task::Check {
                    claude: claude.clone(),
                };
                self.mcp_prefs_start(Step::Check { claude, exe }, task);
            }
            None => {
                ctx.copy_text(shown(&connect::add_args(&exe)));
                self.mcp_prefs_say("Claude Code not found: the command was copied.");
                self.mcp_prefs_ask(code_missing_requester(), Confirm::GetCode);
            }
        }
    }

    fn connect_claude_desktop(&mut self) {
        let system = self.mcp_prefs.system.clone();
        let Some(path) = system.find_mcpb() else {
            return self.mcp_prefs_say(format!(
                "{} is not next to the app.",
                dpx_mcp::bundle::this_mcpb_file_name()
            ));
        };
        if !system.desktop_installed() {
            self.mcp_prefs_say("Claude Desktop not found.");
            return self.mcp_prefs_ask(desktop_missing_requester(), Confirm::GetDesktop);
        }
        self.mcp_prefs_start(Step::Open, Task::Open { path });
    }

    fn disconnect_claude(&mut self) {
        match self.mcp_prefs.system.find_claude() {
            Some(claude) => self.mcp_prefs_ask(remove_requester(), Confirm::Remove { claude }),
            None => self.mcp_prefs_ask(turn_off_requester(), Confirm::TurnOff),
        }
    }

    /// The choice requester's button `button` answers the requester's question, if it asked
    /// one (from `choice_event`, which has closed the choice requester).
    pub(crate) fn mcp_prefs_answer(&mut self, button: usize, ctx: &egui::Context) -> bool {
        let confirm = self.mcp_prefs.confirm.take();
        if self.autosave.pending.is_some() {
            // Recover or Quit replaced the question: it is dropped.
            return false;
        }
        let Some(confirm) = confirm else {
            return false;
        };
        let yes = button == 0;
        match confirm {
            Confirm::Add { claude, exe } if yes => {
                self.mcp_prefs_start(Step::Add, Task::Add { claude, exe });
            }
            Confirm::Replace { claude, exe } if yes => {
                self.mcp_prefs_start(Step::Add, Task::Replace { claude, exe });
            }
            Confirm::Replace { .. } => self.mcp_prefs_say("Claude Code already has DeluxePainter."),
            Confirm::Remove { claude } if yes => {
                self.mcp_prefs_start(Step::Remove, Task::Remove { claude });
            }
            Confirm::TurnOff if yes => {
                self.mcp.settings.enabled = false;
                self.mcp.session = false;
                self.mcp_sync();
            }
            Confirm::GetCode if yes => {
                ctx.open_url(egui::OpenUrl::new_tab(connect::CLAUDE_CODE_SETUP_URL));
            }
            Confirm::GetDesktop if yes => {
                ctx.open_url(egui::OpenUrl::new_tab(connect::CLAUDE_DOWNLOAD_URL));
            }
            _ => {}
        }
        self.mcp_prefs_refresh();
        true
    }

    /// From `mcp_logic`: a connect step that has ended. Returns how soon to look again
    /// while one runs.
    pub(crate) fn mcp_prefs_poll(&mut self) -> Option<Duration> {
        let t = self.mcp_prefs.task.as_ref()?;
        let out = match t.rx.try_recv() {
            Ok(out) => out,
            Err(std::sync::mpsc::TryRecvError::Empty) => return Some(Duration::from_millis(50)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the step ended".into()),
        };
        let step = self.mcp_prefs.task.take().map(|t| t.step)?;
        self.mcp_prefs_step_ended(step, out);
        // The requester shows the outcome also under a question that opened now.
        self.mcp_prefs_refresh();
        None
    }

    fn mcp_prefs_step_ended(&mut self, step: Step, out: Result<RunOutput, String>) {
        self.dirty = true;
        match (step, out) {
            (Step::Check { claude, exe }, Ok(o)) => {
                let command = shown(&connect::add_args(&exe));
                if self.ui.mcp_requester.is_none() {
                    return;
                }
                if o.ok() {
                    self.mcp_prefs_ask(
                        replace_requester(&command),
                        Confirm::Replace { claude, exe },
                    );
                } else {
                    self.mcp_prefs_ask(add_requester(&command), Confirm::Add { claude, exe });
                }
            }
            (Step::Check { .. }, Err(e)) => self.mcp_prefs_say(format!("Claude Code: {e}")),
            (Step::Add, Ok(o)) if o.ok() => {
                self.mcp.settings.enabled = true;
                self.mcp_sync();
                self.mcp_prefs_say("Claude Code is connected: start a new Claude Code session.");
            }
            (Step::Add, out) => self.mcp_prefs_say(failed("claude mcp add", out)),
            (Step::Remove, Ok(o)) if o.ok() => self.mcp_prefs_say("Removed from Claude Code."),
            (Step::Remove, out) => self.mcp_prefs_say(failed("claude mcp remove", out)),
            (Step::Open, Ok(_)) => {
                self.mcp.settings.enabled = true;
                self.mcp_sync();
                self.mcp_prefs_say("Confirm the install in Claude Desktop.");
            }
            (Step::Open, Err(e)) => self.mcp_prefs_say(format!("Cannot open the extension: {e}")),
        }
    }
}

fn failed(what: &str, out: Result<RunOutput, String>) -> String {
    match out {
        Ok(o) => match o.first_line() {
            Some(line) => format!("{what} failed: {line}"),
            None => format!("{what} failed (exit code {:?})", o.code),
        },
        Err(e) => format!("{what} failed: {e}"),
    }
}

/// The example executable the snapshots show instead of the build's own path.
pub const SNAPSHOT_EXE: &str = "/Applications/DeluxePainter.app/Contents/MacOS/deluxepainter";

/// Snapshot options (ADR-43): `--mcp-prefs STATE` shows the AI & MCP requester; STATE is
/// `off`, `ready`, `connected` (with the calls of `--mcp-calls` in its activity and a
/// Claude client), `fallback` (port 8766 instead of 8765), `error` (a start that really
/// failed: the token directory is under `/dev/null`), `code-confirm`, `code-missing`,
/// `desktop-missing` or `disconnect` (the question over a ready requester). Without the flag
/// only `--mcp-calls` runs, as before. Returns the document to show.
pub fn snapshot(
    args: &[String],
    doc: &Document,
    ui: &mut dpx_ui::UiState,
) -> Result<Document, String> {
    let Some(state) = crate::arg_value(args, "--mcp-prefs") else {
        return Ok(crate::mcp::snapshot_calls(args, doc)?.unwrap_or_else(|| doc.clone()));
    };
    let file = crate::arg_value(args, "--mcp-calls");
    let mut app = crate::mcp::calls_app(file.as_deref(), doc)?;
    app.ui = ui.clone();
    app.mcp.settings = crate::mcp::McpSettings::default();
    let ready = |port: u16| Status::Running {
        url: dpx_mcp::endpoint::url_for(port),
        port,
    };
    let exe = Path::new(SNAPSHOT_EXE);
    let status = match state.as_str() {
        "off" => Status::Off,
        "ready" | "code-confirm" | "code-missing" | "desktop-missing" | "disconnect" => {
            app.mcp.settings.enabled = true;
            ready(8765)
        }
        "connected" => {
            app.mcp.settings.enabled = true;
            if let Some(s) = app.mcp.server_state() {
                s.activity().client("claude-code via deluxepainter-bridge");
                s.activity().tool_call();
            }
            ready(8765)
        }
        "fallback" => {
            app.mcp.settings.enabled = true;
            ready(8766)
        }
        "error" => {
            // A real failed start; without approved directories it creates none.
            let dirs = std::mem::take(&mut app.mcp.settings.approved_dirs);
            app.mcp_stop();
            app.mcp.settings.enabled = true;
            app.mcp.config_dir = Some(PathBuf::from("/dev/null/DeluxePainter"));
            app.mcp_sync();
            app.mcp.settings.approved_dirs = dirs;
            app.mcp.status()
        }
        _ => {
            return Err(format!(
                "--mcp-prefs expects off, ready, connected, fallback, error, code-confirm, \
                 code-missing, desktop-missing or disconnect, not '{state}'"
            ));
        }
    };
    let command = shown(&connect::add_args(exe));
    match state.as_str() {
        "code-confirm" => app.ui.choice_requester = Some(add_requester(&command)),
        "code-missing" => {
            app.mcp_prefs.message = Some("Claude Code not found: the command was copied.".into());
            app.ui.choice_requester = Some(code_missing_requester());
        }
        "desktop-missing" => {
            app.mcp_prefs.message = Some("Claude Desktop not found.".into());
            app.ui.choice_requester = Some(desktop_missing_requester());
        }
        "disconnect" => app.ui.choice_requester = Some(remove_requester()),
        _ => {}
    }
    let view = app.mcp_view_with(&status);
    app.ui.mcp_requester = Some(McpRequester::new(view, app.view().ui_size()));
    let doc = app.engine.document().clone();
    *ui = app.ui.clone();
    app.mcp_stop();
    Ok(doc)
}

#[cfg(test)]
mod tests;
