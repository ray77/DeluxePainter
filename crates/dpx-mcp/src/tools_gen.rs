//! The image generation tools (ADR-45, wave-8 integration; the owner's spec §4.1, §5.2,
//! §5.3, §7.7 and acceptance tests 5 and 6): `generate_image`, `get_generation_status`,
//! `apply_generated_image` and `cancel_generation`.
//!
//! They use the workspace's image generator ([`Workspace::image_generation`]): in the running
//! GUI the generator, job manager (and so its in-flight limit), key store and choices of
//! Picture ▸ Generate..; the conversion is `dpx_core::import::convert` (a new layer:
//! `convert_for_layer`, the brush: `convert_for_brush`), the one the Generate requester uses,
//! and Apply goes through the requester's own Apply ([`Workspace::apply_generated`]).
//! Without a set-up generator nothing is generated and nothing is made up
//! (`IMAGE_GENERATOR_NOT_CONFIGURED`); the headless server has no generator
//! (`IMAGE_GENERATION_UNAVAILABLE`).
//!
//! `generate_image` only starts a job and answers at once; `get_generation_status` shows the
//! converted picture when it is there; `apply_generated_image` applies it as one undo step,
//! and replacing the picture is asked first (spec §7.7). No API key is ever part of a result,
//! a log line or an error: the providers read their keys themselves and clean the messages
//! of the services they call.

use super::*;
use crate::generation::GenTargetArg;

/// Most characters of a description (the Generate requester's limit; the provider may have
/// a lower one of its own).
#[cfg_attr(not(feature = "imagegen"), allow(dead_code))]
pub const MAX_PROMPT_CHARS: usize = 1000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenDitherArg {
    None,
    Ordered,
    /// Floyd-Steinberg error diffusion (the Generate requester's "Diffusion").
    #[serde(alias = "diffusion")]
    FloydSteinberg,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenStyleArg {
    /// Classic Amiga-era pixel art with a limited palette.
    PixelArt,
    /// A painted illustration with flat areas of colour.
    Painted,
    /// The description as it is.
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundArg {
    Opaque,
    /// Transparent pixels become colour 0 (transparent on a brush and a new layer).
    Transparent,
}

/// Arguments of `generate_image`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(feature = "imagegen"), allow(dead_code))]
pub struct GenerateImageArgs {
    /// What the picture should show, in words (1 to 1000 characters). A style sentence is
    /// added (see `style`).
    pub prompt: String,
    /// A named format, as create_document: ntsc_lores, pal_lores, pixelart_320x240,
    /// ntsc_hires, pal_hires, ntsc_interlace, pal_interlace.
    pub preset: Option<PresetArg>,
    /// Page width in pixels; with height the mode is chosen as create_document does
    /// (320x240: PAL Low Res with a custom screen).
    pub width: Option<u32>,
    /// Page height in pixels.
    pub height: Option<u32>,
    /// `ocs` (12-bit colours; 32 in Low Res, 16 in High Res) or `aga` (256).
    pub chipset: Option<ChipsetArg>,
    /// Colours of a new palette: a power of two up to the mode's maximum.
    #[serde(alias = "palette_size")]
    pub colors: Option<u16>,
    /// `none`, `ordered` or `floyd_steinberg` (also `diffusion`).
    pub dithering: Option<GenDitherArg>,
    /// `new` (a palette made for the picture) or `current` (the picture's palette).
    pub palette: Option<PaletteArg>,
    /// `pixel_art`, `painted` or `none`.
    pub style: Option<GenStyleArg>,
    /// `opaque` or `transparent`.
    pub background: Option<BackgroundArg>,
    /// Where apply_generated_image puts it unless it names another target: `spare`
    /// (default), `picture` (asks the user first), `brush`, `new_layer` (the picture's page
    /// and palette) or `new_document` (the spare page with a format of its own).
    pub target: Option<GenTargetArg>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(feature = "imagegen"), allow(dead_code))]
pub struct JobIdArgs {
    /// The job_id generate_image returned, e.g. `gen-3`.
    pub job_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(feature = "imagegen"), allow(dead_code))]
pub struct ApplyGeneratedArgs {
    /// The job_id generate_image returned, e.g. `gen-3`.
    pub job_id: String,
    /// `spare`, `picture`, `brush`, `new_layer` or `new_document`. Default: the target
    /// generate_image was given.
    pub target: Option<GenTargetArg>,
}

pub(super) const GENERATE_IMAGE: Spec = Spec {
    // ADR-45: the description leaves DeluxePainter for the generator's service.
    open_world: true,
    ..edit(
        "generate_image",
        "Generate image",
        "Ask the image generator the user set up in DeluxePainter (Prefs > AI & MCP > Set up..; e.g. OpenAI Images) for a picture from a description. The request is sent at once: the description goes over the internet to that service, and every request is billed to the user's own account there (a cancelled one too); the result's `cost` repeats the provider's note. The picture is converted to exactly the chosen size, chipset and colours, as Picture > Generate.. does. Returns a job_id at once: poll get_generation_status, then apply_generated_image. Without a set-up generator nothing is generated (IMAGE_GENERATOR_NOT_CONFIGURED); no picture is ever made up.",
        schema::<GenerateImageArgs>,
        false,
    )
};

pub(super) const GET_GENERATION_STATUS: Spec = read(
    "get_generation_status",
    "Generation status",
    "The state of a generate_image job: running, done, failed (with its error code) or cancelled. When done, the converted picture as a PNG image (integer-scaled) with its width, height and colours used.",
    schema::<JobIdArgs>,
);

