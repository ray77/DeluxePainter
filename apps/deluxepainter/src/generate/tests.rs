//! ui-loop tests of Picture ▸ Generate.. and the Image Generator setup (ADR-45): real egui
//! events through `PainterApp::ui`; generations use the deterministic test pattern or test
//! providers (no network, no key store).

use std::io::{Read, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use dpx_core::{Chipset, DisplayMode, IndexedImage, Rgb8};
use dpx_imagegen::fixture::TestPattern;
use dpx_imagegen::{CancelToken, Capabilities};
use dpx_ui::{GenControl, GenRow, SetupControl, SetupField};

use super::*;
use crate::tests::{click_ui, key_event, menu_item_path, pointer_button, tap, ui_pass, ui_point};

/// The test pattern, counting its calls.
#[derive(Default)]
struct Counting {
    calls: AtomicUsize,
}

impl ImageGenerationProvider for Counting {
    fn id(&self) -> &str {
        "counting"
    }
    fn display_name(&self) -> &str {
        "Counting test pattern"
    }
    fn needs_network(&self) -> bool {
        false
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        TestPattern.generate(request, cancel)
    }
}

/// A "network" provider that waits until the test lets it go, then answers.
struct Gated {
    gate: Mutex<std::sync::mpsc::Receiver<()>>,
    answer: Result<(), GenError>,
}

impl Gated {
    fn new(answer: Result<(), GenError>) -> (Arc<Self>, Sender<()>) {
        let (tx, rx) = channel();
        (
            Arc::new(Self {
                gate: Mutex::new(rx),
                answer,
            }),
            tx,
        )
    }
}

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
        let _ = self
            .gate
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        self.answer.clone()?;
        TestPattern
            .generate(request, &CancelToken::new())
            .and_then(|i| cancel.check().map(|_| i))
    }
}

fn app_with(provider: Option<Arc<dyn ImageGenerationProvider>>) -> (PainterApp, egui::Context) {
    let mut app = PainterApp::new();
    app.imagegen.debug_provider = provider;
    let ctx = egui::Context::default();
    // The first pass learns the window, so pointer positions land where they are drawn.
    ui_pass(&mut app, &ctx, 0.0, vec![]);
    (app, ctx)
}

fn req(app: &PainterApp) -> &GenerateRequester {
    app.ui
        .generate_requester
        .as_ref()
        .expect("Generate requester open")
}

/// A click on a Generate requester control through `ui()`.
fn click(app: &mut PainterApp, ctx: &egui::Context, t: f64, c: GenControl) {
    let r = req(app);
    let (ox, oy) = r.origin();
    let rect = r
        .controls()
        .into_iter()
        .find(|&(k, _)| k == c)
        .unwrap_or_else(|| panic!("{c:?} shown"))
        .1;
    let at = (ox + rect.x + rect.w / 2, oy + rect.y + rect.h / 2);
    click_ui(app, ctx, t, at, egui::PointerButton::Primary);
}

/// A click on a setup requester control through `ui()`.
fn click_setup(app: &mut PainterApp, ctx: &egui::Context, t: f64, c: SetupControl) {
    let r = app.ui.imagegen_setup.as_ref().expect("setup open");
    let (ox, oy) = r.origin();
    let rect = r.controls().into_iter().find(|&(k, _)| k == c).unwrap().1;
    let at = (ox + rect.x + rect.w / 2, oy + rect.y + rect.h / 2);
    click_ui(app, ctx, t, at, egui::PointerButton::Primary);
}

fn type_text(app: &mut PainterApp, ctx: &egui::Context, t: f64, text: &str) {
    ui_pass(app, ctx, t, vec![egui::Event::Text(text.into())]);
}

/// Picture ▸ Generate.. with the right mouse button through `ui()` (as `pick` does, in
/// passes from `t` to `t + 0.005`).
fn open_by_menu(app: &mut PainterApp, ctx: &egui::Context, t: f64) {
    use egui::PointerButton::Secondary;
    let [title, item] = menu_item_path(app, "picture.generate").map(|p| ui_point(app, p));
    ui_pass(
        app,
        ctx,
        t + 0.001,
        vec![
            egui::Event::PointerMoved(title),
            pointer_button(title, Secondary, true),
        ],
    );
    assert!(app.ui.menu.active, "the menu strip");
    ui_pass(app, ctx, t + 0.002, vec![egui::Event::PointerMoved(item)]);
    ui_pass(
        app,
        ctx,
        t + 0.003,
        vec![pointer_button(item, Secondary, false)],
    );
    assert!(!app.ui.menu.active);
    assert!(app.ui.generate_requester.is_some(), "Generate.. opened it");
}

/// Runs passes until the job has ended (the test providers answer at once or when let go).
fn settle(app: &mut PainterApp, ctx: &egui::Context, mut t: f64) -> f64 {
    for _ in 0..1000 {
        t += 0.01;
        ui_pass(app, ctx, t, vec![]);
        if !matches!(req(app).phase(), GenPhase::Working { .. }) {
            return t;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("the job did not end");
}

/// Opens the requester, types `prompt`, picks `target` and generates.
fn generate_for(app: &mut PainterApp, ctx: &egui::Context, prompt: &str, target: GenTarget) -> f64 {
    open_by_menu(app, ctx, 0.0);
    type_text(app, ctx, 0.2, prompt);
    click(app, ctx, 0.3, GenControl::More);
    let mut t = 0.4;
    while req(app).options().target != target {
        click(app, ctx, t, GenControl::Next(GenRow::Target));
        t += 0.1;
    }
    click(app, ctx, t, GenControl::Generate);
    let t = settle(app, ctx, t + 0.1);
    assert_eq!(req(app).phase(), GenPhase::Result);
    assert!(req(app).thumbnail().is_some());
    t
}

/// What the requester's result is: the test pattern for `prompt` (generated with 32 colours
/// asked for), converted here for the requester's current options.
fn expected(app: &PainterApp, prompt: &str) -> Imported {
    let o = req(app).options();
    let target = o.target();
    let mut request = GenerationRequest::for_page(
        style_of(o.style).prompt(prompt, 32),
        &TestPattern.capabilities(),
        target.screen,
        target.mode.pixel_shape(),
    );
    request.transparent_background = o.transparent;
    let img = TestPattern
        .generate(&request, &CancelToken::new())
        .unwrap()
        .remove(0);
    let doc = app.engine.document();
    import::convert(
        &img,
        &convert_options(&o),
        &ConvertTarget {
            target,
            page: target.screen,
            palette: doc.palette(),
            bg: app.ui.bg,
            transparent: files::TRANSPARENT_PINK,
        },
    )
    .unwrap()
}

fn ocs_320x240() -> Target {
    Target::new(Chipset::Ocs, DisplayMode::PAL_LORES).with_screen(320, 240)
}

#[test]
fn the_menu_opens_it_and_typing_and_pasting_fill_the_description() {
    let (mut app, ctx) = app_with(None);
    let before = app.engine.document().clone();
    open_by_menu(&mut app, &ctx, 0.0);
    assert!(app.modal_open());
    type_text(&mut app, &ctx, 0.1, "Burg");
    ui_pass(
        &mut app,
        &ctx,
        0.2,
        vec![egui::Event::Paste(" im Abendlicht\nmit Türmen".into())],
    );
    assert_eq!(req(&app).prompt(), "Burg im Abendlicht mit Türmen");
    // Letters and the paste are no paint-screen shortcuts: nothing else changed.
    assert_eq!(app.engine.document(), &before);
    // Esc closes it; the next opening remembers the description and the choices.
    click(&mut app, &ctx, 0.3, GenControl::Next(GenRow::Chipset));
    ui_pass(
        &mut app,
        &ctx,
        0.4,
        vec![key_event(egui::Key::Escape, true)],
    );
    assert!(app.ui.generate_requester.is_none());
    assert_eq!(app.engine.document(), &before);
    open_by_menu(&mut app, &ctx, 0.6);
    assert_eq!(req(&app).prompt(), "Burg im Abendlicht mit Türmen");
    assert_eq!(req(&app).options().chipset, Chipset::Aga);
    click(&mut app, &ctx, 0.8, GenControl::Cancel);
    assert!(app.ui.generate_requester.is_none());
}

#[test]
fn without_a_generator_it_says_so_and_set_up_opens_the_setup() {
    let (mut app, ctx) = app_with(None);
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "A castle");
    let r = req(&app);
    assert!(!r.provider().ready);
    assert_eq!(r.provider().summary, "No image generator is set up.");
    assert!(r.provider().note.starts_with(NO_GENERATOR));
    assert!(!r.enabled(GenControl::Generate));
    // Neither the button nor Return starts anything; nothing is faked.
    click(&mut app, &ctx, 0.2, GenControl::Generate);
    ui_pass(&mut app, &ctx, 0.3, vec![key_event(egui::Key::Enter, true)]);
    assert_eq!(req(&app).phase(), GenPhase::Ask);
    assert!(app.imagegen.jobs.list().is_empty());
    // OpenAI chosen without a key: still not set up, and it says why.
    app.imagegen.settings.provider = GenProvider::OpenAi;
    click(&mut app, &ctx, 0.4, GenControl::SetUp);
    assert!(app.ui.imagegen_setup.is_some());
    click_setup(&mut app, &ctx, 0.5, SetupControl::Cancel);
    let r = req(&app);
    assert!(!r.provider().ready);
    assert_eq!(
        r.provider().summary,
        "OpenAI Images is not set up: no OpenAI API key."
    );
    assert!(app.imagegen.jobs.list().is_empty());
}

#[test]
fn the_setup_takes_a_pasted_key_and_the_model_and_cancel_changes_nothing() {
    let (mut app, ctx) = app_with(None);
    app.imagegen.keys = keychain_keys();
    app.open_imagegen_setup();
    assert!(app.modal_open());
    // Cancel after typing: nothing stored, nothing chosen.
    click_setup(
        &mut app,
        &ctx,
        0.1,
        SetupControl::Provider(SetupProvider::OpenAi),
    );
    ui_pass(
        &mut app,
        &ctx,
        0.2,
        vec![egui::Event::Paste("sk-test-1".into())],
    );
    click_setup(&mut app, &ctx, 0.3, SetupControl::Cancel);
    assert!(app.ui.imagegen_setup.is_none());
    assert_eq!(app.imagegen.settings.provider, GenProvider::None);
    assert!(app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).is_none());
    // Ok: the key goes to the key store, provider and model to the settings.
    app.open_imagegen_setup();
    click_setup(
        &mut app,
        &ctx,
        0.4,
        SetupControl::Provider(SetupProvider::OpenAi),
    );
    ui_pass(
        &mut app,
        &ctx,
        0.5,
        vec![egui::Event::Paste("  sk-test-2\n".into())],
    );
    click_setup(&mut app, &ctx, 0.6, SetupControl::Field(SetupField::Model));
    for i in 0..5 {
        ui_pass(
            &mut app,
            &ctx,
            0.7 + f64::from(i) * 0.01,
            vec![key_event(egui::Key::Backspace, true)],
        );
    }
    type_text(&mut app, &ctx, 0.8, "sunburst");
    ui_pass(&mut app, &ctx, 0.9, vec![key_event(egui::Key::Enter, true)]);
    assert!(app.ui.imagegen_setup.is_none());
    assert_eq!(app.imagegen.settings.provider, GenProvider::OpenAi);
    assert_eq!(app.imagegen.settings.openai.model, "gpt-image-2.6-sunburst");
    let (key, _) = app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).unwrap();
    assert_eq!(key.expose(), "sk-test-2");
    // The key never reaches the settings file.
    let json = crate::config::Config::capture(&app).to_json().to_string();
    assert!(!json.contains("sk-test"), "{json}");
    assert!(json.contains("gpt-image-2.6-sunburst"));
    // OpenAI is now set up: Generate offers it (nothing is sent until Generate).
    app.open_generate();
    assert!(req(&app).provider().ready);
    assert_eq!(
        req(&app).provider().summary,
        "OpenAI Images: billed per picture"
    );
    assert!(req(&app).provider().note.contains("gpt-image-2.6-sunburst"));
    app.close_generate();
    // Remove key waits for Ok, then forgets the key.
    app.open_imagegen_setup();
    click_setup(&mut app, &ctx, 1.0, SetupControl::RemoveKey);
    assert!(app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).is_some());
    click_setup(&mut app, &ctx, 1.1, SetupControl::Ok);
    assert!(app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).is_none());
    // A bad model id keeps the requester open with the reason.
    app.open_imagegen_setup();
    click_setup(&mut app, &ctx, 1.2, SetupControl::Field(SetupField::Model));
    type_text(&mut app, &ctx, 1.3, " two words");
    click_setup(&mut app, &ctx, 1.4, SetupControl::Ok);
    let setup = app.ui.imagegen_setup.as_ref().expect("still open");
    assert!(setup.test().unwrap().contains("model id"));
}

