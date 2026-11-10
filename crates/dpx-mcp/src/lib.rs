//! DeluxePainter MCP server (ADR-5, ADR-21): rmcp over stdio, headless in-process engine.
//!
//! Tools: `app_status`, `new_document`, `get_document_state`, `execute_command`, `undo`,
//! `redo`, `get_palette`, `get_pixels`, `render_png`, `open_picture`, `save_picture`
//! (ADR-21, ADR-24). Every document change goes through [`dpx_core::Engine`] commands,
//! exactly like the GUI. File tools only touch approved directories ([`FileAccess`]). stdout
//! carries protocol messages only; diagnostics go to stderr.

pub mod access;
pub mod error;
pub mod pictures;
pub mod view;

use std::path::PathBuf;

use std::sync::{Arc, Mutex, MutexGuard};

use dpx_core::{
    Chipset, Command, DisplayMode, Document, Engine, Palette, Resolution, Target, Video,
};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use access::FileAccess;
pub use error::ToolError;
use error::{error_result, json_or_error};
use view::{PixelEncoding, RectArg};

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
    /// Animation frames (ADR-28): how many, and the current one (counted from 0).
    pub frames: usize,
    pub current_frame: usize,
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
        frames: doc.frame_count(),
        current_frame: doc.current_frame(),
    }
}

fn default_engine() -> Engine {
    Engine::new(Document::new(320, 256, Palette::dpaint_default_32()))
}

/// Arguments of `execute_command`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct ExecuteArgs {
    /// A dpx-core `Command` as JSON, tagged by `op` (snake_case). Ops: `stroke` {brush, color,
    /// points, continuous}, `shape` {shape:{kind: line|curve|rect|ellipse|polygon|dots, ...},
    /// brush, color, filled, symmetry}, `flood_fill` {x, y, color}, `clear` {color}, `flip`
    /// {axis: horizontal|vertical}, `replace_color` {from, to}, `swap_colors` {a, b},
    /// `set_palette` {colors:[{r,g,b}]}, `set_format` {mode, colors, width, height, retain,
    /// screen}, `set_page_size` {width, height, fill}, `set_page` {width, height, pixels},
    /// `merge_page` {width, height, pixels, front, transparent}; frames (ADR-28, numbered
    /// from 0, page ops act on the current frame): `set_frame_count` {count}, `add_frames`
    /// {count}, `copy_frame` {first, last}, `insert_frame_copies` {first, last, before},
    /// `delete_frames` {first, last, fill}, `goto_frame` {frame}, `set_frame_rate` {fps}. Points and brush pixels are
    /// [x, y] pairs; a one-pixel brush is [[0, 0]]. Example: {"op":"shape","shape":{"kind":
    /// "rect","from":[10,10],"to":[50,40]},"brush":[[0,0]],"color":5,"filled":true}.
    pub command: serde_json::Value,
    /// Reject with REVISION_CONFLICT unless the document is at this revision.
    pub expected_revision: Option<u64>,
    /// Validate and report what would change without changing anything. Default false.
    #[serde(default)]
    pub dry_run: bool,
}

/// Result of `execute_command`, `undo` and `redo`.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct OutcomeInfo {
    /// Document revision afterwards (unchanged for a dry run).
    pub revision: u64,
    /// Page rectangle that changed (or would change), clipped to the page; null if none.
    pub changed_rect: Option<RectArg>,
    pub palette_changed: bool,
    /// Page size or display mode changed.
    pub format_changed: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dry_run: bool,
}

fn outcome_info(o: &dpx_core::Outcome, dry_run: bool) -> OutcomeInfo {
    OutcomeInfo {
        revision: o.revision,
        changed_rect: o.affected.map(|r| RectArg {
            x: r.x,
            y: r.y,
            w: r.w.max(0) as u32,
            h: r.h.max(0) as u32,
        }),
        palette_changed: o.palette_changed,
        format_changed: o.format_changed,
        can_undo: o.undo_available,
        can_redo: o.redo_available,
        dry_run,
    }
}

