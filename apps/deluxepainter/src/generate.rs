//! Picture ▸ Generate.. and the Image Generator setup (ADR-45): AI pictures in the GUI.
//!
//! The requester (`dpx_ui::generate_requester`) asks what to show and how; this module runs
//! the generation as a `dpx_imagegen::JobManager` job (a worker thread: the GUI never
//! blocks), shows the elapsed time and lets the user stop waiting, converts the picture with
//! `dpx_core::import::convert` for the requester's options (again after every option change,
//! without generating again) and shows it as a true-colour thumbnail. Nothing changes until
//! Apply:
//!
//! - **Spare page** (default): the spare page becomes the picture, with its own screen format
//!   and palette (one undo step on the spare page); the shown page stays untouched, `j` shows
//!   the picture.
//! - **Picture**: the picture replaces the shown page, its format and palette in one undo step
//!   (`Command::ReplacePicture`).
//! - **Brush**: the picture becomes the custom brush, fitted to the picture palette like a
//!   brush file (Brush ▸ Load: every colour to the nearest picture colour; Color ▸ Palette ▸
//!   Use Brush Palette then takes the generated palette).
//!
//! Without a generator that is set up the requester says so and offers Set up..; nothing is
//! generated or faked. The setup requester chooses the generator (None / OpenAI Images), takes
//! the API key (stored with `dpx_imagegen::ApiKeys`: the system keychain, never a file) and the
//! model id, tests the key with OpenAI's free model list and opens OpenAI's key page.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use dpx_core::import::{self, ConvertOptions, ConvertTarget, Imported, Margins, PaletteChoice};
use dpx_core::{BrushImage, Target};
use dpx_imagegen::openai::{self, DEFAULT_MODEL, ENV_KEY, KEY_ACCOUNT, MODELS_CHECKED};
use dpx_imagegen::{
    ApiKeys, GenError, GenerationRequest, ImageGenerationProvider, JobId, JobManager, JobState,
    KeyCheck, KeySource, OpenAiConfig, OpenAiProvider, ProviderStatus, Quality, Secret, Style,
};
use dpx_ui::{
    GenPhase, GenQuality, GenStyle, GenTarget, GenerateOptions, GenerateRequester,
    GenerateResponse, ImageGenSetupRequester, ProviderInfo, ReqEvent, SetupInfo, SetupProvider,
    SetupResponse, UiState,
};
use eframe::egui;
use serde_json::{Value, json};

use crate::{PainterApp, View, files};

/// What the requester says without a generator (spec: never a fake picture).
pub const NO_GENERATOR: &str =
    "No image generator is set up. Set one up, or paint or import instead.";

/// What the requester says about OpenAI before generating, the most important first (the
/// box may show only the start of it): `dpx_imagegen::openai::COST_NOTE` in short.
pub const OPENAI_NOTE: &str = "Your description goes over the internet to OpenAI. Every \
    picture is billed to your OpenAI account, also a cancelled one. No free tier; the price \
    depends on model, quality and size (see OpenAI's pricing page). Low quality is the \
    cheapest.";

/// The generator chosen in the setup (remembered in `settings.json`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenProvider {
    #[default]
    None,
    OpenAi,
}

impl GenProvider {
    pub fn name(self) -> &'static str {
        match self {
            GenProvider::None => "none",
            GenProvider::OpenAi => openai::ID,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(GenProvider::None),
            openai::ID => Some(GenProvider::OpenAi),
            _ => None,
        }
    }
}

/// The settings-file section `image_generator` (ADR-33): the generator and OpenAI's model,
/// quality and timeouts. The API key is never written there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageGenSettings {
    pub provider: GenProvider,
    pub openai: OpenAiConfig,
}

impl ImageGenSettings {
    pub fn to_json(&self) -> Value {
        json!({
            "provider": self.provider.name(),
            "openai": serde_json::to_value(&self.openai).unwrap_or(Value::Null),
        })
    }

    /// Unknown or broken fields keep their defaults.
    pub fn from_json(o: &serde_json::Map<String, Value>) -> Self {
        let d = Self::default();
        Self {
            provider: o
                .get("provider")
                .and_then(Value::as_str)
                .and_then(GenProvider::parse)
                .unwrap_or(d.provider),
            openai: o
                .get("openai")
                .and_then(|v| serde_json::from_value::<OpenAiConfig>(v.clone()).ok())
                .unwrap_or(d.openai),
        }
    }
}

/// The requester's last choices and description, for this session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Remembered {
    pub(crate) options: GenerateOptions,
    pub(crate) prompt: String,
    pub(crate) more: bool,
}

