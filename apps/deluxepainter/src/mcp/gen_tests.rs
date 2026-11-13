//! MCP's image generation tools on the GUI workspace (ADR-45): the real tools through the
//! Rig, with the requester's job manager and generator. Generations use the deterministic
//! test pattern or a gated test provider (no network, no key store).

use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};

use dpx_core::{Chipset, Rgb8};
use dpx_imagegen::fixture::TestPattern;
use dpx_imagegen::{
    CancelToken, Capabilities, GenError, GenerationRequest, ImageGenerationProvider, JobId,
    ProviderStatus,
};

use super::*;
use crate::tests::{key_event, tap};

/// A "network" provider that waits until the test lets it go (a request in flight).
struct Gated(Mutex<std::sync::mpsc::Receiver<()>>);

impl ImageGenerationProvider for Gated {
    fn id(&self) -> &str {
        "gated"
    }
    fn display_name(&self) -> &str {
        "Gated test"
    }
    fn needs_network(&self) -> bool {
        true
    }
    fn cost_note(&self) -> &str {
        "Test only."
    }
    fn capabilities(&self) -> Capabilities {
        TestPattern.capabilities()
    }
    fn status(&self) -> ProviderStatus {
        ProviderStatus::Ready
    }
    fn generate(
        &self,
        request: &GenerationRequest,
        cancel: &CancelToken,
    ) -> Result<Vec<dpx_core::RgbaImage>, GenError> {
        let _ = self.0.lock().unwrap().recv_timeout(Duration::from_secs(10));
        cancel.check()?;
        TestPattern.generate(request, cancel)
    }
}

fn gated() -> (Arc<Gated>, Sender<()>) {
    let (tx, rx) = channel();
    (Arc::new(Gated(Mutex::new(rx))), tx)
}

fn rig_with(provider: Option<Arc<dyn ImageGenerationProvider>>) -> Rig {
    let mut rig = Rig::new(None);
    rig.app.imagegen.debug_provider = provider;
    rig.tools.state.activity().client("claude-code");
    rig
}

fn job_of(v: &Value) -> String {
    v["job_id"].as_str().expect("a job id").to_string()
}

/// Waits for the job to end, then asks its status (the whole result: image and JSON).
fn done(rig: &mut Rig, job: &str) -> CallToolResult {
    let id = JobId(job.strip_prefix("gen-").unwrap().parse().unwrap());
    rig.app.imagegen.jobs.wait(id, Duration::from_secs(10));
    let rx = rig.start("get_generation_status", json!({ "job_id": job }));
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        rig.pass();
        if let Ok((_, r)) = rx.recv_timeout(Duration::from_millis(2)) {
            return r.expect("known tool");
        }
        assert!(Instant::now() < until, "the status was not answered");
    }
}

fn generated(rig: &mut Rig, args: Value) -> String {
    let job = job_of(&rig.ok("generate_image", args));
    let r = done(rig, &job);
    let s = r.structured_content.as_ref().unwrap();
    assert_eq!(s["status"], "done", "{s}");
    job
}

#[test]
fn without_a_generator_the_tool_says_where_to_set_one_up() {
    // Acceptance test 6 through MCP: a clear error, nothing sent, the editor goes on.
    let mut rig = rig_with(None);
    let (code, msg) = rig.code("generate_image", json!({"prompt": "a castle at dusk"}));
    assert_eq!(code, "IMAGE_GENERATOR_NOT_CONFIGURED");
    assert!(msg.contains("Prefs > AI & MCP.. > Set up.."), "{msg}");
    assert!(rig.app.imagegen.jobs.list().is_empty(), "nothing was sent");
    let log = rig.app.mcp.log();
    let last = log.last().unwrap();
    assert_eq!(
        (last.tool.as_str(), last.ok, last.code.as_deref()),
        (
            "generate_image",
            false,
            Some("IMAGE_GENERATOR_NOT_CONFIGURED")
        )
    );
    rig.ok("draw_pixel", json!({"x": 1, "y": 1, "color": 2}));
}