#[test]
fn get_key_opens_openais_key_page() {
    let (mut app, ctx) = app_with(None);
    app.imagegen.settings.provider = GenProvider::OpenAi;
    app.open_imagegen_setup();
    click_setup(&mut app, &ctx, 0.1, SetupControl::GetKey);
    assert_eq!(app.imagegen.opened_url.as_deref(), Some(openai::KEYS_PAGE));
    assert!(app.ui.imagegen_setup.is_some(), "the setup stays open");
}

/// Serves one model list on 127.0.0.1; returns the URL and the request it got.
fn serve_models() -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = [0u8; 4096];
        let n = s.read(&mut buf).unwrap_or(0);
        let body = r#"{"object":"list","data":[{"id":"gpt-image-2.6-flare","object":"model"}]}"#;
        let answer = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(answer.as_bytes()).unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    (url, handle)
}

#[test]
fn test_checks_the_typed_key_with_the_model_list_on_a_worker() {
    let (mut app, ctx) = app_with(None);
    let (url, server) = serve_models();
    app.imagegen.models_endpoint = url;
    app.imagegen.settings.provider = GenProvider::OpenAi;
    app.open_imagegen_setup();
    // Nothing to test yet.
    assert!(
        !app.ui
            .imagegen_setup
            .as_ref()
            .unwrap()
            .enabled(SetupControl::Test)
    );
    ui_pass(
        &mut app,
        &ctx,
        0.1,
        vec![egui::Event::Paste("sk-typed-9".into())],
    );
    click_setup(&mut app, &ctx, 0.2, SetupControl::Test);
    let mut t = 0.3;
    loop {
        ui_pass(&mut app, &ctx, t, vec![]);
        let text = app.ui.imagegen_setup.as_ref().unwrap().test().unwrap_or("");
        if !text.starts_with("Testing") {
            assert_eq!(text, "The key works. gpt-image-2.6-flare is available.");
            break;
        }
        assert!(t < 10.0, "no answer");
        t += 0.01;
        std::thread::sleep(Duration::from_millis(5));
    }
    let request = server.join().unwrap().to_ascii_lowercase();
    assert!(request.starts_with("get /v1/models"), "{request}");
    assert!(request.contains("authorization: bearer sk-typed-9"));
    // Testing stored nothing.
    assert!(app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).is_none());
}

#[test]
fn a_generated_picture_goes_to_the_spare_page_and_the_shown_page_stays() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let before = app.engine.document().clone();
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Spare);
    let want = expected(&app, "A castle");
    // The thumbnail is exactly the converted page made small.
    let (w, h) = req(&app).thumb_size();
    assert_eq!(
        req(&app).thumbnail(),
        Some(&import::thumbnail(&want.page, &want.palette, w, h))
    );
    assert_eq!(
        req(&app).message(),
        Some(apply_text(GenTarget::Spare, 1, 0).as_str())
    );
    assert_eq!(app.engine.document(), &before, "nothing before Apply");
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert!(app.ui.generate_requester.is_none());
    assert_eq!(
        app.engine.document(),
        &before,
        "the shown page is untouched"
    );
    let spare = app.menu_data.spare.as_ref().unwrap().engine.document();
    assert_eq!(spare.target(), ocs_320x240());
    assert_eq!(spare.page(), &want.page);
    assert_eq!(spare.palette(), &want.palette);
    assert_eq!(spare.palette().len(), 32);
    assert!(app.spare_differs());
    assert!(app.light_table_spare().is_none(), "other colour numbers");
    // j shows it with its own format; j again brings the work back unchanged.
    ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::J));
    assert_eq!(app.engine.document().target(), ocs_320x240());
    assert_eq!(app.engine.document().page(), &want.page);
    assert_eq!(app.view().ui_size(), (320, 240));
    ui_pass(&mut app, &ctx, t + 0.4, tap(egui::Key::J));
    assert_eq!(app.engine.document(), &before);
    assert_eq!(
        app.menu_data
            .spare
            .as_ref()
            .unwrap()
            .engine
            .document()
            .page(),
        &want.page,
        "the picture stays on the spare page"
    );
    // Undo on the spare page brings back what was there (a blank page of the old format).
    ui_pass(&mut app, &ctx, t + 0.5, tap(egui::Key::J));
    ui_pass(&mut app, &ctx, t + 0.6, tap(egui::Key::U));
    assert_eq!(app.engine.document().target(), before.target());
    assert!(
        app.engine
            .document()
            .page()
            .pixels()
            .iter()
            .all(|&p| p == 0)
    );
}

#[test]
fn merge_copy_and_delete_respect_a_spare_page_of_its_own_format() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let before = app.engine.document().clone();
    let t = generate_for(&mut app, &ctx, "x", GenTarget::Spare);
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    // Merging colour numbers of another palette is refused, with a note.
    app.merge_spare(true);
    assert_eq!(app.engine.document(), &before);
    assert!(app.ui.note_requester.is_some());
    app.ui.note_requester = None;
    // Delete This Page shows the spare's picture in one undo step.
    let spare_page = app
        .menu_data
        .spare
        .as_ref()
        .unwrap()
        .engine
        .document()
        .page()
        .clone();
    app.delete_page();
    assert_eq!(app.engine.document().target(), ocs_320x240());
    assert_eq!(app.engine.document().page(), &spare_page);
    assert!(app.menu_data.spare.is_none() && app.menu_data.spare_own.is_none());
    app.undo();
    assert_eq!(app.engine.document().page(), before.page());
    assert_eq!(app.engine.document().target(), before.target());
    // Copy to Spare makes the spare an ordinary copy again.
    app.open_generate();
    app.start_generation();
    settle(&mut app, &ctx, t + 1.0);
    app.generate_apply();
    assert!(app.spare_differs());
    app.copy_to_spare();
    assert!(!app.spare_differs());
    assert!(app.menu_data.spare_own.is_none());
    let spare = app.menu_data.spare.as_ref().unwrap().engine.document();
    assert_eq!(spare.target(), before.target());
    assert_eq!(spare.page(), before.page());
}

#[test]
fn apply_to_the_picture_is_one_undo_step() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let before = app.engine.document().clone();
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Picture);
    let want = expected(&app, "A castle");
    assert_eq!(
        req(&app).message(),
        Some(apply_text(GenTarget::Picture, 1, 0).as_str())
    );
    // Return applies once there is a result.
    ui_pass(
        &mut app,
        &ctx,
        t + 0.1,
        vec![key_event(egui::Key::Enter, true)],
    );
    assert!(app.ui.generate_requester.is_none());
    let doc = app.engine.document();
    assert_eq!(doc.target(), ocs_320x240());
    assert_eq!((doc.page().width(), doc.page().height()), (320, 240));
    assert_eq!(doc.page(), &want.page);
    assert_eq!(doc.palette().len(), 32);
    for c in doc.palette().colors() {
        assert_eq!(Rgb8::from_rgb4(c.to_rgb4()), *c, "12-bit OCS colour");
    }
    assert!(doc.page().pixels().iter().all(|&p| p < 32));
    assert_eq!(app.view().ui_size(), (320, 240));
    // One undo brings back the old picture, format and palette; redo replaces again.
    ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::U));
    let doc = app.engine.document();
    assert_eq!(doc.target(), before.target());
    assert_eq!(doc.palette(), before.palette());
    assert_eq!(doc.page(), before.page());
    app.redo();
    assert_eq!(app.engine.document().page(), &want.page);
}

