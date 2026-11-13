//! The running GUI as MCP workspace (ADR-43, part 2).
//!
//! - **Server:** while the `mcp` settings section is enabled (or for one session with
//!   `--mcp`), the app runs `dpx_mcp`'s Streamable HTTP server on `127.0.0.1`. It starts
//!   with the app, stops when it is turned off and on exit (the endpoint file goes with
//!   it). The bearer token lives in `<config dir>/mcp-token` (0600 on Unix), so a client
//!   configured once reconnects after a restart; [`PainterApp::mcp_revoke`] rotates it.
//! - **Off sticks:** the app keeps `mcp-app.json`'s `enabled` equal to MCP control (and on
//!   exit to the saved setting), and the first GUI holds `mcp-app.lock` while it runs
//!   ([`claim_instance`]). So the bridge never starts the app while MCP control is off, and
//!   never starts a second one; a `--mcp` start while another GUI runs ends at once.
//! - **Jobs:** the server queues jobs and wakes the window (`request_repaint`);
//!   [`PainterApp::mcp_logic`] runs them on the UI thread from `App::logic`, before the
//!   pass handles input, also while the window is hidden. Reading jobs always run. A
//!   changing job (or a question) is refused with `EDITOR_BUSY` while a requester is open,
//!   and waits while a mouse button is held on the picture, at most [`HOLD_LIMIT`].
//! - **Changes** go the way of a menu command: Stencil > Paint ends, a gesture in progress
//!   is committed, CLR keeps a fixed background, the area stencil applies, frame commands
//!   move the stencils with their frame; each is one undo step of the shown page. After
//!   it the colours and scroll position are clamped and the screen is redrawn.
//! - **Files:** `save_project` / `export_image` write with `files.rs` (the bytes Picture >
//!   Save / Export write), inside the approved directories; replacing a file asks with
//!   the choice requester unless "overwrite without asking" is set.
//!
//! Prefs > AI & MCP.. (`mcp_prefs.rs`, part 3) shows [`Host`]'s status and activity log and
//! edits its settings. Its requester holds no copy of the document, so changing calls run
//! under it ([`PainterApp::mcp_editor_busy`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dpx_core::{BrushImage, Command, Document, Engine, IndexedImage, Outcome, Request};
use dpx_formats::project::ProjectExtras;
use dpx_mcp::endpoint;
use dpx_mcp::generation::{ApplyTo, GenApply, GenHost};
use dpx_mcp::http::{self, HttpOptions, HttpServer};
use dpx_mcp::jobs::{self, JobKind, JobQueue};
use dpx_mcp::pictures::Format;
use dpx_mcp::state::{ActivitySnapshot, LogEntry};
use dpx_mcp::{
    Brush, Codec, FileAccess, Overwrite, PictureInfo, Question, ServerState, Settings, Slot,
    ToolError, Workspace,
};
use dpx_ui::{Action, ChoiceRequester, PaintMode};
use eframe::egui;
use serde_json::{Map, Value, json};

use crate::PainterApp;
use crate::files::{self, PictureExtras, SaveFormat};

/// How long a changing call waits while a mouse button is held on the picture; then it is
/// refused with `EDITOR_BUSY`.
pub const HOLD_LIMIT: Duration = Duration::from_secs(10);

/// The `mcp` section of the settings file (ADR-33).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpSettings {
    /// The server runs while the app runs.
    pub enabled: bool,
    /// The port tried first (then the next ten, then any free one; `0`: any).
    pub port: u16,
    /// AI clients may read the document (off: only `get_app_status` answers).
    pub allow_read: bool,
    /// AI clients may change the document.
    pub allow_edit: bool,
    /// `overwrite: true` replaces an existing file without asking the user.
    pub overwrite_without_asking: bool,
    /// The only directories file tools may use.
    pub approved_dirs: Vec<PathBuf>,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: http::DEFAULT_PORT,
            allow_read: true,
            allow_edit: true,
            overwrite_without_asking: false,
            approved_dirs: default_pictures_dir().into_iter().collect(),
        }
    }
}

impl McpSettings {
    /// Reads the fields it knows from the `mcp` section; the rest keeps `d`.
    pub fn from_json(o: &Map<String, Value>, d: &McpSettings) -> McpSettings {
        let flag = |k: &str, def: bool| o.get(k).and_then(Value::as_bool).unwrap_or(def);
        McpSettings {
            enabled: flag("enabled", d.enabled),
            port: o
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(d.port),
            allow_read: flag("allow_read", d.allow_read),
            allow_edit: flag("allow_edit", d.allow_edit),
            overwrite_without_asking: flag("overwrite_without_asking", d.overwrite_without_asking),
            approved_dirs: match o.get("approved_dirs").and_then(Value::as_array) {
                Some(a) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect(),
                None => d.approved_dirs.clone(),
            },
        }
    }