pub(super) const APPLY_GENERATED_IMAGE: Spec = edit(
    "apply_generated_image",
    "Apply generated image",
    "Apply a finished generation as one undo step: to the spare page (default; DeluxePainter asks the user first when the spare page holds a picture), the picture (replaces every frame and layer; DeluxePainter asks the user first), the brush (fitted to the picture's palette), a new layer (the picture's page and palette) or new_document (the spare page with a format of its own). Returns the document it went to.",
    schema::<ApplyGeneratedArgs>,
    true,
);

pub(super) const CANCEL_GENERATION: Spec = Spec {
    needs: Needs::Read,
    idempotent: true,
    ..edit(
        "cancel_generation",
        "Cancel generation",
        "Stop waiting for a generate_image job. A request already sent to the provider may still be billed.",
        schema::<JobIdArgs>,
        false,
    )
};

/// True for the tools of this module.
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "generate_image" | "get_generation_status" | "apply_generated_image" | "cancel_generation"
    )
}

/// Without the `imagegen` feature (the bridge): the arguments are checked as with it, then
/// the tool is unavailable.
#[cfg(not(feature = "imagegen"))]
pub(super) async fn dispatch(
    _ctx: &ToolCtx,
    name: &str,
    args: Option<JsonObject>,
) -> Result<CallToolResult, ToolError> {
    match name {
        "generate_image" => drop(parse::<GenerateImageArgs>(args)?),
        "apply_generated_image" => drop(parse::<ApplyGeneratedArgs>(args)?),
        _ => drop(parse::<JobIdArgs>(args)?),
    }
    Err(crate::generation::unavailable())
}

#[cfg(feature = "imagegen")]
pub(super) async fn dispatch(
    ctx: &ToolCtx,
    name: &str,
    args: Option<JsonObject>,
) -> Result<CallToolResult, ToolError> {
    match name {
        "generate_image" => work::generate_image(ctx, parse(args)?).await,
        "get_generation_status" => work::status(ctx, parse(args)?).await,
        "apply_generated_image" => work::apply(ctx, parse(args)?).await,
        "cancel_generation" => work::cancel(ctx, parse(args)?).await,
        other => Err(ToolError::internal(format!("no handler for {other}"))),
    }
}

#[cfg(feature = "imagegen")]
mod work {
    use dpx_core::import::{ConvertOptions, ConvertTarget, Imported, Margins};
    use dpx_core::layer::MAX_LAYERS;
    use dpx_imagegen::{
        GenError, GenerationRequest, ImageGenerationProvider, JobId, JobState, JobStatus,
        ProviderStatus, Style,
    };

    use super::*;
    use crate::generation::{ApplyTo, GenApply, GenDefaults, GenHost, NOT_CONFIGURED, Plan};

    /// A job id that is not an AI client's generation.
    fn not_found(id: &str) -> ToolError {
        ToolError::new(
            "JOB_NOT_FOUND",
            format!(
                "no generation {id:?} started by an AI client is known (ids look like gen-3; \
                 the {} newest are known, and a finished picture that was never fetched may \
                 be dropped sooner)",
                crate::generation::GenPlans::KEEP
            ),
        )
    }

    fn job_id(text: &str) -> Result<JobId, ToolError> {
        text.trim()
            .strip_prefix("gen-")
            .and_then(|n| n.parse().ok())
            .map(JobId)
            .ok_or_else(|| not_found(text))
    }

    /// A job of an AI client that the job manager no longer has and whose picture was never
    /// fetched.
    fn no_longer_kept(id: JobId) -> ToolError {
        ToolError::new(
            "JOB_NOT_FOUND",
            format!("{id} is no longer kept: generate it again"),
        )
    }

    /// A job's state as the tools report it: done once its picture is kept with its plan
    /// (the job manager may have dropped the job since: it keeps only its newest finished
    /// jobs), else the job manager's state; neither: no longer kept. One rule for
    /// get_generation_status and cancel_generation.
    fn state_of(plan: &Plan, status: Option<&JobStatus>) -> Result<JobState, ToolError> {
        match (status, &plan.picture) {
            (_, Some(_)) => Ok(JobState::Done),
            (Some(s), None) => Ok(s.state),
            (None, None) => Err(no_longer_kept(plan.job)),
        }
    }

    /// A provider error as a tool error: its stable code (ADR-45) and its cleaned text.
    fn gen_error(e: &GenError) -> ToolError {
        match e {
            GenError::NotConfigured(_) => not_configured(&e.to_string()),
            _ => ToolError::new(e.code(), e.to_string()),
        }
    }

    fn not_configured(why: &str) -> ToolError {
        ToolError::new(
            NOT_CONFIGURED,
            format!(
                "{why}. The user sets a generator up in DeluxePainter: Prefs > AI & MCP.. > Set \
                 up.. (or Picture > Generate.. > Set up..). Nothing was generated"
            ),
        )
    }

    /// The generator set up, if it is ready.
    fn ready_provider(host: &GenHost) -> Result<Arc<dyn ImageGenerationProvider>, ToolError> {
        let Some(p) = &host.provider else {
            return Err(not_configured(
                "No image generator is set up in DeluxePainter",
            ));
        };
        match p.status() {
            ProviderStatus::Ready => Ok(p.clone()),
            ProviderStatus::NotConfigured(why) => Err(not_configured(&format!(
                "{} is not set up: {why}",
                p.display_name()
            ))),
        }
    }

