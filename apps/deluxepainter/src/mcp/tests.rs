//! The GUI workspace through the real tools (`dpx_mcp::tools::call`), driven like the window:
//! a call is sent from a thread of its own and the test runs UI passes (`ui_pass`, which
//! runs `App::logic` first) until it is answered.

use std::io::{Read, Write};
use std::sync::mpsc::Receiver;

use dpx_mcp::CallToolResult;
use dpx_mcp::tools::ToolCtx;
use serde_json::Value;

use super::*;
use crate::tests::{click_ui, pointer_button, ui_pass, ui_point};

// ADR-45: generate_image and friends on the GUI workspace, with this Rig.
#[path = "gen_tests.rs"]
mod gen_tests;

fn scratch(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "dpx-gui-mcp-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

/// An app serving a fake job sender: the tools' own executor, without a transport.
struct Rig {
    app: PainterApp,
    ctx: egui::Context,
    tools: ToolCtx,
    t: f64,
}

type Pending = Receiver<(String, Option<CallToolResult>)>;

impl Rig {
    fn new(dir: Option<&Path>) -> Rig {
        let mut app = PainterApp::new();
        app.mcp.settings.approved_dirs = dir.map(Path::to_path_buf).into_iter().collect();
        let (exec, queue) = jobs::channel(|| {});
        let (access, _) = approve(&app.mcp.settings.approved_dirs);
        let state =
            ServerState::with_codec(app.mcp.settings.server_settings(access), Arc::new(AppCodec));
        app.mcp.attach(queue, state.clone());
        let ctx = egui::Context::default();
        let mut rig = Rig {
            app,
            ctx,
            tools: ToolCtx {
                exec,
                state,
                transport: "test",
            },
            t: 1.0,
        };
        rig.pass();
        rig
    }

    fn pass_with(&mut self, events: Vec<egui::Event>) {
        self.t += 0.02;
        ui_pass(&mut self.app, &self.ctx, self.t, events);
    }

    fn pass(&mut self) {
        self.pass_with(Vec::new());
    }

    fn start(&self, name: &str, args: Value) -> Pending {
        dpx_mcp::tools::spawn_calls(
            self.tools.clone(),
            vec![(name.to_string(), args.as_object().cloned())],
        )
    }

    /// Passes until the call is answered (at most 5 s).
    fn wait(&mut self, rx: &Pending) -> Value {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            self.pass();
            if let Ok((name, r)) = rx.recv_timeout(Duration::from_millis(2)) {
                let r = r.unwrap_or_else(|| panic!("no tool {name}"));
                return r.structured_content.expect("structured result");
            }
            assert!(Instant::now() < until, "the call was not answered");
        }
    }

    /// Passes for a while and asserts the call is still waiting.
    fn still_waiting(&mut self, rx: &Pending) {
        for _ in 0..10 {
            self.pass();
            std::thread::sleep(Duration::from_millis(2));
            assert!(rx.try_recv().is_err(), "answered too early");
        }
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        let rx = self.start(name, args);
        self.wait(&rx)
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let v = self.call(name, args);
        assert_eq!(v["success"], true, "{name}: {v}");
        v
    }

    fn code(&mut self, name: &str, args: Value) -> (String, String) {
        let v = self.call(name, args);
        assert_eq!(v["success"], false, "{name}: {v}");
        (
            v["code"].as_str().unwrap().to_string(),
            v["message"].as_str().unwrap().to_string(),
        )
    }

    fn pixel(&self, x: i32, y: i32) -> u8 {
        self.app.engine.document().page().get(x, y).unwrap()
    }
}

fn rect(x: i32, y: i32, color: u8) -> Value {
    json!({"x": x, "y": y, "width": 20, "height": 10, "color": color, "filled": true})
}