/// The app's image generation state.
pub(crate) struct ImageGenState {
    pub(crate) settings: ImageGenSettings,
    /// API keys: only this session's memory in tests and snapshots; the system keychain in
    /// the GUI ([`ApiKeys::system`], opened when Generate or the setup is first used).
    pub(crate) keys: Arc<ApiKeys>,
    /// The GUI uses the system keychain; it is opened on first use ([`Self::ensure_keys`]).
    pub(crate) system_keys: bool,
    pub(crate) jobs: JobManager,
    /// Tests and the clearly marked debug snapshot only: a provider used instead of the one
    /// set up (the test pattern). The GUI never sets it.
    pub(crate) debug_provider: Option<Arc<dyn ImageGenerationProvider>>,
    pub(crate) last: Option<Remembered>,
    /// The generation running for the requester.
    job: Option<JobId>,
    /// The generated picture (the first variant) and its conversion for the options it was
    /// made with.
    picture: Option<dpx_core::RgbaImage>,
    result: Option<(GenerateOptions, Imported)>,
    /// A key test on a worker thread.
    key_test: Option<Receiver<Result<KeyCheck, GenError>>>,
    /// The model list the key test asks (tests point it at a local server).
    pub(crate) models_endpoint: String,
    /// The job manager wakes the window when a job changes.
    waker: bool,
    /// A web page to open after this pass's events (Get key..), and the last one opened.
    open_url: Option<String>,
    pub(crate) opened_url: Option<String>,
}

impl Default for ImageGenState {
    fn default() -> Self {
        Self {
            settings: ImageGenSettings::default(),
            keys: Arc::new(ApiKeys::in_memory()),
            system_keys: false,
            jobs: JobManager::new(),
            debug_provider: None,
            last: None,
            job: None,
            picture: None,
            result: None,
            key_test: None,
            models_endpoint: openai::MODELS_ENDPOINT.to_string(),
            waker: false,
            open_url: None,
            opened_url: None,
        }
    }
}

impl ImageGenState {
    /// Opens the system keychain the first time it is needed (the GUI only), so starting
    /// DeluxePainter never touches it (on Linux it means a D-Bus connection).
    pub(crate) fn ensure_keys(&mut self) {
        if std::mem::take(&mut self.system_keys) {
            self.keys = Arc::new(ApiKeys::system());
        }
    }
}

fn quality_of(q: GenQuality) -> Quality {
    match q {
        GenQuality::Low => Quality::Low,
        GenQuality::Medium => Quality::Medium,
        GenQuality::High => Quality::High,
        GenQuality::Auto => Quality::Auto,
    }
}

fn gen_quality(q: Quality) -> GenQuality {
    match q {
        Quality::Low => GenQuality::Low,
        Quality::Medium => GenQuality::Medium,
        Quality::High => GenQuality::High,
        Quality::Auto => GenQuality::Auto,
    }
}

fn style_of(s: GenStyle) -> Style {
    match s {
        GenStyle::PixelArt => Style::PixelArt,
        GenStyle::Painted => Style::Painted,
        GenStyle::None => Style::None,
    }
}

/// The conversion for the requester's options: transparent margins come with a transparent
/// background, BG-coloured ones otherwise.
pub(crate) fn convert_options(o: &GenerateOptions) -> ConvertOptions {
    ConvertOptions {
        framing: o.framing,
        scaler: o.scaler,
        margins: if o.transparent {
            Margins::Transparent
        } else {
            Margins::Bg
        },
        palette: o.palette,
        colors: o.colors,
        dither: o.dither,
        transparent_background: o.transparent,
    }
}

/// The line under the options once there is a result: what Apply does.
pub(crate) fn apply_text(t: GenTarget) -> &'static str {
    match t {
        GenTarget::Spare => "Apply: onto the spare page (j shows it)",
        GenTarget::Picture => "Apply: replaces the picture (undoable)",
        GenTarget::Brush => "Apply: becomes the brush",
    }
}

/// The UI rectangles of the Generate and setup requesters: they cover the toolbox column, so
/// marks drawn over the composed screen stay under them (as under the Import requester).
pub(crate) fn kept_rects(ui: &UiState) -> Vec<dpx_core::Rect> {
    let mut out = Vec::new();
    if let Some(r) = &ui.generate_requester {
        let ((x, y), (w, h)) = (r.origin(), r.size());
        out.push(dpx_core::Rect::new(x, y, w, h));
    }
    if let Some(r) = &ui.imagegen_setup {
        let ((x, y), (w, h)) = (r.origin(), r.size());
        out.push(dpx_core::Rect::new(x, y, w, h));
    }
    out
}

