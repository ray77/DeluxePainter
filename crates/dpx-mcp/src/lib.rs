//! DeluxePainter MCP server (ADR-5, ADR-21): rmcp over stdio, headless in-process engine.
//!
//! Spike scope: `app_status`, `new_document`, `get_document_state`. Every document change
//! goes through [`dpx_core::Engine`] commands, exactly like the GUI. stdout carries protocol
//! messages only; diagnostics go to stderr.

use std::sync::{Arc, Mutex, MutexGuard};

use dpx_core::{
    Chipset, Command, DisplayMode, Document, Engine, Palette, Resolution, Target, Video,
};
use rmcp::{
    ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::CallToolResult, tool,
    tool_handler, tool_router, transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SERVER_NAME: &str = "deluxepainter";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChipsetArg {
    Ocs,
    #[default]
    Aga,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VideoArg {
    #[default]
    Pal,
    Ntsc,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionArg {
    #[default]
    Lo,
    Hi,
    SuperHi,
}

/// Parameters of `new_document` (Picture > Screen Format semantics).
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct NewDocumentArgs {
    /// Amiga chipset; AGA allows 256 colours in every mode, OCS fewer. Default `aga`.
    #[serde(default)]
    pub chipset: ChipsetArg,
    /// Video standard. Default `pal`.
    #[serde(default)]
    pub video: VideoArg,
    /// Horizontal resolution: `lo` (320), `hi` (640), `super_hi` (1280). Default `lo`.
    #[serde(default)]
    pub resolution: ResolutionArg,
    /// Interlaced (double height). Default false.
    #[serde(default)]
    pub interlace: bool,
    /// Palette size, a power of two from 2 to 256. Default 32.
    pub colors: Option<u16>,
    /// Page width in pixels. Default: the mode's screen width.
    pub width: Option<u32>,
    /// Page height in pixels. Default: the mode's screen height.
    pub height: Option<u32>,
}

/// Summary of the current document.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentState {
    pub width: u32,
    pub height: u32,
    pub chipset: ChipsetArg,
    /// Display mode name, e.g. `PAL:Low Res`.
    pub display_mode: String,
    pub screen_width: u32,
    pub screen_height: u32,
    pub palette_len: usize,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Result of `app_status`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct AppStatus {
    pub name: String,
    pub version: String,
    /// Transport this server runs on.
    pub transport: String,
    /// `headless`: an in-process engine, not a running GUI (GUI bridge: later phase).
    pub engine: String,
    pub document: DocumentState,
}

fn describe(engine: &Engine) -> DocumentState {
    let doc = engine.document();
    let t = doc.target();
    DocumentState {
        width: doc.page().width(),
        height: doc.page().height(),
        chipset: match t.chipset {
            Chipset::Ocs => ChipsetArg::Ocs,
            Chipset::Aga => ChipsetArg::Aga,
        },
        display_mode: t.mode.name(),
        screen_width: t.screen.0,
        screen_height: t.screen.1,
        palette_len: doc.palette().len(),
        revision: doc.revision(),
        can_undo: engine.can_undo(),
        can_redo: engine.can_redo(),
    }
}

fn default_engine() -> Engine {
    Engine::new(Document::new(320, 256, Palette::dpaint_default_32()))
}

/// Shared server state: one headless engine.
#[derive(Clone)]
pub struct Painter {
    engine: Arc<Mutex<Engine>>,
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

impl Painter {
    pub fn new() -> Self {
        Self {
            engine: Arc::new(Mutex::new(default_engine())),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Engine> {
        self.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> AppStatus {
        AppStatus {
            name: SERVER_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            transport: "stdio".into(),
            engine: "headless".into(),
            document: describe(&self.lock()),
        }
    }

    pub fn state(&self) -> DocumentState {
        describe(&self.lock())
    }

    /// Replaces the document with a blank one. The format is validated by the engine's
    /// `SetFormat` command (same rules as the Screen Format requester); on error the current
    /// document stays untouched.
    pub fn create_document(&self, args: &NewDocumentArgs) -> Result<DocumentState, String> {
        let chipset = match args.chipset {
            ChipsetArg::Ocs => Chipset::Ocs,
            ChipsetArg::Aga => Chipset::Aga,
        };
        let video = match args.video {
            VideoArg::Pal => Video::Pal,
            VideoArg::Ntsc => Video::Ntsc,
        };
        let resolution = match args.resolution {
            ResolutionArg::Lo => Resolution::Lo,
            ResolutionArg::Hi => Resolution::Hi,
            ResolutionArg::SuperHi => Resolution::SuperHi,
        };
        let mode = DisplayMode::new(video, resolution, args.interlace);
        let (sw, sh) = mode.size();
        // Scratch engine on a document of the requested chipset; the validated command
        // produces the new document.
        let blank = Document::new(320, 256, Palette::dpaint_default_32());
        let seed = Document::from_picture(
            blank.page().clone(),
            blank.palette().clone(),
            Target::new(chipset, DisplayMode::PAL_LORES),
        );
        let mut scratch = Engine::new(seed);
        scratch
            .execute(Command::SetFormat {
                mode,
                colors: args.colors.unwrap_or(32),
                width: args.width.unwrap_or(sw),
                height: args.height.unwrap_or(sh),
                retain: false,
                screen: None,
            })
            .map_err(|e| format!("{:?}: {}", e.code, e.message))?;
        // Fresh engine: revision 0, empty undo history.
        let d = scratch.document();
        let doc = Document::from_picture(d.page().clone(), d.palette().clone(), d.target());
        let mut engine = self.lock();
        *engine = Engine::new(doc);
        Ok(describe(&engine))
    }
}

fn json_result<T: Serialize>(value: &T) -> CallToolResult {
    CallToolResult::success(vec![rmcp::model::ContentBlock::text(
        serde_json::to_string_pretty(value).expect("serialisable"),
    )])
}

#[tool_router]
impl Painter {
    #[tool(description = "Server version, transport and a summary of the current document")]
    fn app_status(&self) -> CallToolResult {
        json_result(&self.status())
    }

    #[tool(
        description = "Create a new blank document (replaces the current one). Display mode, palette size and page size are validated against the Amiga chipset rules."
    )]
    fn new_document(&self, Parameters(args): Parameters<NewDocumentArgs>) -> CallToolResult {
        match self.create_document(&args) {
            Ok(state) => json_result(&state),
            Err(message) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(message)]),
        }
    }

    #[tool(
        description = "Size, display mode, palette length, revision and undo/redo availability of the current document"
    )]
    fn get_document_state(&self) -> CallToolResult {
        json_result(&self.state())
    }
}

#[tool_handler(name = "deluxepainter")]
impl ServerHandler for Painter {}

/// Runs the server on stdin/stdout until the client disconnects. Blocking; builds its own
/// tokio runtime so the (synchronous) app entry point can call it directly.
pub fn run_stdio_blocking() -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        eprintln!("deluxepainter mcp: stdio server ready");
        let service = Painter::new().serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_default_document() {
        let s = Painter::new().status();
        assert_eq!(s.name, "deluxepainter");
        assert_eq!((s.document.width, s.document.height), (320, 256));
        assert_eq!(s.document.revision, 0);
        assert!(!s.document.can_undo);
    }

    #[test]
    fn new_document_applies_format() {
        let p = Painter::new();
        let st = p
            .create_document(&NewDocumentArgs {
                video: VideoArg::Ntsc,
                resolution: ResolutionArg::Hi,
                colors: Some(16),
                ..Default::default()
            })
            .unwrap();
        assert_eq!((st.width, st.height), (640, 200));
        assert_eq!(st.palette_len, 16);
        assert_eq!(st.display_mode, "NTSC:High Res");
        assert_eq!(st.revision, 0);
        assert_eq!(p.state(), st);
    }

    #[test]
    fn invalid_format_is_rejected_and_keeps_document() {
        let p = Painter::new();
        let before = p.state();
        let err = p
            .create_document(&NewDocumentArgs {
                chipset: ChipsetArg::Ocs,
                resolution: ResolutionArg::SuperHi,
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.contains("UnsupportedMode"), "{err}");
        let err = p
            .create_document(&NewDocumentArgs {
                colors: Some(33),
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.contains("PaletteRangeInvalid"), "{err}");
        let err = p
            .create_document(&NewDocumentArgs {
                width: Some(100),
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.contains("smaller"), "{err}");
        assert_eq!(p.state(), before);
    }
}