#[test]
fn a_rectangle_lands_in_the_document_and_is_undoable() {
    let mut rig = Rig::new(None);
    let st = rig.ok("get_app_status", json!({}));
    assert_eq!(st["engine"], "gui");
    assert_eq!(st["document"]["document_id"], "page1");
    let before = rig.app.engine.document().revision();
    let r = rig.ok("draw_rectangle", rect(10, 20, 5));
    assert_eq!(r["revision"], before + 1);
    assert_eq!(rig.pixel(10, 20), 5);
    assert_eq!(rig.pixel(29, 29), 5);
    assert_eq!(rig.pixel(30, 20), 0);
    assert!(rig.app.unsaved(), "an MCP change is a change");
    // The window shows it: the change set the redraw flag, the pass uploaded it.
    assert!(!rig.app.dirty, "drawn in the same pass");
    // The GUI's own Undo (`u`) takes it back, Shift+U brings it back.
    rig.pass_with(vec![crate::tests::key_event(egui::Key::U, true)]);
    assert_eq!(rig.pixel(10, 20), 0);
    rig.app.redo();
    assert_eq!(rig.pixel(10, 20), 5);
    // And so does MCP undo.
    rig.ok("undo", json!({}));
    assert_eq!(rig.pixel(10, 20), 0);
    let (code, _) = rig.code("undo", json!({}));
    assert_eq!(code, "NOTHING_TO_UNDO");
    // The activity log names what was done.
    let log = rig.app.mcp.log();
    let draw = log.iter().find(|e| e.tool == "draw_rectangle").unwrap();
    assert!(draw.ok);
    assert_eq!(
        draw.detail.as_deref(),
        Some("draw_rectangle 10,20 20x10 colour 5 filled")
    );
    let last = log.last().unwrap();
    assert_eq!(
        (last.tool.as_str(), last.ok, last.code.as_deref()),
        ("undo", false, Some("NOTHING_TO_UNDO"))
    );
}

#[test]
fn changes_follow_the_menu_command_rules() {
    let mut rig = Rig::new(None);
    // A polygon in progress (no button held) is committed first, like before a menu item.
    rig.app.ui.tool = dpx_ui::Tool::Polygon;
    let p = ui_point(&rig.app, (40, 40));
    let q = ui_point(&rig.app, (80, 60));
    use egui::PointerButton::Primary;
    rig.pass_with(vec![
        egui::Event::PointerMoved(p),
        pointer_button(p, Primary, true),
    ]);
    rig.pass_with(vec![egui::Event::PointerMoved(q)]);
    rig.pass_with(vec![pointer_button(q, Primary, false)]);
    assert!(rig.app.stroke.is_some() && !rig.app.gesture_held());
    let rev = rig.app.engine.document().revision();
    let r = rig.ok("draw_pixel", json!({"x": 1, "y": 1, "color": 3}));
    assert!(rig.app.stroke.is_none(), "the polygon was committed");
    assert_eq!(r["revision"], rev + 2, "the polygon, then the pixel");
    // CLR on a fixed background restores the background (as Clear from the menu does).
    rig.app
        .run_action(Action::Command("background.fix"), &rig.ctx.clone());
    rig.ok("draw_pixel", json!({"x": 2, "y": 1, "color": 4}));
    rig.ok(
        "execute_command",
        json!({"command": {"op": "clear", "color": 7}}),
    );
    assert_eq!(rig.pixel(1, 1), 3, "the fixed background came back");
    assert_eq!(rig.pixel(2, 1), 0);
    // Colours beyond a smaller palette are clamped afterwards.
    rig.app.ui.fg = 31;
    rig.ok(
        "create_document",
        json!({"width": 320, "height": 256, "chipset": "aga", "colors": 8}),
    );
    assert_eq!(rig.app.engine.document().palette().len(), 8);
    assert_eq!(rig.app.ui.fg, 7);
}

#[test]
fn requesters_make_changes_busy_but_reads_work() {
    let mut rig = Rig::new(None);
    rig.app
        .run_action(Action::Command("palette.mixer"), &rig.ctx.clone());
    assert!(rig.app.ui.palette_requester.is_some());
    let (code, msg) = rig.code("draw_rectangle", rect(0, 0, 3));
    assert_eq!(code, "EDITOR_BUSY");
    assert!(msg.contains("requester is open"), "{msg}");
    let (code, _) = rig.code("set_palette_color", json!({"index": 1, "color": "#123456"}));
    assert_eq!(code, "EDITOR_BUSY");
    assert_eq!(rig.pixel(0, 0), 0, "nothing changed");
    // Reads still answer.
    let info = rig.ok("get_document_info", json!({}));
    assert_eq!(info["width"], 320);
    rig.ok("get_palette", json!({}));
    // Closed: changes work again.
    rig.app.ui.palette_requester = None;
    rig.ok("draw_rectangle", rect(0, 0, 3));
    assert_eq!(rig.pixel(0, 0), 3);
}