/// Arguments of `get_pixels`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PixelsArgs {
    /// Rectangle to read, clipped to the page. Default: the whole page (only if it has at
    /// most 65 536 pixels, else an error).
    pub rect: Option<RectArg>,
    /// `rows` (default): array of rows of palette indices. `base64`: raw index bytes, row by row.
    #[serde(default)]
    pub encoding: PixelEncoding,
}

/// Arguments of `render_png`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct RenderArgs {
    /// Rectangle to render, clipped to the page. Default: the whole page.
    pub rect: Option<RectArg>,
    /// Integer nearest-neighbour zoom, 1 to 8. Default 1. The image may have at most
    /// 4 194 304 pixels.
    pub scale: Option<u32>,
}

/// Arguments of `open_picture`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct OpenArgs {
    /// Path of an indexed IFF ILBM or indexed PNG inside an approved directory (relative
    /// paths start at the first approved directory).
    pub path: String,
}

/// Arguments of `save_picture`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct SaveArgs {
    /// Target path inside an approved directory; `.iff`/`.ilbm`/`.lbm` write IFF ILBM, `.png`
    /// writes indexed PNG. Other extensions are refused.
    pub path: String,
    /// Replace an existing regular file. Default false (an existing file is an error).
    #[serde(default)]
    pub overwrite: bool,
}

/// Result of `open_picture`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct OpenResult {
    pub document: DocumentState,
    /// What the document model does not keep from the file.
    pub notes: Vec<String>,
}

/// Result of `save_picture`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SaveResult {
    /// The canonical path written.
    pub path: String,
    pub format: String,
    pub bytes: usize,
    /// What the format cannot store.
    pub notes: Vec<String>,
}

/// Result of `get_palette`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PaletteResult {
    pub len: usize,
    /// One `#RRGGBB` string per palette index.
    pub colors: Vec<String>,
    pub revision: u64,
}

/// Shared server state: one headless engine and the approved directories.
#[derive(Clone)]
pub struct Painter {
    engine: Arc<Mutex<Engine>>,
    access: Arc<FileAccess>,
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

impl Painter {
    /// A server with file access off.
    pub fn new() -> Self {
        Self::with_access(FileAccess::off())
    }

