//! Prefs > AI & MCP.. driven like the window (`ui`/`logic` passes, clicks on UI pixels),
//! with a fake computer for the connect flows: no real `claude` runs and nothing is opened.

use std::sync::Mutex;
use std::time::Instant;

use dpx_mcp::jobs;
use dpx_mcp::tools::ToolCtx;
use dpx_mcp::{FileAccess, Overwrite, ServerState};
use serde_json::{Value, json};

use super::*;
use crate::mcp::AppCodec;
use crate::tests::{key_event, menu_item_path, pick, pointer_button, ui_point};

/// The computer as the flows see it; records every process and opened file.
#[derive(Default)]
struct Fake {
    /// This executable (default [`EXE`]).
    exe: Option<PathBuf>,
    /// `claude mcp get` ends with this code and stderr line instead of 0 or 1.
    get_fails: Option<(i32, &'static str)>,
    claude: Option<PathBuf>,
    mcpb: Option<PathBuf>,
    desktop: bool,
    registered: Mutex<bool>,
    add_fails: bool,
    runs: Mutex<Vec<(PathBuf, Vec<String>)>>,
    opened: Mutex<Vec<PathBuf>>,
}

const EXE: &str = "/Apps/Deluxe Painter/deluxepainter";
const CLAUDE: &str = "/fake/bin/claude";

impl Fake {
    fn with_claude(registered: bool) -> Fake {
        Fake {
            claude: Some(PathBuf::from(CLAUDE)),
            registered: Mutex::new(registered),
            ..Fake::default()
        }
    }

    fn runs(&self) -> Vec<Vec<String>> {
        let runs = self.runs.lock().unwrap();
        assert!(runs.iter().all(|(p, _)| p == Path::new(CLAUDE)), "{runs:?}");
        runs.iter().map(|(_, a)| a.clone()).collect()
    }
}

impl System for Fake {
    fn exe(&self) -> Option<PathBuf> {
        Some(self.exe.clone().unwrap_or_else(|| PathBuf::from(EXE)))
    }

    fn find_claude(&self) -> Option<PathBuf> {
        self.claude.clone()
    }

    fn find_mcpb(&self) -> Option<PathBuf> {
        self.mcpb.clone()
    }

    fn desktop_installed(&self) -> bool {
        self.desktop
    }

    fn run(&self, program: &Path, args: &[String]) -> Result<RunOutput, String> {
        self.runs
            .lock()
            .unwrap()
            .push((program.to_path_buf(), args.to_vec()));
        let mut registered = self.registered.lock().unwrap();
        let code = |c: i32| RunOutput {
            code: Some(c),
            ..RunOutput::default()
        };
        Ok(match args.get(1).map(String::as_str) {
            Some("get") if self.get_fails.is_some() => {
                let (c, line) = self.get_fails.unwrap();
                RunOutput {
                    code: Some(c),
                    stdout: String::new(),
                    stderr: format!("{line}\n"),
                }
            }
            Some("get") => code(if *registered { 0 } else { 1 }),
            Some("add") if self.add_fails => RunOutput {
                code: Some(1),
                stdout: String::new(),
                stderr: "\nMCP server deluxe-painter already exists in user config\n".into(),
            },
            Some("add") => {
                *registered = true;
                code(0)
            }
            Some("remove") => {
                *registered = false;
                code(0)
            }
            _ => code(2),
        })
    }

    fn open_file(&self, path: &Path) -> Result<(), String> {
        self.opened.lock().unwrap().push(path.to_path_buf());
        Ok(())
    }
}

/// An app with the fake computer and the requester open through the Prefs menu.
struct Rig {
    app: PainterApp,
    ctx: egui::Context,
    fake: Arc<Fake>,
    t: f64,
}

impl Rig {
    fn new(fake: Fake) -> Rig {
        Rig::with_app(PainterApp::new(), fake)
    }

    fn with_app(mut app: PainterApp, fake: Fake) -> Rig {
        // Any free port: a test never takes 8765 from a running app.
        app.mcp.settings.port = 0;
        app.mcp.settings.approved_dirs.clear();
        let fake = Arc::new(fake);
        app.mcp_prefs.system = fake.clone();
        let mut rig = Rig {
            app,
            ctx: egui::Context::default(),
            fake,
            t: 1.0,
        };
        rig.pass(vec![]);
        let path = menu_item_path(&rig.app, "app.ai_mcp");
        pick(&mut rig.app, &rig.ctx, rig.t, &path);
        rig.t += 0.2;
        assert!(
            rig.app.ui.mcp_requester.is_some(),
            "Prefs > AI & MCP.. opens it"
        );
        rig
    }