#[test]
fn a_held_mouse_button_makes_changes_wait_a_bounded_time() {
    use egui::PointerButton::Primary;
    let mut rig = Rig::new(None);
    rig.app.ui.tool = dpx_ui::Tool::ContinuousFreehand;
    let p = ui_point(&rig.app, (60, 60));
    rig.pass_with(vec![
        egui::Event::PointerMoved(p),
        pointer_button(p, Primary, true),
    ]);
    assert!(rig.app.gesture_held());
    // Reads do not wait (calls run one at a time in arrival order, so a read never queues
    // behind a held change).
    assert_eq!(rig.ok("get_palette", json!({}))["size"], 32);
    let rx = rig.start("draw_pixel", json!({"x": 5, "y": 5, "color": 9}));
    rig.still_waiting(&rx);
    // The release ends the stroke; the change runs after it, as its own undo step.
    rig.pass_with(vec![pointer_button(p, Primary, false)]);
    let r = rig.wait(&rx);
    assert_eq!(r["success"], true, "{r}");
    assert_eq!(rig.pixel(5, 5), 9);
    rig.app.undo();
    assert_eq!(rig.pixel(5, 5), 0, "the pixel was the last step");
    assert_eq!(rig.pixel(60, 60), 1, "the stroke stays");

    // Held for longer than the limit: refused.
    rig.app.mcp.hold_limit = Duration::from_millis(40);
    rig.pass_with(vec![pointer_button(p, Primary, true)]);
    let rx = rig.start("draw_pixel", json!({"x": 6, "y": 5, "color": 9}));
    std::thread::sleep(Duration::from_millis(60));
    let r = rig.wait(&rx);
    assert_eq!(r["code"], "EDITOR_BUSY", "{r}");
    assert!(r["message"].as_str().unwrap().contains("painting"));
    assert_eq!(rig.pixel(6, 5), 0);
}

#[test]
fn permissions_follow_the_settings_live() {
    let mut rig = Rig::new(None);
    rig.app.mcp.settings.allow_edit = false;
    rig.pass(); // the server gets settings in a pass
    let (code, msg) = rig.code("draw_pixel", json!({"x": 1, "y": 1, "color": 1}));
    assert_eq!(code, "PERMISSION_DENIED");
    assert!(msg.contains("Prefs > AI & MCP"), "{msg}");
    rig.ok("get_palette", json!({}));
    rig.app.mcp.settings.allow_read = false;
    rig.pass(); // the server gets settings in a pass
    for (tool, args) in [
        ("get_palette", json!({})),
        ("list_documents", json!({})),
        ("get_preview", json!({})),
        ("draw_pixel", json!({"x": 1, "y": 1, "color": 1})),
    ] {
        assert_eq!(rig.code(tool, args).0, "PERMISSION_DENIED", "{tool}");
    }
    // Reading off: changing tools are refused too, even with editing on.
    rig.app.mcp.settings.allow_edit = true;
    rig.pass(); // the server gets settings in a pass
    assert_eq!(
        rig.code("draw_pixel", json!({"x": 1, "y": 1, "color": 1}))
            .0,
        "PERMISSION_DENIED"
    );
    let st = rig.ok("get_app_status", json!({}));
    assert_eq!(st["permissions"]["read"], false);
    assert!(st["document"].is_null());
    assert_eq!(rig.pixel(1, 1), 0);
    rig.app.mcp.settings.allow_read = true;
    rig.pass(); // the server gets settings in a pass
    rig.ok("draw_pixel", json!({"x": 1, "y": 1, "color": 1}));
}

/// Clicks button `i` of the open choice requester.
fn click_choice(rig: &mut Rig, i: usize) {
    let req = rig.app.ui.choice_requester.as_ref().expect("the question");
    let (w, h) = rig.app.view().ui_size();
    let (ox, oy) = req.origin(w, h);
    let (bx, by) = req.button_centre(i);
    rig.t += 0.1;
    click_ui(
        &mut rig.app,
        &rig.ctx,
        rig.t,
        (ox + bx, oy + by),
        egui::PointerButton::Primary,
    );
}