#[test]
fn apply_to_the_brush_fits_the_picture_palette() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let before = app.engine.document().clone();
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Brush);
    let want = expected(&app, "A castle");
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert_eq!(app.engine.document(), &before, "the picture is untouched");
    let brush = app.custom_brush.as_ref().expect("a custom brush");
    assert_eq!((brush.width, brush.height), (320, 240));
    assert_eq!(
        brush.transparent, None,
        "opaque background: no transparent colour"
    );
    let palette = before.palette();
    for (i, &p) in brush.pixels.iter().enumerate() {
        let own = want.palette.get(want.page.pixels()[i]).unwrap();
        assert_eq!(
            palette.get(p),
            palette.get(palette.nearest(own)),
            "pixel {i}"
        );
    }
    // Its own palette stays for Use Brush Palette, like a loaded brush file.
    let loaded = app.menu_data.palette_stencil.loaded.as_ref().unwrap();
    assert_eq!(loaded.palette, want.palette);
    assert_eq!(app.picked_brush.as_ref(), Some(brush));
}

#[test]
fn a_transparent_background_gives_a_brush_with_a_transparent_colour() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "A sprite");
    click(&mut app, &ctx, 0.2, GenControl::More);
    click(&mut app, &ctx, 0.3, GenControl::Next(GenRow::Background));
    for i in 0..2 {
        click(
            &mut app,
            &ctx,
            0.4 + f64::from(i) * 0.1,
            GenControl::Next(GenRow::Target),
        );
    }
    assert_eq!(req(&app).options().target, GenTarget::Brush);
    click(&mut app, &ctx, 0.7, GenControl::Generate);
    let t = settle(&mut app, &ctx, 0.8);
    click(&mut app, &ctx, t, GenControl::Apply);
    let brush = app.custom_brush.as_ref().unwrap();
    assert_eq!(brush.transparent, Some(0));
    assert!(brush.pixels.contains(&0), "the clear sky is transparent");
}

#[test]
fn option_changes_convert_again_without_generating_again() {
    let counting = Arc::new(Counting::default());
    let (mut app, ctx) = app_with(Some(counting.clone()));
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Spare);
    assert_eq!(counting.calls.load(Ordering::SeqCst), 1);
    let first = req(&app).thumbnail().cloned();
    click(&mut app, &ctx, t + 0.1, GenControl::Prev(GenRow::Colors));
    click(&mut app, &ctx, t + 0.2, GenControl::Next(GenRow::Dither));
    assert_eq!(req(&app).options().colors, 16);
    assert_ne!(req(&app).thumbnail().cloned(), first);
    let want = expected(&app, "A castle");
    assert_eq!(app.imagegen.result.as_ref().unwrap().1, want);
    assert_eq!(want.palette.len(), 16);
    assert_eq!(
        counting.calls.load(Ordering::SeqCst),
        1,
        "no new generation"
    );
    // Again generates a new one.
    click(&mut app, &ctx, t + 0.3, GenControl::Generate);
    settle(&mut app, &ctx, t + 0.4);
    assert_eq!(counting.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn cancel_while_generating_stops_waiting_and_the_late_picture_is_dropped() {
    let (gated, gate) = Gated::new(Ok(()));
    let (mut app, ctx) = app_with(Some(gated));
    let before = app.engine.document().clone();
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "A castle");
    ui_pass(&mut app, &ctx, 0.2, vec![key_event(egui::Key::Enter, true)]);
    assert!(matches!(req(&app).phase(), GenPhase::Working { .. }));
    assert!(!req(&app).enabled(GenControl::Generate));
    // The first Cancel stops waiting; the requester stays open.
    click(&mut app, &ctx, 0.3, GenControl::Cancel);
    assert_eq!(req(&app).phase(), GenPhase::Ask);
    assert_eq!(
        req(&app).error(),
        Some("Stopped waiting. A request already sent may still be billed.")
    );
    gate.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    ui_pass(&mut app, &ctx, 0.5, vec![]);
    assert_eq!(
        req(&app).phase(),
        GenPhase::Ask,
        "the late answer is dropped"
    );
    assert!(app.imagegen.result.is_none());
    // The second Cancel closes it; nothing changed.
    click(&mut app, &ctx, 0.6, GenControl::Cancel);
    assert!(app.ui.generate_requester.is_none());
    assert_eq!(app.engine.document(), &before);
    assert!(app.menu_data.spare.is_none());
}

#[test]
fn a_failed_generation_says_why_and_the_editor_goes_on() {
    let refused = GenError::Auth {
        status: 401,
        message: "Incorrect API key".into(),
    };
    let (gated, gate) = Gated::new(Err(refused));
    let (mut app, ctx) = app_with(Some(gated));
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "A castle");
    click(&mut app, &ctx, 0.2, GenControl::Generate);
    gate.send(()).unwrap();
    settle(&mut app, &ctx, 0.3);
    let r = req(&app);
    assert_eq!(r.phase(), GenPhase::Ask);
    assert_eq!(
        r.error(),
        Some("No picture: The API key was refused (check or replace it).")
    );
    assert!(r.enabled(GenControl::Generate), "it can be tried again");
}

#[test]
fn a_conversion_that_cannot_be_made_is_explained_and_not_applied() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    // A 64-colour picture palette cannot be kept on OCS LoRes (32 at most).
    app.engine = dpx_core::Engine::new(dpx_core::Document::new(
        320,
        256,
        dpx_core::Palette::new(vec![Rgb8::BLACK; 64]).unwrap(),
    ));
    let before = app.engine.document().clone();
    generate_for(&mut app, &ctx, "x", GenTarget::Picture);
    click(&mut app, &ctx, 1.0, GenControl::Next(GenRow::Palette));
    let r = req(&app);
    assert_eq!(r.phase(), GenPhase::Result);
    assert!(r.thumbnail().is_none());
    assert!(
        r.error().unwrap().contains("cannot be made"),
        "{:?}",
        r.error()
    );
    assert!(!r.enabled(GenControl::Apply));
    ui_pass(&mut app, &ctx, 1.1, vec![key_event(egui::Key::Enter, true)]);
    assert!(app.ui.generate_requester.is_some());
    assert_eq!(app.engine.document(), &before);
}

/// A `ui()` pass that redraws; returns the uploaded screen image.
fn screen_pass(app: &mut PainterApp, ctx: &egui::Context, t: f64) -> egui::ColorImage {
    app.dirty = true;
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 1024.0),
        )),
        time: Some(t),
        ..Default::default()
    };
    let mut frame = eframe::Frame::_new_kittest();
    let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
    let (w, h) = (app.view().screen_w as usize, app.view().screen_h as usize);
    let screen = out
        .textures_delta
        .set
        .values()
        .flat_map(|deltas| deltas.iter())
        .find_map(|delta| match &delta.image {
            egui::ImageData::Color(img) if img.size == [w, h] => Some((**img).clone()),
            _ => None,
        });
    out.textures_delta.clear();
    screen.expect("the screen was uploaded")
}

#[test]
fn the_window_shows_the_thumbnail_in_true_colour_and_not_under_the_setup() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Spare);
    let shown = screen_pass(&mut app, &ctx, t + 0.1);
    let r = req(&app);
    let thumb = r.thumbnail().unwrap().clone();
    let rect = r.thumb_rect();
    let v = app.view();
    let (w, _) = (v.screen_w as usize, v.screen_h as usize);
    let at = |x: i32, y: i32| {
        let p = shown.pixels[(y * v.uy) as usize * w + (x * v.ux) as usize];
        [p.r(), p.g(), p.b()]
    };
    for (tx, ty) in [(0, 0), (thumb.width() - 1, thumb.height() - 1), (40, 30)] {
        let p = thumb.get(tx, ty).unwrap();
        assert_eq!(
            at(rect.x + tx as i32, rect.y + ty as i32),
            [p.r, p.g, p.b],
            "{tx},{ty}"
        );
    }
    // The setup covers it: no thumbnail over the setup requester.
    click(&mut app, &ctx, t + 0.2, GenControl::SetUp);
    let covered = screen_pass(&mut app, &ctx, t + 0.3);
    let p = covered.pixels
        [(rect.y * v.uy + 30 * v.uy) as usize * w + (rect.x * v.ux + 40 * v.ux) as usize];
    let pal = app.engine.document().palette();
    assert!(
        pal.colors()
            .iter()
            .any(|c| [c.r, c.g, c.b] == [p.r(), p.g(), p.b()]),
        "an indexed screen pixel"
    );
}

#[test]
fn settings_keep_the_generator_and_model_but_never_a_key() {
    let mut s = ImageGenSettings::default();
    assert_eq!(s.provider, GenProvider::None);
    assert_eq!(s.openai.model, DEFAULT_MODEL);
    s.provider = GenProvider::OpenAi;
    s.openai.model = "gpt-image-2.5-flare".into();
    s.openai.quality = Quality::Low;
    let json = s.to_json();
    assert_eq!(json["provider"], "openai");
    assert_eq!(json["openai"]["model"], "gpt-image-2.5-flare");
    let back = ImageGenSettings::from_json(json.as_object().unwrap());
    assert_eq!(back, s);
    // Old or broken sections keep the defaults.
    let broken: Value = serde_json::from_str(r#"{"provider": "dalle", "openai": 7}"#).unwrap();
    assert_eq!(
        ImageGenSettings::from_json(broken.as_object().unwrap()),
        ImageGenSettings::default()
    );
    let old = crate::config::Config::from_json(&json!({"pixel_size": 2})).unwrap();
    assert_eq!(old.image_generator, ImageGenSettings::default());
    // Round trip through the whole settings file.
    let c = crate::config::Config {
        image_generator: s.clone(),
        ..Default::default()
    };
    let again = crate::config::Config::from_json(&c.to_json()).unwrap();
    assert_eq!(again.image_generator, s);
    let mut app = PainterApp::new();
    again.apply(&mut app);
    assert_eq!(app.imagegen.settings, s);
}

#[test]
fn the_quality_choice_is_remembered_in_the_settings() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "x");
    click(&mut app, &ctx, 0.2, GenControl::More);
    click(&mut app, &ctx, 0.3, GenControl::Prev(GenRow::Quality));
    click(&mut app, &ctx, 0.4, GenControl::Generate);
    settle(&mut app, &ctx, 0.5);
    assert_eq!(app.imagegen.settings.openai.quality, Quality::Low);
}