    pub fn to_json(&self) -> Value {
        let dirs: Vec<String> = self
            .approved_dirs
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect();
        json!({
            "enabled": self.enabled,
            "port": self.port,
            "allow_read": self.allow_read,
            "allow_edit": self.allow_edit,
            "overwrite_without_asking": self.overwrite_without_asking,
            "approved_dirs": dirs,
        })
    }

    /// The server's permissions for these settings with `access`. Overwriting without
    /// asking needs both permissions, as the requester ghosts that box otherwise (a ticked
    /// box under the ghosting stays saved, but asks).
    fn server_settings(&self, access: FileAccess) -> Settings {
        Settings {
            read: self.allow_read,
            edit: self.allow_edit,
            overwrite: if self.allow_read && self.allow_edit && self.overwrite_without_asking {
                Overwrite::Allowed
            } else {
                Overwrite::Ask
            },
            access,
        }
    }
}

/// `<Pictures>/DeluxePainter`: the approved directory of a new settings file.
pub fn default_pictures_dir() -> Option<PathBuf> {
    pictures_dir_with(|k| std::env::var_os(k), dpx_mcp::paths::Platform::current())
}

/// [`default_pictures_dir`] with the environment and platform given (for tests): macOS and
/// Windows `~/Pictures` (`%USERPROFILE%\Pictures`), elsewhere `$XDG_PICTURES_DIR` or
/// `~/Pictures`. The XDG `user-dirs.dirs` file is not read.
fn pictures_dir_with(
    env: impl Fn(&str) -> Option<std::ffi::OsString>,
    platform: dpx_mcp::paths::Platform,
) -> Option<PathBuf> {
    use dpx_mcp::paths::Platform;
    let env = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let pictures = match platform {
        Platform::MacOs => env("HOME")?.join("Pictures"),
        Platform::Windows => env("USERPROFILE")?.join("Pictures"),
        Platform::Other => {
            env("XDG_PICTURES_DIR").or_else(|| env("HOME").map(|h| h.join("Pictures")))?
        }
    };
    Some(pictures.join("DeluxePainter"))
}

/// The approved directories that exist (the default one is created when missing) and the
/// ones that do not.
fn approve(dirs: &[PathBuf]) -> (FileAccess, Vec<PathBuf>) {
    let default = default_pictures_dir();
    let (mut ok, mut missing) = (Vec::new(), Vec::new());
    for d in dirs {
        if !d.is_dir()
            && default.as_ref() == Some(d)
            && let Err(e) = std::fs::create_dir_all(d)
        {
            eprintln!("mcp: cannot create {}: {e}", d.display());
        }
        if d.is_dir() {
            ok.push(d.clone());
        } else {
            missing.push(d.clone());
        }
    }
    let access = FileAccess::new(ok).unwrap_or_else(|e| {
        eprintln!("mcp: {e}");
        FileAccess::off()
    });
    (access, missing)
}

/// The server as the Prefs requester shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Off,
    /// Listening on this URL.
    Running {
        #[cfg_attr(not(test), allow(dead_code))]
        url: String,
        port: u16,
    },
    /// It could not start (the reason); it tries again when the settings change or MCP
    /// control is turned off and on.
    Failed(String),
}

/// A running server (or, in tests and the snapshot flag, a queue without one).
struct Running {
    server: Option<HttpServer>,
    state: Arc<ServerState>,
    queue: JobQueue,
    /// The settings the server runs with.
    applied: McpSettings,
    /// Approved directories that do not exist (not approved).
    missing: Vec<PathBuf>,
}

/// The reply of an open question.
type Answer = Box<dyn FnOnce(Result<bool, ToolError>) + Send>;

/// A question shown in the choice requester for an MCP call.
struct Asked {
    answer: Answer,
    deadline: Instant,
}