    pub fn with_access(access: FileAccess) -> Self {
        Self {
            engine: Arc::new(Mutex::new(default_engine())),
            access: Arc::new(access),
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
    pub fn create_document(&self, args: &NewDocumentArgs) -> Result<DocumentState, ToolError> {
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
        scratch.execute(Command::SetFormat {
            mode,
            colors: args.colors.unwrap_or(32),
            width: args.width.unwrap_or(sw),
            height: args.height.unwrap_or(sh),
            retain: false,
            screen: None,
        })?;
        // Fresh engine: revision 0, empty undo history.
        let d = scratch.document();
        let doc = Document::from_picture(d.page().clone(), d.palette().clone(), d.target());
        let mut engine = self.lock();
        *engine = Engine::new(doc);
        Ok(describe(&engine))
    }

    /// Runs a command through the engine (the same path as the GUI).
    pub fn execute(&self, args: &ExecuteArgs) -> Result<OutcomeInfo, ToolError> {
        let command: Command = serde_json::from_value(args.command.clone())
            .map_err(|e| ToolError::invalid(format!("not a valid command: {e}")))?;
        let request = dpx_core::Request {
            command,
            expected_revision: args.expected_revision,
            dry_run: args.dry_run,
        };
        let outcome = self.lock().execute(request)?;
        Ok(outcome_info(&outcome, args.dry_run))
    }

    pub fn undo(&self) -> Result<OutcomeInfo, ToolError> {
        let o = self.lock().undo()?;
        Ok(outcome_info(&o, false))
    }

    pub fn redo(&self) -> Result<OutcomeInfo, ToolError> {
        let o = self.lock().redo()?;
        Ok(outcome_info(&o, false))
    }

    pub fn palette(&self) -> PaletteResult {
        let engine = self.lock();
        let doc = engine.document();
        PaletteResult {
            len: doc.palette().len(),
            colors: doc
                .palette()
                .colors()
                .iter()
                .map(|c| c.to_string())
                .collect(),
            revision: doc.revision(),
        }
    }

    pub fn pixels(&self, args: &PixelsArgs) -> Result<view::PixelsResult, ToolError> {
        view::get_pixels(self.lock().document(), args.rect, args.encoding)
    }

    pub fn render(&self, args: &RenderArgs) -> Result<view::Rendered, ToolError> {
        view::render_png(self.lock().document(), args.rect, args.scale)
    }

    /// Replaces the document with the picture in `path`; a failed open keeps the document.
    pub fn open(&self, path: &str) -> Result<OpenResult, ToolError> {
        let resolved = self.access.resolve_read(path)?;
        let len = std::fs::metadata(&resolved)
            .map_err(|e| ToolError::new("IO_ERROR", format!("{path}: {e}")))?
            .len();
        if len > pictures::MAX_FILE_BYTES {
            return Err(ToolError::invalid(format!(
                "{path}: {len} bytes exceed the {} byte limit",
                pictures::MAX_FILE_BYTES
            )));
        }
        let bytes = std::fs::read(&resolved)
            .map_err(|e| ToolError::new("IO_ERROR", format!("{path}: {e}")))?;
        let (doc, notes) = pictures::decode_picture(path, &bytes)?;
        let mut engine = self.lock();
        *engine = Engine::new(doc);
        Ok(OpenResult {
            document: describe(&engine),
            notes,
        })
    }

    /// Writes the document after validating its hardware limits.
    pub fn save(&self, path: &str, overwrite: bool) -> Result<SaveResult, ToolError> {
        let target = self.access.resolve_write(path, overwrite)?;
        let format = pictures::format_for_save(path)?;
        let (bytes, notes) = pictures::encode_document(self.lock().document(), format)?;
        FileAccess::write_file(&target, &bytes, overwrite)?;
        Ok(SaveResult {
            path: target.display().to_string(),
            format: match format {
                pictures::Format::Ilbm => "iff_ilbm".into(),
                pictures::Format::Png => "png_indexed".into(),
                pictures::Format::Anim => "iff_anim".into(),
            },
            bytes: bytes.len(),
            notes,
        })
    }
}

#[tool_router]
impl Painter {
    #[tool(description = "Server version, transport and a summary of the current document")]
    fn app_status(&self) -> CallToolResult {
        json_or_error(Ok::<_, ToolError>(self.status()))
    }

    #[tool(
        description = "Create a new blank document (replaces the current one). Display mode, palette size and page size are validated against the Amiga chipset rules."
    )]
    fn new_document(&self, Parameters(args): Parameters<NewDocumentArgs>) -> CallToolResult {
        json_or_error(self.create_document(&args))
    }