#[test]
fn a_320x240_ocs_picture_goes_to_the_spare_page_in_its_own_format() {
    // Acceptance test 5 with the test pattern: exactly 320x240 pixels, at most 32 colours,
    // every one a 12-bit OCS colour.
    let mut rig = rig_with(Some(Arc::new(TestPattern)));
    let before = rig.app.engine.document().clone();
    let started = rig.ok(
        "generate_image",
        json!({"prompt": "Mittelalterliche Burg bei Sonnenuntergang", "width": 320,
               "height": 240, "chipset": "OCS", "palette_size": 32, "dithering": "ordered"}),
    );
    assert_eq!(started["target"], "spare", "the default target");
    assert_eq!(started["status"], "running");
    let job = job_of(&started);
    let r = done(&mut rig, &job);
    assert!(r.content[0].as_image().is_some(), "a preview image");
    let s = r.structured_content.unwrap();
    assert_eq!(
        (s["width"].clone(), s["height"].clone()),
        (json!(320), json!(240))
    );
    assert!(s["colors_used"].as_u64().unwrap() <= 32, "{s}");
    let a = rig.ok("apply_generated_image", json!({ "job_id": job }));
    assert_eq!(a["document_id"], "page2");
    assert_eq!(a["undoable"], true);
    assert_eq!(rig.app.engine.document(), &before, "the shown page stays");
    let list = rig.ok("list_documents", json!({}));
    let spare = &list["documents"][1];
    assert_eq!(
        (spare["width"].clone(), spare["height"].clone()),
        (json!(320), json!(240))
    );
    assert_eq!(spare["chipset"], "ocs");
    assert!(spare["colors"].as_u64().unwrap() <= 32, "{spare}");
    // Shown, it is exactly that picture.
    rig.ok("select_document", json!({"document_id": "page2"}));
    let doc = rig.app.engine.document();
    assert_eq!(doc.target().chipset, Chipset::Ocs);
    assert_eq!((doc.page().width(), doc.page().height()), (320, 240));
    assert!(doc.palette().len() <= 32);
    for c in doc.palette().colors() {
        assert_eq!(Rgb8::from_rgb4(c.to_rgb4()), *c, "{c:?} is a 12-bit colour");
    }
    assert!(
        doc.page()
            .pixels()
            .iter()
            .all(|&p| usize::from(p) < doc.palette().len())
    );
    let log = rig.app.mcp.log();
    let started = log.iter().find(|e| e.tool == "generate_image").unwrap();
    assert!(
        started
            .detail
            .as_deref()
            .unwrap()
            .starts_with(&format!("generate_image {job} 320x240 OCS 32 colours")),
        "{:?}",
        started.detail
    );
    assert!(
        log.iter()
            .any(|e| e.tool == "apply_generated_image" && e.ok)
    );
}

#[test]
fn a_new_layer_holds_the_picture_in_the_current_frame_and_is_one_undo_step() {
    let mut rig = rig_with(Some(Arc::new(TestPattern)));
    rig.ok(
        "execute_command",
        json!({"command": {"op": "add_frames", "count": 2}}),
    );
    rig.ok(
        "execute_command",
        json!({"command": {"op": "goto_frame", "frame": 1}}),
    );
    let job = generated(
        &mut rig,
        json!({"prompt": "a tree", "target": "new_layer", "background": "transparent"}),
    );
    let before = rig.app.engine.document().clone();
    let rev = before.revision();
    let a = rig.ok("apply_generated_image", json!({ "job_id": job }));
    assert_eq!(a["document_id"], "page1");
    assert_eq!(a["revision"], rev + 1, "one step");
    let doc = rig.app.engine.document();
    let names: Vec<&str> = doc.layers().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "Generated"]);
    assert_eq!((doc.active_layer(), doc.current_frame()), (1, 1));
    assert_eq!(doc.frame_count(), 3);
    let cel = doc.cel(1, 1).unwrap();
    assert!(cel.pixels().contains(&0), "the transparent sky is colour 0");
    assert!(cel.pixels().iter().any(|&p| p != 0));
    for f in [0, 2] {
        assert!(
            doc.cel(f, 1).unwrap().pixels().iter().all(|&p| p == 0),
            "frame {f} is clear"
        );
    }
    // The GUI's Undo takes the whole apply back.
    rig.pass_with(vec![key_event(egui::Key::U, true)]);
    let doc = rig.app.engine.document();
    assert_eq!(doc.layers().len(), 1);
    assert_eq!(doc.page(), before.page());
    assert_eq!((doc.active_layer(), doc.current_frame()), (0, 1));
    // A second layer gets its own name.
    rig.app.redo();
    rig.ok("apply_generated_image", json!({ "job_id": job }));
    let names: Vec<String> = rig
        .app
        .engine
        .document()
        .layers()
        .iter()
        .map(|l| l.name.clone())
        .collect();
    assert_eq!(names, ["Background", "Generated", "Generated 2"]);
}