/// The app's MCP state.
pub struct Host {
    pub settings: McpSettings,
    /// `--mcp`: the server runs this session whatever `settings.enabled` says (not saved).
    pub session: bool,
    /// Where the token, endpoint and launch files go (the settings directory); `None`
    /// (tests): a random token per start and no files.
    pub config_dir: Option<PathBuf>,
    /// Wakes the window when a job is queued; `None` in tests.
    pub wake: Option<egui::Context>,
    running: Option<Running>,
    failed: Option<(McpSettings, String)>,
    question: Option<Asked>,
    /// The name `create_document` gave the picture (until it is saved or replaced).
    pub name: Option<String>,
    /// The shown page is page2: flipped by every Picture > Spare > Swap.
    pub swapped: bool,
    pub hold_limit: Duration,
    pub ask_timeout: Duration,
    /// This GUI's claim on `mcp-app.lock` ([`claim_instance`]): the bridge sees the app
    /// running and never starts another one.
    pub instance: Option<endpoint::Claim>,
    /// What `mcp-app.json` says now about MCP control (`None`: not written yet).
    recorded: Option<bool>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            settings: McpSettings::default(),
            session: false,
            config_dir: None,
            wake: None,
            running: None,
            failed: None,
            question: None,
            name: None,
            swapped: false,
            hold_limit: HOLD_LIMIT,
            ask_timeout: jobs::ASK_TIMEOUT,
            instance: None,
            recorded: None,
        }
    }
}

/// The GUI's claim on `dir` (`mcp-app.lock`). `Ok(None)`: no config directory, or another
/// DeluxePainter holds it and this one is a second window (it runs without the claim);
/// `Err`: another one holds it and this start is a `--mcp` one, which must not open a
/// second window for an MCP client.
pub fn claim_instance(
    dir: Option<&Path>,
    mcp_flag: bool,
) -> Result<Option<endpoint::Claim>, String> {
    let Some(dir) = dir else {
        return Ok(None);
    };
    match endpoint::claim_app(dir) {
        Ok(Some(c)) => Ok(Some(c)),
        Ok(None) if mcp_flag => Err(
            "DeluxePainter is already running: --mcp does not start a second one (turn on MCP control in its Prefs > AI & MCP)".into(),
        ),
        Ok(None) => Ok(None),
        Err(e) => {
            eprintln!("mcp: cannot claim {}: {e}", endpoint::APP_LOCK);
            Ok(None)
        }
    }
}

impl Host {
    /// MCP control is on: the setting, or `--mcp` for this session.
    pub fn wanted(&self) -> bool {
        self.settings.enabled || self.session
    }

    pub fn status(&self) -> Status {
        match (&self.running, &self.failed) {
            (Some(r), _) => match &r.server {
                Some(s) => Status::Running {
                    url: s.url(),
                    port: s.port(),
                },
                None => Status::Off,
            },
            // A failed start is only news while MCP control is on.
            (None, Some((_, e))) if self.wanted() => Status::Failed(e.clone()),
            (None, _) => Status::Off,
        }
    }

    /// Requests and the client's name, while the server runs.
    pub fn activity(&self) -> Option<ActivitySnapshot> {
        Some(self.running.as_ref()?.state.activity().snapshot())
    }

    /// The last tool calls (oldest first), while the server runs.
    pub fn log(&self) -> Vec<LogEntry> {
        self.running
            .as_ref()
            .map(|r| r.state.activity().log())
            .unwrap_or_default()
    }

    /// Approved directories that do not exist and so are not approved.
    pub fn missing_dirs(&self) -> &[PathBuf] {
        self.running.as_ref().map_or(&[], |r| &r.missing)
    }

    /// The bearer token of the running server (tests). Never draw it on screen, put it in
    /// a command or write it into a project: the bridge reads it from the endpoint file.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn token(&self) -> Option<&str> {
        self.running
            .as_ref()?
            .server
            .as_ref()
            .map(HttpServer::token)
    }

    /// A question for an MCP client is open.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn asking(&self) -> bool {
        self.question.is_some()
    }

    /// The running server's settings, encoders and activity.
    pub(crate) fn server_state(&self) -> Option<&Arc<ServerState>> {
        self.running.as_ref().map(|r| &r.state)
    }

    /// Serves `queue` with `state` without an HTTP server (tests, `--mcp-calls`).
    pub(crate) fn attach(&mut self, queue: JobQueue, state: Arc<ServerState>) {
        self.running = Some(Running {
            server: None,
            state,
            queue,
            applied: self.settings.clone(),
            missing: Vec::new(),
        });
    }
}

/// The question as the choice requester shows it: Return and Esc keep the file, only a
/// click on the first button replaces it (it appears without the user asking for it).
pub fn question_requester(q: &Question) -> ChoiceRequester {
    ChoiceRequester::new(
        &q.title,
        std::slice::from_ref(&q.text),
        [q.yes.as_str(), q.no.as_str()],
        1,
        1,
    )
}

