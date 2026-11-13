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
    // Only generate_image reaches beyond DeluxePainter (the generator's service).
    for t in &defs {
        let a = t.annotations.as_ref().unwrap();
        let open = t.name == "generate_image";
        assert_eq!(a.open_world_hint, Some(open), "{}", t.name);
    }
    let text = get("generate_image")
        .description
        .as_deref()
        .unwrap_or_default();
    assert!(
        text.contains("over the internet") && text.contains("billed"),
        "{text}"
    );
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
            G::with_workspace(provider, HeadlessWorkspace::default())
        }

        /// With `inner` as the picture and its spare page.
        fn with_workspace(
            provider: Option<Arc<dyn ImageGenerationProvider>>,
            inner: HeadlessWorkspace,
        ) -> G {
            let jobs = JobManager::new();
            let applied = Arc::new(Mutex::new(Vec::new()));
            let ws = GenWs {
                inner,
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

    /// ADR-46: with the current palette on an OCS target, entries that are not 12-bit are
    /// rounded; status and apply say how many, as the Generate requester's Apply note does.
    #[test]
    fn the_current_palettes_colours_rounded_for_ocs_are_counted() {
        let g = G::new(test_pattern());
        // The headless picture is AGA: two entries that OCS cannot show.
        g.ok("set_palette_color", json!({"index": 1, "color": "#080808"}));
        g.ok("set_palette_color", json!({"index": 3, "color": "#102031"}));
        let started = g.ok(
            "generate_image",
            json!({"prompt": "x", "chipset": "ocs", "palette": "current", "target": "spare"}),
        );
        let job = started["job_id"].as_str().unwrap().to_string();
        let s = g.done(&job).structured_content.unwrap();
        assert_eq!(s["status"], "done", "{s}");
        assert_eq!(s["rounded_colors"], 2, "{s}");
        let notes = s["notes"].to_string();
        assert!(
            notes.contains("2 colours rounded to 12-bit OCS values"),
            "{notes}"
        );
        let applied = g.ok("apply_generated_image", json!({ "job_id": job }));
        assert_eq!(applied["rounded_colors"], 2, "{applied}");
        // A new palette is made in 12-bit colours: nothing is rounded, nothing is said.
        let started = g.ok(
            "generate_image",
            json!({"prompt": "x", "chipset": "ocs", "palette": "new", "target": "spare"}),
        );
        let job = started["job_id"].as_str().unwrap().to_string();
        let s = g.done(&job).structured_content.unwrap();
        assert!(s.get("rounded_colors").is_none(), "{s}");
        assert!(!s["notes"].to_string().contains("rounded"), "{s}");
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
    fn the_picture_question_names_every_frame_and_layer_it_replaces() {
        let one = replace_question(Some("claude-code"), 320, 256, 32, 1, 1);
        assert_eq!(
            one.text,
            "Claude wants to replace the picture with a generated one (320x256, 32 colours). \
             Undo brings the old one back. Replace?"
        );
        let anim = replace_question(Some("claude-code"), 320, 256, 32, 3, 2);
        assert!(
            anim.text
                .starts_with("Claude wants to replace the picture (all 3 frames, 2 layers) "),
            "{}",
            anim.text
        );
        assert!(
            replace_question(None, 8, 8, 2, 1, 4)
                .text
                .contains("the picture (4 layers)")
        );
        assert!(
            replace_question(None, 8, 8, 2, 20, 1)
                .text
                .contains("the picture (all 20 frames)")
        );
        let spare = replace_spare_question(Some("claude-code"), Slot::First, 320, 240, 32);
        assert_eq!(spare.title, "Replace Spare Page?");
        assert!(spare.text.contains("spare page (page1)"), "{}", spare.text);
    }

    #[test]
    fn the_brush_target_reports_the_brush_fitted_to_the_picture_palette() {
        let g = G::new(test_pattern());
        g.ok(
            "create_document",
            json!({"width": 320, "height": 256, "chipset": "ocs", "colors": 4}),
        );
        let picture = g.ok("get_palette", json!({}));
        let colors: Vec<Rgb8> = picture["colors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| parse_color(c.as_str().unwrap()).unwrap())
            .collect();
        assert_eq!(colors.len(), 4);
        let s = g.ok(
            "generate_image",
            json!({"prompt": "a castle", "target": "brush", "chipset": "aga", "colors": 256}),
        );
        assert_eq!(s["colors"], 256, "the conversion has its own palette");
        let job = s["job_id"].as_str().unwrap().to_string();
        let r = g.done(&job);
        let image = r.content[0].as_image().expect("a preview").data.clone();
        let st = r.structured_content.unwrap();
        assert_eq!(st["status"], "done");
        assert!(st["colors_used"].as_u64().unwrap() <= 4, "{st}");
        assert_eq!(st["palette_size"], 4, "{st}");
        assert_eq!(st["palette_bits"], 4, "the picture is OCS: {st}");
        assert_eq!(st["chipset"], "ocs", "{st}");
        assert!(
            st["notes"].to_string().contains("picture's palette"),
            "{st}"
        );
        // The preview shows the brush as it lands: only the picture's colours.
        let bytes = view::from_base64(&image).unwrap();
        let (png, _, _) = crate::pictures::decode_rgba("preview", &bytes).unwrap();
        assert!(
            png.pixels()
                .iter()
                .all(|p| colors.contains(&Rgb8::new(p.r, p.g, p.b))),
            "every preview pixel is a picture colour"
        );
        let a = g.ok(
            "apply_generated_image",
            json!({"job_id": job, "target": "brush"}),
        );
        assert!(a["colors_used"].as_u64().unwrap() <= 4, "{a}");
    }

    #[test]
    fn a_spare_page_with_a_picture_is_asked_about_and_a_blank_one_is_not() {
        let g = G::new(test_pattern());
        let job = g.ok("generate_image", json!({"prompt": "a castle"}))["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        g.done(&job);
        // No spare page yet: no question (acceptance test 5's way).
        g.ok("apply_generated_image", json!({ "job_id": job }));
        assert_eq!(g.applied.lock().unwrap().len(), 1);
        // The user's picture is the spare page after a swap: asked first (nobody answers
        // here, so it is kept).
        g.ok("draw_pixel", json!({"x": 1, "y": 1, "color": 2}));
        g.ok("select_document", json!({"document_id": "page2"}));
        let r = g.call("apply_generated_image", json!({ "job_id": job }));
        assert_eq!(code_of(&r), "APPLY_DECLINED");
        let msg = r.structured_content.unwrap()["message"].to_string();
        assert!(msg.contains("kept the spare page"), "{msg}");
        assert_eq!(g.applied.lock().unwrap().len(), 1, "not applied");
        // The brush needs no question.
        g.ok(
            "apply_generated_image",
            json!({"job_id": job, "target": "brush"}),
        );
        assert_eq!(g.applied.lock().unwrap().len(), 2);
    }

    #[test]
    fn cancel_reports_a_kept_picture_as_done_after_the_job_manager_dropped_it() {
        let g = G::new(test_pattern());
        let start = |prompt: &str| {
            g.ok("generate_image", json!({ "prompt": prompt }))["job_id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let fetched = start("one");
        let st = g.done(&fetched).structured_content.unwrap();
        assert_eq!(
            st["status"], "done",
            "its picture is kept with the plan now"
        );
        let never_fetched = start("two");
        g.jobs.wait(job_of(&never_fetched), Duration::from_secs(10));
        for n in 0..JobManager::KEEP_FINISHED {
            let id = start(&format!("more {n}"));
            g.jobs.wait(job_of(&id), Duration::from_secs(10));
        }
        assert!(g.jobs.status(job_of(&fetched)).is_none(), "dropped");
        assert!(g.jobs.status(job_of(&never_fetched)).is_none(), "dropped");
        let c = g.ok("cancel_generation", json!({ "job_id": fetched }));
        assert_eq!(
            (c["status"].clone(), c["cancelled"].clone()),
            (json!("done"), json!(false))
        );
        assert!(c["notes"].to_string().contains("not running (done)"), "{c}");
        assert_eq!(
            g.ok("get_generation_status", json!({ "job_id": fetched }))["status"],
            "done"
        );
        g.ok("apply_generated_image", json!({ "job_id": fetched }));
        // Never fetched and dropped: no longer kept, for both tools alike.
        for tool in ["cancel_generation", "get_generation_status"] {
            assert_eq!(
                g.code(tool, json!({ "job_id": never_fetched })),
                "JOB_NOT_FOUND",
                "{tool}"
            );
        }
    }

    fn job_of(text: &str) -> dpx_imagegen::JobId {
        dpx_imagegen::JobId(text.strip_prefix("gen-").unwrap().parse().unwrap())
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

    /// The status preview as a PNG of RGBA pixels and its colour type's channel count.
    fn preview_of(r: &CallToolResult) -> dpx_core::RgbaImage {
        let image = r.content[0].as_image().expect("a preview").data.clone();
        let bytes = view::from_base64(&image).unwrap();
        crate::pictures::decode_rgba("preview", &bytes).unwrap().0
    }

    /// Wave-8 integration review: one more layer must also fit the memory of every frame's
    /// pictures (the rule `create_layer` meets); a new layer past it is refused before
    /// anything is sent (and billed), and so is Apply of a job made for another target.
    #[test]
    fn a_new_layer_past_the_picture_memory_is_refused_before_anything_is_sent() {
        let g = G::new(test_pattern());
        let job = g.ok("generate_image", json!({"prompt": "x", "target": "brush"}))["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        g.done(&job);
        // 320x256 pages: 820 frames of one layer fit, of two layers not.
        g.ok(
            "execute_command",
            json!({"command": {"op": "set_frame_count", "count": 820}}),
        );
        let refused = g.call("create_layer", json!({}));
        assert_eq!(code_of(&refused), "INVALID_ARGUMENT");
        let sent = g.jobs.list().len();
        let r = g.call(
            "generate_image",
            json!({"prompt": "a tree", "target": "new_layer"}),
        );
        assert_eq!(code_of(&r), "LAYER_LIMIT");
        let msg = r.structured_content.unwrap()["message"].to_string();
        assert!(msg.contains("820 frames of 2 layers"), "{msg}");
        assert_eq!(g.jobs.list().len(), sent, "nothing was sent");
        assert_eq!(
            g.code(
                "apply_generated_image",
                json!({"job_id": job, "target": "new_layer"})
            ),
            "LAYER_LIMIT"
        );
        assert!(g.applied.lock().unwrap().is_empty(), "nothing applied");
    }

    /// Wave-8 integration review: a new layer with a transparent background is previewed as
    /// it lands: its colour 0 is left transparent (alpha 0), as `get_preview` shows a layer
    /// above the bottom one, and the note says so.
    #[test]
    fn a_new_layers_preview_leaves_colour_0_transparent() {
        let g = G::new(test_pattern());
        g.ok("fill_area", json!({"x": 0, "y": 0, "color": 5}));
        let job = g.ok(
            "generate_image",
            json!({"prompt": "a tree", "target": "new_layer", "background": "transparent"}),
        )["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let r = g.done(&job);
        let png = preview_of(&r);
        let st = r.structured_content.unwrap();
        let scale = st["preview_scale"].as_u64().unwrap() as usize;
        assert!(
            st["notes"].to_string().contains("transparent (alpha 0)"),
            "{st}"
        );
        g.ok("apply_generated_image", json!({ "job_id": job }));
        let layer = g.applied.lock().unwrap()[0].result.page.clone();
        let clear = layer.pixels().iter().filter(|&&p| p == 0).count();
        assert!(clear > 0, "the test pattern's sky is transparent");
        let transparent = png.pixels().iter().filter(|p| p.a == 0).count();
        assert_eq!(
            transparent,
            clear * scale * scale,
            "colour 0 is see-through"
        );
        assert!(png.pixels().iter().all(|p| p.a == 0 || p.a == 255));
        // Where the layer is clear the picture shows the layer below (colour 5).
        let at = layer.pixels().iter().position(|&p| p == 0).unwrap() as i32;
        let (x, y) = (at % layer.width() as i32, at / layer.width() as i32);
        let px = g.ok(
            "get_pixels",
            json!({"rect": {"x": x, "y": y, "w": 1, "h": 1}}),
        );
        assert_eq!(px["pixels"], json!([[5]]), "{px}");
        // The brush and the other targets keep opaque previews.
        let job = g.ok(
            "generate_image",
            json!({"prompt": "a tree", "target": "brush"}),
        )["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let png = preview_of(&g.done(&job));
        assert!(png.pixels().iter().all(|p| p.a == 255));
    }

    const OLD_OCS: &[u8] = include_bytes!("../../dpx-formats/tests/data/v1-ocs-not-12-bit.dpxproj");

    /// Wave-8 integration review (ADR-46): the brush and a new layer keep an older OCS
    /// project's palette as it is stored, so their results say which entries are not 12-bit
    /// (as open_picture does) instead of claiming every entry is, report 8 palette bits, and
    /// the picture is matched against the stored colours: the new layer's preview has only
    /// the picture's colours and the brush needs no colour fitted. The spare page takes the
    /// result's rounded palette and keeps the 12-bit note.
    #[test]
    fn the_brush_and_a_new_layer_keep_an_older_ocs_palette_and_say_so() {
        let doc = dpx_formats::project::read_project(OLD_OCS)
            .unwrap()
            .document;
        let stored = doc.palette().clone();
        assert!(dpx_core::hardware::first_unshowable(Chipset::Ocs, stored.colors()).is_some());
        let g = G::with_workspace(test_pattern(), HeadlessWorkspace::new(doc));
        for target in ["new_layer", "brush"] {
            let job = g.ok(
                "generate_image",
                json!({"prompt": "x", "target": target, "palette": "current"}),
            )["job_id"]
                .as_str()
                .unwrap()
                .to_string();
            let r = g.done(&job);
            let png = preview_of(&r);
            let st = r.structured_content.unwrap();
            let applied = g.ok("apply_generated_image", json!({ "job_id": job }));
            for (what, v) in [("status", &st), ("apply", &applied)] {
                let notes = v["notes"].to_string();
                assert!(
                    !notes.contains("every palette entry is a 12-bit colour"),
                    "{target} {what}: {notes}"
                );
                assert!(
                    notes.contains("palette entries 9-27, 29, 30 are not 12-bit OCS colours"),
                    "{target} {what}: {notes}"
                );
                assert!(notes.contains("set_palette_color"), "{target} {what}");
                assert!(v.get("rounded_colors").is_none(), "{target} {what}: {v}");
            }
            assert_eq!(st["palette_bits"], 8, "{target}: {st}");
            // Matched against the stored colours: nothing to fit, nothing foreign shown.
            assert!(
                png.pixels()
                    .iter()
                    .filter(|p| p.a == 255)
                    .all(|p| stored.colors().contains(&Rgb8::new(p.r, p.g, p.b))),
                "{target}: every preview pixel is a stored colour"
            );
            if target == "brush" {
                assert!(
                    st["notes"]
                        .to_string()
                        .contains("0 of its colours had to change"),
                    "{st}"
                );
            }
        }
        let list = g.applied.lock().unwrap();
        assert_eq!(
            list[0].result.palette, stored,
            "the layer's palette as stored"
        );
        assert_eq!(list[0].result.rounded_colors, 0);
        assert_eq!(
            list[1].result.palette, stored,
            "the brush's own palette too"
        );
        drop(list);
        // The spare page takes the result's palette, rounded to 12 bits.
        let job = g.ok(
            "generate_image",
            json!({"prompt": "x", "target": "spare", "palette": "current"}),
        )["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let st = g.done(&job).structured_content.unwrap();
        assert_eq!(st["palette_bits"], 4, "{st}");
        assert_eq!(st["rounded_colors"], 21, "{st}");
        assert!(
            st["notes"]
                .to_string()
                .contains("every palette entry is a 12-bit colour"),
            "{st}"
        );
    }
}