#[test]
fn apply_texts_and_the_summary_fit_their_line() {
    let font = dpx_ui::font::Font::builtin();
    for t in [
        GenTarget::Spare,
        GenTarget::Picture,
        GenTarget::Brush,
        GenTarget::NewLayer,
    ] {
        for (frames, fitted) in [(1, 0), (dpx_core::anim::MAX_FRAMES, 256)] {
            let text = apply_text(t, frames, fitted);
            assert!(font.text_width(&text) <= 300, "{text}");
        }
    }
    assert_eq!(
        apply_text(GenTarget::Picture, 20, 0),
        "Apply: replaces all 20 frames (undoable)"
    );
    assert_eq!(
        apply_text(GenTarget::NewLayer, 20, 0),
        "Apply: as a new layer (undoable)"
    );
    let full = format!("No new layer: {MAX_LAYERS} layers is the most");
    assert!(font.text_width(&full) <= 300, "{full}");
    assert_eq!(layer_name(&[Layer::background()]), "Generated");
    assert_eq!(
        layer_name(&[Layer::background(), Layer::new("Generated")]),
        "Generated 2"
    );
    assert_eq!(
        apply_text(GenTarget::Brush, 1, 20),
        "Apply: as the brush, 20 colours fitted"
    );
    assert!(font.text_width("OpenAI Images: billed per picture") <= 300);
    assert!(font.text_width(TestPattern.display_name()) <= 300);
    assert!(OPENAI_NOTE.is_ascii() && NO_GENERATOR.is_ascii());
}

#[test]
fn the_setup_note_shows_whole_on_the_smallest_screens() {
    let (app, _ctx) = app_with(None);
    let info = app.setup_info();
    assert!(info.note.is_ascii());
    for screen in [(320, 200), (640, 200), (320, 256), (640, 512)] {
        let r =
            ImageGenSetupRequester::new(SetupProvider::OpenAi, DEFAULT_MODEL, info.clone(), screen);
        assert!(r.note_fits(), "{screen:?}");
    }
}

#[test]
fn snapshot_specs_parse_and_bad_ones_are_refused() {
    let s = parse_spec(
        "prompt=A castle;resolution=4;chipset=aga;colors=64;style=painted;palette=current;\
         dither=diffusion;fit=whole;scaler=nearest;background=transparent;quality=high;\
         target=brush;more;provider=test_pattern",
    )
    .unwrap();
    assert_eq!(s.prompt, "A castle");
    assert!(s.more);
    assert_eq!(s.provider.as_deref(), Some("test_pattern"));
    let o = s.options;
    assert_eq!(o.resolution, 4);
    assert_eq!(o.chipset, Chipset::Aga);
    assert_eq!(o.colors, 64);
    assert_eq!(o.style, GenStyle::Painted);
    assert_eq!(o.palette, PaletteChoice::Current);
    assert_eq!(o.target, GenTarget::Brush);
    assert!(o.transparent);
    for bad in [
        "colors=12",
        "colors=64",
        "resolution=7",
        "chipset=ecs",
        "provider=dalle",
        "key=sk-real",
        "keychain=system",
        "nonsense",
        "size=3",
    ] {
        assert!(parse_spec(bad).is_err(), "{bad}");
    }
    assert_eq!(parse_spec("").unwrap().options, GenerateOptions::default());
    assert!(parse_spec("keychain=stand_in").unwrap().stand_in_keychain);
    assert!(!parse_spec("").unwrap().stand_in_keychain);
    // Without colors the default count follows the resolution (OCS HiRes: 16).
    assert_eq!(parse_spec("resolution=3").unwrap().options.colors, 16);
}

#[test]
fn the_mcp_question_snapshot_shows_either_question_or_refuses() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let page = IndexedImage::new(320, 256, 0);
    let doc = dpx_core::Document::from_layers(
        vec![vec![page.clone(), page]; 3],
        vec![Layer::background(), Layer::new("Sky")],
        0,
        1,
        10,
        Palette::dpaint_default_32(),
        dpx_core::Target::default(),
    )
    .unwrap();
    for (which, title, text) in [
        (
            "picture",
            "Replace Picture?",
            "the picture (all 3 frames, 2 layers)",
        ),
        ("spare", "Replace Spare Page?", "spare page (page1)"),
    ] {
        let mut ui = UiState::default();
        snapshot(&args(&["--mcp-gen-question", which]), &doc, &mut ui).unwrap();
        let q = ui.choice_requester.expect("the question");
        assert_eq!(q.title(), title);
        let lines = q.lines().join(" ");
        assert!(lines.contains(text), "{lines}");
        assert!(!q.truncated());
    }
    let mut ui = UiState::default();
    assert!(snapshot(&args(&["--mcp-gen-question", "brush"]), &doc, &mut ui).is_err());
}

#[test]
fn the_longest_mcp_generation_questions_fit_the_smallest_screen() {
    // A client name is cut to 24 characters; the most frames and layers and the largest
    // page and palette.
    let client = Some("An MCP client named wwwwwwwwwwwwwwwww");
    for q in [
        dpx_mcp::tools::replace_question(client, 1280, 512, 256, 9999, 32),
        dpx_mcp::tools::replace_spare_question(client, dpx_mcp::Slot::Second, 1280, 512, 256),
    ] {
        let req = crate::mcp::question_requester(&q);
        assert!(!req.truncated(), "{:?}", req.lines());
        assert!(req.height() <= 200, "{}: {:?}", req.height(), req.lines());
    }
}

#[test]
fn snapshot_runs_only_the_marked_test_pattern() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let doc = PainterApp::new().engine.document().clone();
    let mut ui = UiState::default();
    // Nothing asked: nothing changes.
    assert_eq!(snapshot(&args(&[]), &doc, &mut ui).unwrap(), doc);
    assert!(snapshot(&args(&["--generate-run"]), &doc, &mut ui).is_err());
    // Running needs the test pattern: OpenAI is never called from a snapshot.
    let e = snapshot(
        &args(&[
            "--generate-requester",
            "--generate",
            "provider=openai;key=placeholder;prompt=x",
            "--generate-run",
        ]),
        &doc,
        &mut ui,
    )
    .unwrap_err();
    assert_eq!(e.0, 2);
    let shown = snapshot(
        &args(&[
            "--generate-requester",
            "--generate",
            "provider=test_pattern;prompt=x;target=picture",
            "--generate-apply",
        ]),
        &doc,
        &mut ui,
    )
    .unwrap();
    assert_eq!(shown.target(), ocs_320x240());
    assert!(ui.generate_requester.is_none());
    let mut ui = UiState::default();
    snapshot(
        &args(&[
            "--generate-requester",
            "--generate",
            "provider=test_pattern;prompt=x",
            "--generate-run",
        ]),
        &doc,
        &mut ui,
    )
    .unwrap();
    let r = ui.generate_requester.as_ref().unwrap();
    assert_eq!(r.phase(), GenPhase::Result);
    assert!(r.provider().summary.contains("not AI"), "clearly marked");
}

#[test]
fn the_keychain_is_opened_only_when_generate_or_the_setup_is_used() {
    let mut s = ImageGenState::default();
    assert!(!s.system_keys, "tests and snapshots never use the keychain");
    let before = Arc::as_ptr(&s.keys);
    s.ensure_keys();
    assert_eq!(Arc::as_ptr(&s.keys), before, "nothing opened");
}

// ---- Review fixes (ADR-45): details with their page, in-flight limit, Again, the setup.

/// Keys in a keychain stand-in (a working system keychain; no real one is touched).
fn keychain_keys() -> Arc<ApiKeys> {
    Arc::new(ApiKeys::new(
        Some(Box::new(dpx_imagegen::secret::MemoryStore::new())),
        None,
        Arc::new(|_: &str| None::<String>),
    ))
}

/// Colour ranges, a grab point and a transparent colour for the shown picture.
fn some_extras() -> files::PictureExtras {
    let range = |low, high| dpx_iff::ilbm::ColorRange {
        rate: 8192,
        flags: 1,
        low,
        high,
    };
    files::PictureExtras {
        ranges: vec![range(20, 27), range(8, 15)],
        grab: Some((3, 4)),
        transparent_color: Some(3),
    }
}

/// Sets the open requester's options (the rows' clicks are tested above).
fn set_options(app: &mut PainterApp, f: impl FnOnce(&mut GenerateOptions)) {
    let r = app.ui.generate_requester.as_mut().expect("open");
    let mut o = r.options();
    f(&mut o);
    r.set_options(o);
}

/// Opens Generate with `prompt` and `options`, generates through `ui()` and applies.
fn generate_and_apply(
    app: &mut PainterApp,
    ctx: &egui::Context,
    t: f64,
    prompt: &str,
    options: impl FnOnce(&mut GenerateOptions),
) -> f64 {
    app.open_generate();
    req_mut(app).set_prompt(prompt);
    set_options(app, options);
    click(app, ctx, t + 0.01, GenControl::Generate);
    let t = settle(app, ctx, t + 0.04);
    assert_eq!(req(app).phase(), GenPhase::Result, "{:?}", req(app).error());
    click(app, ctx, t + 0.01, GenControl::Apply);
    assert!(app.ui.generate_requester.is_none(), "applied");
    t + 0.04
}

fn req_mut(app: &mut PainterApp) -> &mut GenerateRequester {
    app.ui.generate_requester.as_mut().expect("open")
}

fn aga_320x256_current(o: &mut GenerateOptions) {
    o.resolution = 1;
    o.chipset = Chipset::Aga;
    o.palette = PaletteChoice::Current;
}

#[test]
fn swapping_with_a_spare_in_the_pictures_own_format_keeps_its_details() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    app.extras = some_extras();
    let before = app.engine.document().clone();
    assert_eq!(before.target().screen, (320, 256));
    // PAL LoRes 320x256 on AGA with the current palette: the document's own format.
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", aga_320x256_current);
    assert!(!app.spare_differs(), "an ordinary spare");
    assert!(app.menu_data.spare_own.is_none(), "it shares the details");
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    assert_eq!(app.extras, some_extras(), "shared with the generated page");
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::J));
    assert_eq!(app.engine.document(), &before);
    assert_eq!(app.extras, some_extras(), "j j drops nothing");
    // Undo on the spare page (the generated one) changes nothing of them either.
    ui_pass(&mut app, &ctx, t + 0.2, tap(egui::Key::J));
    ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::U));
    ui_pass(&mut app, &ctx, t + 0.4, tap(egui::Key::J));
    assert_eq!(app.extras, some_extras());
}