fn busy_requester() -> ToolError {
    ToolError::new(
        "EDITOR_BUSY",
        "a requester is open in DeluxePainter, so nothing was changed; reading still works. \
         Try again after the user has closed it.",
    )
}

fn busy_painting(limit: Duration) -> ToolError {
    ToolError::new(
        "EDITOR_BUSY",
        format!(
            "the user has been painting (a mouse button held on the picture) for more than {} s, \
             so nothing was changed. Try again.",
            limit.as_secs()
        ),
    )
}

fn stopped() -> ToolError {
    ToolError::new(
        "APP_UNAVAILABLE",
        "the MCP server of DeluxePainter was turned off",
    )
}

/// The app's own file code behind `save_project` and `export_image`: the bytes Picture >
/// Save and Export write (`files::encode_as`). A project keeps no spare page, as Picture >
/// Save.
#[derive(Debug)]
pub struct AppCodec;

impl Codec for AppCodec {
    fn encode_project(
        &self,
        doc: &Document,
        extras: &ProjectExtras,
        _spare: Option<&IndexedImage>,
    ) -> Result<Vec<u8>, ToolError> {
        files::encode_as(doc, &PictureExtras::from(extras), SaveFormat::Project)
            .map(|(bytes, _)| bytes)
            .map_err(|e| ToolError::new("IO_ERROR", e))
    }

    fn encode_export(
        &self,
        doc: &Document,
        extras: &ProjectExtras,
        format: Format,
    ) -> Result<(Vec<u8>, Vec<String>), ToolError> {
        let (format, code) = match format {
            Format::Ilbm => (SaveFormat::Ilbm, "HARDWARE_LIMIT"),
            Format::Anim => (SaveFormat::Anim, "HARDWARE_LIMIT"),
            Format::Png => (SaveFormat::Png, "INVALID_FILE"),
        };
        files::encode_as(doc, &PictureExtras::from(extras), format)
            .map_err(|e| ToolError::new(code, e))
    }
}

/// Starts the HTTP server for `s`.
fn start_server(
    s: &McpSettings,
    dir: Option<PathBuf>,
    wake: Option<egui::Context>,
) -> Result<Running, String> {
    let (access, missing) = approve(&s.approved_dirs);
    let token = match &dir {
        Some(d) => Some(
            endpoint::load_or_create_token(d)
                .map_err(|e| format!("cannot keep the token in {}: {e}", d.display()))?,
        ),
        None => None,
    };
    let (exec, queue) = jobs::channel(move || {
        if let Some(ctx) = &wake {
            ctx.request_repaint();
        }
    });
    let state = ServerState::with_codec(s.server_settings(access), Arc::new(AppCodec));
    let server = HttpServer::start(
        exec,
        state.clone(),
        HttpOptions {
            port: s.port,
            token,
            endpoint_dir: dir.clone(),
        },
    )
    .map_err(|e| format!("cannot start the MCP server: {e}"))?;
    Ok(Running {
        server: Some(server),
        state,
        queue,
        applied: s.clone(),
        missing,
    })
}

impl PainterApp {
    /// Once per pass, from `App::logic` (also while the window is hidden): starts, stops or
    /// updates the server for the settings, times out an open question and runs the queued
    /// jobs. Asks for another pass while a job waits or a question is open.
    pub(crate) fn mcp_logic(&mut self, ctx: &egui::Context) {
        self.mcp_sync();
        let asked = self.mcp_question_tick();
        let held = self.mcp_drain();
        // Part 3: a connect step of Prefs > AI & MCP that has ended.
        let step = self.mcp_prefs_poll();
        if let Some(wait) = [asked, held, step].into_iter().flatten().min() {
            ctx.request_repaint_after(wait);
        }
    }

    /// The server follows the settings: on, off, another port, new permissions or
    /// directories (live, without a restart). `mcp-app.json` follows MCP control.
    pub(crate) fn mcp_sync(&mut self) {
        let want = self.mcp.wanted();
        self.mcp_record(want);
        let Some(r) = &self.mcp.running else {
            if !want {
                // Off forgets a failed start: the status says Off, and On tries again.
                self.mcp.failed = None;
                return;
            }
            let failed_before = self
                .mcp
                .failed
                .as_ref()
                .is_some_and(|(s, _)| *s == self.mcp.settings);
            if !failed_before {
                self.mcp_start();
            }
            return;
        };
        let served = r.server.is_some();
        if served && !want {
            return self.mcp_stop();
        }
        if r.applied == self.mcp.settings {
            return;
        }
        if served && r.applied.port != self.mcp.settings.port {
            self.mcp_stop();
            return self.mcp_start();
        }
        let (access, missing) = approve(&self.mcp.settings.approved_dirs);
        let settings = self.mcp.settings.clone();
        if let Some(r) = &mut self.mcp.running {
            r.state.set_settings(settings.server_settings(access));
            r.applied = settings;
            r.missing = missing;
        }
    }