    /// One pass as the window runs it; returns the platform commands (clipboard, URLs).
    fn pass(&mut self, events: Vec<egui::Event>) -> Vec<egui::OutputCommand> {
        self.t += 0.02;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 1024.0),
            )),
            time: Some(self.t),
            events,
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            eframe::App::logic(app, ui.ctx(), &mut frame);
            eframe::App::ui(app, ui, &mut frame)
        });
        out.textures_delta.clear();
        out.platform_output.commands
    }

    /// A click on UI pixel `at`; the commands of both passes.
    fn click_at(&mut self, at: (i32, i32)) -> Vec<egui::OutputCommand> {
        use egui::PointerButton::Primary;
        let pos = ui_point(&self.app, at);
        let mut out = self.pass(vec![
            egui::Event::PointerMoved(pos),
            pointer_button(pos, Primary, true),
        ]);
        out.extend(self.pass(vec![pointer_button(pos, Primary, false)]));
        out
    }

    fn req(&self) -> &McpRequester {
        self.app
            .ui
            .mcp_requester
            .as_ref()
            .expect("the requester is open")
    }

    fn click(&mut self, c: McpControl) -> Vec<egui::OutputCommand> {
        let (ox, oy) = self.req().origin();
        let r = self
            .req()
            .controls()
            .into_iter()
            .find(|&(k, _)| k == c)
            .unwrap()
            .1;
        self.click_at((ox + r.x + r.w / 2, oy + r.y + r.h / 2))
    }

    fn choice(&self) -> &ChoiceRequester {
        self.app
            .ui
            .choice_requester
            .as_ref()
            .expect("a question is open")
    }

    fn choice_text(&self) -> String {
        self.choice().lines().join(" ")
    }

    /// Clicks button `i` of the open question.
    fn answer(&mut self, i: usize) -> Vec<egui::OutputCommand> {
        let (w, h) = self.app.view().ui_size();
        let (ox, oy) = self.choice().origin(w, h);
        let (bx, by) = self.choice().button_centre(i);
        let out = self.click_at((ox + bx, oy + by));
        assert!(self.app.ui.choice_requester.is_none(), "answered");
        out
    }

    /// Passes until the running connect step has ended (at most 5 s).
    fn settle(&mut self) {
        let until = Instant::now() + Duration::from_secs(5);
        while self.app.mcp_prefs.task.is_some() {
            assert!(Instant::now() < until, "the step did not end");
            std::thread::sleep(Duration::from_millis(2));
            self.pass(vec![]);
        }
        self.pass(vec![]);
    }

    fn view(&self) -> McpView {
        self.req().view().clone()
    }

    fn message(&self) -> String {
        self.view().message.unwrap_or_default()
    }
}

fn copied(out: &[egui::OutputCommand]) -> Vec<String> {
    out.iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

fn opened_urls(out: &[egui::OutputCommand]) -> Vec<String> {
    out.iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(u) => Some(u.url.clone()),
            _ => None,
        })
        .collect()
}

fn add_command() -> String {
    connect::command_line(Platform::current(), &connect::add_args(Path::new(EXE)))
}

#[test]
fn on_and_off_start_and_stop_the_server_and_the_status_says_so() {
    let mut rig = Rig::new(Fake::default());
    assert!(rig.app.modal_open());
    let v = rig.view();
    assert!(!v.on);
    assert_eq!(v.status, "Off");
    assert_eq!(v.folder, "none (file tools are off)");
    assert!(v.activity.is_empty());
    rig.click(McpControl::On);
    assert!(rig.app.mcp.settings.enabled, "saved in the settings");
    let Status::Running { port, .. } = rig.app.mcp.status() else {
        panic!("the server runs: {:?}", rig.app.mcp.status());
    };
    let v = rig.view();
    assert!(v.on);
    assert_eq!(v.status, format!("Ready on 127.0.0.1:{port}"));
    rig.click(McpControl::Off);
    assert!(!rig.app.mcp.settings.enabled);
    assert_eq!(rig.app.mcp.status(), Status::Off);
    assert_eq!(rig.view().status, "Off");
    // `--mcp` turns it on for the session; Off ends that too.
    rig.app.mcp.session = true;
    rig.pass(vec![]);
    assert!(rig.view().on && matches!(rig.app.mcp.status(), Status::Running { .. }));
    rig.click(McpControl::Off);
    assert!(!rig.app.mcp.session && rig.app.mcp.status() == Status::Off);
    // Esc closes the requester; nothing else changes.
    rig.pass(vec![key_event(egui::Key::Escape, true)]);
    assert!(rig.app.ui.mcp_requester.is_none());
    assert!(!rig.app.modal_open());
}

#[test]
fn close_button_and_return_close_it() {
    let mut rig = Rig::new(Fake::default());
    rig.click(McpControl::Close);
    assert!(rig.app.ui.mcp_requester.is_none());
    let mut rig = Rig::new(Fake::default());
    rig.pass(vec![key_event(egui::Key::Enter, true)]);
    assert!(rig.app.ui.mcp_requester.is_none());
}

#[test]
fn permissions_follow_the_boxes_and_reach_the_running_server_at_once() {
    let app = crate::mcp::calls_app(None, &PainterApp::new().engine.document().clone()).unwrap();
    let mut rig = Rig::with_app(app, Fake::default());
    let state = rig.app.mcp.server_state().unwrap().clone();
    assert!(state.settings().read && state.settings().edit);
    rig.click(McpControl::Read);
    assert!(!rig.app.mcp.settings.allow_read);
    assert!(!state.settings().read, "the next call sees it");
    // Changing needs reading: the Edit box is ghosted and does nothing.
    assert!(!rig.req().enabled(McpControl::Edit));
    rig.click(McpControl::Edit);
    assert!(rig.app.mcp.settings.allow_edit);
    rig.click(McpControl::Read);
    rig.click(McpControl::Overwrite);
    assert!(rig.app.mcp.settings.overwrite_without_asking);
    assert_eq!(state.settings().overwrite, Overwrite::Allowed);
    rig.click(McpControl::Edit);
    assert!(!state.settings().edit);
    assert!(!rig.req().enabled(McpControl::Overwrite));
    let v = rig.view();
    assert!(v.read && !v.edit && v.overwrite);
    // The ghosted box stays ticked (and saved) but does not count: overwriting asks.
    assert!(rig.app.mcp.settings.overwrite_without_asking);
    assert_eq!(state.settings().overwrite, Overwrite::Ask);
    assert!(!state.settings().overwrite_without_asking());
    rig.click(McpControl::Edit);
    assert_eq!(
        state.settings().overwrite,
        Overwrite::Allowed,
        "both on again"
    );
}