#[test]
fn a_spare_of_its_own_format_keeps_its_details_when_the_formats_become_alike() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    app.extras = some_extras();
    let before = app.engine.document().clone();
    // The default (OCS 320x240) onto the spare page, j, the same picture onto the spare
    // page again (now in the shown page's format), j: the old picture is on the spare
    // engine's undo list.
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |_| {});
    assert!(app.spare_differs());
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    let generated = app.extras.clone();
    assert_eq!(generated.ranges, Vec::new(), "a new palette has no ranges");
    assert_eq!(
        app.menu_data.spare_own.as_ref(),
        Some(&some_extras()),
        "the old picture's details travel with it"
    );
    let t = generate_and_apply(&mut app, &ctx, t + 0.1, "x", |_| {});
    assert!(!app.spare_differs(), "alike now");
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    // Undo on that page brings back the old picture and its own details.
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::U));
    assert_eq!(app.engine.document().page(), before.page());
    assert_eq!(app.engine.document().target(), before.target());
    assert_eq!(app.extras, some_extras());
    assert_eq!(app.menu_data.spare_own.as_ref(), Some(&generated));
    // j j keeps each page's own.
    ui_pass(&mut app, &ctx, t + 0.2, tap(egui::Key::J));
    assert_eq!(app.extras, generated);
    ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::J));
    assert_eq!(app.extras, some_extras());
    // Redo there sets the generated picture again, sharing the generated page's details.
    app.redo();
    assert!(app.menu_data.spare_own.is_none());
    assert_eq!(app.extras, generated);
}

#[test]
fn undo_of_apply_to_the_spare_brings_back_its_file_background_and_details() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let castle = std::path::PathBuf::from("/nonexistent/castle.iff");
    app.extras = some_extras();
    app.picture_path = Some(castle.clone());
    app.menu_data.background = Some(crate::menu_actions::FixedBackground {
        image: app.engine.document().page().clone(),
        layer: 0,
    });
    app.ui.bg = 5;
    let before = app.engine.document().clone();
    // Castle goes to the spare page (j after a generated spare), then another generated
    // picture replaces it there.
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |_| {});
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    let other = app.menu_data.spare.as_ref().unwrap();
    assert_eq!(other.picture_path.as_ref(), Some(&castle));
    let t = generate_and_apply(&mut app, &ctx, t + 0.1, "a different castle", |o| {
        o.colors = 16;
    });
    assert!(app.spare_differs());
    let other = app.menu_data.spare.as_ref().unwrap();
    assert_eq!(other.picture_path, None, "a new picture has no file");
    assert!(other.background.is_none());
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::U));
    assert_eq!(app.engine.document().page(), before.page());
    assert_eq!(app.picture_path.as_ref(), Some(&castle));
    assert_eq!(
        app.menu_data.background.as_ref().map(|b| &b.image),
        Some(before.page())
    );
    assert_eq!(app.ui.bg, 5);
    assert_eq!(app.extras, some_extras());
    // Redo: the generated picture again, without them.
    app.redo();
    assert_eq!(app.picture_path, None);
    assert!(app.menu_data.background.is_none());
    assert_eq!(app.extras.ranges, Vec::new());
}

#[test]
fn apply_to_the_picture_sets_and_undo_restores_its_transparent_colour() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    assert_eq!(app.extras.transparent_color, None);
    let before_extras = app.extras.clone();
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |o| {
        o.transparent = true;
        o.target = GenTarget::Picture;
    });
    assert_eq!(app.extras.transparent_color, Some(0));
    assert_eq!(app.extras.ranges, Vec::new(), "a new palette has no ranges");
    let applied = app.extras.clone();
    ui_pass(&mut app, &ctx, t, tap(egui::Key::U));
    assert_eq!(
        app.extras, before_extras,
        "Undo brings the old details back"
    );
    app.redo();
    assert_eq!(app.extras, applied);
    // An opaque picture over one with a transparent colour has none.
    app.extras.transparent_color = Some(5);
    generate_and_apply(&mut app, &ctx, t + 0.1, "y", |o| {
        o.transparent = false;
        o.target = GenTarget::Picture;
    });
    assert_eq!(app.extras.transparent_color, None);
    assert!(app.engine.document().page().pixels().contains(&5));
    // The current palette keeps its colour ranges (as Picture > Load's Import requester).
    app.extras = some_extras();
    generate_and_apply(&mut app, &ctx, t + 0.2, "z", |o| {
        o.target = GenTarget::Picture;
        o.palette = PaletteChoice::Current;
    });
    assert_eq!(app.extras.ranges, some_extras().ranges);
    assert_eq!(app.extras.grab, None);
}

/// A provider that answers what the test tells it, after the test lets it go.
fn gated(answer: Result<(), GenError>) -> (Arc<Gated>, Sender<()>) {
    Gated::new(answer)
}

#[test]
fn again_keeps_the_result_until_a_new_one_is_there() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Spare);
    let first = app.imagegen.result.clone().expect("a result");
    let thumb = req(&app).thumbnail().cloned().unwrap();
    // Again with a generator that does not answer in time: the result stays.
    let (slow, gate) = gated(Err(GenError::Timeout));
    app.imagegen.debug_provider = Some(slow);
    click(&mut app, &ctx, t + 0.1, GenControl::Generate);
    assert!(matches!(req(&app).phase(), GenPhase::Working { .. }));
    // Option changes wait for the new picture (no result shown meanwhile).
    click(&mut app, &ctx, t + 0.2, GenControl::Next(GenRow::Dither));
    assert!(matches!(req(&app).phase(), GenPhase::Working { .. }));
    gate.send(()).unwrap();
    let t = settle(&mut app, &ctx, t + 0.3);
    let r = req(&app);
    assert_eq!(r.phase(), GenPhase::Result);
    assert_eq!(
        r.error(),
        Some("No new picture: The service did not answer in time.")
    );
    assert!(r.enabled(GenControl::Apply));
    // Shown again for the options as they are now (the dither changed meanwhile).
    let (o, kept) = app.imagegen.result.clone().unwrap();
    assert_eq!(o, req(&app).options());
    assert_eq!(kept, expected(&app, "A castle"));
    assert_ne!(o, first.0);
    // Back to the first options: exactly the first result and thumbnail.
    click(&mut app, &ctx, t + 0.1, GenControl::Prev(GenRow::Dither));
    assert_eq!(app.imagegen.result.as_ref(), Some(&first));
    assert_eq!(req(&app).thumbnail(), Some(&thumb));
    assert_eq!(req(&app).error(), None);
    // Again, then Cancel: still there, and Apply takes it.
    let (waiting, gate) = gated(Ok(()));
    app.imagegen.debug_provider = Some(waiting);
    click(&mut app, &ctx, t + 0.2, GenControl::Generate);
    click(&mut app, &ctx, t + 0.3, GenControl::Cancel);
    let r = req(&app);
    assert_eq!(r.phase(), GenPhase::Result);
    assert_eq!(
        r.error(),
        Some("Stopped waiting. A request already sent may still be billed.")
    );
    assert_eq!(app.imagegen.result.as_ref(), Some(&first));
    click(&mut app, &ctx, t + 0.4, GenControl::Apply);
    assert_eq!(
        app.menu_data
            .spare
            .as_ref()
            .unwrap()
            .engine
            .document()
            .page(),
        &first.1.page
    );
    gate.send(()).unwrap();
}

#[test]
fn more_after_a_result_makes_the_thumbnail_again_for_the_new_box() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "A castle");
    click(&mut app, &ctx, 0.2, GenControl::Generate);
    let t = settle(&mut app, &ctx, 0.3);
    let big = req(&app).thumb_size();
    let size = |app: &PainterApp| {
        let t = req(app).thumbnail().unwrap();
        (t.width(), t.height())
    };
    assert_eq!(size(&app), big);
    for (i, more) in [true, false].into_iter().enumerate() {
        click(&mut app, &ctx, t + 0.1 * (i + 1) as f64, GenControl::More);
        assert_eq!(req(&app).more(), more);
        assert_eq!(size(&app), req(&app).thumb_size(), "more={more}");
        let want = expected(&app, "A castle");
        let (w, h) = req(&app).thumb_size();
        assert_eq!(
            req(&app).thumbnail(),
            Some(&import::thumbnail(&want.page, &want.palette, w, h))
        );
    }
    // On screen the thumbnail stays inside the requester while More shows.
    click(&mut app, &ctx, t + 0.5, GenControl::More);
    let r = req(&app);
    let (rect, (ox, _), (w, _)) = (r.thumb_rect(), r.origin(), r.size());
    assert!(rect.x + rect.w <= ox + w);
    assert!(size(&app).0 < big.0);
}

#[test]
fn cancel_then_generate_cannot_pile_up_requests_in_flight() {
    let (slow, gate) = gated(Ok(()));
    let (mut app, ctx) = app_with(Some(slow.clone()));
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "A castle");
    let mut t = 0.2;
    for _ in 0..2 {
        ui_pass(&mut app, &ctx, t, tap(egui::Key::Enter));
        assert!(matches!(req(&app).phase(), GenPhase::Working { .. }));
        ui_pass(&mut app, &ctx, t + 0.01, tap(egui::Key::Escape));
        assert_eq!(req(&app).phase(), GenPhase::Ask);
        t += 0.1;
    }
    // Both requests are still in flight: the third is refused before anything is sent.
    let in_flight = |app: &PainterApp| {
        app.imagegen
            .jobs
            .list()
            .iter()
            .filter(|s| s.in_flight)
            .count()
    };
    assert_eq!(in_flight(&app), 2);
    ui_pass(&mut app, &ctx, t, tap(egui::Key::Enter));
    assert_eq!(req(&app).phase(), GenPhase::Ask);
    assert_eq!(
        req(&app).error(),
        Some(GenError::Busy { running: 2 }.to_string().as_str())
    );
    assert_eq!(in_flight(&app), 2);
    // Once they have ended (their answers are dropped), generating works again.
    gate.send(()).unwrap();
    gate.send(()).unwrap();
    for _ in 0..1000 {
        if app.imagegen.jobs.list().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.imagegen.jobs.list().is_empty(), "forgotten once ended");
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::Enter));
    gate.send(()).unwrap();
    settle(&mut app, &ctx, t + 0.2);
    assert_eq!(req(&app).phase(), GenPhase::Result);
}