    #[tool(
        description = "Size, display mode, palette length, revision and undo/redo availability of the current document"
    )]
    fn get_document_state(&self) -> CallToolResult {
        json_or_error(Ok::<_, ToolError>(self.state()))
    }

    #[tool(
        description = "Run one document command (stroke, shape, flood_fill, clear, flip, replace_color, swap_colors, set_palette, set_format, set_page_size, set_page, merge_page) through the command engine as one undo step. Optional expected_revision guards against concurrent changes; dry_run validates only. Returns the new revision and the changed page rectangle."
    )]
    fn execute_command(&self, Parameters(args): Parameters<ExecuteArgs>) -> CallToolResult {
        json_or_error(self.execute(&args))
    }

    #[tool(
        name = "undo",
        description = "Undo the last command. Error NOTHING_TO_UNDO if there is none."
    )]
    fn undo_tool(&self) -> CallToolResult {
        json_or_error(Painter::undo(self))
    }

    #[tool(
        name = "redo",
        description = "Redo the last undone command. Error NOTHING_TO_REDO if there is none."
    )]
    fn redo_tool(&self) -> CallToolResult {
        json_or_error(Painter::redo(self))
    }

    #[tool(description = "The palette: one #RRGGBB string per colour index")]
    fn get_palette(&self) -> CallToolResult {
        json_or_error(Ok::<_, ToolError>(self.palette()))
    }

    #[tool(
        description = "Read palette indices of a page rectangle (at most 65536 pixels per call; the rect is clipped to the page). Returns rows of numbers, or base64 raw bytes with encoding=base64."
    )]
    fn get_pixels(&self, Parameters(args): Parameters<PixelsArgs>) -> CallToolResult {
        json_or_error(self.pixels(&args))
    }

    #[tool(
        description = "Render the page, or a rectangle of it, as a PNG image with the palette applied, so you can look at your work. Optional integer scale 1..8 (nearest neighbour)."
    )]
    fn render_png(&self, Parameters(args): Parameters<RenderArgs>) -> CallToolResult {
        match self.render(&args) {
            Ok(r) => {
                let info = serde_json::json!({
                    "rect": {"x": r.rect.x, "y": r.rect.y, "w": r.rect.w, "h": r.rect.h},
                    "scale": r.scale,
                    "image_width": r.width,
                    "image_height": r.height,
                });
                CallToolResult::success(vec![
                    ContentBlock::image(view::to_base64(&r.png), "image/png"),
                    ContentBlock::text(info.to_string()),
                ])
            }
            Err(e) => error_result(&e),
        }
    }

    #[tool(
        description = "Open an indexed IFF ILBM or indexed PNG from an approved directory, replacing the current document. True-colour PNG and JPEG are not supported. Needs the server to be started with --allow-dir."
    )]
    fn open_picture(&self, Parameters(args): Parameters<OpenArgs>) -> CallToolResult {
        json_or_error(self.open(&args.path))
    }

    #[tool(
        description = "Save the document as IFF ILBM (.iff/.ilbm/.lbm) or indexed PNG (.png) in an approved directory. Hardware limits of the display mode are validated and never reduced silently. Refuses to replace a file unless overwrite=true. Needs --allow-dir."
    )]
    fn save_picture(&self, Parameters(args): Parameters<SaveArgs>) -> CallToolResult {
        json_or_error(self.save(&args.path, args.overwrite))
    }
}

#[tool_handler(name = "deluxepainter")]
impl ServerHandler for Painter {
    /// Like the generated handler, but argument deserialisation errors become ordinary
    /// `INVALID_ARGUMENT: ...` error results (ADR-24) instead of JSON-RPC errors.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        match Self::tool_router().call(tcc).await? {
            rmcp::model::CallToolResponse::Complete(r) => Ok(invalid_argument(r).into()),
            other => Ok(other),
        }
    }
}

/// rmcp reports a tool-argument deserialisation failure as an `isError` result reading
/// `failed to deserialize parameters: ...`; this rewrites it to `INVALID_ARGUMENT: ...`.
fn invalid_argument(r: CallToolResult) -> CallToolResult {
    const PREFIX: &str = "failed to deserialize parameters:";
    if r.is_error == Some(true)
        && let Some(detail) = r
            .content
            .first()
            .and_then(|c| c.as_text())
            .and_then(|t| t.text.strip_prefix(PREFIX))
    {
        return error_result(&ToolError::invalid(detail.trim_start()));
    }
    r
}

const USAGE: &str = "usage: deluxepainter mcp [--allow-dir DIR]...";

