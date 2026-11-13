//! Tests of the image generation tools through `tools::call`: the headless server (no image
//! generator) and, with the `imagegen` feature, a headless workspace given the test pattern
//! generator (`dpx_imagegen::fixture::TestPattern`, never a real one; no network, no keys).

use super::*;
use crate::jobs::spawn_headless;
use crate::state::Settings;
use crate::workspace::HeadlessWorkspace;
use serde_json::{Value, json};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn call_on(rt: &tokio::runtime::Runtime, ctx: &ToolCtx, name: &str, args: Value) -> CallToolResult {
    rt.block_on(super::super::call(ctx, name, args.as_object().cloned()))
        .expect("known tool")
}

fn code_of(r: &CallToolResult) -> String {
    let s = r.structured_content.as_ref().unwrap();
    assert_eq!(s["success"], false, "{s}");
    s["code"].as_str().unwrap().to_string()
}

#[test]
fn the_tools_are_listed_and_annotated() {
    let defs = super::super::definitions();
    let get = |n: &str| {
        defs.iter()
            .find(|t| t.name == n)
            .unwrap_or_else(|| panic!("{n} listed"))
    };
    for (name, read_only, destructive) in [
        ("generate_image", false, false),
        ("get_generation_status", true, false),
        ("apply_generated_image", false, true),
        ("cancel_generation", false, false),
    ] {
        let a = get(name).annotations.as_ref().unwrap();
        assert_eq!(a.read_only_hint, Some(read_only), "{name}");
        assert_eq!(a.destructive_hint, Some(destructive), "{name}");
        assert!(super::super::is_read_only(name) == read_only);
    }
    let schema = &get("generate_image").input_schema;
    assert_eq!(schema["required"], json!(["prompt"]));
    for key in [
        "preset",
        "width",
        "height",
        "chipset",
        "colors",
        "dithering",
        "palette",
        "style",
        "background",
        "target",
    ] {
        assert!(schema["properties"].get(key).is_some(), "{key}");
    }
    let text = serde_json::to_string(schema).unwrap();
    for target in ["spare", "picture", "brush", "new_layer", "new_document"] {
        assert!(text.contains(target), "{target}");
    }
}

#[test]
fn the_headless_server_has_no_generator_and_says_where_it_is() {
    let rt = runtime();
    let ctx = ToolCtx {
        exec: spawn_headless(HeadlessWorkspace::default()),
        state: ServerState::new(Settings::headless(FileAccess::off())),
        transport: "test",
    };
    let call = |name: &str, args: Value| call_on(&rt, &ctx, name, args);
    for (name, args) in [
        ("generate_image", json!({"prompt": "a castle"})),
        ("get_generation_status", json!({"job_id": "gen-1"})),
        ("apply_generated_image", json!({"job_id": "gen-1"})),
        ("cancel_generation", json!({"job_id": "gen-1"})),
    ] {
        let r = call(name, args);
        assert_eq!(code_of(&r), crate::generation::UNAVAILABLE, "{name}");
        let msg = r.structured_content.unwrap()["message"].to_string();
        assert!(msg.contains("DeluxePainter window"), "{msg}");
    }
    // Arguments are checked before anything else: strict, and a description is needed.
    assert_eq!(
        code_of(&call("generate_image", json!({"prompt": "x", "size": 3}))),
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        code_of(&call("generate_image", json!({}))),
        "INVALID_ARGUMENT"
    );
    if cfg!(feature = "imagegen") {
        assert_eq!(
            code_of(&call("generate_image", json!({"prompt": "   "}))),
            "INVALID_ARGUMENT"
        );
        let long = "x".repeat(MAX_PROMPT_CHARS + 1);
        assert_eq!(
            code_of(&call("generate_image", json!({"prompt": long}))),
            "INVALID_ARGUMENT"
        );
    }
    // The activity log names the refusals.
    let log = ctx.state.activity().log();
    assert!(
        log.iter()
            .any(|e| e.tool == "generate_image" && e.code.as_deref() == Some(UNAVAILABLE_CODE))
    );
}

const UNAVAILABLE_CODE: &str = crate::generation::UNAVAILABLE;

#[cfg(feature = "imagegen")]
mod with_generator {
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::Duration;

    use dpx_formats::project::ProjectExtras;
    use dpx_imagegen::fixture::TestPattern;
    use dpx_imagegen::{
        CancelToken, Capabilities, GenError, GenerationRequest, ImageGenerationProvider,
        JobManager, ProviderStatus,
    };

    use super::*;
    use crate::generation::{GenApply, GenDefaults, GenHost, GenPlans};

    /// A headless workspace with an image generator, recording what it was asked to apply.
    struct GenWs {
        inner: HeadlessWorkspace,
        host: GenHost,
        applied: Arc<Mutex<Vec<GenApply>>>,
    }

