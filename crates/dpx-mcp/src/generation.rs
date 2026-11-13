//! What the image generation tools share with a workspace (ADR-45, wave-8 integration).
//!
//! `generate_image`, `get_generation_status`, `apply_generated_image` and `cancel_generation`
//! (`tools_gen.rs`) use the generator, the job manager (with its in-flight limit), the key
//! store and the conversion of the workspace that has them: the running GUI shares those of
//! Picture ▸ Generate.. ([`GenHost`]). A workspace without a generator (the headless
//! `deluxepainter mcp`) answers [`UNAVAILABLE`]. Nothing here ever holds or returns an API
//! key: providers read their keys themselves.
//!
//! The argument types are compiled always (the stdio bridge lists the tools); the rest needs
//! the `imagegen` feature, which the app turns on and the bridge leaves off.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a generated picture goes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GenTargetArg {
    /// The spare page (page2 while the picture is page1) becomes the picture in the format
    /// and palette it was made for; the shown picture is not touched. Undoable there.
    #[default]
    Spare,
    /// Replaces the shown picture (format, palette, every frame; undoable). DeluxePainter
    /// asks the user first.
    Picture,
    /// Becomes the custom brush, fitted to the picture's palette.
    Brush,
    /// A new layer above the active one, in the picture's page size and palette (colour 0
    /// is transparent there); undoable.
    NewLayer,
    /// The spare page with a format of its own (DeluxePainter has two pages: the picture
    /// and the spare); the same as `spare`.
    NewDocument,
}

impl GenTargetArg {
    pub fn name(self) -> &'static str {
        match self {
            GenTargetArg::Spare => "spare",
            GenTargetArg::Picture => "picture",
            GenTargetArg::Brush => "brush",
            GenTargetArg::NewLayer => "new_layer",
            GenTargetArg::NewDocument => "new_document",
        }
    }
}

/// The error code without a set-up generator.
pub const NOT_CONFIGURED: &str = "IMAGE_GENERATOR_NOT_CONFIGURED";
/// The error code of a workspace that has no image generator at all (headless).
pub const UNAVAILABLE: &str = "IMAGE_GENERATION_UNAVAILABLE";

/// Image generation is not part of this workspace (the headless server).
pub fn unavailable() -> crate::ToolError {
    crate::ToolError::new(
        UNAVAILABLE,
        "image generation runs in the DeluxePainter window (Picture > Generate..); this \
         headless server (deluxepainter mcp) has no image generator. Connect to the running \
         app (Prefs > AI & MCP), or paint or use import_image instead",
    )
}

#[cfg(feature = "imagegen")]
pub use with_generator::*;

#[cfg(feature = "imagegen")]
mod with_generator {
    use std::sync::{Arc, Mutex};

    use dpx_core::import::{ConvertOptions, Imported, PaletteChoice};
    use dpx_core::{RgbaImage, Target};
    use dpx_imagegen::{ImageGenerationProvider, JobId, JobManager, Style};

    use super::GenTargetArg;

    /// What `generate_image` takes when the client leaves an option out: the Generate
    /// requester's choices of this session, else the shown picture's format.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct GenDefaults {
        /// Chipset, mode and screen.
        pub format: Target,
        /// The page size (the screen's for a requester preset).
        pub page: (u32, u32),
        /// Palette entries with a new palette.
        pub colors: u16,
        /// Framing, scaler, palette, dither and background as Generate has them.
        pub convert: ConvertOptions,
        pub style: Style,
    }

    /// A workspace's image generator, shared with its GUI (cheap clones).
    #[derive(Clone)]
    pub struct GenHost {
        pub jobs: JobManager,
        /// The generator set up (`None`: none); its status says whether it is ready.
        pub provider: Option<Arc<dyn ImageGenerationProvider>>,
        /// The model the provider uses, if it has one to name.
        pub model: Option<String>,
        /// The generations AI clients started.
        pub plans: GenPlans,
        pub defaults: GenDefaults,
    }

    impl std::fmt::Debug for GenHost {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("GenHost")
                .field(
                    "provider",
                    &self.provider.as_ref().map(|p| p.id().to_string()),
                )
                .field("model", &self.model)
                .finish_non_exhaustive()
        }
    }

    /// One generation an AI client started, and what it was made for.
    #[derive(Clone, Debug)]
    pub struct Plan {
        pub job: JobId,
        pub target: GenTargetArg,
        pub format: Target,
        pub page: (u32, u32),
        /// Colours of a new palette (the current palette's size otherwise).
        pub colors: u16,
        pub convert: ConvertOptions,
        pub provider: String,
        /// The picture once it was fetched (kept here when the job manager drops old jobs).
        pub picture: Option<Arc<RgbaImage>>,
    }

    /// The generations AI clients started (the newest [`GenPlans::KEEP`]); other jobs of
    /// the job manager (the Generate requester's) are not theirs to see, apply or cancel.
    #[derive(Clone, Debug, Default)]
    pub struct GenPlans(Arc<Mutex<Vec<Plan>>>);

    impl GenPlans {
        /// Plans kept; older ones are forgotten.
        pub const KEEP: usize = 16;

        pub fn add(&self, plan: Plan) {
            let mut plans = self.0.lock().unwrap_or_else(|e| e.into_inner());
            plans.retain(|p| p.job != plan.job);
            plans.push(plan);
            let extra = plans.len().saturating_sub(Self::KEEP);
            plans.drain(..extra);
        }

        pub fn get(&self, job: JobId) -> Option<Plan> {
            let plans = self.0.lock().unwrap_or_else(|e| e.into_inner());
            plans.iter().find(|p| p.job == job).cloned()
        }

        /// Keeps the job's picture with its plan.
        pub fn keep_picture(&self, job: JobId, picture: Arc<RgbaImage>) {
            let mut plans = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(p) = plans.iter_mut().find(|p| p.job == job) {
                p.picture = Some(picture);
            }
        }
    }

    /// Where an applied picture goes (`new_document` is the spare page).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ApplyTo {
        Spare,
        Picture,
        Brush,
        NewLayer,
    }

    /// A converted generated picture for [`crate::Workspace::apply_generated`], as the
    /// Generate requester's Apply gets it.
    #[derive(Clone, Debug)]
    pub struct GenApply {
        pub to: ApplyTo,
        /// The format the picture was made for (the spare page and the picture take it).
        pub format: Target,
        pub palette: PaletteChoice,
        pub result: Imported,
    }
}
