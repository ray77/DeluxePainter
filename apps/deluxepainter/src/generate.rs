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
//!   and palette (one undo step on the spare page, which also brings back its file name,
//!   fixed background, BG colour and colour ranges); the shown page stays untouched, `j`
//!   shows the picture.
//! - **Picture**: the picture replaces the whole picture, every animation frame, its format
//!   and palette in one undo step (`Command::ReplacePicture`; the Apply line says how many
//!   frames go); Undo also brings back its colour ranges and transparent colour
//!   ([`page_state`]).
//! - **Brush**: the picture becomes the custom brush, fitted to the picture palette like a
//!   brush file (Brush ▸ Load: every colour to the nearest picture colour, never onto the
//!   transparent one; Color ▸ Palette ▸ Use Brush Palette then takes the generated palette).
//!   The thumbnail shows the fitted brush, and the Apply line how many colours were fitted.
//! - **New layer** (ADR-44): the picture becomes a new layer above the active one, in the
//!   picture's page size and palette (`import::convert_for_layer`: colour 0 is transparent on
//!   that layer, so only transparent pixels take it), in the current frame (the layer is
//!   clear in the other frames); one undo step (`Command::AddPictureLayer`). The thumbnail
//!   shows the frame with the new layer on top.
//!
//! MCP's `apply_generated_image` (ADR-45, wave-8 integration) applies through the same
//! function as Apply ([`PainterApp::apply_generated`]).
//!
//! "Again" keeps the result shown until the new picture is there: a new generation that fails
//! or is stopped leaves it (and Apply) as it was.
//!
//! Without a generator that is set up the requester says so and offers Set up..; nothing is
//! generated or faked. The setup requester chooses the generator (None / OpenAI Images), takes
//! the API key (stored with `dpx_imagegen::ApiKeys`: the system keychain, never a file) and the
//! model id, tests the key with OpenAI's free model list and opens OpenAI's key page.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use dpx_core::import::{self, ConvertOptions, ConvertTarget, Imported, Margins, PaletteChoice};
use dpx_core::layer::{Layer, MAX_LAYERS};
use dpx_core::{BrushImage, Command, Palette, Target};
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

pub(crate) mod page_state;

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
    /// The generations MCP clients started (ADR-45, `generate_image`); they share `jobs`
    /// with the requester, and with it the limit of requests in flight.
    pub(crate) mcp_plans: dpx_mcp::generation::GenPlans,
    /// The generation running for the requester.
    job: Option<JobId>,
    /// The generated picture (the first variant) and its conversion for the options it was
    /// made with.
    picture: Option<dpx_core::RgbaImage>,
    result: Option<(GenerateOptions, Imported)>,
    /// A key test on a worker thread, with the setup's revision it was made for (an answer
    /// for a key or model edited since is dropped).
    key_test: Option<(Receiver<Result<KeyCheck, GenError>>, u64)>,
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
            mcp_plans: Default::default(),
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

/// The line under the options once there is a result: what Apply does. `frames`: the
/// picture's frames (Picture replaces them all); `fitted`: the brush colours that had to be
/// fitted to the picture palette (as Brush ▸ Load says in its note).
pub(crate) fn apply_text(t: GenTarget, frames: usize, fitted: usize) -> String {
    match t {
        GenTarget::Spare => "Apply: onto the spare page (j shows it)".into(),
        GenTarget::Picture if frames > 1 => {
            format!("Apply: replaces all {frames} frames (undoable)")
        }
        GenTarget::Picture => "Apply: replaces the picture (undoable)".into(),
        GenTarget::Brush if fitted > 0 => {
            format!("Apply: as the brush, {fitted} colours fitted")
        }
        GenTarget::Brush => "Apply: becomes the brush".into(),
        GenTarget::NewLayer => "Apply: as a new layer (undoable)".into(),
    }
}

/// Why no new layer can be added: the picture has the most layers allowed.
pub(crate) fn layers_full() -> String {
    format!("the picture has {MAX_LAYERS} layers, the most allowed: delete or merge one first")
}

/// The name of a generated picture's layer: "Generated", or "Generated N" (the smallest N
/// from 2 that is not taken).
pub(crate) fn layer_name(layers: &[Layer]) -> String {
    let taken = |n: &str| layers.iter().any(|l| l.name == n);
    if !taken("Generated") {
        return "Generated".into();
    }
    (2..)
        .map(|n| format!("Generated {n}"))
        .find(|n| !taken(n))
        .expect("fewer layers than numbers")
}