impl PainterApp {
    /// The provider Generate uses: the debug one in tests, else the one set up.
    fn imagegen_provider(&self) -> Option<Arc<dyn ImageGenerationProvider>> {
        if let Some(p) = &self.imagegen.debug_provider {
            return Some(p.clone());
        }
        match self.imagegen.settings.provider {
            GenProvider::None => None,
            GenProvider::OpenAi => Some(Arc::new(OpenAiProvider::new(
                self.imagegen.settings.openai.clone(),
                self.imagegen.keys.clone(),
            ))),
        }
    }

    /// What the requester says about the generator.
    fn provider_info(&self) -> ProviderInfo {
        let not_set_up = |summary: String| ProviderInfo {
            ready: false,
            summary,
            note: format!("{NO_GENERATOR} Set up.. chooses a generator and its API key."),
        };
        let Some(p) = self.imagegen_provider() else {
            return not_set_up("No image generator is set up.".into());
        };
        match p.status() {
            ProviderStatus::NotConfigured(why) => {
                not_set_up(format!("{} is not set up: {why}.", p.display_name()))
            }
            ProviderStatus::Ready => {
                let openai = self.imagegen.debug_provider.is_none()
                    && self.imagegen.settings.provider == GenProvider::OpenAi;
                let (summary, note) = if openai {
                    (
                        "OpenAI Images: billed per picture".to_string(),
                        format!(
                            "{OPENAI_NOTE} Model: {}.",
                            self.imagegen.settings.openai.model
                        ),
                    )
                } else if p.needs_network() {
                    (
                        format!("{}: over the internet", p.display_name()),
                        p.cost_note().to_string(),
                    )
                } else {
                    (p.display_name().to_string(), p.cost_note().to_string())
                };
                ProviderInfo {
                    ready: true,
                    summary,
                    note,
                }
            }
        }
    }

    /// Picture ▸ Generate..: opens the requester with the last choices of this session.
    pub(crate) fn open_generate(&mut self) {
        self.imagegen.ensure_keys();
        self.finish_stroke();
        self.ui.menu.close();
        let doc = self.engine.document();
        let screen = View::of_target(doc.target()).ui_size();
        let doc_colors = doc.palette().len().min(256) as u16;
        let last = self.imagegen.last.clone().unwrap_or_else(|| Remembered {
            options: GenerateOptions {
                quality: gen_quality(self.imagegen.settings.openai.quality),
                ..GenerateOptions::default()
            },
            prompt: String::new(),
            more: false,
        });
        let mut req = GenerateRequester::new(
            last.options,
            &last.prompt,
            doc_colors,
            screen,
            self.provider_info(),
        );
        req.set_more(last.more);
        self.ui.generate_requester = Some(req);
        self.imagegen.picture = None;
        self.imagegen.result = None;
        self.dirty = true;
    }

    /// Opens the Image Generator setup (over the Generate requester when that is open).
    // TODO(wave8-integration): Prefs ▸ "AI & MCP.." (ADR-43) gets a button that calls this.
    pub(crate) fn open_imagegen_setup(&mut self) {
        self.imagegen.ensure_keys();
        self.finish_stroke();
        self.ui.menu.close();
        let s = &self.imagegen.settings;
        let provider = match s.provider {
            GenProvider::None => SetupProvider::None,
            GenProvider::OpenAi => SetupProvider::OpenAi,
        };
        let screen = View::of_target(self.engine.document().target()).ui_size();
        let req = ImageGenSetupRequester::new(provider, &s.openai.model, self.setup_info(), screen);
        self.ui.imagegen_setup = Some(req);
        self.imagegen.key_test = None;
        self.dirty = true;
    }

    /// The setup's lines about the key (never the key).
    fn setup_info(&self) -> SetupInfo {
        let status = self.imagegen.keys.status(KEY_ACCOUNT, ENV_KEY);
        let mut key_status = status.describe();
        if status.source == Some(KeySource::Session)
            && let Some(why) = &status.session_only
        {
            key_status = format!("API key kept for this session only ({why})");
        }
        SetupInfo {
            key_status,
            has_key: status.source.is_some(),
            removable: matches!(
                status.source,
                Some(KeySource::Keychain | KeySource::Session)
            ),
            model_hint: format!("Default {DEFAULT_MODEL}"),
            note: format!(
                "OpenAI bills each picture to your API account, also a cancelled one; no free \
                 tier. Your text goes over the internet. The key goes to the system \
                 keychain, never to a file. Test only lists your models. Default model checked \
                 {MODELS_CHECKED}."
            ),
        }
    }