#[test]
fn the_brush_target_makes_the_brush_and_the_picture_target_asks_first() {
    let mut rig = rig_with(Some(Arc::new(TestPattern)));
    // A 4-colour picture and a brush converted with 256 colours of its own: the numbers
    // describe the brush as it lands, fitted to the picture's 4 colours.
    rig.ok(
        "create_document",
        json!({"width": 320, "height": 256, "chipset": "ocs", "colors": 4}),
    );
    let job = job_of(&rig.ok(
        "generate_image",
        json!({"prompt": "a castle", "target": "brush", "chipset": "aga", "colors": 256}),
    ));
    let s = done(&mut rig, &job).structured_content.unwrap();
    assert_eq!(s["status"], "done", "{s}");
    assert_eq!(s["palette_size"], 4, "{s}");
    assert!(s["colors_used"].as_u64().unwrap() <= 4, "{s}");
    let before = rig.app.engine.document().clone();
    let a = rig.ok("apply_generated_image", json!({ "job_id": job }));
    assert_eq!(a["undoable"], false);
    assert!(a["colors_used"].as_u64().unwrap() <= 4, "{a}");
    let brush = rig.app.custom_brush.as_ref().expect("a custom brush");
    assert_eq!((brush.width, brush.height), (320, 256));
    assert!(
        brush.pixels.iter().all(|&p| p < 4),
        "in the picture palette"
    );
    let mut used: Vec<u8> = brush.pixels.clone();
    used.sort_unstable();
    used.dedup();
    assert_eq!(a["colors_used"], used.len(), "what the brush uses");
    assert_eq!(
        rig.app.engine.document(),
        &before,
        "the picture is untouched"
    );

    // The picture: the user is asked; Return keeps the picture.
    let rx = rig.start(
        "apply_generated_image",
        json!({"job_id": job, "target": "picture"}),
    );
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
        assert!(rx.try_recv().is_err());
    }
    let req = rig.app.ui.choice_requester.as_ref().unwrap();
    assert_eq!(req.title(), "Replace Picture?");
    assert!(
        req.lines()
            .join(" ")
            .starts_with("Claude wants to replace the picture"),
        "{:?}",
        req.lines()
    );
    assert_eq!(req.labels(), ["Replace", "Keep"]);
    rig.still_waiting(&rx);
    rig.pass_with(tap(egui::Key::Enter));
    let r = rig.wait(&rx);
    assert_eq!(r["code"], "APPLY_DECLINED", "{r}");
    assert_eq!(rig.app.engine.document(), &before);
    // Replace: one undo step.
    let rx = rig.start(
        "apply_generated_image",
        json!({"job_id": job, "target": "picture"}),
    );
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
    }
    click_choice(&mut rig, 0);
    let r = rig.wait(&rx);
    assert_eq!(r["success"], true, "{r}");
    assert_ne!(rig.app.engine.document().page(), before.page());
    rig.pass_with(vec![key_event(egui::Key::U, true)]);
    assert_eq!(rig.app.engine.document().page(), before.page());
    assert_eq!(rig.app.engine.document().palette(), before.palette());
}

#[test]
fn the_picture_question_names_every_frame_and_layer_that_goes() {
    let mut rig = rig_with(Some(Arc::new(TestPattern)));
    rig.ok(
        "execute_command",
        json!({"command": {"op": "add_frames", "count": 2}}),
    );
    rig.ok(
        "execute_command",
        json!({"command": {"op": "add_layer", "name": "Sky"}}),
    );
    let job = generated(&mut rig, json!({"prompt": "a castle", "target": "picture"}));
    let rx = rig.start("apply_generated_image", json!({ "job_id": job }));
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
    }
    let req = rig.app.ui.choice_requester.as_ref().unwrap();
    let text = req.lines().join(" ");
    assert!(
        text.starts_with("Claude wants to replace the picture (all 3 frames, 2 layers) with"),
        "{text}"
    );
    assert!(
        !req.truncated(),
        "the whole question shows: {:?}",
        req.lines()
    );
    // It fits the smallest UI screen.
    assert!(req.height() <= 200, "{}", req.height());
    click_choice(&mut rig, 0);
    assert_eq!(rig.wait(&rx)["success"], true);
    let doc = rig.app.engine.document();
    assert_eq!((doc.frame_count(), doc.layer_count()), (1, 1));
}

