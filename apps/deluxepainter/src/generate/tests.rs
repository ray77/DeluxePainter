//! ui-loop tests of Picture ▸ Generate.. and the Image Generator setup (ADR-45): real egui
//! events through `PainterApp::ui`; generations use the deterministic test pattern or test
//! providers (no network, no key store).

use std::io::{Read, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use dpx_core::{Chipset, DisplayMode, Rgb8};
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
    assert_eq!(req(&app).message(), Some(apply_text(GenTarget::Spare)));
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
    assert_eq!(req(&app).message(), Some(apply_text(GenTarget::Picture)));
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
    for t in [GenTarget::Spare, GenTarget::Picture, GenTarget::Brush] {
        assert!(font.text_width(apply_text(t)) <= 300, "{}", apply_text(t));
    }
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
        "nonsense",
        "size=3",
    ] {
        assert!(parse_spec(bad).is_err(), "{bad}");
    }
    assert_eq!(parse_spec("").unwrap().options, GenerateOptions::default());
    // Without colors the default count follows the resolution (OCS HiRes: 16).
    assert_eq!(parse_spec("resolution=3").unwrap().options.colors, 16);
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