    /// What a conversion needs from the shown picture, taken in one job.
    #[derive(Clone, Debug)]
    struct DocContext {
        revision: u64,
        slot: Slot,
        engine: u64,
        target: Target,
        page: (u32, u32),
        palette: Palette,
        bg: u8,
        layers: usize,
        frames: usize,
    }

    impl DocContext {
        fn of(ws: &dyn Workspace) -> Self {
            let doc = ws.engine().document();
            DocContext {
                revision: doc.revision(),
                slot: ws.shown(),
                engine: ws.engine().id(),
                target: doc.target(),
                page: (doc.page().width(), doc.page().height()),
                palette: doc.palette().clone(),
                bg: ws.bg_color(),
                layers: doc.layer_count(),
                frames: doc.frame_count(),
            }
        }

        fn max_colors(&self) -> u16 {
            self.target
                .mode
                .max_colors(self.target.chipset)
                .unwrap_or(16)
        }
    }

    /// A new layer needs room (the most layers, and the memory of every frame's pictures:
    /// `dpx_core::layer::room_for_layer`, the rule `AddPictureLayer` applies) and a palette its
    /// mode shows.
    fn layer_fits(dc: &DocContext) -> Result<(), ToolError> {
        if dc.layers >= MAX_LAYERS {
            return Err(ToolError::new(
                "LAYER_LIMIT",
                format!(
                    "the picture has {} layers, the most allowed ({MAX_LAYERS}): delete or \
                     merge one, or use another target",
                    dc.layers
                ),
            ));
        }
        if let Err(e) = dpx_core::layer::room_for_layer(dc.layers, dc.frames, dc.page.0, dc.page.1)
        {
            return Err(ToolError::new(
                "LAYER_LIMIT",
                format!(
                    "no room for another layer: {}; delete frames or layers, or use another \
                     target",
                    e.message
                ),
            ));
        }
        let max = dc.max_colors();
        if dc.palette.len() > usize::from(max) {
            return Err(ToolError::new(
                "PALETTE_RANGE_INVALID",
                format!(
                    "the picture's palette has {} colours, but {} on {} shows {max}: a new \
                     layer cannot hold a picture in it",
                    dc.palette.len(),
                    dc.target.mode.name(),
                    chipset_name(dc.target.chipset)
                ),
            ));
        }
        Ok(())
    }

    /// What generate_image makes.
    struct Planned {
        target: GenTargetArg,
        format: Target,
        page: (u32, u32),
        colors: u16,
        convert: ConvertOptions,
        style: Style,
    }

    fn plan(a: &GenerateImageArgs, d: &GenDefaults, dc: &DocContext) -> Result<Planned, ToolError> {
        let target = a.target.unwrap_or_default();
        let transparent = a.background.map_or(d.convert.transparent_background, |b| {
            b == BackgroundArg::Transparent
        });
        let dither = match a.dithering {
            Some(GenDitherArg::None) => Dither::None,
            Some(GenDitherArg::Ordered) => Dither::Ordered,
            Some(GenDitherArg::FloydSteinberg) => Dither::FloydSteinberg,
            None => d.convert.dither,
        };
        let style = match a.style {
            Some(GenStyleArg::PixelArt) => Style::PixelArt,
            Some(GenStyleArg::Painted) => Style::Painted,
            Some(GenStyleArg::None) => Style::None,
            None => d.style,
        };
        let convert = |palette, colors| ConvertOptions {
            framing: d.convert.framing,
            scaler: d.convert.scaler,
            margins: if transparent {
                Margins::Transparent
            } else {
                Margins::Bg
            },
            palette,
            colors,
            dither,
            transparent_background: transparent,
        };
        if target == GenTargetArg::NewLayer {
            // A layer shares the picture's page, format and palette.
            let doc_colors = dc.palette.len().min(256) as u16;
            let conflicts = a.preset.is_some()
                || a.width.is_some_and(|w| w != dc.page.0)
                || a.height.is_some_and(|h| h != dc.page.1)
                || a.chipset.is_some_and(|c| c.chipset() != dc.target.chipset)
                || a.palette == Some(PaletteArg::New)
                || a.colors.is_some_and(|c| c != doc_colors);
            if conflicts {
                return Err(ToolError::invalid(format!(
                    "a new layer takes the picture's page ({}x{}), chipset ({}) and palette \
                     ({doc_colors} colours): leave out preset, width, height, chipset, colors \
                     and palette, or use another target",
                    dc.page.0,
                    dc.page.1,
                    chipset_name(dc.target.chipset)
                )));
            }
            layer_fits(dc)?;
            return Ok(Planned {
                target,
                format: dc.target,
                page: dc.page,
                colors: doc_colors,
                convert: convert(PaletteChoice::Current, doc_colors),
                style,
            });
        }
        let args = CreateDocumentArgs {
            width: a.width,
            height: a.height,
            chipset: a.chipset,
            colors: None,
            preset: a.preset,
            ..Default::default()
        };
        let first = plan_format(&args, d.format)?;
        let max = first.mode.max_colors(first.chipset).unwrap_or(16);
        let colors = a
            .colors
            .unwrap_or_else(|| import::color_count(d.colors.max(2), max));
        let f = plan_format(
            &CreateDocumentArgs {
                colors: Some(colors),
                ..args
            },
            d.format,
        )?;
        let format = Target::new(f.chipset, f.mode).with_screen(f.screen.0, f.screen.1);
        if !format.page_fits(f.width, f.height) {
            let ((a, b), (c, e)) = format.page_limits();
            return Err(ToolError::invalid(format!(
                "page {}x{}: {} allows {a}x{b} to {c}x{e}",
                f.width,
                f.height,
                f.mode.name()
            )));
        }
        let palette = match a.palette {
            Some(PaletteArg::New) => PaletteChoice::New,
            Some(PaletteArg::Current) => PaletteChoice::Current,
            None => d.convert.palette,
        };
        if palette == PaletteChoice::Current && dc.palette.len() > usize::from(max) {
            return Err(ToolError::new(
                "PALETTE_RANGE_INVALID",
                format!(
                    "the picture's palette has {} colours, but {} {} shows at most {max}: use \
                     palette=new, AGA or another format",
                    dc.palette.len(),
                    chipset_name(f.chipset),
                    f.mode.name()
                ),
            ));
        }
        Ok(Planned {
            target,
            format,
            page: (f.width, f.height),
            colors: f.colors,
            convert: convert(palette, f.colors),
            style,
        })
    }