    /// Pointer and keys while the Generate requester or the setup is open (both modal).
    pub(crate) fn generate_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        if !self.imagegen.waker {
            let c = ctx.clone();
            self.imagegen
                .jobs
                .set_waker(Arc::new(move || c.request_repaint()));
            self.imagegen.waker = true;
        }
        if self.generate_poll() || self.imagegen.key_test.is_some() {
            // The elapsed seconds and the key test's answer.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.key_test_poll();
        let at = match (&self.ui.imagegen_setup, &self.ui.generate_requester) {
            (Some(s), _) => s.origin(),
            (None, Some(g)) => g.origin(),
            (None, None) => return,
        };
        // A held Return (the one that picked the menu item) does not answer.
        let (events, _) = self.modal_events_with(ctx, origin, ppp, scale, view, at, false);
        for event in events {
            if self.ui.imagegen_setup.is_some() {
                self.setup_event(event);
            } else if self.ui.generate_requester.is_some() {
                self.generate_event(event);
            } else {
                break;
            }
        }
        if let Some(url) = self.imagegen.open_url.take() {
            ctx.open_url(egui::OpenUrl::new_tab(&url));
            self.imagegen.opened_url = Some(url);
        }
    }

    /// Feeds one event to the Generate requester.
    pub(crate) fn generate_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.generate_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        match response {
            GenerateResponse::Continue => {}
            GenerateResponse::Changed => self.generate_convert(),
            GenerateResponse::Generate => self.start_generation(),
            GenerateResponse::CancelJob => {
                if let Some(id) = self.imagegen.job {
                    self.imagegen.jobs.cancel(id);
                }
                self.generate_poll();
            }
            GenerateResponse::Cancel => self.close_generate(),
            GenerateResponse::Apply => self.generate_apply(),
            GenerateResponse::SetUp => self.open_imagegen_setup(),
        }
    }

    /// Closes the requester (remembering its choices); a running job is stopped.
    fn close_generate(&mut self) {
        if let Some(id) = self.imagegen.job.take() {
            self.imagegen.jobs.cancel(id);
            self.imagegen.jobs.forget(id);
        }
        if let Some(req) = self.ui.generate_requester.take() {
            self.imagegen.last = Some(Remembered {
                options: req.options(),
                prompt: req.prompt(),
                more: req.more(),
            });
        }
        self.imagegen.picture = None;
        self.imagegen.result = None;
    }

    /// Starts a generation for the requester's description and options.
    fn start_generation(&mut self) {
        let Some(req) = &self.ui.generate_requester else {
            return;
        };
        let o = req.options();
        let description = req.prompt();
        // The quality is the provider's setting (remembered); the provider is made with it.
        self.imagegen.settings.openai.quality = quality_of(o.quality);
        let info = self.provider_info();
        let provider = self.imagegen_provider().filter(|_| info.ready);
        let colors = match o.palette {
            PaletteChoice::New => o.colors,
            PaletteChoice::Current => self.engine.document().palette().len() as u16,
        };
        let target = o.target();
        let submitted = provider.map(|p| {
            let prompt = style_of(o.style).prompt(&description, colors);
            let mut request = GenerationRequest::for_page(
                prompt,
                &p.capabilities(),
                target.screen,
                target.mode.pixel_shape(),
            );
            request.transparent_background = o.transparent;
            self.imagegen.jobs.submit(p, request)
        });
        let Some(req) = &mut self.ui.generate_requester else {
            return;
        };
        req.set_provider(info);
        match submitted {
            None => req.set_error(Some(NO_GENERATOR.into())),
            Some(Ok(id)) => {
                self.imagegen.job = Some(id);
                self.imagegen.picture = None;
                self.imagegen.result = None;
                req.set_phase(GenPhase::Working { seconds: 0 });
                req.set_error(None);
                req.set_message(None);
            }
            Some(Err(e)) => {
                req.set_phase(GenPhase::Ask);
                req.set_error(Some(e.to_string()));
            }
        }
    }

    /// Follows the running job: the elapsed time, then its picture or why it failed. Returns
    /// whether it is still running.
    pub(crate) fn generate_poll(&mut self) -> bool {
        let Some(id) = self.imagegen.job else {
            return false;
        };
        let Some(status) = self.imagegen.jobs.status(id) else {
            self.imagegen.job = None;
            return false;
        };
        if status.state == JobState::Running {
            if let Some(req) = &mut self.ui.generate_requester {
                let seconds = status.elapsed.as_secs();
                if req.phase() != (GenPhase::Working { seconds }) {
                    req.set_phase(GenPhase::Working { seconds });
                    self.dirty = true;
                }
            }
            return true;
        }
        let images = self.imagegen.jobs.images(id);
        self.imagegen.jobs.forget(id);
        self.imagegen.job = None;
        self.dirty = true;
        let network = self.imagegen_provider().is_some_and(|p| p.needs_network());
        if let Some(req) = &mut self.ui.generate_requester {
            match status.state {
                JobState::Done => {}
                JobState::Failed => {
                    req.set_phase(GenPhase::Ask);
                    let why = status
                        .error
                        .map_or_else(|| "it failed".into(), |e| e.to_string());
                    req.set_error(Some(format!("No picture: {why}.")));
                }
                _ => {
                    req.set_phase(GenPhase::Ask);
                    req.set_error(Some(if network {
                        "Stopped waiting. A request already sent may still be billed.".into()
                    } else {
                        "Stopped.".into()
                    }));
                }
            }
        }
        if status.state == JobState::Done {
            self.imagegen.picture = images.and_then(|i| i.first().cloned());
            self.generate_convert();
        }
        false
    }

    /// Converts the generated picture for the requester's options and shows the thumbnail.
    fn generate_convert(&mut self) {
        let (Some(req), Some(picture)) = (&mut self.ui.generate_requester, &self.imagegen.picture)
        else {
            return;
        };
        let o = req.options();
        let target = o.target();
        let doc = self.engine.document();
        let t = ConvertTarget {
            target,
            page: target.screen,
            palette: doc.palette(),
            bg: self.ui.bg,
            transparent: files::TRANSPARENT_PINK,
        };
        req.set_phase(GenPhase::Result);
        match import::convert(picture, &convert_options(&o), &t) {
            Ok(r) => {
                let (w, h) = req.thumb_size();
                req.set_thumbnail(Some(import::thumbnail(&r.page, &r.palette, w, h)));
                req.set_error(None);
                req.set_message(Some(apply_text(o.target).into()));
                self.imagegen.result = Some((o, r));
            }
            Err(e) => {
                req.set_thumbnail(None);
                req.set_error(Some(format!("This picture cannot be made: {}.", e.message)));
                req.set_message(None);
                self.imagegen.result = None;
            }
        }
        self.dirty = true;
    }

    /// Apply: the converted picture goes to its target; the requester closes.
    fn generate_apply(&mut self) {
        let Some((o, r)) = self.imagegen.result.take() else {
            return;
        };
        let target = o.target();
        let applied = match o.target {
            GenTarget::Spare => {
                self.spare_from_generated(&r, target);
                Ok(())
            }
            GenTarget::Picture => self.picture_from_generated(&r, target),
            GenTarget::Brush => {
                self.brush_from_generated(&r);
                Ok(())
            }
        };
        match applied {
            Ok(()) => self.close_generate(),
            Err(e) => {
                if let Some(req) = &mut self.ui.generate_requester {
                    req.set_error(Some(format!("Not applied: {e}.")));
                }
                self.imagegen.result = Some((o, r));
            }
        }
        self.dirty = true;
    }

    /// Spare page: it becomes the picture with its own format and palette (undoable on the
    /// spare page); the shown page is not touched.
    fn spare_from_generated(&mut self, r: &Imported, target: Target) {
        let mut other = match self.menu_data.spare.take() {
            Some(s) => s,
            None => self.blank_page(),
        };
        if other.engine.execute(r.replace_command(target)).is_err() {
            other.engine = dpx_core::Engine::new(dpx_core::Document::from_picture(
                r.page.clone(),
                r.palette.clone(),
                target,
            ));
        }
        other.scroll = (0, 0);
        other.picture_path = None;
        other.background = None;
        other.bg = 0;
        self.menu_data.spare = Some(other);
        self.menu_data.spare_own = Some(files::PictureExtras {
            ranges: Vec::new(),
            grab: None,
            transparent_color: r.transparent_color.map(u16::from),
        });
    }

    /// Picture: one undoable command replaces page, format and palette.
    fn picture_from_generated(&mut self, r: &Imported, target: Target) -> Result<(), String> {
        self.finish_stroke();
        self.engine
            .execute(r.replace_command(target))
            .map_err(|e| e.message)?;
        if let Some(t) = r.transparent_color {
            self.extras.transparent_color = Some(u16::from(t));
        }
        self.menu_data.cycled = None;
        self.scroll = (0, 0);
        self.clamp_colors();
        self.clamp_scroll();
        Ok(())
    }

    /// Brush: the picture in the picture palette's colours, like a loaded brush file (its own
    /// palette stays for Use Brush Palette); transparent pixels are its transparent colour.
    fn brush_from_generated(&mut self, r: &Imported) {
        let own = BrushImage::from_image(&r.page, r.transparent_color);
        let palette = self.engine.document().palette().clone();
        let (brush, _) = crate::brush_menu::fit_brush(&own, &r.palette, &palette);
        self.remember_loaded_brush(own, r.palette.clone(), Vec::new(), brush.clone());
        self.custom_brush = Some(brush.clone());
        self.picked_brush = Some(brush);
        self.tools.brush_gen += 1;
        self.leave_anim_brush();
        self.tools.brush_palette = Some(palette);
    }

    /// Feeds one event to the setup requester.
    pub(crate) fn setup_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.imagegen_setup else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        match response {
            SetupResponse::Continue => {}
            SetupResponse::Test => self.start_key_test(),
            SetupResponse::GetKey => self.imagegen.open_url = Some(openai::KEYS_PAGE.into()),
            SetupResponse::Cancel => self.close_setup(),
            SetupResponse::Ok => self.setup_ok(),
        }
    }

    fn close_setup(&mut self) {
        self.ui.imagegen_setup = None;
        self.imagegen.key_test = None;
        // The Generate requester below shows the generator as it is now.
        let info = self.provider_info();
        if let Some(req) = &mut self.ui.generate_requester {
            if info.ready && req.error() == Some(NO_GENERATOR) {
                req.set_error(None);
            }
            req.set_provider(info);
        }
    }

    /// Ok: the generator and model go to the settings, a typed key to the keychain; Remove
    /// key forgets the stored one. A key or model that cannot be used keeps the requester open.
    fn setup_ok(&mut self) {
        let Some(req) = &mut self.ui.imagegen_setup else {
            return;
        };
        let provider = match req.provider() {
            SetupProvider::None => GenProvider::None,
            SetupProvider::OpenAi => GenProvider::OpenAi,
        };
        let mut config = self.imagegen.settings.openai.clone();
        if provider == GenProvider::OpenAi {
            config.model = match req.model() {
                m if m.is_empty() => DEFAULT_MODEL.to_string(),
                m => m,
            };
            if !config.model_is_valid() {
                req.set_test(Some(
                    "The model id must be 1 to 64 characters without spaces.".into(),
                ));
                return;
            }
            if let Some(typed) = req.typed_key() {
                let Some(secret) = Secret::new(&typed) else {
                    req.set_test(Some(
                        "That is no API key: printable characters without spaces only.".into(),
                    ));
                    return;
                };
                self.imagegen.keys.set(KEY_ACCOUNT, &secret);
            } else if req.remove_key()
                && let Err(e) = self.imagegen.keys.clear(KEY_ACCOUNT)
            {
                req.set_test(Some(format!("The key could not be removed: {e}.")));
                return;
            }
        }
        self.imagegen.settings.provider = provider;
        self.imagegen.settings.openai = config;
        self.close_setup();
    }

    /// Test: asks OpenAI's model list with the typed key (else the stored one) on a worker.
    fn start_key_test(&mut self) {
        let Some(req) = &mut self.ui.imagegen_setup else {
            return;
        };
        let secret = match req.typed_key() {
            Some(k) => match Secret::new(&k) {
                Some(s) => Some(s),
                None => {
                    req.set_test(Some(
                        "That is no API key: printable characters without spaces only.".into(),
                    ));
                    return;
                }
            },
            None => self.imagegen.keys.key(KEY_ACCOUNT, ENV_KEY).map(|(s, _)| s),
        };
        let Some(secret) = secret else {
            req.set_test(Some("Enter or paste an API key first.".into()));
            return;
        };
        let mut config = self.imagegen.settings.openai.clone();
        if !req.model().is_empty() {
            config.model = req.model();
        }
        let url = self.imagegen.models_endpoint.clone();
        let env_proxy = url == openai::MODELS_ENDPOINT;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("dpx-imagegen-key-test".into())
            .spawn(move || {
                let _ = tx.send(openai::check_key_at(&url, &config, &secret, env_proxy));
            })
            .ok();
        self.imagegen.key_test = Some(rx);
        req.set_test(Some(
            "Testing the key: asking OpenAI for your model list..".into(),
        ));
    }

    /// Shows the key test's answer once it is there.
    pub(crate) fn key_test_poll(&mut self) {
        let Some(rx) = &self.imagegen.key_test else {
            return;
        };
        let text = match rx.try_recv() {
            Ok(Ok(check)) => check.describe(),
            Ok(Err(e)) => format!("{e}."),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => "The test stopped unexpectedly.".into(),
        };
        self.imagegen.key_test = None;
        if let Some(req) = &mut self.ui.imagegen_setup {
            req.set_test(Some(text));
        }
        self.dirty = true;
    }

    /// Paints the thumbnail into the RGBA screen (`frame_w` pixels wide, one pixel per screen
    /// pixel) unless the setup or the quit question covers it.
    pub(crate) fn generate_overlay(&self, rgba: &mut [u8], frame_w: usize) {
        if self.ui.choice_requester.is_some() {
            return;
        }
        let v = self.view();
        overlay(&self.ui, rgba, frame_w, v.ux as usize, v.uy as usize);
    }
}