    fn mcp_start(&mut self) {
        let s = self.mcp.settings.clone();
        match start_server(&s, self.mcp.config_dir.clone(), self.mcp.wake.clone()) {
            Ok(r) => {
                if let Some(server) = &r.server {
                    eprintln!("mcp: server ready on {}", server.url());
                }
                self.mcp.running = Some(r);
                self.mcp.failed = None;
            }
            Err(e) => {
                eprintln!("mcp: {e}");
                self.mcp.failed = Some((s, e));
            }
        }
    }

    /// Writes `mcp-app.json` when MCP control changed: on records this executable and that
    /// the bridge may start it; off is written over an existing record only (an app whose
    /// MCP control was never on leaves no file), so the bridge never starts the app, or a
    /// second one, while it is off.
    fn mcp_record(&mut self, enabled: bool) {
        if self.mcp.recorded == Some(enabled) {
            return;
        }
        self.mcp.recorded = Some(enabled);
        let Some(dir) = &self.mcp.config_dir else {
            return;
        };
        if !enabled && !endpoint::app_path(dir).exists() {
            return;
        }
        let Some(launch) = endpoint::this_app(enabled) else {
            return;
        };
        if let Err(e) = endpoint::write_app_file(dir, &launch) {
            eprintln!(
                "mcp: cannot write {}: {e}",
                endpoint::app_path(dir).display()
            );
        }
    }

    /// On exit: the server stops, and `mcp-app.json` keeps the saved setting (what the next
    /// start does; a `--mcp` session is not saved).
    pub(crate) fn mcp_exit(&mut self) {
        self.mcp_stop();
        let saved = self.mcp.settings.enabled;
        self.mcp_record(saved);
        self.mcp.instance = None;
    }

    /// Stops the server: queued calls and an open question fail, the endpoint file is
    /// removed.
    pub(crate) fn mcp_stop(&mut self) {
        let Some(mut r) = self.mcp.running.take() else {
            return;
        };
        r.queue.refuse_all(&stopped());
        self.mcp_close_question(Err(stopped()));
        if let Some(server) = r.server.take() {
            server.stop();
            eprintln!("mcp: server stopped");
        }
        r.queue.refuse_all(&stopped());
    }

    /// A new token (stored for the next start); requests with the old one get 401 at once.
    /// Prefs > AI & MCP's Revoke access also turns the permissions off (`mcp_prefs.rs`).
    pub(crate) fn mcp_revoke(&mut self) -> Result<(), String> {
        let token = match &self.mcp.config_dir {
            Some(d) => endpoint::rotate_token(d),
            None => endpoint::new_token(),
        }
        .map_err(|e| format!("cannot make a new token: {e}"))?;
        if let Some(server) = self.mcp.running.as_mut().and_then(|r| r.server.as_mut()) {
            server
                .set_token(token)
                .map_err(|e| format!("cannot write the endpoint file: {e}"))?;
        }
        Ok(())
    }

    /// Runs the queued jobs; returns how soon to look again when one is held.
    fn mcp_drain(&mut self) -> Option<Duration> {
        loop {
            let job = self.mcp.running.as_mut()?.queue.take()?;
            if job.kind() == JobKind::Read {
                job.run(&mut GuiWorkspace(self));
                continue;
            }
            if self.mcp_editor_busy() {
                job.refuse(busy_requester());
                continue;
            }
            if self.gesture_held() {
                let limit = self.mcp.hold_limit;
                let waited = job.waited();
                if waited >= limit {
                    job.refuse(busy_painting(limit));
                    continue;
                }
                self.mcp.running.as_mut()?.queue.hold(job);
                return Some((limit - waited).min(Duration::from_millis(50)));
            }
            job.run(&mut GuiWorkspace(self));
        }
    }

    /// A requester is open that changing calls must not run under: any but Prefs > AI &
    /// MCP, which holds no copy of the document (part 3: the user may watch the calls).
    pub(crate) fn mcp_editor_busy(&mut self) -> bool {
        let prefs = self.ui.mcp_requester.take();
        let busy = self.modal_open();
        self.ui.mcp_requester = prefs;
        busy
    }