#[test]
fn a_ghosted_overwrite_box_still_asks_before_replacing_a_file() {
    let dir = std::env::temp_dir().join(format!("dpx-mcp-ghost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let (mut rig, tools) = serving(Some(&dir));
    rig.click(McpControl::Overwrite);
    assert_eq!(tools.state.settings().overwrite, Overwrite::Allowed);
    let v = call(&mut rig, &tools, "export_image", json!({"path": "a.iff"}));
    assert_eq!(v["success"], true, "{v}");
    // Read on, Edit off, Overwrite ticked under the ghosting.
    rig.click(McpControl::Edit);
    assert!(!rig.req().enabled(McpControl::Overwrite));
    assert!(rig.view().overwrite);
    let st = call(&mut rig, &tools, "get_app_status", json!({}));
    assert_eq!(st["permissions"]["overwrite_without_asking"], false, "{st}");
    let before = std::fs::read(dir.join("a.iff")).unwrap();
    let rx = dpx_mcp::tools::spawn_calls(
        tools.clone(),
        vec![(
            "export_image".into(),
            json!({"path": "a.iff", "overwrite": true})
                .as_object()
                .cloned(),
        )],
    );
    let until = Instant::now() + Duration::from_secs(5);
    while rig.app.ui.choice_requester.is_none() {
        assert!(Instant::now() < until, "the user is asked");
        std::thread::sleep(Duration::from_millis(2));
        rig.pass(vec![]);
    }
    assert_eq!(rig.choice().title(), "Replace File?");
    rig.pass(vec![key_event(egui::Key::Enter, true)]);
    let (_, r) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        r.unwrap().structured_content.unwrap()["code"],
        "OVERWRITE_DECLINED"
    );
    assert_eq!(std::fs::read(dir.join("a.iff")).unwrap(), before, "kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A rig whose app serves a fake job sender (the tools' executor, no transport).
fn serving(dir: Option<&Path>) -> (Rig, ToolCtx) {
    let mut app = PainterApp::new();
    let (exec, queue) = jobs::channel(|| {});
    let access = match dir {
        Some(d) => FileAccess::new(vec![d.to_path_buf()]).unwrap(),
        None => FileAccess::off(),
    };
    let mut settings = crate::mcp::McpSettings::default();
    settings.approved_dirs.clear();
    let state = ServerState::with_codec(
        dpx_mcp::Settings {
            read: true,
            edit: true,
            overwrite: Overwrite::Ask,
            access,
        },
        Arc::new(AppCodec),
    );
    app.mcp.settings = settings;
    app.mcp.attach(queue, state.clone());
    let mut rig = Rig::with_app(app, Fake::default());
    // The server follows the settings: the approved directory is the test's.
    rig.app.mcp.settings.approved_dirs = dir.map(Path::to_path_buf).into_iter().collect();
    rig.pass(vec![]);
    let tools = ToolCtx {
        exec,
        state,
        transport: "test",
    };
    (rig, tools)
}

fn call(rig: &mut Rig, tools: &ToolCtx, name: &str, args: Value) -> Value {
    let rx = dpx_mcp::tools::spawn_calls(
        tools.clone(),
        vec![(name.to_string(), args.as_object().cloned())],
    );
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        rig.pass(vec![]);
        if let Ok((_, r)) = rx.recv_timeout(Duration::from_millis(2)) {
            // The log entry is written after the job ran; the next pass shows it.
            rig.pass(vec![]);
            return r.unwrap().structured_content.unwrap();
        }
        assert!(Instant::now() < until, "{name} was not answered");
    }
}

#[test]
fn changes_run_under_the_open_requester_and_show_in_its_activity() {
    let (mut rig, tools) = serving(None);
    let v = call(
        &mut rig,
        &tools,
        "draw_pixel",
        json!({"x": 3, "y": 4, "color": 5}),
    );
    assert_eq!(v["success"], true, "{v}");
    assert_eq!(rig.app.engine.document().page().get(3, 4), Some(5));
    assert!(rig.app.ui.mcp_requester.is_some(), "still open");
    let v = rig.view();
    assert_eq!(v.activity.len(), 1);
    assert!(
        v.activity[0].ends_with("draw_pixel 3,4 colour 5"),
        "{:?}",
        v.activity
    );
    assert!(v.activity[0].starts_with("0s "), "{:?}", v.activity);
    assert!(
        v.status.starts_with("Off"),
        "no HTTP server here: {}",
        v.status
    );
    // A refused call shows its code.
    let v = call(
        &mut rig,
        &tools,
        "draw_pixel",
        json!({"x": 3, "y": 4, "color": 99}),
    );
    assert_eq!(v["success"], false);
    let last = rig.view().activity.last().cloned().unwrap();
    let code = v["code"].as_str().unwrap();
    assert_eq!(last, format!("0s draw_pixel {code}"));
    // Another requester still blocks changes.
    rig.app.ui.mcp_requester = None;
    rig.app.ui.palette_requester = Some(dpx_ui::PaletteRequester::new(
        rig.app.engine.document().palette(),
        1,
        rig.app.engine.document().target().chipset,
    ));
    let v = call(
        &mut rig,
        &tools,
        "draw_pixel",
        json!({"x": 5, "y": 5, "color": 5}),
    );
    assert_eq!(v["code"], "EDITOR_BUSY");
}

#[test]
fn the_replace_question_shows_over_the_requester() {
    let dir = std::env::temp_dir().join(format!("dpx-mcp-prefs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let (mut rig, tools) = serving(Some(&dir));
    let args = json!({"path": "a.iff", "overwrite": true});
    let v = call(&mut rig, &tools, "export_image", args.clone());
    assert_eq!(v["success"], true, "{v}");
    let rx = dpx_mcp::tools::spawn_calls(
        tools.clone(),
        vec![("export_image".into(), args.as_object().cloned())],
    );
    let until = Instant::now() + Duration::from_secs(5);
    while rig.app.ui.choice_requester.is_none() {
        assert!(Instant::now() < until, "the question appears");
        std::thread::sleep(Duration::from_millis(2));
        rig.pass(vec![]);
    }
    assert_eq!(rig.choice().title(), "Replace File?");
    assert!(rig.app.ui.mcp_requester.is_some(), "under the question");
    // Return keeps the file; the requester stays.
    rig.pass(vec![key_event(egui::Key::Enter, true)]);
    let (_, r) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        r.unwrap().structured_content.unwrap()["code"],
        "OVERWRITE_DECLINED"
    );
    assert!(rig.app.ui.mcp_requester.is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn connect_claude_code_shows_the_exact_command_and_runs_it_only_after_run() {
    let mut rig = Rig::new(Fake::with_claude(false));
    rig.click(McpControl::ConnectCode);
    assert!(rig.view().busy || rig.app.ui.choice_requester.is_some());
    rig.settle();
    assert_eq!(rig.fake.runs(), [connect::get_args()], "only the check ran");
    assert_eq!(rig.choice().title(), "Connect Claude Code");
    let text = rig.choice_text();
    assert!(text.contains(&add_command()), "{text}");
    assert!(
        add_command().contains("'/Apps/Deluxe Painter/deluxepainter'") || cfg!(windows),
        "a path with a space is quoted"
    );
    rig.answer(0);
    rig.settle();
    assert_eq!(
        rig.fake.runs(),
        [connect::get_args(), connect::add_args(Path::new(EXE))]
    );
    assert_eq!(
        rig.fake.runs()[1],
        [
            "mcp",
            "add",
            "--scope",
            "user",
            "deluxe-painter",
            "--",
            EXE,
            "mcp-bridge"
        ]
    );
    assert!(
        rig.message().starts_with("Claude Code is connected"),
        "{}",
        rig.message()
    );
    assert!(
        rig.app.mcp.settings.enabled,
        "connecting turns MCP control on"
    );
    assert!(matches!(rig.app.mcp.status(), Status::Running { .. }));
    // The token is in no command.
    let token = rig.app.mcp.token().unwrap().to_string();
    assert!(
        rig.fake
            .runs()
            .iter()
            .flatten()
            .all(|a| !a.contains(&token))
    );
    assert!(!rig.view().busy);
}

#[test]
fn cancel_runs_nothing_and_a_failed_add_says_why() {
    let mut rig = Rig::new(Fake::with_claude(false));
    rig.click(McpControl::ConnectCode);
    rig.settle();
    rig.answer(1);
    rig.settle();
    assert_eq!(rig.fake.runs(), [connect::get_args()]);
    assert!(!rig.app.mcp.settings.enabled);
    assert!(rig.app.ui.mcp_requester.is_some());
    let mut rig = Rig::new(Fake {
        add_fails: true,
        ..Fake::with_claude(false)
    });
    rig.click(McpControl::ConnectCode);
    rig.settle();
    rig.answer(0);
    rig.settle();
    assert_eq!(
        rig.message(),
        "claude mcp add failed: MCP server deluxe-painter already exists in user config"
    );
    assert!(!rig.app.mcp.settings.enabled);
}

/// The question's lines joined without the breaks the wrapping put into long words.
fn squeezed(lines: &[String]) -> String {
    lines.concat().split_whitespace().collect()
}

/// A long but real path (a download unpacked in place) and the question for it.
const LONG_EXE: &str = "/Users/someone/Downloads/DeluxePainter-0.1.0-macos-aarch64 (1)/DeluxePainter.app/Contents/MacOS/deluxepainter";

#[test]
fn the_connect_questions_show_the_whole_command_even_for_a_long_path() {
    for registered in [true, false] {
        let mut rig = Rig::new(Fake {
            exe: Some(PathBuf::from(LONG_EXE)),
            ..Fake::with_claude(registered)
        });
        rig.click(McpControl::ConnectCode);
        rig.settle();
        let lines = rig.choice().lines().to_vec();
        assert!(!rig.choice().truncated(), "{lines:?}");
        assert!(lines.iter().all(|l| l != "..."), "{lines:?}");
        if registered {
            // More than an ordinary question shows (it was cut there before).
            assert!(
                lines.len() > dpx_ui::choice_requester::MAX_LINES,
                "{lines:?}"
            );
        }
        let text = squeezed(&lines);
        let exe: String = LONG_EXE.split_whitespace().collect();
        assert!(text.contains(&exe), "{text}");
        assert!(text.contains("mcp-bridge"), "{text}");
        assert!(
            text.contains("claudemcpadd--scopeuserdeluxe-painter--"),
            "{text}"
        );
        if registered {
            assert!(
                text.contains("claudemcpremovedeluxe-painter-suser"),
                "{text}"
            );
        }
        // It fits the smallest UI screen.
        assert!(rig.choice().height() <= 200);
    }
}

#[test]
fn a_command_too_long_to_show_is_not_offered() {
    let exe = format!("/{}/deluxepainter", "x".repeat(700));
    let mut rig = Rig::new(Fake {
        exe: Some(PathBuf::from(&exe)),
        ..Fake::with_claude(false)
    });
    rig.click(McpControl::ConnectCode);
    rig.settle();
    assert!(rig.app.ui.choice_requester.is_none(), "nothing to click");
    assert!(
        rig.message().contains("too long to show"),
        "{}",
        rig.message()
    );
    assert_eq!(rig.fake.runs(), [connect::get_args()], "nothing else ran");
}

#[test]
fn claude_code_failing_to_answer_is_reported_not_taken_for_a_missing_entry() {
    // An npm `claude` whose `node` is not on the app's PATH ends with 127.
    let mut rig = Rig::new(Fake {
        get_fails: Some((127, "env: node: No such file or directory")),
        ..Fake::with_claude(false)
    });
    rig.click(McpControl::ConnectCode);
    rig.settle();
    assert!(rig.app.ui.choice_requester.is_none(), "no Run is offered");
    assert_eq!(
        rig.message(),
        "Claude Code: env: node: No such file or directory"
    );
    assert_eq!(rig.fake.runs(), [connect::get_args()]);
    assert!(!rig.app.mcp.settings.enabled);
    // Without a word it says how it ended.
    let mut rig = Rig::new(Fake {
        get_fails: Some((2, "")),
        ..Fake::with_claude(false)
    });
    rig.click(McpControl::ConnectCode);
    rig.settle();
    assert_eq!(rig.message(), "Claude Code ended with Some(2).");
}

#[test]
fn an_existing_claude_code_entry_is_replaced_only_on_request() {
    let mut rig = Rig::new(Fake::with_claude(true));
    rig.click(McpControl::ConnectCode);
    rig.settle();
    assert_eq!(rig.choice().labels(), ["Replace", "Keep"]);
    assert!(
        rig.choice_text()
            .contains("claude mcp remove deluxe-painter -s user")
    );
    let text = squeezed(rig.choice().lines());
    let exe: String = EXE.split_whitespace().collect();
    assert!(text.contains(&exe) && text.contains("mcp-bridge"), "{text}");
    assert!(rig.choice().lines().iter().all(|l| l != "..."));
    rig.answer(1);
    rig.settle();
    assert_eq!(rig.message(), "Claude Code already has DeluxePainter.");
    assert_eq!(rig.fake.runs(), [connect::get_args()]);
    rig.click(McpControl::ConnectCode);
    rig.settle();
    rig.answer(0);
    rig.settle();
    assert_eq!(
        rig.fake.runs(),
        [
            connect::get_args(),
            connect::get_args(),
            connect::remove_args(),
            connect::add_args(Path::new(EXE))
        ]
    );
    assert!(*rig.fake.registered.lock().unwrap());
    assert!(rig.message().starts_with("Claude Code is connected"));
}

#[test]
fn without_claude_code_the_command_is_copied_and_get_claude_code_opens_the_setup_page() {
    let mut rig = Rig::new(Fake::default());
    let out = rig.click(McpControl::ConnectCode);
    assert_eq!(copied(&out), [add_command()]);
    assert_eq!(rig.choice().title(), "Claude Code not found");
    assert_eq!(rig.choice().labels(), ["Get Claude Code", "Close"]);
    assert_eq!(
        rig.message(),
        "Claude Code not found: the command was copied."
    );
    let out = rig.answer(0);
    assert_eq!(opened_urls(&out), [connect::CLAUDE_CODE_SETUP_URL]);
    assert!(rig.fake.runs().is_empty(), "nothing ran");
    // Close opens nothing.
    let out = rig.click(McpControl::ConnectCode);
    assert_eq!(copied(&out).len(), 1);
    let out = rig.answer(1);
    assert!(opened_urls(&out).is_empty());
    assert!(!rig.app.mcp.settings.enabled);
}

#[test]
fn disconnect_asks_then_removes_the_claude_code_entry() {
    let mut rig = Rig::new(Fake::with_claude(true));
    rig.click(McpControl::Disconnect);
    assert_eq!(rig.choice().title(), "Disconnect");
    let text = rig.choice_text();
    assert!(
        text.contains("claude mcp remove deluxe-painter -s user"),
        "{text}"
    );
    assert!(text.contains("Settings > Extensions"), "{text}");
    assert!(rig.fake.runs().is_empty(), "asked first");
    rig.answer(0);
    rig.settle();
    assert_eq!(rig.fake.runs(), [connect::remove_args()]);
    assert_eq!(
        rig.fake.runs()[0],
        ["mcp", "remove", "deluxe-painter", "-s", "user"]
    );
    assert_eq!(rig.message(), "Removed from Claude Code.");
    assert!(!*rig.fake.registered.lock().unwrap());
    // Cancel removes nothing.
    rig.click(McpControl::Disconnect);
    rig.answer(1);
    rig.settle();
    assert_eq!(rig.fake.runs().len(), 1);
}

#[test]
fn disconnect_without_claude_code_offers_to_turn_mcp_off() {
    let mut rig = Rig::new(Fake::default());
    rig.click(McpControl::On);
    assert!(rig.app.mcp.settings.enabled);
    rig.click(McpControl::Disconnect);
    assert_eq!(rig.choice().labels(), ["Turn off", "Cancel"]);
    rig.answer(1);
    assert!(rig.app.mcp.settings.enabled, "Cancel keeps it on");
    rig.click(McpControl::Disconnect);
    rig.answer(0);
    assert!(!rig.app.mcp.settings.enabled);
    assert_eq!(rig.app.mcp.status(), Status::Off);
}

#[test]
fn connect_claude_desktop_opens_the_extension_or_offers_claude() {
    // No extension in this build.
    let mut rig = Rig::new(Fake {
        desktop: true,
        ..Fake::default()
    });
    rig.click(McpControl::ConnectDesktop);
    assert!(
        rig.message().ends_with(".mcpb is not next to the app."),
        "{}",
        rig.message()
    );
    assert!(rig.app.ui.choice_requester.is_none());
    // Claude Desktop missing: nothing is opened; Get Claude opens the download page.
    let mcpb = PathBuf::from("/x/deluxepainter-test.mcpb");
    let mut rig = Rig::new(Fake {
        mcpb: Some(mcpb.clone()),
        ..Fake::default()
    });
    rig.click(McpControl::ConnectDesktop);
    assert_eq!(rig.choice().title(), "Claude Desktop not found");
    let out = rig.answer(0);
    assert_eq!(opened_urls(&out), [connect::CLAUDE_DOWNLOAD_URL]);
    assert!(rig.fake.opened.lock().unwrap().is_empty());
    assert!(!rig.app.mcp.settings.enabled);
    // Installed: the OS opens the extension (Claude Desktop asks the user).
    let mut rig = Rig::new(Fake {
        mcpb: Some(mcpb.clone()),
        desktop: true,
        ..Fake::default()
    });
    rig.click(McpControl::ConnectDesktop);
    rig.settle();
    assert_eq!(*rig.fake.opened.lock().unwrap(), [mcpb]);
    assert_eq!(rig.message(), "Confirm the install in Claude Desktop.");
    assert!(
        rig.app.mcp.settings.enabled,
        "MCP control is on for the extension"
    );
    assert!(rig.fake.runs().is_empty());
}

#[test]
fn revoke_access_makes_a_new_token_and_turns_the_permissions_off() {
    let mut rig = Rig::new(Fake::default());
    rig.click(McpControl::On);
    let old = rig.app.mcp.token().unwrap().to_string();
    rig.click(McpControl::Overwrite);
    rig.click(McpControl::Revoke);
    let new = rig.app.mcp.token().unwrap().to_string();
    assert_ne!(old, new);
    let s = &rig.app.mcp.settings;
    assert!(!s.allow_read && !s.allow_edit && !s.overwrite_without_asking);
    assert_eq!(rig.message(), "Access revoked: new token, permissions off.");
    let v = rig.view();
    assert!(!v.read && !v.edit && !v.overwrite);
    assert!(
        !v.message.unwrap().contains(&new),
        "the token is never shown"
    );
}

#[test]
fn the_buttons_wait_while_a_step_runs() {
    let mut rig = Rig::new(Fake::with_claude(false));
    rig.click(McpControl::ConnectCode);
    // The step may already have ended on a fast machine; while it runs, the view says so.
    if rig.app.mcp_prefs.task.is_some() {
        let v = rig.view();
        assert!(v.busy);
        assert_eq!(
            v.message.as_deref(),
            Some("Looking for DeluxePainter in Claude Code...")
        );
        assert!(!rig.req().enabled(McpControl::ConnectDesktop));
    }
    rig.settle();
    assert!(!rig.view().busy);
}

fn scratch_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dpx-mcp-prefs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

#[test]
fn a_failed_start_shows_off_after_off_and_on_tries_again() {
    let mut rig = Rig::new(Fake::default());
    // A real failure: the token cannot be written under /dev/null.
    rig.app.mcp.config_dir = Some(PathBuf::from("/dev/null/DeluxePainter"));
    rig.click(McpControl::On);
    assert!(matches!(rig.app.mcp.status(), Status::Failed(_)));
    let status = rig.view().status;
    assert!(
        status.starts_with("Error: cannot keep the token"),
        "{status}"
    );
    rig.click(McpControl::Off);
    assert_eq!(rig.app.mcp.status(), Status::Off);
    assert_eq!(rig.view().status, "Off", "Off is off, not an old error");
    // The cause is gone; On tries again.
    let config = scratch_dir("retry");
    rig.app.mcp.config_dir = Some(config.clone());
    rig.click(McpControl::On);
    let Status::Running { port, .. } = rig.app.mcp.status() else {
        panic!("started again: {:?}", rig.app.mcp.status());
    };
    assert_eq!(rig.view().status, format!("Ready on 127.0.0.1:{port}"));
    rig.click(McpControl::Off);
    let _ = std::fs::remove_dir_all(&config);
}

/// Points `mcp-app.json` at a script that leaves `marker` behind when it is started, with
/// `enabled` as given.
#[cfg(unix)]
fn launch_marker(config: &Path, marker: &Path, enabled: bool) {
    use std::os::unix::fs::PermissionsExt;
    let script = config.join("fake-app.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut launch = dpx_mcp::endpoint::read_app_file(config).expect("the launch record");
    launch.app_path = script.display().to_string();
    launch.enabled = enabled;
    dpx_mcp::endpoint::write_app_file(config, &launch).unwrap();
}

/// What a bridge call does first, with short waits.
#[cfg(unix)]
fn bridge_reach(config: &Path) -> Result<(), dpx_mcp::ToolError> {
    dpx_mcp::bridge::Bridge::with_launch_wait(
        Some(config.to_path_buf()),
        Duration::from_millis(500),
    )
    .reach_blocking()
}

#[cfg(unix)]
#[test]
fn turning_mcp_off_keeps_the_bridge_from_starting_the_app() {
    let config = scratch_dir("off-sticks");
    let marker = config.join("launched");
    let mut rig = Rig::new(Fake::default());
    rig.app.mcp.config_dir = Some(config.clone());
    rig.app.mcp.instance = crate::mcp::claim_instance(Some(&config), false).unwrap();
    assert!(
        rig.app.mcp.instance.is_some(),
        "this GUI holds the app lock"
    );
    rig.click(McpControl::On);
    assert!(matches!(rig.app.mcp.status(), Status::Running { .. }));
    assert!(bridge_reach(&config).is_ok(), "the bridge reaches the app");
    assert!(dpx_mcp::endpoint::read_app_file(&config).unwrap().enabled);

    // Off: the server stops and the launch record says off.
    rig.click(McpControl::Off);
    assert!(!dpx_mcp::endpoint::read_app_file(&config).unwrap().enabled);
    launch_marker(&config, &marker, false);
    let e = bridge_reach(&config).unwrap_err();
    assert_eq!(e.code, "APP_UNAVAILABLE", "{e}");
    assert_eq!(e.message, dpx_mcp::bridge::TURNED_OFF);
    // Even a record that allows a start does not start a second app while this one runs.
    launch_marker(&config, &marker, true);
    let e = bridge_reach(&config).unwrap_err();
    assert_eq!(e.message, dpx_mcp::bridge::NOT_ANSWERING, "{e}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!marker.exists(), "nothing was started");

    // Disconnect > Turn off does the same.
    rig.click(McpControl::On);
    assert!(dpx_mcp::endpoint::read_app_file(&config).unwrap().enabled);
    rig.click(McpControl::Disconnect);
    assert_eq!(rig.choice().labels(), ["Turn off", "Cancel"]);
    rig.answer(0);
    assert_eq!(rig.app.mcp.status(), Status::Off);
    assert!(!dpx_mcp::endpoint::read_app_file(&config).unwrap().enabled);
    launch_marker(&config, &marker, false);
    assert_eq!(bridge_reach(&config).unwrap_err().code, "APP_UNAVAILABLE");

    // Quit with MCP control off: the bridge does not start the app either.
    eframe::App::on_exit(&mut rig.app);
    assert!(rig.app.mcp.instance.is_none());
    assert!(!dpx_mcp::endpoint::app_running(&config));
    launch_marker(&config, &marker, false);
    let e = bridge_reach(&config).unwrap_err();
    assert_eq!(e.code, "APP_NOT_RUNNING");
    assert_eq!(e.message, dpx_mcp::bridge::OFF_NOT_RUNNING);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!marker.exists(), "nothing was started");

    // The control: with MCP control on and no app running the bridge does start it.
    launch_marker(&config, &marker, true);
    let _ = bridge_reach(&config);
    let until = Instant::now() + Duration::from_secs(5);
    while !marker.exists() {
        assert!(
            Instant::now() < until,
            "the app named in the record was started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn a_second_gui_with_mcp_ends_and_a_plain_one_runs_without_the_claim() {
    let config = scratch_dir("instance");
    let first = crate::mcp::claim_instance(Some(&config), true).unwrap();
    assert!(first.is_some());
    let e = crate::mcp::claim_instance(Some(&config), true).unwrap_err();
    assert!(e.contains("already running"), "{e}");
    assert!(
        crate::mcp::claim_instance(Some(&config), false)
            .unwrap()
            .is_none(),
        "a second window opened by the user runs, without the claim"
    );
    assert!(crate::mcp::claim_instance(None, true).unwrap().is_none());
    drop(first);
    assert!(
        crate::mcp::claim_instance(Some(&config), true)
            .unwrap()
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn status_lines() {
    let ready = Status::Running {
        url: "http://127.0.0.1:8766/mcp".into(),
        port: 8766,
    };
    assert_eq!(status_text(false, &Status::Off, None, 8765), "Off");
    assert_eq!(status_text(true, &Status::Off, None, 8765), "Starting...");
    assert_eq!(
        status_text(true, &ready, None, 8765),
        "Ready on 127.0.0.1:8766 (port 8765 was in use)"
    );
    assert_eq!(
        status_text(true, &ready, None, 8766),
        "Ready on 127.0.0.1:8766"
    );
    assert_eq!(
        status_text(true, &ready, None, 0),
        "Ready on 127.0.0.1:8766"
    );
    let mut a = ActivitySnapshot {
        last_request: Some(std::time::Instant::now()),
        client: Some("claude-ai via deluxepainter-bridge".into()),
        ..ActivitySnapshot::default()
    };
    assert_eq!(
        status_text(true, &ready, Some(&a), 8766),
        "Claude connected on 127.0.0.1:8766, last call 0 s ago"
    );
    a.client = None;
    assert!(status_text(true, &ready, Some(&a), 8766).starts_with("An AI client connected"));
    a.last_request = std::time::Instant::now().checked_sub(Duration::from_secs(400));
    if a.last_request.is_some() {
        assert_eq!(
            status_text(true, &ready, Some(&a), 8766),
            "Ready on 127.0.0.1:8766"
        );
    }
    assert_eq!(
        status_text(true, &Status::Failed("cannot start".into()), None, 8765),
        "Error: cannot start"
    );
    assert_eq!(age(Duration::from_secs(59)), "59 s");
    assert_eq!(age(Duration::from_secs(120)), "2 min");
    assert_eq!(age(Duration::from_secs(7300)), "2 h");
}

#[test]
fn activity_and_folder_lines() {
    let now = SystemTime::now();
    let entry = |code: Option<&str>, detail: Option<&str>| LogEntry {
        time: now - Duration::from_secs(75),
        tool: "export_image".into(),
        ok: code.is_none(),
        code: code.map(String::from),
        detail: detail.map(String::from),
    };
    assert_eq!(
        activity_line(&entry(None, Some("export_image a.iff")), now),
        "1m export_image a.iff"
    );
    assert_eq!(
        activity_line(&entry(Some("FILE_EXISTS"), Some("a.iff exists")), now),
        "1m export_image FILE_EXISTS"
    );
    assert_eq!(activity_line(&entry(None, None), now), "1m export_image");
    let home = Path::new("/Users/me");
    let pics = PathBuf::from("/Users/me/Pictures/DeluxePainter");
    assert_eq!(
        folder_text(std::slice::from_ref(&pics), &[], Some(home)),
        "~/Pictures/DeluxePainter"
    );
    assert_eq!(
        folder_text(
            &[pics.clone(), PathBuf::from("/tmp")],
            std::slice::from_ref(&pics),
            Some(home)
        ),
        "~/Pictures/DeluxePainter (missing) and 1 more"
    );
    assert_eq!(
        folder_text(&[PathBuf::from("/srv/art")], &[], Some(home)),
        "/srv/art"
    );
    assert_eq!(
        folder_text(&[], &[], Some(home)),
        "none (file tools are off)"
    );
}

/// Clicks a control of the Image Generator setup (over the AI & MCP requester).
fn click_setup(rig: &mut Rig, c: dpx_ui::SetupControl) {
    let r = rig
        .app
        .ui
        .imagegen_setup
        .as_ref()
        .expect("the setup is open");
    let (ox, oy) = r.origin();
    let rect = r.controls().into_iter().find(|&(k, _)| k == c).unwrap().1;
    rig.click_at((ox + rect.x + rect.w / 2, oy + rect.y + rect.h / 2));
}

#[test]
fn set_up_opens_the_image_generator_setup_over_the_requester_and_returns_to_it() {
    use dpx_ui::{SetupControl, SetupProvider};
    let mut rig = Rig::new(Fake::default());
    // A keychain stand-in in memory (the real one is never touched), so Ok closes at once.
    rig.app.imagegen.keys = Arc::new(dpx_imagegen::ApiKeys::new(
        Some(Box::new(dpx_imagegen::secret::MemoryStore::new())),
        None,
        Arc::new(|_: &str| None::<String>),
    ));
    assert_eq!(rig.view().generation, "Not set up");
    // Set up.. opens the setup over the requester; Cancel returns to it.
    rig.click(McpControl::SetUpGenerator);
    assert!(rig.app.ui.imagegen_setup.is_some(), "the setup opened");
    assert!(
        rig.app.ui.mcp_requester.is_some(),
        "AI & MCP stays under it"
    );
    click_setup(&mut rig, SetupControl::Cancel);
    assert!(rig.app.ui.imagegen_setup.is_none());
    assert!(rig.app.ui.mcp_requester.is_some(), "back to AI & MCP");
    assert_eq!(rig.view().generation, "Not set up");
    // Esc in the setup closes only the setup.
    rig.click(McpControl::SetUpGenerator);
    assert!(rig.app.ui.imagegen_setup.is_some());
    rig.pass(vec![key_event(egui::Key::Escape, true)]);
    assert!(rig.app.ui.imagegen_setup.is_none());
    assert!(rig.app.ui.mcp_requester.is_some(), "Esc left AI & MCP open");
    // OpenAI with a pasted key and Ok: the line says a key is there, never the key.
    rig.click(McpControl::SetUpGenerator);
    click_setup(&mut rig, SetupControl::Provider(SetupProvider::OpenAi));
    rig.pass(vec![egui::Event::Paste("sk-test-SECRET-4711".into())]);
    click_setup(&mut rig, SetupControl::Ok);
    assert!(rig.app.ui.imagegen_setup.is_none(), "Ok closed the setup");
    assert!(rig.app.ui.mcp_requester.is_some());
    rig.pass(vec![]);
    assert_eq!(rig.view().generation, "OpenAI, key set");
    assert!(!format!("{:?}", rig.view()).contains("SECRET"));
    // The requester under it still works: Close closes it.
    rig.click(McpControl::Close);
    assert!(rig.app.ui.mcp_requester.is_none());
}

#[test]
fn the_generator_line_names_a_missing_key_or_the_environment() {
    let mut rig = Rig::new(Fake::default());
    rig.app.imagegen.settings.provider = crate::generate::GenProvider::OpenAi;
    rig.pass(vec![]);
    assert_eq!(rig.view().generation, "OpenAI, no API key");
    rig.app.imagegen.keys = Arc::new(dpx_imagegen::ApiKeys::new(
        None,
        Some("test store".into()),
        Arc::new(|v: &str| (v == "OPENAI_API_KEY").then(|| "sk-env-SECRET".to_string())),
    ));
    rig.pass(vec![]);
    assert_eq!(rig.view().generation, "OpenAI, key from OPENAI_API_KEY");
    assert!(!format!("{:?}", rig.view()).contains("SECRET"));
}