#[test]
fn a_current_palette_larger_than_the_mode_is_refused_before_anything_is_sent() {
    let counting = Arc::new(Counting::default());
    let (mut app, ctx) = app_with(Some(counting.clone()));
    assert_eq!(app.engine.document().palette().len(), 32);
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.1, "A castle");
    set_options(&mut app, |o| {
        o.resolution = 3; // NTSC HiRes 640x200: 16 colours on OCS
        o.palette = PaletteChoice::Current;
    });
    assert!(req(&app).palette_problem().is_some());
    assert!(!req(&app).enabled(GenControl::Generate));
    click(&mut app, &ctx, 0.2, GenControl::Generate);
    ui_pass(&mut app, &ctx, 0.3, vec![key_event(egui::Key::Enter, true)]);
    app.start_generation();
    assert_eq!(
        counting.calls.load(Ordering::SeqCst),
        0,
        "nothing generated"
    );
    assert!(app.imagegen.jobs.list().is_empty());
    assert_eq!(req(&app).phase(), GenPhase::Ask);
    // AGA shows 32: generating goes ahead.
    set_options(&mut app, |o| o.chipset = Chipset::Aga);
    click(&mut app, &ctx, 0.4, GenControl::Generate);
    settle(&mut app, &ctx, 0.5);
    assert_eq!(counting.calls.load(Ordering::SeqCst), 1);
    assert_eq!(req(&app).phase(), GenPhase::Result);
}

#[test]
fn the_brush_target_shows_the_fitted_brush_and_says_how_many_colours_were_fitted() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Brush);
    let want = expected(&app, "A castle");
    let doc_palette = app.engine.document().palette().clone();
    let (_, brush, fitted) = fitted_brush(&want, &doc_palette);
    assert!(fitted > 0, "a new palette differs from the picture's");
    assert_eq!(
        req(&app).message(),
        Some(format!("Apply: as the brush, {fitted} colours fitted").as_str())
    );
    // The thumbnail shows the brush Apply makes, in the picture palette.
    let (w, h) = req(&app).thumb_size();
    let page = IndexedImage::from_pixels(brush.width, brush.height, brush.pixels.clone()).unwrap();
    assert_eq!(
        req(&app).thumbnail(),
        Some(&import::thumbnail(&page, &doc_palette, w, h))
    );
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert_eq!(app.custom_brush.as_ref(), Some(&brush));
}

#[test]
fn a_transparent_brush_keeps_every_opaque_pixel_opaque() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    generate_and_apply(&mut app, &ctx, 0.0, "A sprite", |o| {
        o.transparent = true;
        o.target = GenTarget::Brush;
    });
    let brush = app.custom_brush.clone().unwrap();
    assert_eq!(brush.transparent, Some(0));
    let own = &app.menu_data.palette_stencil.loaded.as_ref().unwrap().brush;
    for (i, (&o, &b)) in own.pixels.iter().zip(&brush.pixels).enumerate() {
        assert_eq!(
            o == 0,
            b == 0,
            "pixel {i}: transparent exactly where it was"
        );
    }
}

#[test]
fn picture_names_every_frame_it_replaces_and_delete_this_page_keeps_an_animation() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    app.engine
        .execute(dpx_core::Command::SetFrameCount { count: 20 })
        .unwrap();
    generate_for(&mut app, &ctx, "x", GenTarget::Picture);
    assert_eq!(
        req(&app).message(),
        Some("Apply: replaces all 20 frames (undoable)")
    );
    app.close_generate();
    // A spare of its own format cannot become one frame of a 20-frame animation.
    generate_and_apply(&mut app, &ctx, 1.0, "x", |o| o.target = GenTarget::Spare);
    assert!(app.spare_differs());
    let before = app.engine.document().clone();
    app.delete_page();
    assert_eq!(app.engine.document(), &before, "nothing replaced");
    assert_eq!(app.engine.document().frame_count(), 20);
    assert!(app.menu_data.spare.is_some(), "the spare stays");
    let note = format!("{:?}", app.ui.note_requester.as_ref().expect("it says why"));
    assert!(
        note.contains("Not deleted") && note.contains("all 20"),
        "{note}"
    );
}

#[test]
fn delete_this_page_with_an_own_spare_brings_back_the_details_on_undo() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    app.extras = some_extras();
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |_| {});
    let own = app.menu_data.spare_own.clone().unwrap();
    app.delete_page();
    assert_eq!(app.extras, own, "the spare's details come with its picture");
    ui_pass(&mut app, &ctx, t, tap(egui::Key::U));
    assert_eq!(app.extras, some_extras());
    app.redo();
    assert_eq!(app.extras, own);
}

/// A project saved before ADR-46: OCS PAL LoRes 320x256 whose palette entries 9-27, 29 and
/// 30 are not 12-bit (`crates/dpx-formats/tests/data/README.md`).
const OLD_OCS: &[u8] =
    include_bytes!("../../../../crates/dpx-formats/tests/data/v1-ocs-not-12-bit.dpxproj");

/// Opens the older OCS project as Picture ▸ Load.. does (its notes taken); returns its
/// drawer, to be removed by the test.
fn open_old_ocs(app: &mut PainterApp, name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dpx-gen-old-ocs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Old.dpxproj");
    std::fs::write(&path, OLD_OCS).unwrap();
    app.load_picture(path, true);
    assert!(
        app.ui.note_requester.take().is_some(),
        "the notes on its colours"
    );
    let doc = app.engine.document();
    assert_eq!(doc.target().chipset, Chipset::Ocs);
    let bad = dpx_core::hardware::unshowable_entries(Chipset::Ocs, doc.palette().colors());
    assert_eq!(bad.len(), 21);
    dir
}

/// Wave-8 integration (ADR-46): Copy to Spare onto a generated spare of its own format
/// copies an older OCS picture with its colours as stored, in one undo step on the spare
/// page: j and Undo there bring the generated picture and its format back.
#[test]
fn copy_to_spare_keeps_an_old_ocs_pictures_colours_and_the_spares_undo() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let dir = open_old_ocs(&mut app, "copy");
    let old = app.engine.document().clone();
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |_| {});
    assert!(app.spare_differs());
    let generated = app
        .menu_data
        .spare
        .as_ref()
        .unwrap()
        .engine
        .document()
        .clone();
    assert_eq!(generated.target(), ocs_320x240());
    app.copy_to_spare();
    assert!(app.ui.note_requester.is_none(), "copied: nothing to say");
    assert!(app.menu_data.spare_own.is_none(), "an ordinary spare again");
    let spare = app.menu_data.spare.as_ref().unwrap().engine.document();
    assert_eq!(spare.target(), old.target());
    assert_eq!(spare.palette(), old.palette(), "the colours as stored");
    assert_eq!(spare.page(), old.page());
    // j shows the copy; Undo there brings back the generated picture, format and palette.
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    assert_eq!(app.engine.document().palette(), old.palette());
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::U));
    let doc = app.engine.document();
    assert_eq!(doc.target(), generated.target());
    assert_eq!(doc.palette(), generated.palette());
    assert_eq!(doc.page(), generated.page());
    assert!(app.spare_differs(), "of its own format again");
    // j: the old picture, as it was.
    ui_pass(&mut app, &ctx, t + 0.2, tap(egui::Key::J));
    assert_eq!(app.engine.document().palette(), old.palette());
    assert_eq!(app.engine.document().page(), old.page());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Wave-8 integration (ADR-46): Delete This Page with the older OCS picture on the spare
/// page shows it with its colours as stored, in one undo step.
#[test]
fn delete_this_page_shows_an_old_ocs_spare_with_its_colours() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let dir = open_old_ocs(&mut app, "delete");
    let old = app.engine.document().clone();
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |_| {});
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    let generated = app.engine.document().clone();
    assert_eq!(generated.target(), ocs_320x240());
    app.delete_page();
    assert!(app.menu_data.spare.is_none(), "the spare became the page");
    let doc = app.engine.document();
    assert_eq!(doc.target(), old.target());
    assert_eq!(doc.palette(), old.palette(), "the colours as stored");
    assert_eq!(doc.page(), old.page());
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::U));
    assert_eq!(app.engine.document().palette(), generated.palette());
    assert_eq!(app.engine.document().target(), generated.target());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Wave-8 integration (ADR-46): Generate with the current palette onto an OCS picture