    /// An open question that timed out or lost its requester is answered.
    fn mcp_question_tick(&mut self) -> Option<Duration> {
        let deadline = self.mcp.question.as_ref()?.deadline;
        let ours = self.ui.choice_requester.is_some() && self.autosave.pending.is_none();
        if !ours {
            self.mcp_close_question(Err(ToolError::new(
                "EDITOR_BUSY",
                "the question was closed by another requester",
            )));
            return None;
        }
        let now = Instant::now();
        if now >= deadline {
            self.mcp_close_question(Err(ToolError::new(
                jobs::NO_ANSWER,
                format!(
                    "nobody answered within {} s",
                    self.mcp.ask_timeout.as_secs()
                ),
            )));
            return None;
        }
        Some(deadline - now)
    }

    /// Answers the open question with `answer` and closes its requester (if it is still
    /// shown).
    fn mcp_close_question(&mut self, answer: Result<bool, ToolError>) {
        let Some(q) = self.mcp.question.take() else {
            return;
        };
        if self.autosave.pending.is_none() {
            self.ui.choice_requester = None;
            self.dirty = true;
        }
        (q.answer)(answer);
    }

    /// The choice requester's button `button` answers the MCP question, if one is open
    /// (from `choice_event`, which has closed the requester).
    pub(crate) fn mcp_answer(&mut self, button: usize) -> bool {
        if self.autosave.pending.is_some() {
            return false;
        }
        let Some(q) = self.mcp.question.take() else {
            return false;
        };
        (q.answer)(Ok(button == 0));
        true
    }

    /// What a menu command does first (ADR-39, ADR-14): Stencil > Paint ends and a gesture
    /// in progress (a polygon, a curve's bend, typed text) is committed.
    fn mcp_prepare(&mut self) {
        self.menu_item_ends_stencil_paint(Action::Command("mcp.call"));
        self.finish_stroke();
        // ADR-44: a layer name being edited is taken first, as a click elsewhere takes it.
        self.finish_layer_rename();
    }

    /// The bookkeeping after a change, as after a requester's command.
    fn mcp_changed(&mut self) {
        self.clamp_colors();
        self.clamp_scroll();
        self.dirty = true;
    }

    /// The spare page as a swap would show it (`take_spare`'s rules, without changing it).
    fn mcp_spare(&self) -> Option<Document> {
        let s = self.menu_data.spare.as_ref()?;
        let doc = self.engine.document();
        let spare = s.engine.document();
        // ADR-45: a generated spare page keeps its own format and palette.
        if self.menu_data.spare_own.is_some() {
            return Some(spare.clone());
        }
        let blank = || {
            Document::from_picture(
                IndexedImage::new(doc.page().width(), doc.page().height(), 0),
                doc.palette().clone(),
                doc.target(),
            )
        };
        Some(if spare.target() != doc.target() {
            blank()
        } else if spare.palette() == doc.palette() {
            spare.clone()
        } else if spare.palette().len() == doc.palette().len() {
            spare.with_palette(doc.palette().clone())
        } else {
            blank()
        })
    }

    /// Replaces the shown picture like Picture > Load (not undoable).
    fn mcp_replace_picture(&mut self, doc: Document, info: PictureInfo) {
        self.mcp_prepare();
        let (doc, grown) = files::page_at_least_screen(doc);
        if let Some(note) = grown {
            eprintln!("mcp: {note}");
        }
        let extras = if info.path.is_none() && info.extras == ProjectExtras::default() {
            PictureExtras::for_new_picture(doc.palette())
        } else {
            PictureExtras::from(&info.extras)
        };
        self.menu_data.loaded_palette = Some(doc.palette().clone());
        self.menu_data.cycled = None;
        self.stroke = None;
        self.anim.playing = None;
        self.engine = Engine::new(doc);
        self.extras = extras;
        if let Some(dir) = info.path.as_ref().and_then(|p| p.parent()) {
            self.last_drawer = Some(dir.to_string_lossy().into_owned());
        }
        self.picture_path = info.path;
        self.mcp.name = info.name;
        self.scroll = (0, 0);
        self.mcp_changed();
        if info.saved {
            self.mark_clean();
        } else {
            // Nothing on disk holds it: unsaved (and autosaved) whatever its revision.
            self.autosave.saved_revision = None;
        }
    }

    /// MCP's undo and redo: the GUI's own step (ADR-45: with the page's details of a
    /// recorded ReplacePicture step, e.g. a generated picture applied over MCP).
    fn mcp_history(&mut self, redo: bool) -> Result<Outcome, ToolError> {
        self.mcp_prepare();
        let out = self.page_history_step(redo);
        self.mcp_changed();
        Ok(out?)
    }
}