/// The Generate thumbnail over an RGBA frame, `sx`×`sy` frame pixels per UI pixel; left out
/// while the setup requester covers it.
pub(crate) fn overlay(ui: &UiState, rgba: &mut [u8], frame_w: usize, sx: usize, sy: usize) {
    if ui.imagegen_setup.is_some() {
        return;
    }
    if let Some(req) = &ui.generate_requester {
        req.overlay(rgba, frame_w, sx, sy);
    }
}

/// Parses `--generate 'key=value;…'` (snapshots): `prompt`, `resolution` (0-6), `chipset`
/// (ocs/aga), `colors`, `style` (pixel_art/painted/none), `palette` (new/current), `dither`
/// (none/diffusion/ordered), `fit` (crop/whole), `scaler` (smooth/nearest/pixel_art),
/// `background` (opaque/transparent), `quality` (low/medium/high/auto), `target`
/// (spare/picture/brush), `more`, `provider` (none/openai/test_pattern) and `key=placeholder`
/// (a placeholder key in this session's memory, so OpenAI shows as set up; nothing is sent).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Spec {
    pub(crate) options: GenerateOptions,
    pub(crate) prompt: String,
    pub(crate) more: bool,
    pub(crate) provider: Option<String>,
    pub(crate) placeholder_key: bool,
}