#[test]
fn the_spare_page_is_asked_about_when_it_holds_the_users_picture() {
    let mut rig = rig_with(Some(Arc::new(TestPattern)));
    // The user's picture, then shown page2: page1 is the spare page now.
    rig.ok("draw_pixel", json!({"x": 1, "y": 1, "color": 2}));
    let mine = rig.app.engine.document().clone();
    rig.ok("select_document", json!({"document_id": "page2"}));
    let job = generated(&mut rig, json!({"prompt": "a castle"}));
    let rx = rig.start("apply_generated_image", json!({ "job_id": job }));
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
        assert!(rx.try_recv().is_err(), "it asks first");
    }
    let req = rig.app.ui.choice_requester.as_ref().unwrap();
    assert_eq!(req.title(), "Replace Spare Page?");
    let text = req.lines().join(" ");
    assert!(text.contains("spare page (page1)"), "{text}");
    assert!(!req.truncated() && req.height() <= 200, "{:?}", req.lines());
    assert_eq!(req.labels(), ["Replace", "Keep"]);
    // Return keeps it.
    rig.pass_with(tap(egui::Key::Enter));
    let r = rig.wait(&rx);
    assert_eq!(r["code"], "APPLY_DECLINED", "{r}");
    let spare = rig.app.menu_data.spare.as_ref().unwrap();
    assert_eq!(spare.engine.document().page(), mine.page(), "kept");
    // Replace puts it there; Undo on that page brings the user's picture back.
    let rx = rig.start("apply_generated_image", json!({ "job_id": job }));
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
    }
    click_choice(&mut rig, 0);
    let r = rig.wait(&rx);
    assert_eq!(
        (r["success"].clone(), r["document_id"].clone()),
        (json!(true), json!("page1"))
    );
    rig.ok("select_document", json!({"document_id": "page1"}));
    assert_ne!(rig.app.engine.document().page(), mine.page());
    rig.ok("undo", json!({}));
    assert_eq!(rig.app.engine.document().page(), mine.page());
}

/// apply_generated_image to the picture, answered with Replace.
fn replace_picture(rig: &mut Rig, job: &str) -> Value {
    let rx = rig.start(
        "apply_generated_image",
        json!({"job_id": job, "target": "picture"}),
    );
    while rig.app.ui.choice_requester.is_none() {
        rig.pass();
    }
    click_choice(rig, 0);
    let r = rig.wait(&rx);
    assert_eq!(r["success"], true, "{r}");
    r
}

#[test]
fn mcp_undo_and_redo_of_a_generated_picture_take_its_details_back_and_forth() {
    let mut rig = rig_with(Some(Arc::new(TestPattern)));
    let range = |low, high| dpx_iff::ilbm::ColorRange {
        rate: 8192,
        flags: 1,
        low,
        high,
    };
    let mine = crate::files::PictureExtras {
        ranges: vec![range(20, 27), range(8, 15)],
        grab: Some((3, 4)),
        transparent_color: Some(7),
    };
    rig.app.extras = mine.clone();
    let before = rig.app.engine.document().clone();
    let a = generated(
        &mut rig,
        json!({"prompt": "a castle", "target": "picture", "palette": "new",
               "background": "transparent"}),
    );
    let b = generated(
        &mut rig,
        json!({"prompt": "a tower", "target": "picture", "palette": "new",
               "background": "opaque"}),
    );
    // Two generated pictures in turn, each with the details Palette New gives.
    replace_picture(&mut rig, &a);
    let first = rig.app.extras.clone();
    assert_eq!(
        (first.grab, first.ranges.len(), first.transparent_color),
        (None, 0, Some(0))
    );
    let after_a = rig.app.engine.document().clone();
    replace_picture(&mut rig, &b);
    let second = rig.app.extras.clone();
    assert_eq!(second.transparent_color, None);
    // MCP's undo takes the second back with its details; redo sets them again.
    rig.ok("undo", json!({}));
    assert_eq!(rig.app.engine.document().page(), after_a.page());
    assert_eq!(rig.app.extras, first, "undo brings the details back");
    rig.ok("redo", json!({}));
    assert_ne!(rig.app.engine.document().page(), after_a.page());
    assert_eq!(rig.app.extras, second, "redo sets them again");
    rig.ok("undo", json!({}));
    assert_eq!(rig.app.extras, first);
    // The entry left the list with its step: the GUI's U then finds the first one's.
    rig.pass_with(vec![key_event(egui::Key::U, true)]);
    assert_eq!(rig.app.engine.document().page(), before.page());
    assert_eq!(rig.app.engine.document().palette(), before.palette());
    assert_eq!(rig.app.extras, mine, "the user's own details are back");
    rig.ok("redo", json!({}));
    assert_eq!(rig.app.extras, first);
}