/// The GUI's editor state for the tools, borrowed for one job. Changes paint the active
/// layer; the layer tools (ADR-44) run the engine's layer commands through `execute`, as the
/// layer panel does, and the panel follows the document on the next redraw.
struct GuiWorkspace<'a>(&'a mut PainterApp);

impl Workspace for GuiWorkspace<'_> {
    fn kind(&self) -> &'static str {
        "gui"
    }

    fn engine(&self) -> &Engine {
        &self.0.engine
    }

    fn execute(&mut self, request: Request) -> Result<Outcome, ToolError> {
        let app = &mut *self.0;
        let Request {
            command,
            expected_revision,
            dry_run,
        } = request;
        if dry_run {
            let command = app.with_background_and_area(command);
            return Ok(app.engine.execute(Request {
                command,
                expected_revision,
                dry_run,
            })?);
        }
        app.mcp_prepare();
        // As `run`: CLR on a fixed background, the area stencil (Stencil > Paint has ended).
        let command = app.with_background_and_area(command);
        let out = app.execute_request(Request {
            command,
            expected_revision,
            dry_run: false,
        });
        app.mcp_changed();
        Ok(out?)
    }

    fn undo(&mut self) -> Result<Outcome, ToolError> {
        self.0.mcp_history(false)
    }

    fn redo(&mut self) -> Result<Outcome, ToolError> {
        self.0.mcp_history(true)
    }

    fn replace_picture(&mut self, doc: Document, info: PictureInfo) -> Result<(), ToolError> {
        self.0.mcp_replace_picture(doc, info);
        Ok(())
    }

    fn shown(&self) -> Slot {
        if self.0.mcp.swapped {
            Slot::Second
        } else {
            Slot::First
        }
    }

    fn spare(&self) -> Option<Document> {
        self.0.mcp_spare()
    }

    fn set_spare_page(&mut self, page: IndexedImage) -> Result<(), ToolError> {
        let app = &mut *self.0;
        let mut other = app.take_spare();
        let result = other.engine.execute(Command::SetPage {
            width: page.width(),
            height: page.height(),
            pixels: page.pixels().to_vec(),
        });
        app.menu_data.spare = Some(other);
        app.dirty = true;
        result.map(|_| ()).map_err(Into::into)
    }

    fn swap_spare(&mut self) -> Result<(), ToolError> {
        let app = &mut *self.0;
        app.mcp_prepare();
        app.swap_spare();
        app.mcp_changed();
        Ok(())
    }

    fn set_brush(&mut self, brush: Brush) -> Result<(), ToolError> {
        let app = &mut *self.0;
        app.mcp_prepare();
        let b = BrushImage::from_image(&brush.image, brush.transparent);
        app.custom_brush = Some(b.clone());
        app.picked_brush = Some(b);
        app.tools.brush_gen += 1;
        app.leave_anim_brush(); // ADR-36
        app.tools.brush_palette = Some(app.engine.document().palette().clone());
        // As after a pick-up.
        app.ui.mode = PaintMode::Matte;
        app.dirty = true;
        Ok(())
    }

    fn brush_size(&self) -> Option<(u32, u32)> {
        self.0.custom_brush.as_ref().map(|b| (b.width, b.height))
    }

    fn picture_name(&self) -> Option<String> {
        let app = &*self.0;
        app.mcp.name.clone().or_else(|| {
            app.picture_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
        })
    }

    fn set_picture_name(&mut self, name: Option<String>) {
        self.0.mcp.name = name;
    }

    fn modified(&self) -> bool {
        self.0.unsaved()
    }

    fn mark_saved(&mut self, path: &Path, revision: u64) {
        let app = &mut *self.0;
        app.picture_path = Some(path.to_path_buf());
        app.last_drawer = path.parent().map(|p| p.to_string_lossy().into_owned());
        app.mcp.name = None;
        if revision == app.engine.document().revision() {
            app.mark_clean();
        } else {
            app.autosave.saved_revision = Some(revision);
        }
        app.dirty = true;
    }

    fn project_extras(&self) -> ProjectExtras {
        ProjectExtras::from(&self.0.extras)
    }

    fn bg_color(&self) -> u8 {
        self.0.ui.bg
    }

    /// ADR-45: Picture > Generate..'s generator, jobs and choices.
    fn image_generation(&mut self) -> Result<GenHost, ToolError> {
        Ok(self.0.mcp_gen_host())
    }

    /// ADR-45: the Generate requester's Apply, as a menu command (Stencil > Paint ends, a
    /// gesture in progress is committed first).
    fn apply_generated(&mut self, apply: GenApply) -> Result<(), ToolError> {
        let app = &mut *self.0;
        app.mcp_prepare();
        let to = match apply.to {
            ApplyTo::Spare => dpx_ui::GenTarget::Spare,
            ApplyTo::Picture => dpx_ui::GenTarget::Picture,
            ApplyTo::Brush => dpx_ui::GenTarget::Brush,
            ApplyTo::NewLayer => dpx_ui::GenTarget::NewLayer,
        };
        let applied = app.apply_generated(&apply.result, to, apply.format, apply.palette);
        app.mcp_changed();
        applied.map_err(|e| ToolError::new("APPLY_FAILED", e))
    }

    fn ask(&mut self, question: Question, answer: Answer) {
        let app = &mut *self.0;
        // Over Prefs > AI & MCP the question shows (the choice requester is drawn on top).
        if app.mcp_editor_busy() || app.mcp.question.is_some() {
            return answer(Err(busy_requester()));
        }
        // As any requester that opens: a gesture in progress is committed, the strip goes.
        app.finish_stroke();
        app.ui.menu.close();
        app.ui.choice_requester = Some(question_requester(&question));
        app.mcp.question = Some(Asked {
            answer,
            deadline: Instant::now() + app.mcp.ask_timeout,
        });
        app.dirty = true;
    }

    fn log(&mut self, entry: &str) {
        if let Some(r) = &self.0.mcp.running {
            r.state.activity().note(entry);
        }
    }
}