pub(crate) fn parse_spec(spec: &str) -> Result<Spec, String> {
    use dpx_core::import::{Framing, Scaler};
    use dpx_core::quantize::Dither;
    let mut s = Spec::default();
    let mut colors_given = false;
    let o = &mut s.options;
    for part in spec.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        if part == "more" {
            s.more = true;
            continue;
        }
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("'{part}' is not key=value"))?;
        let bad = || format!("bad {key} '{value}'");
        match key.trim() {
            "prompt" => s.prompt = value.to_string(),
            "resolution" => {
                o.resolution = value
                    .parse()
                    .ok()
                    .filter(|&i: &usize| i < dpx_ui::generate_requester::RESOLUTIONS.len())
                    .ok_or_else(bad)?
            }
            "chipset" => {
                o.chipset = match value {
                    "ocs" => dpx_core::Chipset::Ocs,
                    "aga" => dpx_core::Chipset::Aga,
                    _ => return Err(bad()),
                }
            }
            "colors" | "colours" => {
                colors_given = true;
                o.colors = value
                    .parse()
                    .ok()
                    .filter(|n| import::COLOR_COUNTS.contains(n))
                    .ok_or_else(bad)?
            }
            "style" => {
                o.style = match value {
                    "pixel_art" => GenStyle::PixelArt,
                    "painted" => GenStyle::Painted,
                    "none" => GenStyle::None,
                    _ => return Err(bad()),
                }
            }
            "palette" => {
                o.palette = crate::import::parse_palette(value).ok_or_else(bad)?;
            }
            "dither" => {
                o.dither = match value {
                    "none" => Dither::None,
                    "diffusion" | "fs" => Dither::FloydSteinberg,
                    "ordered" => Dither::Ordered,
                    _ => return Err(bad()),
                }
            }
            "fit" => {
                o.framing = match value {
                    "crop" => Framing::Crop,
                    "whole" => Framing::Fit,
                    _ => return Err(bad()),
                }
            }
            "scaler" => {
                o.scaler = match value {
                    "smooth" => Scaler::Smooth,
                    "nearest" => Scaler::Nearest,
                    "pixel_art" => Scaler::PixelArt,
                    _ => return Err(bad()),
                }
            }
            "background" => {
                o.transparent = match value {
                    "opaque" => false,
                    "transparent" => true,
                    _ => return Err(bad()),
                }
            }
            "quality" => {
                o.quality = match value {
                    "low" => GenQuality::Low,
                    "medium" => GenQuality::Medium,
                    "high" => GenQuality::High,
                    "auto" => GenQuality::Auto,
                    _ => return Err(bad()),
                }
            }
            "target" => {
                o.target = match value {
                    "spare" => GenTarget::Spare,
                    "picture" => GenTarget::Picture,
                    "brush" => GenTarget::Brush,
                    _ => return Err(bad()),
                }
            }
            "provider" => match value {
                "none" | "openai" | "test_pattern" => s.provider = Some(value.to_string()),
                _ => return Err(bad()),
            },
            "key" if value == "placeholder" => s.placeholder_key = true,
            _ => return Err(format!("unknown key '{key}'")),
        }
    }
    if !colors_given {
        // The default count, lowered to what the resolution shows.
        s.options = s.options.clamped();
    } else if s.options.colors > s.options.max_colors() {
        return Err(format!(
            "colors {} is more than this resolution shows on {:?} ({})",
            s.options.colors,
            s.options.chipset,
            s.options.max_colors()
        ));
    }
    Ok(s)
}