/// rounds an older project's colours that are not 12-bit, and the note after Apply says
/// how many (never silent); Undo brings them back. A new palette, or a current palette that
/// is 12-bit already, rounds nothing and shows no note.
#[test]
fn generate_with_the_current_palette_says_which_colours_it_rounded() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let dir = open_old_ocs(&mut app, "round");
    let old = app.engine.document().clone();
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |o| {
        o.palette = PaletteChoice::Current;
        o.target = GenTarget::Picture;
    });
    let note = app.ui.note_requester.take().expect("the rounding is said");
    assert_eq!(note.title(), GENERATED);
    assert_eq!(
        note.notes(),
        [
            "21 colours rounded to 12-bit OCS values",
            "Undo brings the old colours back"
        ]
    );
    let doc = app.engine.document();
    assert_eq!(doc.target().chipset, Chipset::Ocs);
    assert!(
        doc.palette()
            .colors()
            .iter()
            .all(|&c| dpx_core::hardware::is_12_bit(c))
    );
    assert_eq!(
        doc.palette().get(0),
        old.palette().get(0),
        "12-bit ones stay"
    );
    ui_pass(&mut app, &ctx, t, tap(egui::Key::U));
    assert_eq!(app.engine.document().palette(), old.palette());
    // Onto the spare page: the spare has them rounded, the shown page keeps them.
    let t = generate_and_apply(&mut app, &ctx, t + 0.1, "x", |o| {
        o.palette = PaletteChoice::Current;
        o.target = GenTarget::Spare;
    });
    let note = app.ui.note_requester.take().expect("the rounding is said");
    assert_eq!(
        note.notes(),
        [
            "21 colours rounded to 12-bit OCS values",
            "On the spare page; the shown page keeps its colours"
        ]
    );
    assert_eq!(app.engine.document().palette(), old.palette());
    // A new palette rounds nothing of the current one: no note.
    let t = generate_and_apply(&mut app, &ctx, t + 0.1, "x", |o| {
        o.palette = PaletteChoice::New;
        o.target = GenTarget::Picture;
    });
    assert!(app.ui.note_requester.is_none());
    // Now the current palette is 12-bit: nothing to round, no note.
    generate_and_apply(&mut app, &ctx, t + 0.1, "x", |o| {
        o.palette = PaletteChoice::Current;
        o.target = GenTarget::Picture;
    });
    assert!(app.ui.note_requester.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_rounding_note_is_only_for_targets_that_take_the_palette() {
    assert!(rounded_note(GenTarget::Picture, 0).is_none());
    assert!(rounded_note(GenTarget::Brush, 3).is_none());
    let note = rounded_note(GenTarget::Spare, 1).unwrap();
    assert_eq!(note.notes()[0], "1 colour rounded to a 12-bit OCS value");
}

/// Serves one model list on 127.0.0.1 once the test lets it answer.
fn serve_models_when(go: std::sync::mpsc::Receiver<()>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut buf = [0u8; 4096];
        let _ = s.read(&mut buf);
        let _ = go.recv_timeout(Duration::from_secs(10));
        let body = r#"{"object":"list","data":[{"id":"gpt-image-2.6-flare","object":"model"}]}"#;
        let answer = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = s.write_all(answer.as_bytes());
    });
    url
}

#[test]
fn a_key_test_answer_is_not_shown_for_a_key_edited_since() {
    let (mut app, ctx) = app_with(None);
    let (go, wait) = channel();
    app.imagegen.models_endpoint = serve_models_when(wait);
    app.imagegen.settings.provider = GenProvider::OpenAi;
    app.open_imagegen_setup();
    ui_pass(
        &mut app,
        &ctx,
        0.1,
        vec![egui::Event::Paste("sk-old-1".into())],
    );
    click_setup(&mut app, &ctx, 0.2, SetupControl::Test);
    assert!(app.imagegen.key_test.is_some());
    // The key is edited while the test waits for its answer.
    type_text(&mut app, &ctx, 0.3, "2");
    go.send(()).unwrap();
    let mut t = 0.4;
    while app.imagegen.key_test.is_some() {
        ui_pass(&mut app, &ctx, t, vec![]);
        assert!(t < 10.0, "the test was not dropped");
        t += 0.01;
        std::thread::sleep(Duration::from_millis(5));
    }
    for i in 0..20 {
        ui_pass(&mut app, &ctx, t + f64::from(i) * 0.01, vec![]);
    }
    let setup = app.ui.imagegen_setup.as_ref().unwrap();
    assert_eq!(setup.test(), None, "no answer for the old key");
    assert_eq!(setup.typed_key().as_deref(), Some("sk-old-12"));
}

#[test]
fn remove_key_then_none_then_ok_removes_the_key() {
    let (mut app, ctx) = app_with(None);
    app.imagegen.keys = keychain_keys();
    app.imagegen
        .keys
        .set(KEY_ACCOUNT, &Secret::new("sk-stored").unwrap());
    app.imagegen.settings.provider = GenProvider::OpenAi;
    app.open_imagegen_setup();
    click_setup(&mut app, &ctx, 0.1, SetupControl::RemoveKey);
    click_setup(
        &mut app,
        &ctx,
        0.2,
        SetupControl::Provider(SetupProvider::None),
    );
    let setup = app.ui.imagegen_setup.as_ref().unwrap();
    assert_eq!(setup.key_line(), "The API key is removed at Ok.");
    click_setup(&mut app, &ctx, 0.3, SetupControl::Ok);
    assert!(app.ui.imagegen_setup.is_none());
    assert_eq!(app.imagegen.settings.provider, GenProvider::None);
    assert!(
        app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).is_none(),
        "removed as the key line said"
    );
    // A key typed, then None: saved as the key line says (nothing is sent with None).
    app.open_imagegen_setup();
    click_setup(
        &mut app,
        &ctx,
        0.4,
        SetupControl::Provider(SetupProvider::OpenAi),
    );
    ui_pass(
        &mut app,
        &ctx,
        0.5,
        vec![egui::Event::Paste("sk-new".into())],
    );
    click_setup(
        &mut app,
        &ctx,
        0.6,
        SetupControl::Provider(SetupProvider::None),
    );
    assert_eq!(
        app.ui.imagegen_setup.as_ref().unwrap().key_line(),
        "The new key is saved at Ok."
    );
    click_setup(&mut app, &ctx, 0.7, SetupControl::Ok);
    assert!(app.ui.imagegen_setup.is_none());
    let (key, source) = app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).unwrap();
    assert_eq!((key.expose(), source), ("sk-new", KeySource::Keychain));
}

#[test]
fn a_key_kept_for_this_session_only_is_said_before_the_setup_closes() {
    // No keychain (this session's memory only, as on Linux without a Secret Service).
    let (mut app, ctx) = app_with(None);
    app.imagegen.settings.provider = GenProvider::OpenAi;
    app.open_imagegen_setup();
    let setup = app.ui.imagegen_setup.as_ref().unwrap();
    assert_eq!(setup.info().key_status, "No API key (this session only)");
    assert!(
        setup.info().note.contains("No keychain here"),
        "{}",
        setup.info().note
    );
    assert!(!setup.info().note.contains("goes to the system keychain"));
    assert_eq!(setup.test(), Some("Session only: test store."));
    ui_pass(
        &mut app,
        &ctx,
        0.1,
        vec![egui::Event::Paste("sk-test-3".into())],
    );
    click_setup(&mut app, &ctx, 0.2, SetupControl::Ok);
    let setup = app
        .ui
        .imagegen_setup
        .as_ref()
        .expect("still open, to say so");
    assert_eq!(
        setup.test(),
        Some("Saved for this session only (test store); it is gone after a restart.")
    );
    assert_eq!(setup.typed_key(), None);
    assert!(setup.key_line().contains("this session only"));
    let (_, source) = app.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).unwrap();
    assert_eq!(source, KeySource::Session);
    assert_eq!(app.imagegen.settings.provider, GenProvider::OpenAi);
    click_setup(&mut app, &ctx, 0.3, SetupControl::Ok);
    assert!(app.ui.imagegen_setup.is_none(), "the next Ok closes");
    // With a working keychain, Ok closes at once and the note names the keychain.
    let (mut app, ctx) = app_with(None);
    app.imagegen.keys = keychain_keys();
    app.imagegen.settings.provider = GenProvider::OpenAi;
    app.open_imagegen_setup();
    let setup = app.ui.imagegen_setup.as_ref().unwrap();
    assert!(setup.info().note.contains("goes to the system keychain"));
    assert_eq!(setup.test(), None);
    ui_pass(
        &mut app,
        &ctx,
        0.1,
        vec![egui::Event::Paste("sk-test-4".into())],
    );
    click_setup(&mut app, &ctx, 0.2, SetupControl::Ok);
    assert!(app.ui.imagegen_setup.is_none());
}

#[test]
fn own_details_survive_j_j_after_the_shown_page_took_the_spares_format() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    // A generated spare of its own format, then the same picture onto the shown page: the
    // two pages are alike now, but the spare keeps its own details.
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |_| {});
    let t = generate_and_apply(&mut app, &ctx, t, "x", |o| o.target = GenTarget::Picture);
    assert!(!app.spare_differs(), "alike now");
    assert!(app.menu_data.spare_own.is_some());
    // The shown page's ranges are edited; j j must not lose them.
    app.extras.ranges = some_extras().ranges;
    let edited = app.extras.clone();
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    assert_ne!(app.extras, edited, "the spare's own details");
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::J));
    assert_eq!(app.extras, edited);
}

#[test]
fn undo_on_one_page_never_turns_a_spare_of_another_format_into_one_to_drop() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    app.extras = some_extras();
    // A generated spare (OCS 320x240); then the shown picture takes another format (OCS
    // NTSC 320x200, Apply to Picture).
    let t = generate_and_apply(&mut app, &ctx, 0.0, "x", |_| {});
    let t = generate_and_apply(&mut app, &ctx, t, "y", |o| {
        o.resolution = 0;
        o.target = GenTarget::Picture;
    });
    let picture = app.engine.document().clone();
    let picture_extras = app.extras.clone();
    // j, then Undo on the generated page: it is the old blank spare again, of the old
    // format, while the other page has a third format.
    ui_pass(&mut app, &ctx, t, tap(egui::Key::J));
    ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::U));
    assert_eq!(app.engine.document().target().screen, (320, 256));
    assert_eq!(app.extras, some_extras(), "the details it shared then");
    assert!(app.spare_differs(), "the other page keeps its own format");
    // So j shows that picture unchanged, with its own details.
    ui_pass(&mut app, &ctx, t + 0.2, tap(egui::Key::J));
    assert_eq!(app.engine.document(), &picture);
    assert_eq!(app.extras, picture_extras);
}