/// `--mcp-calls FILE.json`: runs a JSON array of `{"tool": NAME, "arguments": {...}}` on a
/// GUI workspace holding `doc` (read and edit allowed, no file access) and returns the
/// document afterwards; each result goes to stderr. `Ok(None)` without the flag.
pub fn snapshot_calls(args: &[String], doc: &Document) -> Result<Option<Document>, String> {
    let Some(file) = crate::arg_value(args, "--mcp-calls") else {
        return Ok(None);
    };
    let app = calls_app(Some(&file), doc)?;
    Ok(Some(app.engine.document().clone()))
}

/// An app holding `doc` that serves a job queue without an HTTP server (read and edit
/// allowed, no file access), after running the calls of `file` ([`snapshot_calls`]); its
/// activity log keeps them (Prefs > AI & MCP's snapshots).
pub(crate) fn calls_app(file: Option<&str>, doc: &Document) -> Result<PainterApp, String> {
    let mut app = PainterApp::new();
    app.engine = Engine::new(doc.clone());
    app.mcp.settings.approved_dirs.clear();
    let (exec, queue) = jobs::channel(|| {});
    let state = ServerState::with_codec(
        app.mcp.settings.server_settings(FileAccess::off()),
        Arc::new(AppCodec),
    );
    app.mcp.attach(queue, state.clone());
    let Some(file) = file else {
        return Ok(app);
    };
    let text = std::fs::read_to_string(file).map_err(|e| format!("--mcp-calls {file}: {e}"))?;
    let list: Vec<Value> =
        serde_json::from_str(&text).map_err(|e| format!("--mcp-calls {file}: {e}"))?;
    let mut calls = Vec::new();
    for v in list {
        let tool = v
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("--mcp-calls {file}: every entry needs \"tool\""))?;
        let arguments = v.get("arguments").and_then(Value::as_object).cloned();
        calls.push((tool.to_string(), arguments));
    }
    let ctx = dpx_mcp::tools::ToolCtx {
        exec,
        state,
        transport: "snapshot",
    };
    let results = dpx_mcp::tools::spawn_calls(ctx, calls);
    loop {
        app.mcp_drain();
        match results.recv_timeout(Duration::from_millis(5)) {
            Ok((name, Some(r))) => {
                let v = r.structured_content.unwrap_or_default();
                match v.get("code").and_then(Value::as_str) {
                    Some(code) => eprintln!(
                        "mcp {name}: {code}: {}",
                        v.get("message").and_then(Value::as_str).unwrap_or("")
                    ),
                    None => eprintln!(
                        "mcp {name}: ok, revision {}",
                        v.get("revision").map_or("-".into(), |r| r.to_string())
                    ),
                }
            }
            Ok((name, None)) => return Err(format!("--mcp-calls: no tool {name}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(app)
}

#[cfg(test)]
mod tests;