/// Snapshot options (ADR-45): `--generate-requester` shows Picture ▸ Generate.. over the
/// current screen, `--generate SPEC` sets it up ([`parse_spec`]), `--generate-run` generates
/// with the **test pattern (not AI, debug only)** and shows the result, `--generate-apply`
/// shows the picture after Apply instead, and `--imagegen-setup` shows the Image Generator
/// setup. No key store, network or settings file is touched. Returns the document to show
/// (the spare page after Apply onto it). Errors carry the exit code 2.
pub(crate) fn snapshot(
    args: &[String],
    doc: &dpx_core::Document,
    ui: &mut UiState,
) -> Result<dpx_core::Document, (i32, String)> {
    let has = |f: &str| args.iter().any(|a| a == f);
    let spec = crate::arg_value(args, "--generate");
    let (requester, run, apply, setup) = (
        has("--generate-requester"),
        has("--generate-run"),
        has("--generate-apply"),
        has("--imagegen-setup"),
    );
    if !(requester || setup) {
        if spec.is_some() || run || apply {
            return Err((
                2,
                "--generate, --generate-run and --generate-apply need --generate-requester".into(),
            ));
        }
        return Ok(doc.clone());
    }
    let spec =
        parse_spec(spec.as_deref().unwrap_or("")).map_err(|e| (2, format!("--generate: {e}")))?;
    let mut app = PainterApp::new();
    app.engine = dpx_core::Engine::new(doc.clone());
    app.ui = ui.clone();
    match spec.provider.as_deref() {
        Some("openai") => app.imagegen.settings.provider = GenProvider::OpenAi,
        Some("test_pattern") => {
            app.imagegen.debug_provider = Some(Arc::new(dpx_imagegen::fixture::TestPattern));
        }
        _ => {}
    }
    if spec.placeholder_key {
        // Only so the snapshot shows OpenAI as set up: in memory, never sent anywhere.
        let key = Secret::new("sk-placeholder-for-snapshots").expect("valid");
        app.imagegen.keys.set(KEY_ACCOUNT, &key);
    }
    if (run || apply) && app.imagegen.debug_provider.is_none() {
        return Err((
            2,
            "--generate-run and --generate-apply need provider=test_pattern".into(),
        ));
    }
    if requester {
        app.imagegen.last = Some(Remembered {
            options: spec.options,
            prompt: spec.prompt.clone(),
            more: spec.more,
        });
        app.open_generate();
        if run || apply {
            app.start_generation();
            if let Some(id) = app.imagegen.job {
                app.imagegen
                    .jobs
                    .wait(id, std::time::Duration::from_secs(30));
            }
            app.generate_poll();
            if app.imagegen.result.is_none() {
                let why = app
                    .ui
                    .generate_requester
                    .as_ref()
                    .and_then(|r| r.error().map(str::to_owned))
                    .unwrap_or_default();
                return Err((2, format!("--generate-run: no result ({why})")));
            }
        }
        if apply {
            app.generate_apply();
            if spec.options.target == GenTarget::Spare {
                app.swap_spare();
            }
        }
    }
    if setup {
        app.open_imagegen_setup();
    }
    *ui = app.ui.clone();
    Ok(app.engine.document().clone())
}

#[cfg(test)]
mod tests;