#[test]
fn changes_wait_for_requesters_and_status_reads_do_not() {
    let mut rig = rig_with(Some(Arc::new(TestPattern)));
    let job = generated(&mut rig, json!({"prompt": "a castle"}));
    rig.app.open_generate();
    assert!(rig.app.ui.generate_requester.is_some());
    let (code, _) = rig.code("generate_image", json!({"prompt": "x"}));
    assert_eq!(code, "EDITOR_BUSY");
    let (code, _) = rig.code("apply_generated_image", json!({ "job_id": job }));
    assert_eq!(code, "EDITOR_BUSY");
    assert_eq!(
        rig.ok("get_generation_status", json!({ "job_id": job }))["status"],
        "done"
    );
    // Closed: changes work again.
    rig.app.ui.generate_requester = None;
    rig.ok("apply_generated_image", json!({ "job_id": job }));
}

#[test]
fn cancel_and_the_limit_of_requests_in_flight_are_shared_with_the_requester() {
    let (slow, gate) = gated();
    let mut rig = rig_with(Some(slow));
    // The requester starts one and is cancelled: the request stays in flight.
    rig.app.open_generate();
    rig.app
        .ui
        .generate_requester
        .as_mut()
        .unwrap()
        .set_prompt("A castle");
    rig.pass_with(tap(egui::Key::Enter));
    rig.pass_with(tap(egui::Key::Escape));
    rig.pass_with(tap(egui::Key::Escape));
    assert!(rig.app.ui.generate_requester.is_none());
    // MCP starts the second; a third is refused at once.
    let first = rig.ok("generate_image", json!({"prompt": "one"}));
    let job = job_of(&first);
    let (code, msg) = rig.code("generate_image", json!({"prompt": "two"}));
    assert_eq!(code, "BUSY", "{msg}");
    assert_eq!(
        rig.ok("get_generation_status", json!({ "job_id": job }))["status"],
        "running"
    );
    let (code, _) = rig.code("apply_generated_image", json!({ "job_id": job }));
    assert_eq!(code, "JOB_NOT_DONE");
    let c = rig.ok("cancel_generation", json!({ "job_id": job }));
    assert_eq!(c["status"], "cancelled");
    assert!(
        c["notes"].to_string().contains("may still be billed"),
        "{c}"
    );
    assert_eq!(
        rig.ok("get_generation_status", json!({ "job_id": job }))["status"],
        "cancelled"
    );
    // The requester sees the same limit.
    rig.app.open_generate();
    rig.app
        .ui
        .generate_requester
        .as_mut()
        .unwrap()
        .set_prompt("A castle");
    rig.pass_with(tap(egui::Key::Enter));
    let req = rig.app.ui.generate_requester.as_ref().unwrap();
    assert_eq!(
        req.error(),
        Some(GenError::Busy { running: 2 }.to_string().as_str())
    );
    gate.send(()).unwrap();
    gate.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while rig.app.imagegen.jobs.list().iter().any(|j| j.in_flight) {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    rig.app.ui.generate_requester = None;
    rig.ok("generate_image", json!({"prompt": "three"}));
    gate.send(()).unwrap();
}

#[test]
fn the_api_key_never_reaches_a_client_or_the_activity_log() {
    use dpx_imagegen::openai::KEY_ACCOUNT;
    let mut rig = rig_with(None);
    rig.app.imagegen.settings.provider = crate::generate::GenProvider::OpenAi;
    let (code, msg) = rig.code("generate_image", json!({"prompt": "x"}));
    assert_eq!(code, "IMAGE_GENERATOR_NOT_CONFIGURED");
    assert!(msg.contains("OpenAI"), "{msg}");
    let secret = dpx_imagegen::Secret::new("sk-test-SECRET-0815").unwrap();
    rig.app.imagegen.keys.set(KEY_ACCOUNT, &secret);
    // Refused before anything is sent (no network in tests).
    let r = rig.call("generate_image", json!({"prompt": "x", "colors": 12}));
    assert_eq!(r["code"], "PALETTE_RANGE_INVALID", "{r}");
    let status = rig.ok("get_app_status", json!({}));
    let host = rig.app.mcp_gen_host();
    for text in [
        r.to_string(),
        status.to_string(),
        format!("{:?}", rig.app.mcp.log()),
        format!("{host:?}"),
        rig.app.generator_line(),
    ] {
        assert!(!text.contains("SECRET"), "{text}");
    }
    assert_eq!(
        host.model.as_deref(),
        Some(dpx_imagegen::openai::DEFAULT_MODEL)
    );
}