#[test]
fn replacing_a_file_asks_the_user_first() {
    let dir = scratch("overwrite");
    let mut rig = Rig::new(Some(&dir));
    rig.tools.state.activity().client("claude-code");
    rig.ok("draw_rectangle", rect(0, 0, 4));
    rig.ok("export_image", json!({"path": "castle.iff"}));
    let first = std::fs::read(dir.join("castle.iff")).unwrap();
    // Without overwrite the file is kept and nobody is asked.
    let (code, _) = rig.code("export_image", json!({"path": "castle.iff"}));
    assert_eq!(code, "FILE_EXISTS");
    assert!(rig.app.ui.choice_requester.is_none());

    rig.ok("draw_rectangle", rect(40, 0, 6));
    // Keep: the requester asks, the call waits, the file stays.
    let rx = rig.start(
        "export_image",
        json!({"path": "castle.iff", "overwrite": true}),
    );
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
        assert!(rx.try_recv().is_err());
    }
    let req = rig.app.ui.choice_requester.as_ref().unwrap();
    assert_eq!(req.title(), "Replace File?");
    assert_eq!(
        req.lines().join(" "),
        "Claude wants to replace castle.iff. Replace?"
    );
    assert_eq!(req.labels(), ["Replace", "Keep"]);
    assert!(rig.app.mcp.asking());
    rig.still_waiting(&rx);
    // Return keeps the file (only a click replaces it).
    rig.pass_with(vec![crate::tests::key_event(egui::Key::Enter, true)]);
    let r = rig.wait(&rx);
    assert_eq!(r["code"], "OVERWRITE_DECLINED", "{r}");
    assert_eq!(std::fs::read(dir.join("castle.iff")).unwrap(), first);
    assert!(rig.app.ui.choice_requester.is_none() && !rig.app.mcp.asking());

    // Replace: written with the app's own file code.
    let rx = rig.start(
        "export_image",
        json!({"path": "castle.iff", "overwrite": true}),
    );
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
    }
    click_choice(&mut rig, 0);
    let r = rig.wait(&rx);
    assert_eq!(r["success"], true, "{r}");
    let (bytes, _) = files::encode_document(
        rig.app.engine.document(),
        &rig.app.extras,
        Path::new("x.iff"),
    )
    .unwrap();
    assert_eq!(std::fs::read(dir.join("castle.iff")).unwrap(), bytes);

    // Nobody answers: refused after the timeout, the requester goes.
    rig.app.mcp.ask_timeout = Duration::from_millis(30);
    let before = std::fs::read(dir.join("castle.iff")).unwrap();
    rig.ok("draw_pixel", json!({"x": 100, "y": 100, "color": 2}));
    let rx = rig.start(
        "export_image",
        json!({"path": "castle.iff", "overwrite": true}),
    );
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
    }
    std::thread::sleep(Duration::from_millis(40));
    let r = rig.wait(&rx);
    assert_eq!(r["code"], "OVERWRITE_DECLINED", "{r}");
    assert!(r["message"].as_str().unwrap().contains("nobody answered"));
    assert!(rig.app.ui.choice_requester.is_none());
    assert_eq!(std::fs::read(dir.join("castle.iff")).unwrap(), before);

    // Another requester open: the question cannot be asked.
    rig.app.ui.note_requester = crate::note_requester("Note", &["x".to_string()]);
    let (code, _) = rig.code(
        "export_image",
        json!({"path": "castle.iff", "overwrite": true}),
    );
    assert_eq!(code, "EDITOR_BUSY");
    rig.app.ui.note_requester = None;

    // "Overwrite without asking": no question. The server gets the setting in the next
    // pass (Prefs > AI & MCP syncs at once); before it, a call could still see Ask.
    rig.app.mcp.settings.overwrite_without_asking = true;
    rig.pass();
    assert_eq!(rig.tools.state.settings().overwrite, Overwrite::Allowed);
    rig.ok(
        "export_image",
        json!({"path": "castle.iff", "overwrite": true}),
    );
    assert!(rig.app.ui.choice_requester.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saving_uses_the_app_file_code_and_marks_the_picture_saved() {
    let dir = scratch("save");
    let mut rig = Rig::new(Some(&dir));
    rig.ok(
        "create_document",
        json!({"width": 320, "height": 240, "chipset": "ocs", "colors": 32, "name": "Castle"}),
    );
    assert_eq!(rig.app.engine.document().target().screen, (320, 240));
    let info = rig.ok("get_document_info", json!({}));
    assert_eq!(info["name"], "Castle");
    rig.ok(
        "draw_ellipse",
        json!({"cx": 160, "cy": 120, "rx": 30, "ry": 20, "color": 3, "filled": true}),
    );
    assert!(rig.app.unsaved());
    let r = rig.ok("save_project", json!({"path": "castle.dpxproj"}));
    assert_eq!(r["format"], "dpxproj");
    assert!(!rig.app.unsaved(), "saved");
    assert_eq!(
        rig.app.picture_path.as_deref(),
        Some(dir.join("castle.dpxproj").as_path())
    );
    let (bytes, _) = files::encode_document(
        rig.app.engine.document(),
        &rig.app.extras,
        Path::new("x.dpxproj"),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(dir.join("castle.dpxproj")).unwrap(),
        bytes,
        "the bytes Picture > Save writes"
    );
    // The app opens it like any project.
    let loaded = files::load(&dir.join("castle.dpxproj")).unwrap();
    let (doc, _, _) = files::open_picture(loaded.picture, Default::default());
    assert_eq!(doc.page(), rig.app.engine.document().page());
    assert_eq!(rig.ok("get_document_info", json!({}))["modified"], false);
    // Outside the approved directory: refused.
    let (code, _) = rig.code("save_project", json!({"path": "/tmp/elsewhere.dpxproj"}));
    assert_eq!(code, "PATH_NOT_ALLOWED");
    // An export beyond the hardware limits is refused before the app encodes it. The
    // engine refuses such a palette on OCS (ADR-46), so the test builds the document.
    let doc = rig.app.engine.document();
    let mut colors = doc.palette().colors().to_vec();
    colors[3] = dpx_core::Rgb8::new(1, 2, 3);
    let bad = dpx_core::Document::from_picture(
        doc.page().clone(),
        dpx_core::Palette::new(colors).unwrap(),
        doc.target(),
    );
    rig.app.engine = dpx_core::Engine::new(bad);
    let (code, msg) = rig.code("export_image", json!({"path": "castle.iff"}));
    assert_eq!(code, "HARDWARE_LIMIT");
    assert!(msg.contains("12-bit"), "{msg}");
    assert!(!dir.join("castle.iff").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Runs the next queued job alone (waiting up to 5 s for one), as `logic` would, and stops
/// there: the test can act like the user before the call's next job.
fn run_next_job(app: &mut PainterApp) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(job) = app.mcp.running.as_mut().unwrap().queue.take() {
            job.run(&mut GuiWorkspace(app));
            return;
        }
        assert!(Instant::now() < until, "no job came");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Picture > Load of a blank `colour` picture, as the user does it.
fn gui_load(app: &mut PainterApp, path: &Path, colour: u8) {
    let doc = Document::from_picture(
        IndexedImage::new(320, 256, colour),
        dpx_core::Palette::dpaint_default_32(),
        app.engine.document().target(),
    );
    app.set_picture(doc, files::PictureExtras::default(), path.to_path_buf());
}

#[test]
fn an_import_converted_while_the_user_loaded_another_picture_is_refused() {
    let dir = scratch("import-race");
    let png = dir.join("in.png");
    let rgba: Vec<u8> = (0..16 * 16).flat_map(|_| [255, 0, 0, 255]).collect();
    crate::write_png(png.to_str().unwrap(), 16, 16, &rgba).unwrap();
    let mut rig = Rig::new(Some(&dir));
    assert_eq!(rig.app.engine.document().revision(), 0);
    let rx = rig.start(
        "import_image",
        json!({"path": "in.png", "target": "spare", "colors": 2}),
    );
    // The job that reads the picture's format and palette runs; the conversion follows
    // off the UI thread, and meanwhile the user loads another picture (also revision 0).
    run_next_job(&mut rig.app);
    gui_load(&mut rig.app, &dir.join("other.iff"), 9);
    assert_eq!(rig.app.engine.document().revision(), 0, "the same revision");
    let v = rig.wait(&rx);
    assert_eq!(v["code"], "REVISION_CONFLICT", "{v}");
    assert!(
        rig.app.menu_data.spare.is_none(),
        "nothing was made for the old picture"
    );
    assert_eq!(rig.pixel(0, 0), 9, "the loaded picture stays");
    // Without a change in between it works.
    let v = rig.ok(
        "import_image",
        json!({"path": "in.png", "target": "spare", "colors": 2}),
    );
    assert_eq!(v["target"], "spare");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_project_saved_while_the_user_loaded_another_picture_names_only_the_saved_one() {
    let dir = scratch("save-race");
    let mut rig = Rig::new(Some(&dir));
    let rx = rig.start("save_project", json!({"path": "mcp.dpxproj"}));
    // The job that copies the picture runs; while the file is written the user loads
    // another picture (a new document at the same revision 0, the same page slot).
    run_next_job(&mut rig.app);
    let elsewhere = dir.join("elsewhere");
    let loaded = elsewhere.join("loaded.iff");
    gui_load(&mut rig.app, &loaded, 4);
    let v = rig.wait(&rx);
    assert_eq!(v["success"], true, "{v}");
    assert!(
        v["notes"][0].as_str().unwrap().contains("not marked saved"),
        "{v}"
    );
    assert!(
        dir.join("mcp.dpxproj").exists(),
        "the old picture was written"
    );
    assert_eq!(
        rig.app.picture_path.as_deref(),
        Some(loaded.as_path()),
        "the loaded picture keeps its own name"
    );
    assert_eq!(
        rig.app.last_drawer.as_deref(),
        Some(elsewhere.to_str().unwrap()),
        "and its drawer"
    );
    assert!(!rig.app.unsaved(), "it is clean as loaded");
    let info = rig.ok("get_document_info", json!({}));
    assert_ne!(info["name"], "mcp.dpxproj", "{info}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn documents_are_the_picture_and_the_spare_page() {
    let mut rig = Rig::new(None);
    rig.ok("draw_pixel", json!({"x": 0, "y": 0, "color": 5}));
    let list = rig.ok("list_documents", json!({}));
    assert_eq!(list["active"], "page1");
    assert_eq!(list["documents"][1]["blank"], true);
    let s = rig.ok("select_document", json!({"document_id": "page2"}));
    assert_eq!(s["changed"], true);
    assert_eq!(rig.pixel(0, 0), 0, "the blank spare is shown");
    rig.ok("draw_pixel", json!({"x": 1, "y": 0, "color": 6}));
    // The GUI's own swap (`j`) keeps the ids with their pages.
    rig.app.swap_spare();
    assert_eq!(rig.ok("list_documents", json!({}))["active"], "page1");
    assert_eq!(rig.pixel(0, 0), 5);
    let s = rig.ok("select_document", json!({"document_id": "page1"}));
    assert_eq!(s["changed"], false);
    let spare = rig.ok("get_document_info", json!({"document_id": "page2"}));
    assert_eq!(spare["active"], false);
    assert_eq!(rig.app.mcp_spare().unwrap().page().get(1, 0), Some(6));
}

#[test]
fn stopping_fails_waiting_calls_and_closes_the_question() {
    let dir = scratch("stop");
    let mut rig = Rig::new(Some(&dir));
    rig.ok("export_image", json!({"path": "a.png"}));
    let rx = rig.start("export_image", json!({"path": "a.png", "overwrite": true}));
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
    }
    rig.app.mcp_stop();
    assert!(rig.app.ui.choice_requester.is_none());
    let (_, r) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let v = r.unwrap().structured_content.unwrap();
    assert_eq!(v["code"], "APP_UNAVAILABLE", "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The status code and body of a `tools/call` POST to the app's server.
fn post(port: u16, token: &str, name: &str, args: Value) -> (u16, Value) {
    let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": name, "arguments": args}}))
    .unwrap();
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         MCP-Protocol-Version: 2025-06-18\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    s.write_all(&body).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

/// `post` from a thread while the window runs passes.
fn post_live(rig: &mut Rig, port: u16, token: &str, name: &str, args: Value) -> (u16, Value) {
    let (token, name) = (token.to_string(), name.to_string());
    let h = std::thread::spawn(move || post(port, &token, &name, args));
    let until = Instant::now() + Duration::from_secs(10);
    while !h.is_finished() {
        rig.pass();
        std::thread::sleep(Duration::from_millis(2));
        assert!(Instant::now() < until, "no answer");
    }
    h.join().unwrap()
}

#[test]
fn the_server_starts_and_stops_with_the_setting() {
    let config = scratch("server-config");
    let pictures = scratch("server-pictures");
    let mut rig = Rig::new(None);
    rig.app.mcp_stop();
    rig.app.mcp.config_dir = Some(config.clone());
    rig.app.mcp.settings.port = 0;
    rig.app.mcp.settings.approved_dirs = vec![pictures.clone(), config.join("gone")];
    rig.pass();
    assert_eq!(rig.app.mcp.status(), Status::Off, "off by default");
    assert!(!config.join(endpoint::ENDPOINT_FILE).exists());

    rig.app.mcp.settings.enabled = true;
    rig.pass();
    let Status::Running { port, url } = rig.app.mcp.status() else {
        panic!("running: {:?}", rig.app.mcp.status());
    };
    assert_eq!(url, format!("http://127.0.0.1:{port}/mcp"));
    assert_eq!(rig.app.mcp.missing_dirs(), [config.join("gone")]);
    let e = endpoint::read_endpoint(&config).expect("the endpoint file");
    assert_eq!(e.port, port);
    let token = std::fs::read_to_string(endpoint::token_path(&config)).unwrap();
    assert_eq!(
        rig.app.mcp.token(),
        Some(token.as_str()),
        "the stored token"
    );
    // The endpoint file names a token for this start only: a file a crash leaves behind
    // gives nothing away.
    assert_ne!(e.token, token);
    assert!(
        endpoint::read_live_endpoint(&config).is_some(),
        "its lock is held"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |f: &str| {
            std::fs::metadata(config.join(f))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(endpoint::TOKEN_FILE), 0o600);
        assert_eq!(mode(endpoint::ENDPOINT_FILE), 0o600);
    }
    let launch = endpoint::read_app_file(&config).expect("the launch file");
    assert_eq!(launch.args, ["--mcp"]);
    assert!(launch.enabled, "the bridge may start the app");

    // A call over HTTP lands in the window (with the endpoint file's token, as the bridge
    // sends it; the stored token works as well).
    let (status, v) = post_live(&mut rig, port, &e.token, "draw_rectangle", rect(2, 2, 7));
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["result"]["structuredContent"]["success"], true, "{v}");
    assert_eq!(rig.pixel(2, 2), 7);
    let (_, v) = post_live(&mut rig, port, &token, "get_app_status", json!({}));
    let st = &v["result"]["structuredContent"];
    assert_eq!(st["engine"], "gui");
    assert_eq!(st["transport"], "http");
    assert_eq!(
        st["file_access"]["approved_dirs"],
        json!([pictures.display().to_string()])
    );
    assert!(rig.app.mcp.activity().unwrap().tool_calls >= 2);
    assert_eq!(
        rig.app.mcp.log().first().map(|e| e.tool.clone()).as_deref(),
        Some("draw_rectangle")
    );

    // Off: stopped, the endpoint file is gone, calls get no answer, and the launch record
    // says off.
    rig.app.mcp.settings.enabled = false;
    rig.pass();
    assert_eq!(rig.app.mcp.status(), Status::Off);
    assert!(endpoint::read_endpoint(&config).is_none());
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert!(!endpoint::read_app_file(&config).unwrap().enabled);

    // On again (also for one session with --mcp): the same stored token, so clients
    // configured with it reconnect; the endpoint file has a new one of its own.
    rig.app.mcp.session = true;
    rig.pass();
    let Status::Running { port, .. } = rig.app.mcp.status() else {
        panic!("running again");
    };
    assert!(endpoint::read_app_file(&config).unwrap().enabled);
    assert_eq!(rig.app.mcp.token(), Some(token.as_str()));
    let again = endpoint::read_endpoint(&config).unwrap().token;
    assert!(again != e.token && again != token);
    assert_eq!(
        post_live(&mut rig, port, &token, "get_palette", json!({})).0,
        200
    );
    assert_eq!(
        post_live(&mut rig, port, &e.token, "get_palette", json!({})).0,
        401,
        "the last start's endpoint token is dead"
    );

    // Revoke: a new token, the old one is refused at once.
    rig.app.mcp_revoke().unwrap();
    let new = std::fs::read_to_string(endpoint::token_path(&config)).unwrap();
    assert_ne!(new, token);
    assert_eq!(rig.app.mcp.token(), Some(new.as_str()));
    assert_eq!(
        post_live(&mut rig, port, &token, "get_palette", json!({})).0,
        401
    );
    assert_eq!(
        post_live(&mut rig, port, &new, "get_palette", json!({})).0,
        200
    );
    let revoked = endpoint::read_endpoint(&config).unwrap().token;
    assert_ne!(revoked, again, "the endpoint file gets a new token too");
    assert_eq!(
        post_live(&mut rig, port, &again, "get_palette", json!({})).0,
        401
    );
    assert_eq!(
        post_live(&mut rig, port, &revoked, "get_palette", json!({})).0,
        200
    );

    // Exit stops it; the launch record keeps the saved setting (off: the session's `--mcp`
    // is not saved).
    eframe::App::on_exit(&mut rig.app);
    assert!(endpoint::read_endpoint(&config).is_none());
    assert!(!endpoint::read_app_file(&config).unwrap().enabled);
    let _ = std::fs::remove_dir_all(&config);
    let _ = std::fs::remove_dir_all(&pictures);
}

#[test]
fn settings_section_round_trip_and_defaults() {
    let d = McpSettings::default();
    assert!(!d.enabled, "off by default");
    assert_eq!(d.port, 8765);
    assert!(d.allow_read && d.allow_edit && !d.overwrite_without_asking);
    let s = McpSettings {
        enabled: true,
        port: 9000,
        allow_read: true,
        allow_edit: false,
        overwrite_without_asking: true,
        approved_dirs: vec![PathBuf::from("/a"), PathBuf::from("/b c")],
    };
    let back = McpSettings::from_json(s.to_json().as_object().unwrap(), &d);
    assert_eq!(back, s);
    // Missing keys keep the defaults; an explicit empty list means no file access.
    let o = json!({"enabled": "yes", "port": 70000, "approved_dirs": []});
    let c = McpSettings::from_json(o.as_object().unwrap(), &d);
    assert_eq!(c.port, 8765);
    assert!(!c.enabled);
    assert!(c.approved_dirs.is_empty());
    let c = McpSettings::from_json(&Map::new(), &d);
    assert_eq!(c, d);
}

#[test]
fn pictures_directory_per_platform() {
    use dpx_mcp::paths::Platform;
    let env = |vars: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    };
    assert_eq!(
        pictures_dir_with(env(&[("HOME", "/Users/a")]), Platform::MacOs),
        Some(PathBuf::from("/Users/a/Pictures/DeluxePainter"))
    );
    assert_eq!(
        pictures_dir_with(env(&[("USERPROFILE", "C:/Users/a")]), Platform::Windows),
        Some(PathBuf::from("C:/Users/a/Pictures/DeluxePainter"))
    );
    assert_eq!(
        pictures_dir_with(
            env(&[("XDG_PICTURES_DIR", "/p"), ("HOME", "/h")]),
            Platform::Other
        ),
        Some(PathBuf::from("/p/DeluxePainter"))
    );
    assert_eq!(
        pictures_dir_with(env(&[("HOME", "/h")]), Platform::Other),
        Some(PathBuf::from("/h/Pictures/DeluxePainter"))
    );
    assert_eq!(pictures_dir_with(env(&[]), Platform::MacOs), None);
}

#[test]
fn asker_names_claude_or_the_client() {
    use dpx_mcp::tools::asker;
    assert_eq!(asker(Some("claude-code")), "Claude");
    assert_eq!(
        asker(Some("Claude Desktop via deluxepainter-bridge")),
        "Claude"
    );
    assert_eq!(asker(Some("inspector")), "inspector");
    assert_eq!(asker(None), "An AI client");
}

#[test]
fn the_snapshot_flag_runs_calls_on_the_gui_workspace() {
    let file = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/screenshots/wave8-mcp-gui-calls.json"
    );
    let args: Vec<String> = ["--snapshot", "x.png", "--mcp-calls", file]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let start = PainterApp::new().engine.document().clone();
    let doc = snapshot_calls(&args, &start).unwrap().expect("the flag");
    let t = doc.target();
    assert_eq!((t.chipset, t.screen), (dpx_core::Chipset::Ocs, (320, 240)));
    assert_eq!(doc.palette().len(), 32);
    let px = |x, y| doc.page().get(x, y).unwrap();
    assert_eq!(px(262, 46), 5, "the sun");
    assert_eq!(px(150, 150), 5, "the door knob");
    assert_eq!(px(60, 200), 21, "the last rectangle was undone");
    assert_eq!(
        doc.palette().colors()[20],
        dpx_core::Rgb8::new(0x22, 0x55, 0xAA)
    );
    assert_eq!(snapshot_calls(&args[..2], &start).unwrap(), None);
}