/// `deluxepainter mcp [--allow-dir DIR]...`: parses the arguments after `mcp` and runs the
/// server on stdio. Each `--allow-dir` must be an existing directory.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let mut dirs = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--allow-dir" {
            let d = it
                .next()
                .ok_or_else(|| format!("--allow-dir needs a directory\n{USAGE}"))?;
            dirs.push(PathBuf::from(d));
        } else if let Some(d) = a.strip_prefix("--allow-dir=") {
            dirs.push(PathBuf::from(d));
        } else {
            return Err(format!("unknown argument {a}\n{USAGE}"));
        }
    }
    let access = FileAccess::new(dirs)?;
    run_stdio_blocking(access).map_err(|e| e.to_string())
}

/// Runs the server on stdin/stdout until the client disconnects. Blocking; builds its own
/// tokio runtime so the (synchronous) app entry point can call it directly.
pub fn run_stdio_blocking(access: FileAccess) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        if access.is_on() {
            for d in access.dirs() {
                eprintln!("deluxepainter mcp: approved directory {}", d.display());
            }
        } else {
            eprintln!("deluxepainter mcp: file access off (use --allow-dir DIR)");
        }
        eprintln!("deluxepainter mcp: stdio server ready");
        let service = Painter::with_access(access).serve(stdio()).await?;
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
        assert_eq!(err.code, "UNSUPPORTED_MODE", "{err}");
        let err = p
            .create_document(&NewDocumentArgs {
                colors: Some(33),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(err.code, "PALETTE_RANGE_INVALID", "{err}");
        let err = p
            .create_document(&NewDocumentArgs {
                width: Some(100),
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.message.contains("smaller"), "{err}");
        assert_eq!(p.state(), before);
    }

    fn rect_cmd(color: u8) -> ExecuteArgs {
        ExecuteArgs {
            command: serde_json::json!({
                "op": "shape",
                "shape": {"kind": "rect", "from": [10, 10], "to": [19, 14]},
                "brush": [[0, 0]], "color": color, "filled": true
            }),
            expected_revision: None,
            dry_run: false,
        }
    }

    fn pixel(p: &Painter, x: i32, y: i32) -> u64 {
        let r = p
            .pixels(&PixelsArgs {
                rect: Some(RectArg { x, y, w: 1, h: 1 }),
                encoding: PixelEncoding::Rows,
            })
            .unwrap();
        r.pixels[0][0].as_u64().unwrap()
    }

    #[test]
    fn execute_undo_redo_and_errors() {
        let p = Painter::new();
        let o = p.execute(&rect_cmd(5)).unwrap();
        assert_eq!(o.revision, 1);
        assert_eq!(
            o.changed_rect,
            Some(RectArg {
                x: 10,
                y: 10,
                w: 10,
                h: 5
            })
        );
        assert_eq!(pixel(&p, 12, 12), 5);
        // Dry run changes nothing.
        let mut dry = rect_cmd(6);
        dry.dry_run = true;
        assert_eq!(p.execute(&dry).unwrap().revision, 1);
        assert_eq!(pixel(&p, 12, 12), 5);
        // Revision conflict, bad colour, bad JSON keep their codes.
        let mut stale = rect_cmd(6);
        stale.expected_revision = Some(0);
        assert_eq!(p.execute(&stale).unwrap_err().code, "REVISION_CONFLICT");
        assert_eq!(
            p.execute(&rect_cmd(200)).unwrap_err().code,
            "PALETTE_RANGE_INVALID"
        );
        let bad = ExecuteArgs {
            command: serde_json::json!({"op": "nope"}),
            expected_revision: None,
            dry_run: false,
        };
        assert_eq!(p.execute(&bad).unwrap_err().code, "INVALID_ARGUMENT");
        p.undo().unwrap();
        assert_eq!(pixel(&p, 12, 12), 0);
        assert_eq!(p.undo().unwrap_err().code, "NOTHING_TO_UNDO");
        p.redo().unwrap();
        assert_eq!(pixel(&p, 12, 12), 5);
        assert_eq!(p.redo().unwrap_err().code, "NOTHING_TO_REDO");
    }

    #[test]
    fn pixels_limits_and_encodings() {
        let p = Painter::new();
        let whole = p.pixels(&PixelsArgs {
            rect: None,
            encoding: PixelEncoding::Rows,
        });
        assert_eq!(whole.unwrap_err().code, "INVALID_ARGUMENT", "320x256 > cap");
        let r = p
            .pixels(&PixelsArgs {
                rect: Some(RectArg {
                    x: 300,
                    y: 250,
                    w: 100,
                    h: 100,
                }),
                encoding: PixelEncoding::Base64,
            })
            .unwrap();
        assert_eq!((r.x, r.y, r.w, r.h), (300, 250, 20, 6), "clipped");
        assert_eq!(r.pixels.as_str().unwrap().len(), 160); // 120 bytes
        let outside = PixelsArgs {
            rect: Some(RectArg {
                x: 400,
                y: 0,
                w: 5,
                h: 5,
            }),
            encoding: PixelEncoding::Rows,
        };
        assert!(p.pixels(&outside).is_err());
    }

    #[test]
    fn render_scales_and_applies_palette() {
        let p = Painter::new();
        p.execute(&rect_cmd(5)).unwrap();
        let r = p
            .render(&RenderArgs {
                rect: Some(RectArg {
                    x: 8,
                    y: 8,
                    w: 4,
                    h: 4,
                }),
                scale: Some(3),
            })
            .unwrap();
        assert_eq!((r.width, r.height), (12, 12));
        let dec = png::Decoder::new(std::io::Cursor::new(&r.png));
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut buf).unwrap();
        let want = p.palette().colors[5].clone();
        let at = |x: usize, y: usize| {
            format!(
                "#{:02X}{:02X}{:02X}",
                buf[(y * 12 + x) * 3],
                buf[(y * 12 + x) * 3 + 1],
                buf[(y * 12 + x) * 3 + 2]
            )
        };
        assert_eq!(at(11, 11), want, "inside the rectangle");
        assert_ne!(at(0, 0), want, "outside");
        assert!(
            p.render(&RenderArgs {
                rect: None,
                scale: Some(9)
            })
            .is_err()
        );
    }

    #[test]
    fn file_tools_are_off_without_allow_dir() {
        let p = Painter::new();
        assert_eq!(p.open("x.iff").unwrap_err().code, "FILE_ACCESS_OFF");
        assert_eq!(p.save("x.iff", false).unwrap_err().code, "FILE_ACCESS_OFF");
    }

    #[test]
    fn save_and_open_round_trip() {
        let dir = access::tests::scratch("pic");
        let p = Painter::with_access(FileAccess::new([dir.clone()]).unwrap());
        p.execute(&rect_cmd(5)).unwrap();
        let before = p.palette().colors;
        for name in ["a.iff", "a.png"] {
            let r = p.save(name, false).unwrap();
            assert!(r.bytes > 0);
            assert_eq!(p.save(name, false).unwrap_err().code, "FILE_EXISTS");
            p.save(name, true).unwrap();
        }
        for name in ["a.iff", "a.png"] {
            let q = Painter::with_access(FileAccess::new([dir.clone()]).unwrap());
            let o = q.open(name).unwrap();
            assert_eq!(o.document.revision, 0);
            assert_eq!(q.palette().colors, before);
            assert_eq!(pixel(&q, 12, 12), 5, "{name}");
        }
        assert_eq!(
            p.save("a.jpg", false).unwrap_err().code,
            "UNSUPPORTED_FILE_TYPE"
        );
        assert_eq!(
            p.save("../esc.iff", false).unwrap_err().code,
            "PATH_NOT_ALLOWED"
        );
        // A failed open keeps the document.
        std::fs::write(dir.join("bad.iff"), b"junk").unwrap();
        let rev = p.state().revision;
        assert!(p.open("bad.iff").is_err());
        assert_eq!(p.state().revision, rev);
    }
}