#[test]
fn a_new_layer_is_made_in_the_pictures_page_and_palette_as_one_undo_step() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    // Something on the picture, in frame 2 of 3.
    app.engine.execute(Command::AddFrames { count: 2 }).unwrap();
    app.engine.execute(Command::GotoFrame { frame: 1 }).unwrap();
    app.engine.execute(Command::Clear { color: 4 }).unwrap();
    let before = app.engine.document().clone();
    let t = generate_for(&mut app, &ctx, "A tree", GenTarget::NewLayer);
    let r = req(&app);
    let page = (before.page().width(), before.page().height());
    assert_eq!(r.format(), (before.target(), page));
    assert_eq!(r.palette_choice(), PaletteChoice::Current);
    assert!(!r.enabled(GenControl::Next(GenRow::Chipset)), "fixed");
    assert_eq!(r.message(), Some("Apply: as a new layer (undoable)"));
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert!(app.ui.generate_requester.is_none(), "applied");
    let doc = app.engine.document();
    let names: Vec<&str> = doc.layers().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "Generated"]);
    assert_eq!((doc.active_layer(), doc.current_frame()), (1, 1));
    assert_eq!(doc.target(), before.target());
    assert_eq!(doc.palette(), before.palette());
    assert_eq!(doc.cel(1, 0), before.cel(1, 0), "the layer under it stays");
    // An opaque picture: no pixel is colour 0, the transparent one of upper layers.
    let cel = doc.cel(1, 1).unwrap();
    assert_eq!((cel.width(), cel.height()), page);
    assert!(
        cel.pixels()
            .iter()
            .all(|&p| p != 0 && usize::from(p) < before.palette().len())
    );
    for f in [0, 2] {
        assert!(doc.cel(f, 1).unwrap().pixels().iter().all(|&p| p == 0));
    }
    // One Undo takes it all back.
    ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::U));
    let doc = app.engine.document();
    assert_eq!(doc.layers().len(), 1);
    assert_eq!(doc.page(), before.page());
}

#[test]
fn the_new_layer_thumbnail_stacks_it_where_apply_puts_it() {
    // [Background (active), Top (opaque)]: the new layer goes between them, so Top covers
    // it after Apply, and the thumbnail shows that.
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let doc = app.engine.document();
    let (w, h) = (doc.page().width(), doc.page().height());
    app.engine
        .execute(Command::AddLayer {
            name: Some("Top".into()),
        })
        .unwrap();
    let mut top = vec![5u8; (w * h) as usize];
    top[..(w as usize)].fill(0); // one clear row: the new layer shows through there
    app.engine
        .execute(Command::SetPage {
            width: w,
            height: h,
            pixels: top,
        })
        .unwrap();
    app.engine
        .execute(Command::SelectLayer { layer: 0 })
        .unwrap();
    let t = generate_for(&mut app, &ctx, "A tree", GenTarget::NewLayer);
    let thumb = req(&app).thumbnail().expect("a thumbnail").clone();
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert!(app.ui.generate_requester.is_none(), "applied");
    let doc = app.engine.document();
    let names: Vec<&str> = doc.layers().iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "Generated", "Top"]);
    let after = import::thumbnail(
        &doc.composite(),
        doc.palette(),
        thumb.width(),
        thumb.height(),
    );
    assert_eq!(thumb, after, "the thumbnail is the frame after Apply");
}

#[test]
fn a_picture_with_the_most_layers_gets_no_new_one_and_says_so() {
    let counting = Arc::new(Counting::default());
    let (mut app, ctx) = app_with(Some(counting.clone()));
    for _ in 1..MAX_LAYERS {
        app.engine
            .execute(Command::AddLayer { name: None })
            .unwrap();
    }
    // New layer is not generated for (nothing is sent or billed): the box says why.
    open_by_menu(&mut app, &ctx, 0.0);
    type_text(&mut app, &ctx, 0.2, "A tree");
    click(&mut app, &ctx, 0.3, GenControl::More);
    let mut t = 0.4;
    while req(&app).options().target != GenTarget::NewLayer {
        click(&mut app, &ctx, t, GenControl::Next(GenRow::Target));
        t += 0.1;
    }
    let why = req(&app).problem().expect("the picture is full");
    assert!(why.starts_with("The picture has 32 layers"), "{why}");
    assert!(!req(&app).enabled(GenControl::Generate));
    click(&mut app, &ctx, t, GenControl::Generate);
    ui_pass(
        &mut app,
        &ctx,
        t + 0.1,
        vec![key_event(egui::Key::Enter, true)],
    );
    app.start_generation();
    assert!(app.imagegen.jobs.list().is_empty(), "nothing was sent");
    assert_eq!(counting.calls.load(Ordering::SeqCst), 0);
    assert_eq!(req(&app).phase(), GenPhase::Ask);
    // A result made for the brush, then shown for New layer, says so and is not applied.
    click(&mut app, &ctx, t + 0.2, GenControl::Prev(GenRow::Target));
    assert_eq!(req(&app).options().target, GenTarget::Brush);
    click(&mut app, &ctx, t + 0.3, GenControl::Generate);
    let t = settle(&mut app, &ctx, t + 0.4);
    assert_eq!(counting.calls.load(Ordering::SeqCst), 1);
    click(&mut app, &ctx, t + 0.05, GenControl::Next(GenRow::Target));
    assert_eq!(req(&app).options().target, GenTarget::NewLayer);
    let full = format!("No new layer: {MAX_LAYERS} layers is the most");
    assert_eq!(req(&app).message(), Some(full.as_str()));
    assert!(!req(&app).enabled(GenControl::Generate), "no Again for it");
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert!(app.ui.generate_requester.is_some(), "not applied");
    let why = format!("Not applied: {}.", layers_full());
    assert_eq!(req(&app).error(), Some(why.as_str()));
    assert_eq!(app.engine.document().layers().len(), MAX_LAYERS);
    // The result stays: another target still takes it.
    click(&mut app, &ctx, t + 0.2, GenControl::Prev(GenRow::Target));
    assert_eq!(req(&app).options().target, GenTarget::Brush);
    let t = settle(&mut app, &ctx, t + 0.3);
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert!(app.ui.generate_requester.is_none(), "applied as the brush");
    assert!(app.custom_brush.is_some());
}

/// Wave-8 integration review: Delete This Page with a generated spare of its own format that
/// was given layers takes the spare's whole picture (every layer, the generated picture
/// on the bottom one) in one undo step; Undo brings the shown page back, Redo the spare's
/// whole picture again.
#[test]
fn delete_this_page_keeps_every_layer_of_a_spare_of_its_own_format() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let shown = app.engine.document().clone();
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Spare);
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    assert!(app.spare_differs());
    // j: on the generated page, a layer "Sky" with a band of colour 3, then back.
    ui_pass(&mut app, &ctx, t + 0.2, tap(egui::Key::J));
    let castle = app.engine.document().page().clone();
    let (w, h) = (castle.width(), castle.height());
    app.engine
        .execute(Command::AddLayer {
            name: Some("Sky".into()),
        })
        .unwrap();
    let mut band = vec![0u8; (w * h) as usize];
    band[..(w * 10) as usize].fill(3);
    app.engine
        .execute(Command::SetPage {
            width: w,
            height: h,
            pixels: band,
        })
        .unwrap();
    let worked = app.engine.document().clone();
    ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::J));
    assert!(app.spare_differs(), "still its own format");
    app.delete_page();
    assert!(app.menu_data.spare.is_none(), "the spare is freed");
    let same_picture = |doc: &dpx_core::Document| {
        assert_eq!(doc.target(), worked.target());
        assert_eq!(doc.palette(), worked.palette());
        assert_eq!(doc.layers(), worked.layers(), "the spare's layers");
        assert_eq!(doc.active_layer(), worked.active_layer());
        assert_eq!(doc.cel(0, 0), Some(&castle), "the generated picture");
        assert_eq!(doc.cel(0, 1), worked.cel(0, 1));
    };
    same_picture(app.engine.document());
    // One undo step: the shown page as it was; Redo: the spare's whole picture.
    ui_pass(&mut app, &ctx, t + 0.4, tap(egui::Key::U));
    let doc = app.engine.document();
    assert_eq!(doc.target(), shown.target());
    assert_eq!(doc.layers(), shown.layers());
    assert_eq!(doc.page(), shown.page());
    app.redo();
    same_picture(app.engine.document());
}

/// Wave-8 integration review: the same with frames: a generated spare of its own format
/// made into three frames keeps them all through Delete This Page.
#[test]
fn delete_this_page_keeps_every_frame_of_a_spare_of_its_own_format() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let t = generate_for(&mut app, &ctx, "A castle", GenTarget::Spare);
    click(&mut app, &ctx, t + 0.1, GenControl::Apply);
    ui_pass(&mut app, &ctx, t + 0.2, tap(egui::Key::J));
    app.engine.execute(Command::AddFrames { count: 2 }).unwrap();
    app.engine.execute(Command::Clear { color: 5 }).unwrap();
    let worked = app.engine.document().clone();
    assert_eq!(worked.frame_count(), 3);
    ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::J));
    assert!(app.spare_differs());
    app.delete_page();
    assert!(app.menu_data.spare.is_none(), "the spare is freed");
    let doc = app.engine.document();
    assert_eq!(doc.frame_count(), 3, "every frame of the spare");
    assert_eq!(doc.current_frame(), worked.current_frame());
    for f in 0..3 {
        assert_eq!(doc.cel(f, 0), worked.cel(f, 0), "frame {}", f + 1);
    }
}

/// Wave-8 integration review (ADR-46): the Brush and New layer targets keep an older OCS
/// project's palette, so the picture is matched against its colours as stored: nothing is
/// rounded, the brush has no colour to fit (the Apply line counts none) and the result's
/// palette is the picture's.
#[test]
fn the_brush_and_a_new_layer_on_an_old_ocs_project_use_its_colours_as_stored() {
    let (mut app, ctx) = app_with(Some(Arc::new(TestPattern)));
    let dir = open_old_ocs(&mut app, "keep");
    let stored = app.engine.document().palette().clone();
    app.open_generate();
    req_mut(&mut app).set_prompt("x");
    set_options(&mut app, |o| {
        o.palette = PaletteChoice::Current;
        o.target = GenTarget::Brush;
    });
    click(&mut app, &ctx, 0.01, GenControl::Generate);
    settle(&mut app, &ctx, 0.04);
    assert_eq!(req(&app).message(), Some("Apply: becomes the brush"));
    let (_, r) = app.imagegen.result.as_ref().expect("a result");
    assert_eq!((&r.palette, r.rounded_colors), (&stored, 0));
    set_options(&mut app, |o| o.target = GenTarget::NewLayer);
    app.generate_convert();
    assert_eq!(
        req(&app).message(),
        Some("Apply: as a new layer (undoable)")
    );
    let (_, r) = app.imagegen.result.as_ref().expect("a result");
    assert_eq!((&r.palette, r.rounded_colors), (&stored, 0));
    let _ = std::fs::remove_dir_all(&dir);
}