/// The conversion of a generated picture for `to` on a document: a new layer is made in the
/// document's palette with colour 0 kept for transparency ([`import::convert_for_layer`]),
/// every other target with [`import::convert`]. One entry point for Generate and MCP.
pub(crate) fn convert_for(
    picture: &dpx_core::RgbaImage,
    opts: &ConvertOptions,
    t: &ConvertTarget,
    to: GenTarget,
) -> Result<Imported, dpx_core::CommandError> {
    match to {
        GenTarget::NewLayer => import::convert_for_layer(picture, opts, t),
        _ => import::convert(picture, opts, t),
    }
}

/// The Brush target's brush: the result as a brush in its own palette, and fitted to the
/// picture palette `page` like Brush ▸ Load (`Imported::fitted_brush`, which MCP's brush
/// target uses too), with the number of colours that had to change.
fn fitted_brush(r: &Imported, page: &Palette) -> (BrushImage, BrushImage, usize) {
    let b = r.fitted_brush(page);
    (b.own, b.fitted, b.changed)
}

/// At most `max` characters of `why`, cut at a word (for the setup's test line).
fn short(why: &str, max: usize) -> String {
    if why.chars().count() <= max {
        return why.to_string();
    }
    let cut: String = why.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a);
    format!("{}..", cut.trim_end_matches([',', ';', ':']))
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
    /// The provider Generate (and MCP's `generate_image`) uses: the debug one in tests, else
    /// the one set up.
    pub(crate) fn imagegen_provider(&self) -> Option<Arc<dyn ImageGenerationProvider>> {
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

    /// What MCP's generation tools share with Picture ▸ Generate.. (ADR-45): the job manager
    /// (one limit of requests in flight), the generator set up (the keychain is opened
    /// first when it is OpenAI), its model, the MCP jobs and the defaults: the requester's
    /// choices of this session, else the shown picture's format with the requester's
    /// default options.
    pub(crate) fn mcp_gen_host(&mut self) -> dpx_mcp::generation::GenHost {
        if self.imagegen.debug_provider.is_none()
            && self.imagegen.settings.provider == GenProvider::OpenAi
        {
            self.imagegen.ensure_keys();
        }
        let model = (self.imagegen.debug_provider.is_none()
            && self.imagegen.settings.provider == GenProvider::OpenAi)
            .then(|| self.imagegen.settings.openai.model.clone());
        let defaults = match &self.imagegen.last {
            Some(l) => {
                let o = l.options.clamped();
                let format = o.target();
                dpx_mcp::generation::GenDefaults {
                    format,
                    page: format.screen,
                    colors: o.colors,
                    convert: convert_options(&o),
                    style: style_of(o.style),
                }
            }
            None => {
                let o = GenerateOptions::default();
                let format = self.engine.document().target();
                let max = format.mode.max_colors(format.chipset).unwrap_or(16);
                let own = self.engine.document().palette().len().min(256) as u16;
                dpx_mcp::generation::GenDefaults {
                    format,
                    page: format.screen,
                    colors: import::color_count(own.max(2), max),
                    convert: convert_options(&o),
                    style: style_of(o.style),
                }
            }
        };
        dpx_mcp::generation::GenHost {
            jobs: self.imagegen.jobs.clone(),
            provider: self.imagegen_provider(),
            model,
            plans: self.imagegen.mcp_plans.clone(),
            defaults,
        }
    }

    /// Prefs ▸ AI & MCP..'s state of the image generator, under "Image generation:" (ADR-45):
    /// which one is set up and whether its key is there (never the key). The keychain is not
    /// opened for it; while it is not open yet (the AI & MCP requester opens it for OpenAI)
    /// the line says "key not checked yet".
    pub(crate) fn generator_line(&self) -> String {
        match (
            &self.imagegen.debug_provider,
            self.imagegen.settings.provider,
        ) {
            (Some(p), _) => p.display_name().to_string(),
            (None, GenProvider::None) => "Not set up".into(),
            (None, GenProvider::OpenAi) if self.imagegen.system_keys => {
                "OpenAI, key not checked yet".into()
            }
            (None, GenProvider::OpenAi) => {
                match self.imagegen.keys.status(KEY_ACCOUNT, ENV_KEY).source {
                    Some(KeySource::Environment) => format!("OpenAI, key from {ENV_KEY}"),
                    Some(_) => "OpenAI, key set".into(),
                    None => "OpenAI, no API key".into(),
                }
            }
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
        let doc_format = (doc.target(), (doc.page().width(), doc.page().height()));
        let doc_layers = doc.layer_count();
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
        req.set_document(doc_format.0, doc_format.1);
        req.set_doc_layers(doc_layers);
        self.ui.generate_requester = Some(req);
        self.imagegen.picture = None;
        self.imagegen.result = None;
        self.dirty = true;
    }

    /// Opens the Image Generator setup (over the Generate requester, or over Prefs ▸ "AI &
    /// MCP.." from its "Set up.." button, when one is open; closing it returns there).
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
        let mut req =
            ImageGenSetupRequester::new(provider, &s.openai.model, self.setup_info(), screen);
        // Why a key would last for this session only (no keychain, or it refused one).
        if let Some(why) = self.imagegen.keys.status(KEY_ACCOUNT, ENV_KEY).session_only {
            req.set_test(Some(format!("Session only: {why}.")));
        }
        self.ui.imagegen_setup = Some(req);
        self.imagegen.key_test = None;
        self.dirty = true;
    }

    /// The setup's lines about the key (never the key). Without a usable system keychain
    /// (none, or it refused a key) the key line and the note say that a key lasts for this
    /// session only ([`Self::open_imagegen_setup`] says why on the test line).
    fn setup_info(&self) -> SetupInfo {
        let status = self.imagegen.keys.status(KEY_ACCOUNT, ENV_KEY);
        let key_status = match (status.source, &status.session_only) {
            (Some(KeySource::Session), Some(why)) => {
                format!("API key kept for this session only ({why})")
            }
            (None, Some(_)) => "No API key (this session only)".into(),
            _ => status.describe(),
        };
        let keychain = if status.session_only.is_some() {
            "No keychain here: the key lasts for this session only."
        } else {
            "The key goes to the system keychain, never to a file."
        };
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
                 tier. Your text goes over the internet. {keychain} Test only lists your \
                 models. Default model checked {MODELS_CHECKED}."
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
            // While a new picture is made the options wait for it: it is converted with them
            // when it comes (or the kept one again if it does not).
            GenerateResponse::Changed if self.imagegen.job.is_some() => {}
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
        let (target, page) = req.format();
        let palette = req.palette_choice();
        // Nothing is generated (and paid for) that cannot be converted or applied for these
        // options: a current palette larger than the mode shows, a new layer on a picture
        // with the most layers (the box says so, Generate is off).
        let doc = self.engine.document();
        let max = target.mode.max_colors(target.chipset).unwrap_or(16);
        if req.problem().is_some()
            || (palette == PaletteChoice::Current && doc.palette().len() > usize::from(max))
            || (o.target == GenTarget::NewLayer && doc.layer_count() >= MAX_LAYERS)
        {
            return;
        }
        // The quality is the provider's setting (remembered); the provider is made with it.
        self.imagegen.settings.openai.quality = quality_of(o.quality);
        let info = self.provider_info();
        let provider = self.imagegen_provider().filter(|_| info.ready);
        let colors = match palette {
            PaletteChoice::New => o.colors,
            PaletteChoice::Current => self.engine.document().palette().len() as u16,
        };
        let submitted = provider.map(|p| {
            let prompt = style_of(o.style).prompt(&description, colors);
            let mut request = GenerationRequest::for_page(
                prompt,
                &p.capabilities(),
                page,
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
                // A picture shown so far (Again) is kept until the new one is there.
                self.imagegen.job = Some(id);
                req.set_phase(GenPhase::Working { seconds: 0 });
                req.set_error(None);
                req.set_message(None);
            }
            Some(Err(e)) => self.generation_failed(e.to_string()),
        }
    }

    /// A generation that brought no new picture (`why`): a picture kept from before (Again)
    /// is shown again for the current options, with `why` beside it, and can still be
    /// applied; else the requester asks again.
    fn generation_failed(&mut self, why: String) {
        if self.imagegen.picture.is_some() {
            self.generate_convert();
        } else if let Some(req) = &mut self.ui.generate_requester {
            req.set_phase(GenPhase::Ask);
        }
        if let Some(req) = &mut self.ui.generate_requester {
            req.set_error(Some(why));
        }
        self.dirty = true;
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
        // A cancelled request still in flight goes once it has ended (it still counts as
        // running for the job manager's limit until then).
        self.imagegen.jobs.forget(id);
        self.imagegen.job = None;
        self.dirty = true;
        let network = self.imagegen_provider().is_some_and(|p| p.needs_network());
        let new = if self.imagegen.picture.is_some() {
            "No new picture"
        } else {
            "No picture"
        };
        match status.state {
            JobState::Done => match images.and_then(|i| i.first().cloned()) {
                Some(picture) => {
                    self.imagegen.picture = Some(picture);
                    self.generate_convert();
                }
                None => self.generation_failed(format!("{new}: none came back.")),
            },
            JobState::Failed => {
                let why = status
                    .error
                    .map_or_else(|| "it failed".into(), |e| e.to_string());
                self.generation_failed(format!("{new}: {why}."));
            }
            _ => self.generation_failed(if network {
                "Stopped waiting. A request already sent may still be billed.".into()
            } else {
                "Stopped.".into()
            }),
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
        let (target, page) = req.format();
        let doc = self.engine.document();
        let t = ConvertTarget {
            target,
            page,
            palette: doc.palette(),
            bg: self.ui.bg,
            transparent: files::TRANSPARENT_PINK,
        };
        let mut opts = convert_options(&o);
        opts.palette = req.palette_choice();
        req.set_phase(GenPhase::Result);
        match convert_for(picture, &opts, &t, o.target) {
            Ok(r) => {
                let (w, h) = req.thumb_size();
                let (thumb, fitted) = match o.target {
                    // The brush as Apply makes it: fitted to the picture palette.
                    GenTarget::Brush => {
                        let b = r.fitted_brush(doc.palette());
                        let (page, palette) = b.preview(&r.palette, doc.palette());
                        (import::thumbnail(&page, &palette, w, h), b.changed)
                    }
                    // The frame as it will show: the new layer right above the active one,
                    // under the visible layers above that.
                    GenTarget::NewLayer => {
                        let shown = dpx_core::layer::composite_with_new_layer(doc, &r.page);
                        (import::thumbnail(&shown, doc.palette(), w, h), 0)
                    }
                    _ => (import::thumbnail(&r.page, &r.palette, w, h), 0),
                };
                let message = match o.target {
                    GenTarget::NewLayer if doc.layer_count() >= MAX_LAYERS => {
                        format!("No new layer: {MAX_LAYERS} layers is the most")
                    }
                    to => apply_text(to, doc.frame_count(), fitted),
                };
                req.set_thumbnail(Some(thumb));
                req.set_error(None);
                req.set_message(Some(message));
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
        let applied = self.apply_generated(&r, o.target, o.target(), o.palette);
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

    /// A converted generated picture `r` goes to `to` (the requester's Apply and MCP's
    /// `apply_generated_image`): `format` and `palette` are what it was made for (the spare
    /// page and the picture take them; a new layer is in the document's page and palette).
    pub(crate) fn apply_generated(
        &mut self,
        r: &Imported,
        to: GenTarget,
        format: Target,
        palette: PaletteChoice,
    ) -> Result<(), String> {
        let applied = match to {
            GenTarget::Spare => {
                self.spare_from_generated(r, format, palette);
                Ok(())
            }
            GenTarget::Picture => self.picture_from_generated(r, format, palette),
            GenTarget::Brush => {
                self.brush_from_generated(r);
                Ok(())
            }
            GenTarget::NewLayer => self.layer_from_generated(r),
        };
        self.dirty = true;
        applied
    }

    /// New layer: the picture as a new layer above the active one, which it becomes, in the
    /// current frame (clear in the others): one undo step. The layer panel and the title-bar
    /// label follow on the redraw this asks for.
    fn layer_from_generated(&mut self, r: &Imported) -> Result<(), String> {
        self.finish_stroke();
        let doc = self.engine.document();
        if doc.layer_count() >= MAX_LAYERS {
            return Err(layers_full());
        }
        let command = Command::AddPictureLayer {
            name: Some(layer_name(doc.layers())),
            pixels: r.page.pixels().to_vec(),
        };
        self.engine.execute(command).map_err(|e| e.message)?;
        self.clamp_colors();
        self.clamp_scroll();
        self.dirty = true;
        Ok(())
    }

    /// Spare page: it becomes the picture with its own format and palette (one undo step on
    /// the spare page, which brings back its file name, fixed background, BG colour and
    /// details too); the shown page is not touched. A result in the shown page's own format
    /// and palette makes an ordinary spare (sharing the shown page's colour ranges); any
    /// other gets details of its own (no colour ranges, its transparent colour).
    fn spare_from_generated(&mut self, r: &Imported, target: Target, palette: PaletteChoice) {
        let mut other = match self.menu_data.spare.take() {
            Some(s) => s,
            None => {
                self.menu_data.page_history.spare = Default::default();
                self.blank_page()
            }
        };
        let before = self.spare_files(&other);
        let command = r.replace_command(target);
        let key = page_state::step_key(&command);
        match (other.engine.execute(command), key) {
            (Ok(_), Some(key)) => self.menu_data.page_history.spare.record(key, before),
            _ => {
                other.engine = dpx_core::Engine::new(dpx_core::Document::from_picture(
                    r.page.clone(),
                    r.palette.clone(),
                    target,
                ));
                self.menu_data.page_history.spare = Default::default();
            }
        }
        other.scroll = (0, 0);
        other.picture_path = None;
        other.background = None;
        // Margins were made with the BG index for the current palette (ADR-42).
        other.bg = match palette {
            PaletteChoice::Current => self.ui.bg,
            PaletteChoice::New => 0,
        };
        let differs =
            !crate::menu_actions::same_format(self.engine.document(), other.engine.document());
        self.menu_data.spare_own = differs.then(|| files::PictureExtras {
            ranges: Vec::new(),
            grab: None,
            transparent_color: r.transparent_color.map(u16::from),
        });
        self.menu_data.spare = Some(other);
    }

    /// Picture: one undoable command replaces the whole picture (every frame), format and
    /// palette. The details follow Picture ▸ Load's Import requester (ADR-42): a current
    /// palette keeps its colour ranges, a new one has none; no grab point; the transparent
    /// colour is the picture's (none when it is opaque). Undo brings the old ones back.
    fn picture_from_generated(
        &mut self,
        r: &Imported,
        target: Target,
        palette: PaletteChoice,
    ) -> Result<(), String> {
        self.finish_stroke();
        let before = self.shown_files();
        let command = r.replace_command(target);
        let key = page_state::step_key(&command);
        self.engine.execute(command).map_err(|e| e.message)?;
        if let Some(key) = key {
            self.menu_data.page_history.shown.record(key, before);
        }
        self.extras = files::PictureExtras {
            ranges: match palette {
                PaletteChoice::Current => std::mem::take(&mut self.extras.ranges),
                PaletteChoice::New => Vec::new(),
            },
            grab: None,
            transparent_color: r.transparent_color.map(u16::from),
        };
        self.menu_data.cycled = None;
        self.scroll = (0, 0);
        self.clamp_colors();
        self.clamp_scroll();
        Ok(())
    }

    /// Brush: the picture in the picture palette's colours, like a loaded brush file (its own
    /// palette stays for Use Brush Palette); transparent pixels are its transparent colour.
    fn brush_from_generated(&mut self, r: &Imported) {
        let palette = self.engine.document().palette().clone();
        let (own, brush, _) = fitted_brush(r, &palette);
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
        self.refresh_generate_provider();
    }

    /// The Generate requester (under the setup) shows the generator as it is now.
    fn refresh_generate_provider(&mut self) {
        let info = self.provider_info();
        if let Some(req) = &mut self.ui.generate_requester {
            if info.ready && req.error() == Some(NO_GENERATOR) {
                req.set_error(None);
            }
            req.set_provider(info);
        }
    }

    /// Ok: the generator and model go to the settings, a typed key to the keychain; Remove
    /// key forgets the stored one. The key line says what Ok does with the key, and Ok does
    /// that whichever generator is chosen (a key removed, then None chosen, is removed). A key
    /// or model that cannot be used keeps the requester open, and so does a key that could
    /// only be kept for this session (it says so; the next Ok closes).
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
        }
        let mut session_only = false;
        if let Some(typed) = req.typed_key() {
            let Some(secret) = Secret::new(&typed) else {
                req.set_test(Some(
                    "That is no API key: printable characters without spaces only.".into(),
                ));
                return;
            };
            session_only = self.imagegen.keys.set(KEY_ACCOUNT, &secret) == KeySource::Session;
        } else if req.remove_key()
            && let Err(e) = self.imagegen.keys.clear(KEY_ACCOUNT)
        {
            req.set_test(Some(format!("The key could not be removed: {e}.")));
            return;
        }
        self.imagegen.settings.provider = provider;
        self.imagegen.settings.openai = config;
        if session_only {
            // The keychain is missing or refused the key: say so before closing.
            let why = self
                .imagegen
                .keys
                .status(KEY_ACCOUNT, ENV_KEY)
                .session_only
                .unwrap_or_else(|| "no system keychain".into());
            let info = self.setup_info();
            if let Some(req) = &mut self.ui.imagegen_setup {
                req.clear_key();
                req.set_info(info);
                req.set_test(Some(format!(
                    "Saved for this session only ({}); it is gone after a restart.",
                    short(&why, 60)
                )));
            }
            self.refresh_generate_provider();
            return;
        }
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
        self.imagegen.key_test = Some((rx, req.revision()));
        req.set_test(Some(
            "Testing the key: asking OpenAI for your model list..".into(),
        ));
    }

    /// Shows the key test's answer once it is there, unless the key, the model or the
    /// generator was changed since the test started (the answer is then dropped).
    pub(crate) fn key_test_poll(&mut self) {
        let Some((rx, revision)) = &self.imagegen.key_test else {
            return;
        };
        let current = self.ui.imagegen_setup.as_ref().map(|r| r.revision());
        if current != Some(*revision) {
            self.imagegen.key_test = None;
            self.dirty = true;
            return;
        }
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
/// (spare/picture/brush/new_layer), `more`, `provider` (none/openai/test_pattern),
/// `key=placeholder`
/// (a placeholder key in this session's memory, so OpenAI shows as set up; nothing is sent)
/// and `keychain=stand_in` (a stand-in in memory where the system keychain would be, so the
/// setup looks as with a working keychain; without it the snapshot's keys are session-only,
/// and the setup says so).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Spec {
    pub(crate) options: GenerateOptions,
    pub(crate) prompt: String,
    pub(crate) more: bool,
    pub(crate) provider: Option<String>,
    pub(crate) placeholder_key: bool,
    pub(crate) stand_in_keychain: bool,
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
                    "new_layer" => GenTarget::NewLayer,
                    _ => return Err(bad()),
                }
            }
            "provider" => match value {
                "none" | "openai" | "test_pattern" => s.provider = Some(value.to_string()),
                _ => return Err(bad()),
            },
            "key" if value == "placeholder" => s.placeholder_key = true,
            "keychain" if value == "stand_in" => s.stand_in_keychain = true,
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

/// `--mcp-gen-question picture|spare` (snapshots): the question before an MCP client's
/// generated picture replaces the picture (naming `doc`'s frames and layers) or a picture
/// on the spare page, for a generated picture of `doc`'s page size and palette size, as
/// Claude Code would ask it.
fn snapshot_question(
    which: &str,
    doc: &dpx_core::Document,
) -> Result<dpx_ui::ChoiceRequester, String> {
    let (w, h) = (doc.page().width(), doc.page().height());
    let colors = doc.palette().len();
    let client = Some("claude-code");
    let q = match which {
        "picture" => dpx_mcp::tools::replace_question(
            client,
            w,
            h,
            colors,
            doc.frame_count(),
            doc.layer_count(),
        ),
        "spare" => {
            dpx_mcp::tools::replace_spare_question(client, dpx_mcp::Slot::First, w, h, colors)
        }
        other => return Err(format!("--mcp-gen-question {other}: picture or spare")),
    };
    Ok(crate::mcp::question_requester(&q))
}

/// Snapshot options (ADR-45): `--generate-requester` shows Picture ▸ Generate.. over the
/// current screen, `--generate SPEC` sets it up ([`parse_spec`]), `--generate-run` generates
/// with the **test pattern (not AI, debug only)** and shows the result, `--generate-apply`
/// shows the picture after Apply instead, `--imagegen-setup` shows the Image Generator
/// setup and `--mcp-gen-question picture|spare` an MCP client's question before Apply
/// ([`snapshot_question`]). No key store, network or settings file is touched. Returns the
/// document to show (the spare page after Apply onto it). Errors carry the exit code 2.
pub(crate) fn snapshot(
    args: &[String],
    doc: &dpx_core::Document,
    ui: &mut UiState,
) -> Result<dpx_core::Document, (i32, String)> {
    if let Some(which) = crate::arg_value(args, "--mcp-gen-question") {
        ui.choice_requester = Some(snapshot_question(&which, doc).map_err(|e| (2, e))?);
    }
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
    if spec.stand_in_keychain {
        // In memory, in the keychain's place: the real keychain is never touched.
        app.imagegen.keys = Arc::new(ApiKeys::new(
            Some(Box::new(dpx_imagegen::secret::MemoryStore::new())),
            None,
            Arc::new(|_: &str| None::<String>),
        ));
    }
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