    impl Workspace for GenWs {
        fn kind(&self) -> &'static str {
            "test"
        }
        fn engine(&self) -> &Engine {
            self.inner.engine()
        }
        fn execute(&mut self, request: Request) -> Result<Outcome, ToolError> {
            self.inner.execute(request)
        }
        fn undo(&mut self) -> Result<Outcome, ToolError> {
            self.inner.undo()
        }
        fn redo(&mut self) -> Result<Outcome, ToolError> {
            self.inner.redo()
        }
        fn replace_picture(&mut self, doc: Document, info: PictureInfo) -> Result<(), ToolError> {
            self.inner.replace_picture(doc, info)
        }
        fn shown(&self) -> Slot {
            self.inner.shown()
        }
        fn spare(&self) -> Option<Document> {
            self.inner.spare()
        }
        fn set_spare_page(&mut self, page: IndexedImage) -> Result<(), ToolError> {
            self.inner.set_spare_page(page)
        }
        fn swap_spare(&mut self) -> Result<(), ToolError> {
            self.inner.swap_spare()
        }
        fn set_brush(&mut self, brush: Brush) -> Result<(), ToolError> {
            self.inner.set_brush(brush)
        }
        fn brush_size(&self) -> Option<(u32, u32)> {
            self.inner.brush_size()
        }
        fn picture_name(&self) -> Option<String> {
            self.inner.picture_name()
        }
        fn set_picture_name(&mut self, name: Option<String>) {
            self.inner.set_picture_name(name)
        }
        fn modified(&self) -> bool {
            self.inner.modified()
        }
        fn mark_saved(&mut self, path: &Path, revision: u64) {
            self.inner.mark_saved(path, revision)
        }
        fn project_extras(&self) -> ProjectExtras {
            self.inner.project_extras()
        }
        fn image_generation(&mut self) -> Result<GenHost, ToolError> {
            Ok(self.host.clone())
        }
        fn apply_generated(&mut self, apply: GenApply) -> Result<(), ToolError> {
            // A new layer through the engine, as the GUI's Apply does it.
            if apply.to == crate::generation::ApplyTo::NewLayer {
                self.inner.execute(
                    Command::AddPictureLayer {
                        name: Some("Generated".into()),
                        pixels: apply.result.page.pixels().to_vec(),
                    }
                    .into(),
                )?;
            }
            self.applied.lock().unwrap().push(apply);
            Ok(())
        }
        fn ask(
            &mut self,
            question: Question,
            answer: Box<dyn FnOnce(Result<bool, ToolError>) + Send>,
        ) {
            self.inner.ask(question, answer)
        }
        fn log(&mut self, entry: &str) {
            self.inner.log(entry)
        }
    }

    /// A provider that waits until the test lets it go (a request in flight).
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

    struct G {
        rt: tokio::runtime::Runtime,
        ctx: ToolCtx,
        jobs: JobManager,
        applied: Arc<Mutex<Vec<GenApply>>>,
    }

    fn defaults() -> GenDefaults {
        GenDefaults {
            format: Target::new(Chipset::Ocs, DisplayMode::PAL_LORES),
            page: (320, 256),
            colors: 32,
            convert: dpx_core::import::ConvertOptions {
                framing: dpx_core::import::Framing::Crop,
                scaler: dpx_core::import::Scaler::PixelArt,
                dither: Dither::None,
                ..Default::default()
            },
            style: dpx_imagegen::Style::PixelArt,
        }
    }

    impl G {
        fn new(provider: Option<Arc<dyn ImageGenerationProvider>>) -> G {
            let jobs = JobManager::new();
            let applied = Arc::new(Mutex::new(Vec::new()));
            let ws = GenWs {
                inner: HeadlessWorkspace::default(),
                host: GenHost {
                    jobs: jobs.clone(),
                    provider,
                    model: None,
                    plans: GenPlans::default(),
                    defaults: defaults(),
                },
                applied: applied.clone(),
            };
            let (exec, mut queue) = crate::jobs::channel(|| {});
            std::thread::spawn(move || {
                let mut ws = ws;
                queue.run_until_closed(&mut ws);
            });
            G {
                rt: runtime(),
                ctx: ToolCtx {
                    exec,
                    state: ServerState::new(Settings::headless(FileAccess::off())),
                    transport: "test",
                },
                jobs,
                applied,
            }
        }

        fn call(&self, name: &str, args: Value) -> CallToolResult {
            call_on(&self.rt, &self.ctx, name, args)
        }

        fn ok(&self, name: &str, args: Value) -> Value {
            let r = self.call(name, args);
            let s = r.structured_content.unwrap();
            assert_eq!(s["success"], true, "{name}: {s}");
            s
        }

        fn code(&self, name: &str, args: Value) -> String {
            code_of(&self.call(name, args))
        }

        /// Waits for the job, then asks its status.
        fn done(&self, job: &str) -> CallToolResult {
            let id = dpx_imagegen::JobId(job.strip_prefix("gen-").unwrap().parse().unwrap());
            self.jobs.wait(id, Duration::from_secs(10));
            self.call("get_generation_status", json!({ "job_id": job }))
        }
    }

    fn test_pattern() -> Option<Arc<dyn ImageGenerationProvider>> {
        Some(Arc::new(TestPattern))
    }

    #[test]
    fn without_a_generator_nothing_is_generated() {
        // Acceptance test 6: a clear message, no picture, no crash.
        let g = G::new(None);
        let r = g.call("generate_image", json!({"prompt": "a castle at dusk"}));
        assert_eq!(code_of(&r), crate::generation::NOT_CONFIGURED);
        let msg = r.structured_content.unwrap()["message"].to_string();
        assert!(msg.contains("Prefs > AI & MCP.. > Set up.."), "{msg}");
        assert!(msg.contains("Picture > Generate.."), "{msg}");
        assert!(g.jobs.list().is_empty(), "nothing was sent");
        assert!(g.applied.lock().unwrap().is_empty());
    }

    #[test]
    fn a_320x240_ocs_picture_has_exactly_its_pixels_and_at_most_32_ocs_colours() {
        // Acceptance test 5, with the test pattern in place of a real generator.
        let g = G::new(test_pattern());
        let started = g.ok(
            "generate_image",
            json!({"prompt": "Mittelalterliche Burg bei Sonnenuntergang", "width": 320,
                   "height": 240, "chipset": "OCS", "palette_size": 32,
                   "dithering": "ordered", "target": "new_document"}),
        );
        assert_eq!(started["status"], "running");
        assert_eq!(started["target"], "new_document");
        assert_eq!(
            (started["width"].clone(), started["height"].clone()),
            (json!(320), json!(240))
        );
        assert_eq!(started["chipset"], "ocs");
        assert_eq!(started["colors"], 32);
        assert_eq!(started["dithering"], "ordered");
        assert_eq!(started["provider"]["id"], "test-pattern");
        assert!(started["cost"].as_str().unwrap().contains("no cost"));
        let job = started["job_id"].as_str().unwrap().to_string();
        let r = g.done(&job);
        // The preview is an image block first, then the JSON.
        assert!(r.content[0].as_image().is_some(), "an image block");
        let s = r.structured_content.unwrap();
        assert_eq!(s["status"], "done");
        assert_eq!(
            (s["width"].clone(), s["height"].clone()),
            (json!(320), json!(240))
        );
        assert!(s["colors_used"].as_u64().unwrap() <= 32, "{s}");
        assert_eq!(s["palette_bits"], 4);
        let applied = g.ok("apply_generated_image", json!({ "job_id": job }));
        assert_eq!(applied["document_id"], "page2", "the spare page");
        let list = g.applied.lock().unwrap();
        let a = &list[0];
        assert_eq!(a.to, crate::generation::ApplyTo::Spare);
        assert_eq!(a.format.chipset, Chipset::Ocs);
        assert_eq!(a.format.screen, (320, 240));
        let page = &a.result.page;
        assert_eq!((page.width(), page.height()), (320, 240));
        assert!(a.result.palette.len() <= 32);
        for c in a.result.palette.colors() {
            assert_eq!(Rgb8::from_rgb4(c.to_rgb4()), *c, "{c:?} is a 12-bit colour");
        }
        assert!(
            page.pixels()
                .iter()
                .all(|&p| usize::from(p) < a.result.palette.len())
        );
    }

    #[test]
    fn hardware_limits_and_layer_rules_are_checked_before_anything_is_sent() {
        let g = G::new(test_pattern());
        assert_eq!(
            g.code(
                "generate_image",
                json!({"prompt": "x", "preset": "pal_hires", "chipset": "ocs", "colors": 32})
            ),
            "PALETTE_RANGE_INVALID"
        );
        assert_eq!(
            g.code("generate_image", json!({"prompt": "x", "colors": 12})),
            "PALETTE_RANGE_INVALID"
        );
        // A new layer has the picture's page and palette: other sizes are refused.
        assert_eq!(
            g.code(
                "generate_image",
                json!({"prompt": "x", "target": "new_layer", "width": 320, "height": 240})
            ),
            "INVALID_ARGUMENT"
        );
        assert!(g.jobs.list().is_empty(), "nothing was sent");
        // Defaults: the requester's choices (here OCS PAL Low Res, 32 colours).
        let s = g.ok("generate_image", json!({"prompt": "x", "chipset": "aga"}));
        assert_eq!(s["chipset"], "aga");
        assert_eq!(
            (s["width"].clone(), s["colors"].clone()),
            (json!(320), json!(32))
        );
        let s = g.ok(
            "generate_image",
            json!({"prompt": "x", "target": "new_layer"}),
        );
        assert_eq!(
            (s["width"].clone(), s["height"].clone()),
            (json!(320), json!(256))
        );
        assert_eq!(s["palette"], "current");
    }

    #[test]
    fn new_layer_and_brush_apply_and_the_picture_is_asked_first() {
        let g = G::new(test_pattern());
        let s = g.ok(
            "generate_image",
            json!({"prompt": "a tree", "target": "new_layer", "background": "transparent"}),
        );
        let job = s["job_id"].as_str().unwrap().to_string();
        g.done(&job);
        let a = g.ok("apply_generated_image", json!({ "job_id": job }));
        assert_eq!(a["document_id"], "page1");
        assert!(a["notes"].to_string().contains("transparent"), "{a}");
        let info = g.ok("get_document_info", json!({}));
        assert!(info["can_undo"].as_bool().unwrap());
        {
            let list = g.applied.lock().unwrap();
            let layer = &list[0];
            assert_eq!(layer.to, crate::generation::ApplyTo::NewLayer);
            // Colour 0 only where the picture is transparent (its sky), never elsewhere.
            assert_eq!(layer.result.transparent_color, Some(0));
            assert!(layer.result.page.pixels().contains(&0));
            assert!(layer.result.page.pixels().iter().any(|&p| p != 0));
        }
        // The same picture again, as the brush.
        g.ok(
            "apply_generated_image",
            json!({"job_id": job, "target": "brush"}),
        );
        assert_eq!(
            g.applied.lock().unwrap()[1].to,
            crate::generation::ApplyTo::Brush
        );
        // The picture target asks; a workspace without a user says no.
        assert_eq!(
            g.code(
                "apply_generated_image",
                json!({"job_id": job, "target": "picture"})
            ),
            "APPLY_DECLINED"
        );
        assert_eq!(g.applied.lock().unwrap().len(), 2);
        // Unknown and foreign jobs.
        assert_eq!(
            g.code("get_generation_status", json!({"job_id": "gen-99"})),
            "JOB_NOT_FOUND"
        );
        assert_eq!(
            g.code("apply_generated_image", json!({"job_id": "castle"})),
            "JOB_NOT_FOUND"
        );
        let foreign = g
            .jobs
            .submit(Arc::new(TestPattern), GenerationRequest::new("x", (64, 48)))
            .unwrap();
        assert_eq!(
            g.code(
                "get_generation_status",
                json!({ "job_id": foreign.to_string() })
            ),
            "JOB_NOT_FOUND",
            "the requester's jobs are not an AI client's"
        );
        assert_eq!(
            g.code(
                "cancel_generation",
                json!({ "job_id": foreign.to_string() })
            ),
            "JOB_NOT_FOUND"
        );
    }

    #[test]
    fn cancelling_says_a_sent_request_may_be_billed_and_the_limit_holds() {
        let (tx, rx) = std::sync::mpsc::channel();
        let g = G::new(Some(Arc::new(Gated(Mutex::new(rx)))));
        let first = g.ok("generate_image", json!({"prompt": "one"}));
        let job = first["job_id"].as_str().unwrap().to_string();
        let s = g.ok("get_generation_status", json!({ "job_id": job }));
        assert_eq!(s["status"], "running");
        assert_eq!(
            g.code("apply_generated_image", json!({ "job_id": job })),
            "JOB_NOT_DONE"
        );
        g.ok("generate_image", json!({"prompt": "two"}));
        // Two requests are in flight: a third is refused at once (every one may cost).
        assert_eq!(g.code("generate_image", json!({"prompt": "three"})), "BUSY");
        let c = g.ok("cancel_generation", json!({ "job_id": job }));
        assert_eq!(c["status"], "cancelled");
        assert_eq!(c["cancelled"], true);
        assert!(
            c["notes"].to_string().contains("may still be billed"),
            "{c}"
        );
        let s = g.ok("get_generation_status", json!({ "job_id": job }));
        assert_eq!(s["status"], "cancelled");
        // A cancelled request still counts until it has ended.
        assert_eq!(g.code("generate_image", json!({"prompt": "three"})), "BUSY");
        tx.send(()).unwrap();
        tx.send(()).unwrap();
        for j in g.jobs.list() {
            g.jobs.wait(j.id, Duration::from_secs(10));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while g.jobs.list().iter().any(|j| j.in_flight) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        g.ok("generate_image", json!({"prompt": "three"}));
        let again = g.ok("cancel_generation", json!({ "job_id": job }));
        assert_eq!(again["cancelled"], false, "idempotent");
        drop(tx);
    }
}