    fn palette_name(p: PaletteChoice) -> &'static str {
        match p {
            PaletteChoice::New => "new",
            PaletteChoice::Current => "current",
        }
    }

    fn dither_name(d: Dither) -> &'static str {
        match d {
            Dither::None => "none",
            Dither::Ordered => "ordered",
            Dither::FloydSteinberg => "floyd_steinberg",
        }
    }

    fn style_name(s: Style) -> &'static str {
        match s {
            Style::PixelArt => "pixel_art",
            Style::Painted => "painted",
            Style::None => "none",
        }
    }

    #[derive(Serialize)]
    struct ProviderOut {
        id: String,
        name: String,
        /// The description is sent over the internet.
        network: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    }

    fn provider_out(p: &dyn ImageGenerationProvider, model: Option<String>) -> ProviderOut {
        ProviderOut {
            id: p.id().to_string(),
            name: p.display_name().to_string(),
            network: p.needs_network(),
            model,
        }
    }

    #[derive(Serialize)]
    struct Started {
        job_id: String,
        status: &'static str,
        target: &'static str,
        width: u32,
        height: u32,
        chipset: ChipsetArg,
        display_mode: String,
        colors: u16,
        palette: &'static str,
        dithering: &'static str,
        style: &'static str,
        background: &'static str,
        provider: ProviderOut,
        /// Who pays and where the description goes (the provider's own note).
        cost: String,
        notes: Vec<String>,
    }

    pub(super) async fn generate_image(
        ctx: &ToolCtx,
        a: GenerateImageArgs,
    ) -> Result<CallToolResult, ToolError> {
        let description = a.prompt.trim().to_string();
        if description.is_empty() {
            return Err(ToolError::invalid(
                "prompt: describe the picture in words (it is empty)",
            ));
        }
        let chars = description.chars().count();
        if chars > MAX_PROMPT_CHARS {
            return Err(ToolError::invalid(format!(
                "prompt: {chars} characters, at most {MAX_PROMPT_CHARS}"
            )));
        }
        let started = ctx
            .exec
            .run(move |ws| -> Result<Started, ToolError> {
                let host = ws.image_generation()?;
                let provider = ready_provider(&host)?;
                let dc = DocContext::of(ws);
                let p = plan(&a, &host.defaults, &dc)?;
                // The style sentence names the colours the picture will have.
                let colors = match p.convert.palette {
                    PaletteChoice::New => p.colors,
                    PaletteChoice::Current => dc.palette.len().min(256) as u16,
                };
                let mut request = GenerationRequest::for_page(
                    p.style.prompt(&description, colors),
                    &provider.capabilities(),
                    p.page,
                    p.format.mode.pixel_shape(),
                );
                request.transparent_background = p.convert.transparent_background;
                // The job manager is the Generate requester's: its limit of requests in
                // flight (BUSY) counts both.
                let job = host
                    .jobs
                    .submit(provider.clone(), request)
                    .map_err(|e| gen_error(&e))?;
                host.plans.add(Plan {
                    job,
                    target: p.target,
                    format: p.format,
                    page: p.page,
                    colors,
                    convert: p.convert,
                    provider: provider.id().to_string(),
                    picture: None,
                });
                let chipset = chipset_name(p.format.chipset);
                ws.log(&format!(
                    "generate_image {job} {}x{} {chipset} {colors} colours, for {}",
                    p.page.0,
                    p.page.1,
                    p.target.name()
                ));
                let mut notes = vec![
                    "poll get_generation_status with job_id; apply_generated_image applies \
                     the picture (one undo step)"
                        .to_string(),
                ];
                match p.target {
                    GenTargetArg::Picture => notes.push(
                        "applying to the picture replaces it: DeluxePainter asks the user \
                         first"
                            .into(),
                    ),
                    GenTargetArg::Spare | GenTargetArg::NewDocument => notes.push(format!(
                        "the spare page ({}) gets the picture in this format and palette; \
                         select_document shows it",
                        dc.slot.other().id()
                    )),
                    _ => {}
                }
                notes.extend(dpx_core::hardware::format_notes(&p.format));
                Ok(Started {
                    job_id: job.to_string(),
                    status: JobState::Running.as_str(),
                    target: p.target.name(),
                    width: p.page.0,
                    height: p.page.1,
                    chipset: ChipsetArg::of(p.format.chipset),
                    display_mode: p.format.mode.name(),
                    colors,
                    palette: palette_name(p.convert.palette),
                    dithering: dither_name(p.convert.dither),
                    style: style_name(p.style),
                    background: if p.convert.transparent_background {
                        "transparent"
                    } else {
                        "opaque"
                    },
                    provider: provider_out(provider.as_ref(), host.model.clone()),
                    cost: provider.cost_note().to_string(),
                    notes,
                })
            })
            .await??;
        Ok(success_result(&started))
    }

    /// Where a job stands, read in one job together with what its conversion needs.
    struct Looked {
        host: GenHost,
        plan: Plan,
        dc: DocContext,
        state: Option<dpx_imagegen::JobStatus>,
    }

    async fn look(ctx: &ToolCtx, text: &str) -> Result<Looked, ToolError> {
        let id = job_id(text)?;
        let shown = text.to_string();
        ctx.exec
            .run(move |ws| -> Result<Looked, ToolError> {
                let host = ws.image_generation()?;
                let plan = host.plans.get(id).ok_or_else(|| not_found(&shown))?;
                let state = host.jobs.status(id);
                Ok(Looked {
                    dc: DocContext::of(ws),
                    host,
                    plan,
                    state,
                })
            })
            .await?
    }

    /// The generated picture of a finished job (kept with its plan from now on).
    fn picture(l: &Looked) -> Result<Arc<dpx_core::RgbaImage>, ToolError> {
        if let Some(p) = &l.plan.picture {
            return Ok(p.clone());
        }
        let id = l.plan.job;
        match &l.state {
            Some(s) if s.state == JobState::Done => {}
            Some(s) => {
                return Err(ToolError::new(
                    "JOB_NOT_DONE",
                    format!(
                        "{id} is {}: get_generation_status says when it is done",
                        s.state.as_str()
                    ),
                ));
            }
            None => return Err(no_longer_kept(id)),
        }
        let first = l
            .host
            .jobs
            .images(id)
            .and_then(|i| i.first().cloned())
            .ok_or_else(|| {
                ToolError::new("JOB_NOT_FOUND", format!("{id} has no picture any more"))
            })?;
        let first = Arc::new(first);
        l.host.plans.keep_picture(id, first.clone());
        Ok(first)
    }

    /// The conversion for `to`: a new layer in the shown picture's page and palette, any
    /// other target in the format and palette the job was planned for.
    fn conversion(
        plan: &Plan,
        dc: &DocContext,
        to: GenTargetArg,
    ) -> (Target, (u32, u32), ConvertOptions) {
        match to {
            GenTargetArg::NewLayer => (
                dc.target,
                dc.page,
                ConvertOptions {
                    palette: PaletteChoice::Current,
                    ..plan.convert
                },
            ),
            _ => (plan.format, plan.page, plan.convert),
        }
    }

    async fn convert(
        picture: Arc<dpx_core::RgbaImage>,
        plan: &Plan,
        dc: &DocContext,
        to: GenTargetArg,
    ) -> Result<Imported, ToolError> {
        let (format, page, opts) = conversion(plan, dc, to);
        let (palette, bg) = (dc.palette.clone(), dc.bg);
        blocking(move || {
            let t = ConvertTarget {
                target: format,
                page,
                palette: &palette,
                bg,
                transparent: TRANSPARENT_PINK,
            };
            // As the Generate requester converts (`generate::convert_for` in the app): the
            // brush and a new layer match the picture's palette as it is stored.
            match to {
                GenTargetArg::NewLayer => import::convert_for_layer(&picture, &opts, &t),
                GenTargetArg::Brush => import::convert_for_brush(&picture, &opts, &t),
                _ => import::convert(&picture, &opts, &t),
            }
        })
        .await?
        .map_err(ToolError::from)
    }

    fn colors_used(page: &IndexedImage) -> usize {
        let mut seen = [false; 256];
        for &p in page.pixels() {
            seen[usize::from(p)] = true;
        }
        seen.iter().filter(|&&s| s).count()
    }

    /// A converted picture as it lands on its target, which the status preview and the
    /// numbers of get_generation_status and apply_generated_image describe: the brush as
    /// Apply makes it (fitted to the shown picture's palette, `Imported::fitted_brush`, as
    /// the Generate requester's thumbnail shows it), any other target the converted page.
    struct Landed {
        page: IndexedImage,
        palette: Palette,
        /// The format whose palette that is: the planned one; the shown picture's for the
        /// brush and a new layer.
        format: Target,
        colors_used: usize,
        /// Brush: colours that had to change to fit the picture's palette.
        fitted: usize,
    }

    fn landed(r: &Imported, plan: &Plan, dc: &DocContext, to: GenTargetArg) -> Landed {
        match to {
            GenTargetArg::Brush => {
                let b = r.fitted_brush(&dc.palette);
                let (page, palette) = b.preview(&r.palette, &dc.palette);
                Landed {
                    page,
                    palette,
                    format: dc.target,
                    colors_used: b.colors_used(),
                    fitted: b.changed,
                }
            }
            _ => Landed {
                page: r.page.clone(),
                palette: r.palette.clone(),
                format: conversion(plan, dc, to).0,
                colors_used: colors_used(&r.page),
                fitted: 0,
            },
        }
    }

    /// The brush and a new layer keep the shown picture's palette as it is stored (ADR-46,
    /// wave-8 integration: an older OCS project's may hold colours that are not 12-bit).
    fn keeps_palette(to: GenTargetArg) -> bool {
        matches!(to, GenTargetArg::Brush | GenTargetArg::NewLayer)
    }

    /// The notes on an older OCS project's stored palette that `to` keeps (none for the
    /// targets that take the result's palette, which is 12-bit on OCS).
    fn kept_palette_notes(dc: &DocContext, to: GenTargetArg) -> Vec<String> {
        if keeps_palette(to) {
            stored_palette_notes(dc.target, &dc.palette)
        } else {
            Vec::new()
        }
    }

    /// Bits per colour channel of the palette the picture lands in: 4 on OCS, 8 on AGA and
    /// for an older OCS project's stored palette that the brush or a new layer keeps.
    fn palette_bits(dc: &DocContext, to: GenTargetArg, landed: &Landed) -> u8 {
        match landed.format.chipset {
            Chipset::Ocs if kept_palette_notes(dc, to).is_empty() => 4,
            _ => 8,
        }
    }

    fn notes_for(r: &Imported, to: GenTargetArg, landed: &Landed, dc: &DocContext) -> Vec<String> {
        let format = landed.format;
        let mut notes = Vec::new();
        if r.transparent_color.is_some() {
            notes.push("transparent pixels are colour 0".to_string());
        }
        if to == GenTargetArg::NewLayer {
            notes.push(
                "colour 0 is transparent on the new layer; no opaque pixel uses it".to_string(),
            );
        }
        if to == GenTargetArg::Brush {
            notes.push(format!(
                "the brush is in the picture's palette ({} colours), as Brush > Load fits \
                 one: {} of its colours had to change",
                landed.palette.len(),
                landed.fitted
            ));
        }
        let stored = kept_palette_notes(dc, to);
        if format.chipset == Chipset::Ocs && stored.is_empty() {
            notes.push("OCS: every palette entry is a 12-bit colour".to_string());
        }
        notes.extend(stored);
        let rounded = rounded_colors(r, to);
        if rounded > 0 {
            notes.push(format!(
                "{} of the current palette for OCS",
                dpx_core::hardware::rounded_note(rounded)
            ));
        }
        notes.extend(dpx_core::hardware::format_notes(&format));
        notes
    }

    /// Entries of the current palette the conversion rounded to 12 bits for an OCS target
    /// (ADR-46, `Imported::rounded_colors`), as the Generate requester's Apply note counts
    /// them: only for the targets that take the result's palette (the spare page and the
    /// picture); a new layer and the brush keep the picture's palette as it is.
    fn rounded_colors(r: &Imported, to: GenTargetArg) -> usize {
        match to {
            GenTargetArg::Spare | GenTargetArg::NewDocument | GenTargetArg::Picture => {
                r.rounded_colors
            }
            GenTargetArg::Brush | GenTargetArg::NewLayer => 0,
        }
    }

    #[derive(Serialize)]
    struct ErrorOut {
        code: String,
        message: String,
        retryable: bool,
    }

    #[derive(Serialize)]
    struct StatusOut {
        job_id: String,
        status: &'static str,
        target: &'static str,
        elapsed_seconds: u64,
        provider: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ErrorOut>,
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        height: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        chipset: Option<ChipsetArg>,
        /// Different palette indices the converted picture uses.
        #[serde(skip_serializing_if = "Option::is_none")]
        colors_used: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        palette_size: Option<usize>,
        /// Bits per colour channel of the palette (4 on OCS, 8 on AGA).
        #[serde(skip_serializing_if = "Option::is_none")]
        palette_bits: Option<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        preview_scale: Option<u32>,
        /// Current-palette entries rounded to 12-bit OCS values (left out when none).
        #[serde(skip_serializing_if = "is_zero")]
        rounded_colors: usize,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        notes: Vec<String>,
    }

    fn is_zero(n: &usize) -> bool {
        *n == 0
    }

    pub(super) async fn status(ctx: &ToolCtx, a: JobIdArgs) -> Result<CallToolResult, ToolError> {
        let l = look(ctx, &a.job_id).await?;
        let id = l.plan.job;
        let state = state_of(&l.plan, l.state.as_ref())?;
        let mut out = StatusOut {
            job_id: id.to_string(),
            status: state.as_str(),
            target: l.plan.target.name(),
            elapsed_seconds: l.state.as_ref().map_or(0, |s| s.elapsed.as_secs()),
            provider: l.plan.provider.clone(),
            error: None,
            width: None,
            height: None,
            chipset: None,
            colors_used: None,
            palette_size: None,
            palette_bits: None,
            preview_scale: None,
            rounded_colors: 0,
            notes: Vec::new(),
        };
        match state {
            JobState::Running => {
                out.notes.push(
                    "still generating: ask again in a few seconds; cancel_generation stops \
                     waiting"
                        .into(),
                );
                Ok(success_result(&out))
            }
            JobState::Failed => {
                let e = l.state.as_ref().and_then(|s| s.error.clone());
                out.error = Some(match e {
                    Some(e) => ErrorOut {
                        code: gen_error(&e).code,
                        message: e.to_string(),
                        retryable: e.retryable(),
                    },
                    None => ErrorOut {
                        code: "BAD_RESPONSE".into(),
                        message: "the generation failed".into(),
                        retryable: false,
                    },
                });
                Ok(success_result(&out))
            }
            JobState::Cancelled => {
                out.notes.push(
                    "cancelled: a request already sent may still be billed by the provider".into(),
                );
                Ok(success_result(&out))
            }
            JobState::Done => {
                let pic = picture(&l)?;
                let to = l.plan.target;
                let r = convert(pic, &l.plan, &l.dc, to).await?;
                let landed = landed(&r, &l.plan, &l.dc, to);
                let (w, h) = (landed.page.width(), landed.page.height());
                let scale = view::preview_scale(w, h, None)?;
                out.width = Some(w);
                out.height = Some(h);
                out.chipset = Some(ChipsetArg::of(landed.format.chipset));
                out.colors_used = Some(landed.colors_used);
                out.palette_size = Some(landed.palette.len());
                out.palette_bits = Some(palette_bits(&l.dc, to, &landed));
                out.rounded_colors = rounded_colors(&r, to);
                out.notes = notes_for(&r, to, &landed, &l.dc);
                out.notes.push(
                    "apply_generated_image applies it (one undo step); the preview shows it \
                     as it lands, in the palette colours, square pixels"
                        .into(),
                );
                // A new layer as get_preview shows a layer above the bottom one: its colour 0
                // is transparent (alpha 0), as it will be in the picture.
                let transparent = (to == GenTargetArg::NewLayer).then_some(0);
                if transparent.is_some() {
                    out.notes.push(
                        "the preview shows the new layer alone; its colour 0 is left \
                         transparent (alpha 0), where the layers below will show"
                            .into(),
                    );
                }
                let Landed { page, palette, .. } = landed;
                let rendered =
                    blocking(move || view::render_png_with(&page, &palette, scale, transparent))
                        .await??;
                out.preview_scale = Some(rendered.scale);
                Ok(image_result(view::to_base64(&rendered.png), &out))
            }
        }
    }

    #[derive(Serialize)]
    struct Applied {
        job_id: String,
        target: &'static str,
        /// The page it went to (`page1` or `page2`).
        document_id: &'static str,
        revision: u64,
        width: u32,
        height: u32,
        colors_used: usize,
        undoable: bool,
        document: Option<DocumentInfo>,
        /// Current-palette entries rounded to 12-bit OCS values (left out when none).
        #[serde(skip_serializing_if = "is_zero")]
        rounded_colors: usize,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        notes: Vec<String>,
    }

    /// The question before a generated picture replaces the shown one (spec §7.7): it
    /// names every frame and layer that goes when there is more than one (Picture is one
    /// frame with one layer afterwards). The Generate requester's Apply line names the
    /// frames only (`generate::apply_text` in the app; ADR-45, "Seam review").
    pub fn replace_question(
        client: Option<&str>,
        w: u32,
        h: u32,
        colors: usize,
        frames: usize,
        layers: usize,
    ) -> Question {
        let mut extent = Vec::new();
        if frames > 1 {
            extent.push(format!("all {frames} frames"));
        }
        if layers > 1 {
            extent.push(format!("{layers} layers"));
        }
        let what = match extent.as_slice() {
            [] => "the picture".to_string(),
            parts => format!("the picture ({})", parts.join(", ")),
        };
        Question {
            title: "Replace Picture?".into(),
            text: format!(
                "{} wants to replace {what} with a generated one ({w}x{h}, {colors} \
                 colours). Undo brings the old one back. Replace?",
                asker(client)
            ),
            yes: "Replace".into(),
            no: "Keep".into(),
        }
    }

    /// The question before a generated picture replaces a picture on the spare page (spec
    /// §4.1.10 and §7.7: existing work is never replaced unasked; a blank spare page is not
    /// asked about).
    pub fn replace_spare_question(
        client: Option<&str>,
        page: Slot,
        w: u32,
        h: u32,
        colors: usize,
    ) -> Question {
        Question {
            title: "Replace Spare Page?".into(),
            text: format!(
                "{} wants to put a generated picture ({w}x{h}, {colors} colours) on the \
                 spare page ({}), replacing the picture there. Undo on that page (j shows \
                 it) brings it back. Replace?",
                asker(client),
                page.id()
            ),
            yes: "Replace".into(),
            no: "Keep".into(),
        }
    }

    /// The spare page holds a picture a generated one would replace: some frame or layer of
    /// it is not one colour all over. The blank page DeluxePainter makes (or none yet) is
    /// not; nor is a page cleared to one colour.
    fn spare_holds_picture(ws: &dyn Workspace) -> bool {
        ws.spare().is_some_and(|doc| {
            doc.cels().any(|cel| {
                let pixels = cel.pixels();
                pixels.iter().any(|&p| p != pixels[0])
            })
        })
    }

    fn pages_swapped() -> ToolError {
        ToolError::new(
            "REVISION_CONFLICT",
            "the pages were swapped (select_document or j) while the generated picture was \
             applied to the spare page; call apply_generated_image again",
        )
    }

    pub(super) async fn apply(
        ctx: &ToolCtx,
        a: ApplyGeneratedArgs,
    ) -> Result<CallToolResult, ToolError> {
        let l = look(ctx, &a.job_id).await?;
        let to = a.target.unwrap_or(l.plan.target);
        if to == GenTargetArg::NewLayer {
            layer_fits(&l.dc)?;
        }
        let apply_to = match to {
            GenTargetArg::Spare | GenTargetArg::NewDocument => ApplyTo::Spare,
            GenTargetArg::Picture => ApplyTo::Picture,
            GenTargetArg::Brush => ApplyTo::Brush,
            GenTargetArg::NewLayer => ApplyTo::NewLayer,
        };
        let pic = picture(&l)?;
        let r = convert(pic, &l.plan, &l.dc, to).await?;
        let (_, _, opts) = conversion(&l.plan, &l.dc, to);
        let landed = landed(&r, &l.plan, &l.dc, to);
        let (w, h, used) = (
            landed.page.width(),
            landed.page.height(),
            landed.colors_used,
        );
        let id = l.plan.job;
        let slot = l.dc.slot;
        // Replacing a picture is asked first: the shown one always, the spare page's when
        // it holds one.
        let question = match apply_to {
            ApplyTo::Picture => Some(replace_question(
                ctx.state.activity().snapshot().client.as_deref(),
                w,
                h,
                used,
                l.dc.frames,
                l.dc.layers,
            )),
            ApplyTo::Spare => {
                let holds = ctx
                    .exec
                    .run(move |ws| -> Result<bool, ToolError> {
                        if ws.shown() != slot {
                            return Err(pages_swapped());
                        }
                        Ok(spare_holds_picture(ws))
                    })
                    .await??;
                holds.then(|| {
                    replace_spare_question(
                        ctx.state.activity().snapshot().client.as_deref(),
                        slot.other(),
                        w,
                        h,
                        used,
                    )
                })
            }
            ApplyTo::Brush | ApplyTo::NewLayer => None,
        };
        let asked = question.is_some();
        if let Some(q) = question {
            let kept = if apply_to == ApplyTo::Picture {
                "the picture"
            } else {
                "the spare page"
            };
            let declined = |why: String| Err(ToolError::new("APPLY_DECLINED", why));
            match ctx.exec.ask(q).await {
                Ok(true) => {}
                Ok(false) => {
                    return declined(format!(
                        "the user kept {kept}; {id} was not applied (the brush and new_layer \
                         targets need no question, nor does a blank spare page)"
                    ));
                }
                Err(e) if e.code == crate::jobs::NO_ANSWER => {
                    return declined(format!("{}; {kept} was kept", e.message));
                }
                Err(e) => return Err(e),
            }
        }
        // A new layer and the current palette were converted for the shown picture.
        let depends = apply_to == ApplyTo::NewLayer || opts.palette == PaletteChoice::Current;
        let dc = l.dc.clone();
        let format = conversion(&l.plan, &l.dc, to).0;
        let mut notes = notes_for(&r, to, &landed, &l.dc);
        let rounded = rounded_colors(&r, to);
        let (slot, revision, document) = ctx
            .exec
            .run(move |ws| -> Result<_, ToolError> {
                if depends
                    && (ws.engine().document().revision() != dc.revision
                        || ws.shown() != dc.slot
                        || ws.engine().id() != dc.engine)
                {
                    return Err(ToolError::new(
                        "REVISION_CONFLICT",
                        "the picture changed (it was edited, replaced or swapped) while the \
                         generated picture was converted for it; call apply_generated_image \
                         again",
                    ));
                }
                if apply_to == ApplyTo::Spare {
                    // The spare page asked about (or found blank) is the one it goes to.
                    if ws.shown() != dc.slot {
                        return Err(pages_swapped());
                    }
                    if !asked && spare_holds_picture(ws) {
                        return Err(ToolError::new(
                            "REVISION_CONFLICT",
                            "the spare page got a picture while the generated one was \
                             applied; call apply_generated_image again",
                        ));
                    }
                }
                ws.apply_generated(GenApply {
                    to: apply_to,
                    format,
                    palette: opts.palette,
                    result: r,
                })?;
                ws.log(&format!("apply_generated_image {id} to {}", to.name()));
                let slot = match apply_to {
                    ApplyTo::Spare => ws.shown().other(),
                    _ => ws.shown(),
                };
                Ok((
                    slot,
                    ws.engine().document().revision(),
                    describe_slot(ws, slot),
                ))
            })
            .await??;
        if to == GenTargetArg::NewDocument {
            notes.push(format!(
                "new_document is the spare page ({}) with a format of its own",
                slot.id()
            ));
        }
        Ok(success_result(&Applied {
            job_id: id.to_string(),
            target: to.name(),
            document_id: slot.id(),
            revision: document.as_ref().map_or(revision, |d| d.revision),
            width: w,
            height: h,
            colors_used: used,
            undoable: apply_to != ApplyTo::Brush,
            document,
            rounded_colors: rounded,
            notes,
        }))
    }

    #[derive(Serialize)]
    struct CancelOut {
        job_id: String,
        /// The job's state now.
        status: &'static str,
        /// It was running and is cancelled now.
        cancelled: bool,
        notes: Vec<String>,
    }

    pub(super) async fn cancel(ctx: &ToolCtx, a: JobIdArgs) -> Result<CallToolResult, ToolError> {
        let id = job_id(&a.job_id)?;
        let shown = a.job_id.clone();
        let out = ctx
            .exec
            .run(move |ws| -> Result<CancelOut, ToolError> {
                let host = ws.image_generation()?;
                let plan = host.plans.get(id).ok_or_else(|| not_found(&shown))?;
                let cancelled = host.jobs.cancel(id);
                // As get_generation_status sees it: a picture kept with the plan is done
                // (and still appliable) after the job manager dropped the job.
                let state = state_of(&plan, host.jobs.status(id).as_ref())?;
                ws.log(&format!("cancel_generation {id}"));
                let mut notes = Vec::new();
                if cancelled {
                    notes.push(format!(
                        "a request already sent to the provider ({}) may still be billed",
                        plan.provider
                    ));
                } else {
                    notes.push(format!("{id} was not running ({})", state.as_str()));
                }
                Ok(CancelOut {
                    job_id: id.to_string(),
                    status: state.as_str(),
                    cancelled,
                    notes,
                })
            })
            .await??;
        Ok(success_result(&out))
    }
}

#[cfg(feature = "imagegen")]
pub use work::{replace_question, replace_spare_question};

#[cfg(test)]
#[path = "tools_gen_tests.rs"]
mod tests;
