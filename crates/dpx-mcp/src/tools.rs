//! The MCP tools and resources (ADR-43; tool list after the owner's spec §5.2, layers and
//! image generation come with ADR-44/45).
//!
//! Every tool is an async function on the server side: it checks the permissions, parses its
//! arguments, does file I/O and encoding on the server's threads and touches the editor state
//! only through jobs on the [`Workspace`] owner ([`Executor::run`]). Whole calls run one at a
//! time in arrival order ([`crate::seq`]), so a tool's jobs never interleave with another
//! call's. Every result is `{success, ...}` with `document_id` and `revision` where they
//! apply; failures are `{success: false, code, message}` (`isError`).
//!
//! Tools read the picture for previews, pixels and exports through [`visible_picture`] only.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dpx_core::import::{self, ImportOptions, ImportTarget, PaletteChoice, Position};
use dpx_core::quantize::{Dither, PaletteDepth, Reduction};
use dpx_core::{
    Chipset, Command, DisplayMode, Document, Engine, IndexedImage, Outcome, Palette, Request,
    Resolution, Rgb8, Shape, Target, Video,
};
use rmcp::model::{
    CallToolResult, JsonObject, ReadResourceResult, Resource, ResourceContents, Tool,
    ToolAnnotations,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::access::FileAccess;
use crate::error::{ToolError, error_result, image_result, success_result};
use crate::jobs::Executor;
use crate::pictures;
use crate::state::{Overwrite, ServerState};
use crate::view::{self, PixelEncoding, RectArg};
use crate::workspace::{Brush, PictureInfo, Question, Slot, Workspace};

pub const SERVER_NAME: &str = "deluxepainter";

/// Instructions sent to clients with `initialize`.
pub const INSTRUCTIONS: &str = "DeluxePainter is an Amiga-style pixel editor (DeluxePaint IV AGA). \
A document is an indexed picture: every pixel is a palette index, drawing tools take a palette \
index as `color` (see get_palette). OCS documents allow 32 colours in Low Res and 16 in High Res \
with 12-bit colours; AGA allows 256 colours with 24-bit colours. Every change is one undo step \
unless the result says `undoable: false`. Use get_preview to look at the picture; save with \
save_project (.dpxproj) and export with export_image (IFF ILBM, IFF ANIM, PNG). File paths must \
lie in the approved directories (get_app_status lists them).";

/// What the server shares with every call.
#[derive(Clone, Debug)]
pub struct ToolCtx {
    pub exec: Executor,
    pub state: Arc<ServerState>,
    /// `stdio` or `http`.
    pub transport: &'static str,
}

impl ToolCtx {
    /// The context of a changing call: its jobs are marked [`crate::jobs::JobKind::Edit`], so
    /// the GUI can hold or refuse them before they run.
    pub fn editing(&self) -> ToolCtx {
        ToolCtx {
            exec: self.exec.editing(),
            ..self.clone()
        }
    }
}

/// The picture the tools look at (previews, pixels, exports): today the shown page.
// TODO(wave8-integration): return the visible layer composite (ADR-44) here.
pub fn visible_picture(ws: &dyn Workspace) -> Cow<'_, Document> {
    Cow::Borrowed(ws.engine().document())
}

// ---------------------------------------------------------------------------------------
// Arguments

/// No arguments.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum ChipsetArg {
    #[serde(rename = "ocs", alias = "OCS", alias = "Ocs")]
    Ocs,
    #[serde(rename = "aga", alias = "AGA", alias = "Aga")]
    Aga,
}

impl ChipsetArg {
    fn of(c: Chipset) -> Self {
        match c {
            Chipset::Ocs => Self::Ocs,
            Chipset::Aga => Self::Aga,
        }
    }

    fn chipset(self) -> Chipset {
        match self {
            Self::Ocs => Chipset::Ocs,
            Self::Aga => Chipset::Aga,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum VideoArg {
    #[serde(rename = "pal", alias = "PAL")]
    Pal,
    #[serde(rename = "ntsc", alias = "NTSC")]
    Ntsc,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionArg {
    /// 320 pixels per line.
    Lo,
    /// 640 pixels per line.
    Hi,
    /// 1280 pixels per line (AGA only).
    SuperHi,
}

/// Picture formats by name (owner's spec §3.1).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PresetArg {
    /// NTSC Low Res, 320x200.
    NtscLores,
    /// PAL Low Res, 320x256.
    PalLores,
    /// 320x240: PAL Low Res with a custom 320x240 screen (not a standard PAL or NTSC size).
    #[serde(rename = "pixelart_320x240")]
    Pixelart320x240,
    /// NTSC High Res, 640x200.
    NtscHires,
    /// PAL High Res, 640x256.
    PalHires,
    /// NTSC High Res Laced, 640x400.
    NtscInterlace,
    /// PAL High Res Laced, 640x512.
    PalInterlace,
}

impl PresetArg {
    // TODO(wave8-integration): take these from the shared preset table of ADR-46.
    fn format(self) -> (DisplayMode, (u32, u32)) {
        let m = |v, r, l| DisplayMode::new(v, r, l);
        let (mode, screen) = match self {
            Self::NtscLores => (m(Video::Ntsc, Resolution::Lo, false), None),
            Self::PalLores => (m(Video::Pal, Resolution::Lo, false), None),
            Self::Pixelart320x240 => (m(Video::Pal, Resolution::Lo, false), Some((320, 240))),
            Self::NtscHires => (m(Video::Ntsc, Resolution::Hi, false), None),
            Self::PalHires => (m(Video::Pal, Resolution::Hi, false), None),
            Self::NtscInterlace => (m(Video::Ntsc, Resolution::Hi, true), None),
            Self::PalInterlace => (m(Video::Pal, Resolution::Hi, true), None),
        };
        (mode, screen.unwrap_or(mode.size()))
    }
}

/// Arguments of `create_document`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDocumentArgs {
    /// Page width in pixels. With only width and height given, the display mode is chosen to
    /// match: 320x200 NTSC Low Res, 320x256 PAL Low Res, 320x240 PAL Low Res with a custom
    /// 320x240 screen, 640x200/640x256 High Res, 640x400/640x512 High Res Laced.
    pub width: Option<u32>,
    /// Page height in pixels.
    pub height: Option<u32>,
    /// `ocs` (12-bit colours; 32 colours in Low Res, 16 in High Res, no Super-High Res) or
    /// `aga` (24-bit colours, 256 everywhere). Default: the current document's chipset.
    pub chipset: Option<ChipsetArg>,
    /// Palette size: a power of two from 2 to the mode's maximum. Default 32 (16 on OCS
    /// High Res).
    #[serde(alias = "palette_size")]
    pub colors: Option<u16>,
    /// `pal` or `ntsc` (chooses the mode explicitly, with `resolution` and `interlace`).
    pub video: Option<VideoArg>,
    /// `lo` (320), `hi` (640) or `super_hi` (1280).
    pub resolution: Option<ResolutionArg>,
    /// Interlaced (double height).
    pub interlace: Option<bool>,
    /// A named format instead of video/resolution/interlace; width/height default to it.
    pub preset: Option<PresetArg>,
    /// A name for the picture (shown as its title until it is saved).
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentIdArgs {
    /// `page1` or `page2` (see list_documents). Default: the active document.
    pub document_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelectDocumentArgs {
    /// `page1` or `page2` (see list_documents).
    pub document_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPaletteColorArgs {
    /// Palette index (0 = background colour).
    pub index: u8,
    /// `#RRGGBB`, or `#RGB` for a 12-bit OCS colour (each digit doubled: `#F80` = `#FF8800`).
    pub color: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DrawPixelArgs {
    pub x: i32,
    pub y: i32,
    /// Palette index.
    pub color: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DrawLineArgs {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    /// Palette index.
    pub color: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DrawRectangleArgs {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Palette index.
    pub color: u8,
    /// Solid instead of a 1-pixel outline. Default false.
    #[serde(default)]
    pub filled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DrawEllipseArgs {
    /// Centre x.
    pub cx: i32,
    /// Centre y.
    pub cy: i32,
    /// Horizontal radius; equal radii give a circle (in pixels, not corrected for the
    /// pixel aspect).
    pub rx: u32,
    /// Vertical radius.
    pub ry: u32,
    /// Palette index.
    pub color: u8,
    /// Solid instead of a 1-pixel outline. Default false.
    #[serde(default)]
    pub filled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillAreaArgs {
    pub x: i32,
    pub y: i32,
    /// Palette index; the 4-connected area of the colour at (x, y) takes it.
    pub color: u8,
}

/// Arguments of `execute_command`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteArgs {
    /// A dpx-core `Command` as JSON, tagged by `op` (snake_case). Ops: `stroke` {brush, color,
    /// points, continuous}, `shape` {shape:{kind: line|curve|rect|ellipse|polygon|dots, ...},
    /// brush, color, filled, symmetry}, `flood_fill` {x, y, color}, `clear` {color}, `flip`
    /// {axis: horizontal|vertical}, `replace_color` {from, to}, `swap_colors` {a, b},
    /// `set_palette` {colors:[{r,g,b}]}, `set_format` {mode, colors, width, height, retain,
    /// screen}, `set_page_size` {width, height, fill}, `set_page` {width, height, pixels},
    /// `merge_page` {width, height, pixels, front, transparent}; frames (numbered from 0, page
    /// ops act on the current frame): `set_frame_count` {count}, `add_frames` {count},
    /// `copy_frame` {first, last}, `insert_frame_copies` {first, last, before},
    /// `delete_frames` {first, last, fill}, `goto_frame` {frame}, `set_frame_rate` {fps}.
    /// Points and brush pixels are [x, y] pairs; a one-pixel brush is [[0, 0]]. Example:
    /// {"op":"shape","shape":{"kind":"rect","from":[10,10],"to":[50,40]},"brush":[[0,0]],
    /// "color":5,"filled":true}.
    pub command: serde_json::Value,
    /// Reject with REVISION_CONFLICT unless the document is at this revision.
    pub expected_revision: Option<u64>,
    /// Validate and report what would change without changing anything. Default false.
    #[serde(default)]
    pub dry_run: bool,
}

/// Arguments of `get_pixels`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PixelsArgs {
    /// Rectangle to read, clipped to the page (at most 65536 pixels). Default: the whole page
    /// if it has no more.
    pub rect: Option<RectArg>,
    /// `rows` (default): rows of palette indices. `base64`: raw index bytes, row by row.
    #[serde(default)]
    pub encoding: PixelEncoding,
}

/// Arguments of `get_preview`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewArgs {
    /// Rectangle to show, clipped to the page. Default: the whole page.
    pub rect: Option<RectArg>,
    /// Integer nearest-neighbour zoom 1..8. Default: up to 4, keeping the longer side within
    /// 1024 pixels. The image may have at most 4194304 pixels.
    pub scale: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenArgs {
    /// An IFF ILBM, IFF ANIM, indexed PNG or .dpxproj file inside an approved directory
    /// (relative paths start at the first approved directory).
    pub path: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportTargetArg {
    /// Replace the picture (keeps its display mode, screen and page size; not undoable).
    #[default]
    Picture,
    /// Become the spare page (Picture > Spare; uses the picture's palette).
    Spare,
    /// Become the custom brush (in the picture's palette, at its own pixel size).
    Brush,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PositionArg {
    #[default]
    #[serde(alias = "center")]
    Centre,
    TopLeft,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaletteArg {
    /// A new palette made from the picture (target picture only).
    New,
    /// The document's palette as it is.
    Current,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DitherArg {
    None,
    #[default]
    FloydSteinberg,
    Ordered,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportImageArgs {
    /// A PNG or JPEG file inside an approved directory. Give `path` or `data`.
    pub path: Option<String>,
    /// A base64 PNG or JPEG (at most 12 MiB decoded; a `data:` URL is accepted).
    pub data: Option<String>,
    /// `picture` (default), `spare` or `brush`.
    #[serde(default)]
    pub target: ImportTargetArg,
    /// `centre` (default) or `top_left`: where the fitted picture lies on the page; the rest
    /// is the BG colour.
    #[serde(default)]
    pub position: PositionArg,
    /// `new` (default for the picture) or `current` (always for spare and brush).
    pub palette: Option<PaletteArg>,
    /// Colours of a new palette: a power of two up to the mode's maximum. Default: the
    /// document's palette size.
    pub colors: Option<u16>,
    /// `none`, `floyd_steinberg` (default) or `ordered`.
    #[serde(default)]
    pub dither: DitherArg,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveArgs {
    /// A .dpxproj path inside an approved directory.
    pub path: String,
    /// Replace an existing file (the user may be asked first). Default false: an existing
    /// file is an error.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormatArg {
    IffIlbm,
    IffAnim,
    Png,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportArgs {
    /// Target path inside an approved directory: .iff/.ilbm/.lbm (IFF ILBM), .anim (IFF
    /// ANIM, every frame) or .png (indexed PNG).
    pub path: String,
    /// The format, if the extension does not say it.
    pub format: Option<ExportFormatArg>,
    /// Replace an existing file (the user may be asked first). Default false.
    #[serde(default)]
    pub overwrite: bool,
}

// ---------------------------------------------------------------------------------------
// The tool table

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Needs {
    Nothing,
    Read,
    Edit,
}

struct Spec {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    needs: Needs,
    schema: fn() -> Arc<JsonObject>,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
}

fn schema<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    rmcp::handler::server::common::schema_for_input::<T>().expect("valid input schema")
}

const fn read(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    schema: fn() -> Arc<JsonObject>,
) -> Spec {
    Spec {
        name,
        title,
        description,
        needs: Needs::Read,
        schema,
        read_only: true,
        destructive: false,
        idempotent: true,
    }
}

const fn edit(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    schema: fn() -> Arc<JsonObject>,
    destructive: bool,
) -> Spec {
    Spec {
        name,
        title,
        description,
        needs: Needs::Edit,
        schema,
        read_only: false,
        destructive,
        idempotent: false,
    }
}

static SPECS: &[Spec] = &[
    Spec {
        needs: Needs::Nothing,
        ..read(
            "get_app_status",
            "App status",
            "Version, engine (gui or headless), transport, permissions, approved directories and a summary of the active document.",
            schema::<NoArgs>,
        )
    },
    read(
        "list_documents",
        "List documents",
        "The two pages (picture and spare, ids page1 and page2) with size, chipset, mode and colours; `active` marks the one the tools work on.",
        schema::<NoArgs>,
    ),
    read(
        "get_document_info",
        "Document info",
        "Size, display mode, screen, chipset, palette size and maximum, frames, revision, undo/redo and unsaved changes of a document (default: the active one).",
        schema::<DocumentIdArgs>,
    ),
    edit(
        "create_document",
        "New picture",
        "Create a new blank picture with width, height, chipset (ocs|aga) and colours (or palette_size); optional video, resolution, interlace, preset, name. Hardware limits are checked: OCS allows 32 colours in Low Res, 16 in High Res. Undoable when the chipset stays; a chipset change replaces the picture (undoable: false in the result).",
        schema::<CreateDocumentArgs>,
        false,
    ),
    edit(
        "select_document",
        "Select document",
        "Make page1 or page2 the active (shown) page, like Picture > Spare > Swap. Selecting the active page changes nothing.",
        schema::<SelectDocumentArgs>,
        false,
    ),
    read(
        "get_palette",
        "Get palette",
        "The palette: one #RRGGBB string per colour index, the palette size, the mode's maximum and the bits per channel (4 on OCS, 8 on AGA).",
        schema::<NoArgs>,
    ),
    Spec {
        idempotent: true,
        ..edit(
            "set_palette_color",
            "Set palette colour",
            "Change one palette entry (one undo step). OCS documents take 12-bit colours only: other colours are refused with the nearest 12-bit colour in the message.",
            schema::<SetPaletteColorArgs>,
            false,
        )
    },
    edit(
        "draw_pixel",
        "Draw pixel",
        "Set one pixel to a palette index (one undo step).",
        schema::<DrawPixelArgs>,
        false,
    ),
    edit(
        "draw_line",
        "Draw line",
        "Draw a 1-pixel line from (x1, y1) to (x2, y2) in a palette index (one undo step).",
        schema::<DrawLineArgs>,
        false,
    ),
    edit(
        "draw_rectangle",
        "Draw rectangle",
        "Draw a rectangle (x, y, width, height), as a 1-pixel outline or filled, in a palette index (one undo step).",
        schema::<DrawRectangleArgs>,
        false,
    ),
    edit(
        "draw_ellipse",
        "Draw ellipse",
        "Draw an ellipse or circle around (cx, cy) with radii rx, ry, as a 1-pixel outline or filled (one undo step).",
        schema::<DrawEllipseArgs>,
        false,
    ),
    edit(
        "fill_area",
        "Fill area",
        "Flood-fill the 4-connected area of the colour at (x, y) with a palette index (one undo step).",
        schema::<FillAreaArgs>,
        false,
    ),
    edit(
        "import_image",
        "Import image",
        "Convert a PNG or JPEG (a path in an approved directory, or base64 data) into the document's format: fitted into the page, centre or top_left, a new palette of `colors` or the current palette, dither none|floyd_steinberg|ordered. Target picture (replaces it, not undoable), spare or brush.",
        schema::<ImportImageArgs>,
        true,
    ),
    edit(
        "open_picture",
        "Open picture",
        "Open an IFF ILBM, IFF ANIM, indexed PNG or .dpxproj project from an approved directory, replacing the active page (not undoable). True-colour PNG and JPEG go through import_image.",
        schema::<OpenArgs>,
        true,
    ),
    edit(
        "undo",
        "Undo",
        "Undo the last change of the active page. Error NOTHING_TO_UNDO if there is none.",
        schema::<NoArgs>,
        false,
    ),
    edit(
        "redo",
        "Redo",
        "Redo the last undone change. Error NOTHING_TO_REDO if there is none.",
        schema::<NoArgs>,
        false,
    ),
    Spec {
        needs: Needs::Read,
        ..edit(
            "save_project",
            "Save project",
            "Save the active page losslessly as a .dpxproj project in an approved directory. An existing file is kept unless overwrite=true (the user may be asked).",
            schema::<SaveArgs>,
            true,
        )
    },
    Spec {
        needs: Needs::Read,
        ..edit(
            "export_image",
            "Export image",
            "Export the picture as IFF ILBM (.iff/.ilbm/.lbm), IFF ANIM (.anim) or indexed PNG (.png) in an approved directory. The chipset's limits are checked, nothing is reduced silently. An existing file is kept unless overwrite=true (the user may be asked).",
            schema::<ExportArgs>,
            true,
        )
    },
    read(
        "get_preview",
        "Preview",
        "The picture (or a rectangle of it) as a PNG image with the palette applied, integer-scaled (default up to 4x, at most 1024 pixels on the longer side), so you can look at it.",
        schema::<PreviewArgs>,
    ),
    read(
        "get_pixels",
        "Get pixels",
        "Palette indices of a small rectangle (at most 65536 pixels, clipped to the page), as rows or base64 bytes. Use get_preview to look at the whole picture.",
        schema::<PixelsArgs>,
    ),
    edit(
        "execute_command",
        "Execute command (advanced)",
        "Run one raw dpx-core command (JSON tagged by op: stroke, shape, flood_fill, clear, flip, replace_color, swap_colors, set_palette, set_format, set_page_size, set_page, merge_page and the frame ops) as one undo step. Optional expected_revision and dry_run.",
        schema::<ExecuteArgs>,
        false,
    ),
];

fn find(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.name == name)
}

/// The tool definitions (also listed by the bridge without a running app).
pub fn definitions() -> Vec<Tool> {
    SPECS
        .iter()
        .map(|s| {
            Tool::new(s.name, s.description, (s.schema)())
                .with_title(s.title)
                .with_annotations(ToolAnnotations::from_raw(
                    Some(s.title.to_string()),
                    Some(s.read_only),
                    Some(s.destructive),
                    Some(s.idempotent),
                    Some(false),
                ))
        })
        .collect()
}

/// The names of every tool.
pub fn names() -> Vec<&'static str> {
    SPECS.iter().map(|s| s.name).collect()
}

/// True for tools that change nothing (safe to retry).
pub fn is_read_only(name: &str) -> bool {
    find(name).is_some_and(|s| s.read_only)
}

/// Reading gates every tool but `get_app_status` (a changing tool reports the document it
/// changed); changing needs both.
fn permission(needs: Needs, ctx: &ToolCtx) -> Result<(), ToolError> {
    let s = ctx.state.settings();
    let denied = |what: &str| {
        Err(ToolError::new(
            "PERMISSION_DENIED",
            format!("{what} is turned off for AI clients (Prefs > AI & MCP)"),
        ))
    };
    match needs {
        Needs::Nothing => Ok(()),
        _ if !s.read => denied("reading the document"),
        Needs::Edit if !s.edit => denied("changing the document"),
        _ => Ok(()),
    }
}

fn parse<T: DeserializeOwned>(args: Option<JsonObject>) -> Result<T, ToolError> {
    serde_json::from_value(serde_json::Value::Object(args.unwrap_or_default()))
        .map_err(|e| ToolError::invalid(e.to_string()))
}

/// Runs tool `name`; `None` if there is no such tool.
/// Every call ends with one entry in the activity log (tool, ok or the error code, what it
/// did).
pub async fn call(ctx: &ToolCtx, name: &str, args: Option<JsonObject>) -> Option<CallToolResult> {
    let spec = find(name)?;
    let activity = ctx.state.activity();
    activity.tool_call();
    let result = match permission(spec.needs, ctx) {
        Ok(()) if spec.needs == Needs::Edit => dispatch(&ctx.editing(), name, args).await,
        Ok(()) => dispatch(ctx, name, args).await,
        Err(e) => Err(e),
    };
    match &result {
        Ok(_) => activity.finish(name, None),
        Err(e) => activity.finish(name, Some((&e.code, &e.message))),
    }
    Some(result.unwrap_or_else(|e| error_result(&e)))
}

/// One tool call for [`spawn_calls`]: the tool's name and its arguments.
pub type CallSpec = (String, Option<JsonObject>);

/// Runs `calls` one after another on a thread with a runtime of its own and sends each
/// result as it is ready (an unknown tool gives `None`). The owner keeps running its job
/// queue meanwhile; the receiver disconnects after the last call. For in-process drivers
/// without a transport: the GUI's `--mcp-calls` snapshot flag and its tests.
pub fn spawn_calls(
    ctx: ToolCtx,
    calls: Vec<CallSpec>,
) -> std::sync::mpsc::Receiver<(String, Option<CallToolResult>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("dpx-mcp-calls".into())
        .spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            for (name, args) in calls {
                let result = rt.block_on(call(&ctx, &name, args));
                if tx.send((name, result)).is_err() {
                    break;
                }
            }
        })
        .expect("spawn the call thread");
    rx
}

async fn dispatch(
    ctx: &ToolCtx,
    name: &str,
    args: Option<JsonObject>,
) -> Result<CallToolResult, ToolError> {
    match name {
        "get_app_status" => app_status(ctx, parse(args)?).await,
        "list_documents" => list_documents(ctx, parse(args)?).await,
        "get_document_info" => document_info(ctx, parse(args)?).await,
        "create_document" => create_document(ctx, parse(args)?).await,
        "select_document" => select_document(ctx, parse(args)?).await,
        "get_palette" => palette(ctx, parse(args)?).await,
        "set_palette_color" => set_palette_color(ctx, parse(args)?).await,
        "draw_pixel" => {
            let a: DrawPixelArgs = parse(args)?;
            let log = format!("draw_pixel {},{} colour {}", a.x, a.y, a.color);
            draw(
                ctx,
                Shape::Dots {
                    points: vec![(a.x, a.y)],
                },
                a.color,
                false,
                log,
            )
            .await
        }
        "draw_line" => {
            let a: DrawLineArgs = parse(args)?;
            let log = format!(
                "draw_line {},{} to {},{} colour {}",
                a.x1, a.y1, a.x2, a.y2, a.color
            );
            let shape = Shape::Line {
                from: (a.x1, a.y1),
                to: (a.x2, a.y2),
            };
            draw(ctx, shape, a.color, false, log).await
        }
        "draw_rectangle" => {
            let a: DrawRectangleArgs = parse(args)?;
            let (w, h) = (extent(a.width, "width")?, extent(a.height, "height")?);
            let log = format!(
                "draw_rectangle {},{} {}x{} colour {}{}",
                a.x,
                a.y,
                a.width,
                a.height,
                a.color,
                if a.filled { " filled" } else { "" }
            );
            let shape = Shape::Rect {
                from: (a.x, a.y),
                to: (a.x.saturating_add(w - 1), a.y.saturating_add(h - 1)),
            };
            draw(ctx, shape, a.color, a.filled, log).await
        }
        "draw_ellipse" => {
            let a: DrawEllipseArgs = parse(args)?;
            let (rx, ry) = (radius(a.rx)?, radius(a.ry)?);
            let log = format!(
                "draw_ellipse {},{} radius {}x{} colour {}{}",
                a.cx,
                a.cy,
                a.rx,
                a.ry,
                a.color,
                if a.filled { " filled" } else { "" }
            );
            let shape = Shape::Ellipse {
                center: (a.cx, a.cy),
                radius: (rx, ry),
            };
            draw(ctx, shape, a.color, a.filled, log).await
        }
        "fill_area" => {
            let a: FillAreaArgs = parse(args)?;
            let log = format!("fill_area {},{} colour {}", a.x, a.y, a.color);
            let cmd = Command::FloodFill {
                x: a.x,
                y: a.y,
                color: a.color,
                fill: Default::default(),
            };
            edit_command(ctx, cmd.into(), log).await
        }
        "import_image" => import_image(ctx, parse(args)?).await,
        "open_picture" => open_picture(ctx, parse(args)?).await,
        "undo" => {
            let NoArgs {} = parse(args)?;
            undo_redo(ctx, true).await
        }
        "redo" => {
            let NoArgs {} = parse(args)?;
            undo_redo(ctx, false).await
        }
        "save_project" => save_project(ctx, parse(args)?).await,
        "export_image" => export_image(ctx, parse(args)?).await,
        "get_preview" => preview(ctx, parse(args)?).await,
        "get_pixels" => pixels(ctx, parse(args)?).await,
        "execute_command" => execute(ctx, parse(args)?).await,
        other => Err(ToolError::internal(format!("no handler for {other}"))),
    }
}

// ---------------------------------------------------------------------------------------
// Descriptions

/// A document as the tools describe it.
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct DocumentInfo {
    pub document_id: &'static str,
    /// `picture` (the shown page) or `spare`.
    pub role: &'static str,
    /// The tools work on the active document.
    pub active: bool,
    pub name: Option<String>,
    pub width: u32,
    pub height: u32,
    pub chipset: ChipsetArg,
    /// Display mode name, e.g. `PAL:Low Res`.
    pub display_mode: String,
    pub screen_width: u32,
    pub screen_height: u32,
    /// The screen is not the mode's standard size.
    pub custom_screen: bool,
    /// Palette entries.
    pub colors: usize,
    /// Most colours the mode shows on this chipset.
    pub max_colors: Option<u16>,
    pub frames: usize,
    /// Counted from 0.
    pub current_frame: usize,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_undo: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_redo: Option<bool>,
    /// Unsaved changes since the last open or save_project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

fn screen_notes(t: Target) -> Vec<String> {
    let (sw, sh) = t.screen;
    let (w, h) = t.mode.size();
    if (sw, sh) == (w, h) {
        return Vec::new();
    }
    vec![format!(
        "custom {sw}x{sh} screen in {} (standard {w}x{h}); exact Amiga hardware compatibility of custom sizes is not guaranteed",
        t.mode.name()
    )]
}

fn describe_doc(doc: &Document, slot: Slot, active: bool) -> DocumentInfo {
    let t = doc.target();
    DocumentInfo {
        document_id: slot.id(),
        role: if active { "picture" } else { "spare" },
        active,
        name: None,
        width: doc.page().width(),
        height: doc.page().height(),
        chipset: ChipsetArg::of(t.chipset),
        display_mode: t.mode.name(),
        screen_width: t.screen.0,
        screen_height: t.screen.1,
        custom_screen: t.screen != t.mode.size(),
        colors: doc.palette().len(),
        max_colors: t.mode.max_colors(t.chipset),
        frames: doc.frame_count(),
        current_frame: doc.current_frame(),
        revision: doc.revision(),
        can_undo: None,
        can_redo: None,
        modified: None,
        notes: screen_notes(t),
    }
}

/// The shown page with its undo state.
fn describe_shown(ws: &dyn Workspace) -> DocumentInfo {
    let engine: &Engine = ws.engine();
    DocumentInfo {
        name: ws.picture_name(),
        can_undo: Some(engine.can_undo()),
        can_redo: Some(engine.can_redo()),
        modified: Some(ws.modified()),
        ..describe_doc(engine.document(), ws.shown(), true)
    }
}

fn describe_slot(ws: &dyn Workspace, slot: Slot) -> Option<DocumentInfo> {
    if slot == ws.shown() {
        Some(describe_shown(ws))
    } else {
        ws.spare().map(|d| describe_doc(&d, slot, false))
    }
}

fn slot_arg(id: &str) -> Result<Slot, ToolError> {
    Slot::parse(id).ok_or_else(|| {
        ToolError::new(
            "DOCUMENT_NOT_FOUND",
            format!("no document {id:?}: the documents are page1 and page2 (see list_documents)"),
        )
    })
}

// ---------------------------------------------------------------------------------------
// Status and documents

#[derive(Serialize)]
struct Permissions {
    read: bool,
    edit: bool,
    overwrite_without_asking: bool,
}

#[derive(Serialize)]
struct FileAccessInfo {
    enabled: bool,
    approved_dirs: Vec<String>,
}

#[derive(Serialize)]
struct AppStatus {
    name: &'static str,
    version: &'static str,
    /// `gui` (the running app) or `headless`.
    engine: &'static str,
    transport: &'static str,
    permissions: Permissions,
    file_access: FileAccessInfo,
    document: Option<DocumentInfo>,
    brush: Option<[u32; 2]>,
}

fn file_access_info(a: &FileAccess) -> FileAccessInfo {
    FileAccessInfo {
        enabled: a.is_on(),
        approved_dirs: a.dirs().iter().map(|d| d.display().to_string()).collect(),
    }
}

async fn app_status(ctx: &ToolCtx, _: NoArgs) -> Result<CallToolResult, ToolError> {
    let s = ctx.state.settings();
    let read = s.read;
    let (engine, document, brush) = ctx
        .exec
        .run(move |ws| {
            (
                ws.kind(),
                read.then(|| describe_shown(ws)),
                ws.brush_size().map(|(w, h)| [w, h]),
            )
        })
        .await?;
    Ok(success_result(&AppStatus {
        name: SERVER_NAME,
        version: env!("CARGO_PKG_VERSION"),
        engine,
        transport: ctx.transport,
        permissions: Permissions {
            read: s.read,
            edit: s.edit,
            overwrite_without_asking: s.overwrite == Overwrite::Allowed,
        },
        file_access: file_access_info(&s.access),
        document,
        brush,
    }))
}

#[derive(Serialize)]
struct DocumentList {
    active: &'static str,
    documents: Vec<serde_json::Value>,
}

async fn list_documents(ctx: &ToolCtx, _: NoArgs) -> Result<CallToolResult, ToolError> {
    let (active, docs) = ctx
        .exec
        .run(|ws| {
            let docs: Vec<_> = [Slot::First, Slot::Second]
                .into_iter()
                .map(|slot| match describe_slot(ws, slot) {
                    Some(d) => serde_json::to_value(d).expect("serialisable"),
                    None => serde_json::json!({
                        "document_id": slot.id(),
                        "role": "spare",
                        "active": false,
                        "blank": true,
                        "notes": ["not used yet: a blank page of the picture's format"],
                    }),
                })
                .collect();
            (ws.shown().id(), docs)
        })
        .await?;
    Ok(success_result(&DocumentList {
        active,
        documents: docs,
    }))
}

async fn document_info(ctx: &ToolCtx, a: DocumentIdArgs) -> Result<CallToolResult, ToolError> {
    let slot = a.document_id.as_deref().map(slot_arg).transpose()?;
    let info = ctx
        .exec
        .run(move |ws| {
            let slot = slot.unwrap_or(ws.shown());
            describe_slot(ws, slot).ok_or_else(|| {
                ToolError::new(
                    "DOCUMENT_NOT_FOUND",
                    format!(
                        "{} is not used yet (it becomes a blank page when selected)",
                        slot.id()
                    ),
                )
            })
        })
        .await??;
    Ok(success_result(&info))
}

async fn select_document(
    ctx: &ToolCtx,
    a: SelectDocumentArgs,
) -> Result<CallToolResult, ToolError> {
    let slot = slot_arg(&a.document_id)?;
    let (changed, info) = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            let changed = ws.shown() != slot;
            if changed {
                ws.swap_spare()?;
                ws.log(&format!("select_document {}", slot.id()));
            }
            Ok((changed, describe_shown(ws)))
        })
        .await??;
    #[derive(Serialize)]
    struct Selected {
        changed: bool,
        document_id: &'static str,
        document: DocumentInfo,
    }
    Ok(success_result(&Selected {
        changed,
        document_id: info.document_id,
        document: info,
    }))
}

// ---------------------------------------------------------------------------------------
// New picture

/// The format `create_document` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Format {
    chipset: Chipset,
    mode: DisplayMode,
    screen: (u32, u32),
    width: u32,
    height: u32,
    colors: u16,
}

fn chipset_name(c: Chipset) -> &'static str {
    match c {
        Chipset::Ocs => "OCS",
        Chipset::Aga => "AGA",
    }
}

/// Modes tried for a size without an exact standard screen, in order.
fn mode_candidates() -> Vec<DisplayMode> {
    let mut out = Vec::new();
    for resolution in [Resolution::Lo, Resolution::Hi, Resolution::SuperHi] {
        for interlace in [false, true] {
            for video in [Video::Pal, Video::Ntsc] {
                out.push(DisplayMode::new(video, resolution, interlace));
            }
        }
    }
    out
}

/// The mode and screen for a page of `w`×`h` without an explicit mode: a mode whose standard
/// screen is exactly that size, else the first mode (Low Res before High Res, not laced
/// before laced, PAL before NTSC) that can have a custom screen of that size, else a mode
/// guessed from the size with its standard screen.
fn infer_mode(w: u32, h: u32) -> (DisplayMode, (u32, u32)) {
    let candidates = mode_candidates();
    if let Some(&m) = candidates.iter().find(|m| m.size() == (w, h)) {
        return (m, (w, h));
    }
    if let Some(&m) = candidates.iter().find(|m| m.screen_fits(w, h)) {
        return (m, (w, h));
    }
    let m = DisplayMode::guess(None, w, h);
    (m, m.size())
}

fn plan_format(a: &CreateDocumentArgs, current: Target) -> Result<Format, ToolError> {
    let chipset = a.chipset.map_or(current.chipset, ChipsetArg::chipset);
    let explicit = a.video.is_some() || a.resolution.is_some() || a.interlace.is_some();
    if a.preset.is_some() && explicit {
        return Err(ToolError::invalid(
            "give either preset or video/resolution/interlace, not both",
        ));
    }
    let (mode, screen, width, height) = if let Some(p) = a.preset {
        let (mode, screen) = p.format();
        (
            mode,
            screen,
            a.width.unwrap_or(screen.0),
            a.height.unwrap_or(screen.1),
        )
    } else if explicit {
        let mode = DisplayMode::new(
            match a.video {
                Some(VideoArg::Ntsc) => Video::Ntsc,
                _ => Video::Pal,
            },
            match a.resolution {
                Some(ResolutionArg::Hi) => Resolution::Hi,
                Some(ResolutionArg::SuperHi) => Resolution::SuperHi,
                _ => Resolution::Lo,
            },
            a.interlace.unwrap_or(false),
        );
        let (w, h) = (
            a.width.unwrap_or(mode.size().0),
            a.height.unwrap_or(mode.size().1),
        );
        let screen = if mode.screen_fits(w, h) {
            (w, h)
        } else {
            mode.size()
        };
        (mode, screen, w, h)
    } else if a.width.is_some() || a.height.is_some() {
        let (w, h) = (
            a.width.unwrap_or(current.screen.0),
            a.height.unwrap_or(current.screen.1),
        );
        let (mode, screen) = infer_mode(w, h);
        (mode, screen, w, h)
    } else {
        (
            current.mode,
            current.screen,
            current.screen.0,
            current.screen.1,
        )
    };
    let Some(max) = mode.max_colors(chipset) else {
        return Err(ToolError::new(
            "UNSUPPORTED_MODE",
            format!(
                "{} cannot show {}: Super-High Res needs AGA",
                chipset_name(chipset),
                mode.name()
            ),
        ));
    };
    let colors = a.colors.unwrap_or(32.min(max));
    if colors > max {
        let hint = match (chipset, mode.resolution) {
            (Chipset::Ocs, Resolution::Hi) => {
                " OCS High Res allows 16 colours; use 16 colours, Low Res (32 colours) or AGA (256 colours)."
            }
            (Chipset::Ocs, _) => " Use 32 colours or fewer, or AGA (256 colours).",
            _ => "",
        };
        return Err(ToolError::new(
            "PALETTE_RANGE_INVALID",
            format!(
                "{} {} shows at most {max} colours at once, not {colors}.{hint}",
                chipset_name(chipset),
                mode.name()
            ),
        ));
    }
    if colors < 2 || !colors.is_power_of_two() {
        return Err(ToolError::new(
            "PALETTE_RANGE_INVALID",
            format!("{colors} colours: the palette size must be a power of two from 2 to {max}"),
        ));
    }
    Ok(Format {
        chipset,
        mode,
        screen,
        width,
        height,
        colors,
    })
}

/// A blank document of `f` with the start palette (DeluxePaint's 32 colours, a grey ramp
/// beyond), validated by the engine's `SetFormat` rules.
fn blank_document(f: &Format) -> Result<Document, ToolError> {
    let seed = Document::from_picture(
        IndexedImage::new(320, 256, 0),
        Palette::dpaint_default_32(),
        Target::new(f.chipset, DisplayMode::PAL_LORES),
    );
    let mut scratch = Engine::new(seed);
    scratch.execute(Command::SetFormat {
        mode: f.mode,
        colors: f.colors,
        width: f.width,
        height: f.height,
        retain: false,
        screen: Some(f.screen),
    })?;
    let d = scratch.document();
    Ok(Document::from_picture(
        d.page().clone(),
        d.palette().clone(),
        d.target(),
    ))
}

fn is_12_bit(c: Rgb8) -> bool {
    Rgb8::from_rgb4(c.to_rgb4()) == c
}

/// A note for OCS palettes that still hold colours OCS cannot show.
fn ocs_palette_note(doc: &Document) -> Option<String> {
    if doc.target().chipset != Chipset::Ocs {
        return None;
    }
    let bad: Vec<String> = doc
        .palette()
        .colors()
        .iter()
        .enumerate()
        .filter(|(_, c)| !is_12_bit(**c))
        .map(|(i, _)| i.to_string())
        .collect();
    (!bad.is_empty()).then(|| {
        format!(
            "palette entries {} are not 12-bit OCS colours; set them with set_palette_color before exporting",
            bad.join(", ")
        )
    })
}

#[derive(Serialize)]
struct Created {
    document_id: &'static str,
    revision: u64,
    undoable: bool,
    document: DocumentInfo,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

fn create_in(ws: &mut dyn Workspace, a: &CreateDocumentArgs) -> Result<Created, ToolError> {
    let current = ws.engine().document();
    let f = plan_format(a, current.target())?;
    let mut notes = Vec::new();
    // TODO(wave8-integration): SetFormat gets a chipset (ADR-46); then a chipset change is an
    // undoable SetFormat as well and the replacement below is only needed for animations.
    let undoable = f.chipset == current.target().chipset && current.frame_count() == 1;
    if undoable {
        ws.execute(
            Command::SetFormat {
                mode: f.mode,
                colors: f.colors,
                width: f.width,
                height: f.height,
                retain: false,
                screen: Some(f.screen),
            }
            .into(),
        )?;
    } else {
        let doc = blank_document(&f)?;
        let old = current.target().chipset;
        ws.replace_picture(
            doc,
            PictureInfo {
                name: a.name.clone(),
                ..Default::default()
            },
        )?;
        notes.push(if old != f.chipset {
            format!(
                "the chipset changed from {} to {}: the picture was replaced (not undoable)",
                chipset_name(old),
                chipset_name(f.chipset)
            )
        } else {
            "the animation was replaced by a one-frame picture (not undoable)".into()
        });
    }
    ws.set_picture_name(a.name.clone());
    notes.extend(ocs_palette_note(ws.engine().document()));
    ws.log(&format!(
        "create_document {}x{} {} {} colours",
        f.width,
        f.height,
        chipset_name(f.chipset),
        f.colors
    ));
    let document = describe_shown(ws);
    Ok(Created {
        document_id: document.document_id,
        revision: document.revision,
        undoable,
        document,
        notes,
    })
}

async fn create_document(
    ctx: &ToolCtx,
    a: CreateDocumentArgs,
) -> Result<CallToolResult, ToolError> {
    if let Some(n) = &a.name
        && (n.len() > 200 || n.chars().any(char::is_control))
    {
        return Err(ToolError::invalid(
            "name: at most 200 characters, no control characters",
        ));
    }
    let created = ctx.exec.run(move |ws| create_in(ws, &a)).await??;
    Ok(success_result(&created))
}

// ---------------------------------------------------------------------------------------
// Palette

#[derive(Serialize)]
struct PaletteInfo {
    document_id: &'static str,
    revision: u64,
    chipset: ChipsetArg,
    size: usize,
    max_colors: Option<u16>,
    /// 4 (OCS, 12-bit colours) or 8 (AGA).
    bits_per_channel: u8,
    /// One `#RRGGBB` string per palette index.
    colors: Vec<String>,
}

fn palette_info(ws: &dyn Workspace) -> PaletteInfo {
    let doc = ws.engine().document();
    let t = doc.target();
    PaletteInfo {
        document_id: ws.shown().id(),
        revision: doc.revision(),
        chipset: ChipsetArg::of(t.chipset),
        size: doc.palette().len(),
        max_colors: t.mode.max_colors(t.chipset),
        bits_per_channel: match t.chipset {
            Chipset::Ocs => 4,
            Chipset::Aga => 8,
        },
        colors: doc
            .palette()
            .colors()
            .iter()
            .map(|c| c.to_string())
            .collect(),
    }
}

async fn palette(ctx: &ToolCtx, _: NoArgs) -> Result<CallToolResult, ToolError> {
    let p = ctx.exec.run(|ws| palette_info(ws)).await?;
    Ok(success_result(&p))
}

/// `#RRGGBB`, `RRGGBB`, `#RGB` or `RGB`.
pub fn parse_color(text: &str) -> Result<Rgb8, ToolError> {
    let t = text.trim();
    let hex = t.strip_prefix('#').unwrap_or(t);
    let bad = || ToolError::invalid(format!("color {text:?}: give #RRGGBB or #RGB"));
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let v = u32::from_str_radix(hex, 16).map_err(|_| bad())?;
    match hex.len() {
        6 => Ok(Rgb8::new((v >> 16) as u8, (v >> 8) as u8, v as u8)),
        3 => Ok(Rgb8::from_rgb4(v as u16)),
        _ => Err(bad()),
    }
}

#[derive(Serialize)]
struct EditResult {
    document_id: &'static str,
    revision: u64,
    /// Page rectangle that changed (or would change); null if none.
    changed_rect: Option<RectArg>,
    palette_changed: bool,
    format_changed: bool,
    can_undo: bool,
    can_redo: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    dry_run: bool,
}

impl EditResult {
    fn new(slot: Slot, o: &Outcome, dry_run: bool) -> Self {
        Self {
            document_id: slot.id(),
            revision: o.revision,
            changed_rect: o.affected.map(RectArg::from),
            palette_changed: o.palette_changed,
            format_changed: o.format_changed,
            can_undo: o.undo_available,
            can_redo: o.redo_available,
            dry_run,
        }
    }
}

async fn set_palette_color(
    ctx: &ToolCtx,
    a: SetPaletteColorArgs,
) -> Result<CallToolResult, ToolError> {
    let color = parse_color(&a.color)?;
    let index = a.index;
    let (r, slot) = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            let doc = ws.engine().document();
            let len = doc.palette().len();
            if usize::from(index) >= len {
                return Err(ToolError::new(
                    "PALETTE_RANGE_INVALID",
                    format!("index {index}: the palette has {len} colours (0-{})", len - 1),
                ));
            }
            if doc.target().chipset == Chipset::Ocs && !is_12_bit(color) {
                let near = Rgb8::from_rgb4(color.to_rgb4());
                return Err(ToolError::new(
                    "HARDWARE_LIMIT",
                    format!(
                        "{color} is not a 12-bit OCS colour (OCS has 4 bits per channel); the nearest 12-bit colour is {near}"
                    ),
                ));
            }
            let mut palette = doc.palette().clone();
            palette.set(index, color);
            let o = ws.execute(
                Command::SetPalette {
                    colors: palette.colors().to_vec(),
                }
                .into(),
            )?;
            ws.log(&format!("set_palette_color {index} {color}"));
            Ok((o, ws.shown()))
        })
        .await??;
    #[derive(Serialize)]
    struct Set {
        #[serde(flatten)]
        edit: EditResult,
        index: u8,
        color: String,
    }
    Ok(success_result(&Set {
        edit: EditResult::new(slot, &r, false),
        index,
        color: color.to_string(),
    }))
}

// ---------------------------------------------------------------------------------------
// Drawing

fn extent(v: u32, what: &str) -> Result<i32, ToolError> {
    if v == 0 || v > dpx_core::shapes::MAX_SHAPE_COORD as u32 {
        return Err(ToolError::invalid(format!(
            "{what} must be 1 to {}",
            dpx_core::shapes::MAX_SHAPE_COORD
        )));
    }
    Ok(v as i32)
}

fn radius(v: u32) -> Result<i32, ToolError> {
    if v > dpx_core::shapes::MAX_SHAPE_COORD as u32 {
        return Err(ToolError::invalid(format!(
            "radius must be 0 to {}",
            dpx_core::shapes::MAX_SHAPE_COORD
        )));
    }
    Ok(v as i32)
}

fn shape_command(shape: Shape, color: u8, filled: bool) -> Command {
    Command::Shape {
        shape,
        brush: vec![(0, 0)],
        color,
        filled,
        symmetry: None,
        custom: None,
        mode: Default::default(),
        range: None,
        stencil: None,
        right: false,
        fx: Default::default(),
    }
}

async fn draw(
    ctx: &ToolCtx,
    shape: Shape,
    color: u8,
    filled: bool,
    log: String,
) -> Result<CallToolResult, ToolError> {
    edit_command(ctx, shape_command(shape, color, filled).into(), log).await
}

async fn edit_command(
    ctx: &ToolCtx,
    request: Request,
    log: String,
) -> Result<CallToolResult, ToolError> {
    let dry_run = request.dry_run;
    let (o, slot) = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            let o = ws.execute(request)?;
            if !dry_run {
                ws.log(&log);
            }
            Ok((o, ws.shown()))
        })
        .await??;
    Ok(success_result(&EditResult::new(slot, &o, dry_run)))
}

async fn execute(ctx: &ToolCtx, a: ExecuteArgs) -> Result<CallToolResult, ToolError> {
    let command: Command = serde_json::from_value(a.command.clone())
        .map_err(|e| ToolError::invalid(format!("not a valid command: {e}")))?;
    let op = a
        .command
        .get("op")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();
    let request = Request {
        command,
        expected_revision: a.expected_revision,
        dry_run: a.dry_run,
    };
    edit_command(ctx, request, format!("execute_command {op}")).await
}

async fn undo_redo(ctx: &ToolCtx, undo: bool) -> Result<CallToolResult, ToolError> {
    let (o, slot) = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            let o = if undo { ws.undo()? } else { ws.redo()? };
            ws.log(if undo { "undo" } else { "redo" });
            Ok((o, ws.shown()))
        })
        .await??;
    Ok(success_result(&EditResult::new(slot, &o, false)))
}

// ---------------------------------------------------------------------------------------
// Looking

async fn pixels(ctx: &ToolCtx, a: PixelsArgs) -> Result<CallToolResult, ToolError> {
    let (r, slot) = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            Ok((
                view::get_pixels(&visible_picture(ws), a.rect, a.encoding)?,
                ws.shown(),
            ))
        })
        .await??;
    #[derive(Serialize)]
    struct Pixels {
        document_id: &'static str,
        #[serde(flatten)]
        pixels: view::PixelsResult,
    }
    Ok(success_result(&Pixels {
        document_id: slot.id(),
        pixels: r,
    }))
}

#[derive(Serialize)]
struct PreviewInfo {
    document_id: &'static str,
    revision: u64,
    rect: RectArg,
    scale: u32,
    image_width: u32,
    image_height: u32,
    page_width: u32,
    page_height: u32,
    mime_type: &'static str,
}

/// The preview PNG (base64) and what it shows.
async fn render_preview(ctx: &ToolCtx, a: PreviewArgs) -> Result<(String, PreviewInfo), ToolError> {
    let (crop, palette, rect, scale, revision, slot, page) = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            let doc = visible_picture(ws);
            let r = view::clip(doc.page().bounds(), a.rect)?;
            let scale = view::preview_scale(r.w as u32, r.h as u32, a.scale)?;
            Ok((
                doc.page().crop(r),
                doc.palette().clone(),
                r,
                scale,
                doc.revision(),
                ws.shown(),
                (doc.page().width(), doc.page().height()),
            ))
        })
        .await??;
    let rendered = blocking(move || view::render_png(&crop, &palette, scale)).await??;
    Ok((
        view::to_base64(&rendered.png),
        PreviewInfo {
            document_id: slot.id(),
            revision,
            rect: rect.into(),
            scale: rendered.scale,
            image_width: rendered.width,
            image_height: rendered.height,
            page_width: page.0,
            page_height: page.1,
            mime_type: "image/png",
        },
    ))
}

async fn preview(ctx: &ToolCtx, a: PreviewArgs) -> Result<CallToolResult, ToolError> {
    let (png, info) = render_preview(ctx, a).await?;
    Ok(image_result(png, &info))
}

// ---------------------------------------------------------------------------------------
// Files

/// Runs blocking work (file I/O, decoding, encoding, colour reduction) off the runtime's
/// worker threads.
async fn blocking<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> Result<R, ToolError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ToolError::internal(format!("background work failed: {e}")))
}

fn io(path: &str, e: std::io::Error) -> ToolError {
    ToolError::new("IO_ERROR", format!("{path}: {e}"))
}

/// Reads a file from an approved directory, at most `limit` bytes.
async fn read_file(
    access: &FileAccess,
    path: &str,
    limit: u64,
) -> Result<(PathBuf, Vec<u8>), ToolError> {
    let resolved = access.resolve_read(path)?;
    let (shown, r) = (path.to_string(), resolved.clone());
    let bytes = blocking(move || -> Result<Vec<u8>, ToolError> {
        let len = std::fs::metadata(&r).map_err(|e| io(&shown, e))?.len();
        if len > limit {
            return Err(ToolError::invalid(format!(
                "{shown}: {len} bytes exceed the {limit} byte limit"
            )));
        }
        std::fs::read(&r).map_err(|e| io(&shown, e))
    })
    .await??;
    Ok((resolved, bytes))
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// Who asks, for the overwrite question: "Claude" for Claude's clients (`claude-code`,
/// `claude-ai`, ...), else the client's name, else "An AI client".
pub fn asker(client: Option<&str>) -> String {
    match client {
        Some(c) if c.to_ascii_lowercase().contains("claude") => "Claude".into(),
        Some(c) if !c.trim().is_empty() => c.trim().chars().take(24).collect(),
        _ => "An AI client".into(),
    }
}

/// The question before an existing file is replaced.
pub fn overwrite_question(client: Option<&str>, file: &str) -> Question {
    Question {
        title: "Replace File?".into(),
        text: format!("{} wants to replace {file}. Replace?", asker(client)),
        yes: "Replace".into(),
        no: "Keep".into(),
    }
}

/// With `overwrite` and an existing file: allowed, or asked (a "no", no answer in time or a
/// question that could not be asked fails the call; the file is kept).
async fn confirm_overwrite(ctx: &ToolCtx, target: &Path, overwrite: bool) -> Result<(), ToolError> {
    if !overwrite || !target.exists() {
        return Ok(());
    }
    match ctx.state.settings().overwrite {
        Overwrite::Allowed => Ok(()),
        Overwrite::Ask => {
            let name = file_name(target);
            let client = ctx.state.activity().snapshot().client;
            let declined = |why: String| Err(ToolError::new("OVERWRITE_DECLINED", why));
            match ctx
                .exec
                .ask(overwrite_question(client.as_deref(), &name))
                .await
            {
                Ok(true) => Ok(()),
                Ok(false) => declined(format!("the user kept the existing file {name}")),
                Err(e) if e.code == crate::jobs::NO_ANSWER => {
                    declined(format!("{}; the existing file {name} was kept", e.message))
                }
                Err(e) => Err(e),
            }
        }
    }
}

#[derive(Serialize)]
struct Written {
    document_id: &'static str,
    revision: u64,
    /// The canonical path written.
    path: String,
    format: &'static str,
    bytes: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

fn has_extension(path: &str, ext: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

async fn save_project(ctx: &ToolCtx, a: SaveArgs) -> Result<CallToolResult, ToolError> {
    if !has_extension(&a.path, "dpxproj") {
        return Err(ToolError::new(
            "UNSUPPORTED_FILE_TYPE",
            "save_project writes .dpxproj projects; use export_image for IFF ILBM, IFF ANIM or PNG",
        ));
    }
    let access = ctx.state.settings().access;
    let target = access.resolve_write(&a.path, a.overwrite)?;
    confirm_overwrite(ctx, &target, a.overwrite).await?;
    let (doc, extras, spare, slot) = ctx
        .exec
        .run(|ws| {
            (
                ws.engine().document().clone(),
                ws.project_extras(),
                ws.spare().map(|d| d.page().clone()),
                ws.shown(),
            )
        })
        .await?;
    let revision = doc.revision();
    let (t, overwrite) = (target.clone(), a.overwrite);
    let codec = ctx.state.codec();
    let bytes = blocking(move || -> Result<usize, ToolError> {
        let bytes = codec.encode_project(&doc, &extras, spare.as_ref())?;
        FileAccess::write_file(&t, &bytes, overwrite)?;
        Ok(bytes.len())
    })
    .await??;
    let t = target.clone();
    ctx.exec
        .run(move |ws| {
            if ws.shown() == slot {
                ws.mark_saved(&t, revision);
            }
            ws.log(&format!("save_project {}", file_name(&t)));
        })
        .await?;
    Ok(success_result(&Written {
        document_id: slot.id(),
        revision,
        path: target.display().to_string(),
        format: "dpxproj",
        bytes,
        notes: Vec::new(),
    }))
}

fn format_name(f: pictures::Format) -> &'static str {
    match f {
        pictures::Format::Ilbm => "iff_ilbm",
        pictures::Format::Png => "png",
        pictures::Format::Anim => "iff_anim",
    }
}

async fn export_image(ctx: &ToolCtx, a: ExportArgs) -> Result<CallToolResult, ToolError> {
    let by_ext = pictures::format_for_save(&a.path);
    let format = match a.format {
        None => by_ext?,
        Some(f) => {
            let f = match f {
                ExportFormatArg::IffIlbm => pictures::Format::Ilbm,
                ExportFormatArg::IffAnim => pictures::Format::Anim,
                ExportFormatArg::Png => pictures::Format::Png,
            };
            match by_ext {
                Ok(e) if e != f => {
                    return Err(ToolError::invalid(format!(
                        "format {} does not match the extension of {}",
                        format_name(f),
                        a.path
                    )));
                }
                Err(e)
                    if has_extension(&a.path, "dpxproj")
                        || has_extension(&a.path, "jpg")
                        || has_extension(&a.path, "jpeg") =>
                {
                    return Err(e);
                }
                _ => f,
            }
        }
    };
    let access = ctx.state.settings().access;
    let target = access.resolve_write(&a.path, a.overwrite)?;
    confirm_overwrite(ctx, &target, a.overwrite).await?;
    let (doc, extras, slot) = ctx
        .exec
        .run(|ws| {
            (
                visible_picture(ws).into_owned(),
                ws.project_extras(),
                ws.shown(),
            )
        })
        .await?;
    let revision = doc.revision();
    let (t, overwrite) = (target.clone(), a.overwrite);
    let codec = ctx.state.codec();
    let (bytes, notes) = blocking(move || -> Result<_, ToolError> {
        // Hardware limits first, whichever encoder writes the file (nothing reduced silently).
        pictures::validate_hardware(&doc)?;
        let (bytes, mut notes) = codec.encode_export(&doc, &extras, format)?;
        notes.extend(screen_notes(doc.target()));
        FileAccess::write_file(&t, &bytes, overwrite)?;
        Ok((bytes.len(), notes))
    })
    .await??;
    let t = target.clone();
    ctx.exec
        .run(move |ws| ws.log(&format!("export_image {}", file_name(&t))))
        .await?;
    Ok(success_result(&Written {
        document_id: slot.id(),
        revision,
        path: target.display().to_string(),
        format: format_name(format),
        bytes,
        notes,
    }))
}

async fn open_picture(ctx: &ToolCtx, a: OpenArgs) -> Result<CallToolResult, ToolError> {
    let access = ctx.state.settings().access;
    let (resolved, bytes) = read_file(&access, &a.path, pictures::MAX_FILE_BYTES).await?;
    let name = a.path.clone();
    let (doc, extras, spare, notes) = blocking(move || -> Result<_, ToolError> {
        if dpx_formats::project::is_zip(&bytes) {
            let (doc, extras, spare) = pictures::decode_project(&name, &bytes)?;
            Ok((doc, extras, spare, Vec::new()))
        } else {
            let (doc, notes) = pictures::decode_picture(&name, &bytes)?;
            Ok((doc, Default::default(), None, notes))
        }
    })
    .await??;
    let info = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            let name = file_name(&resolved);
            ws.replace_picture(
                doc,
                PictureInfo {
                    path: Some(resolved),
                    name: None,
                    extras,
                    saved: true,
                },
            )?;
            if let Some(page) = spare {
                ws.set_spare_page(page)?;
            }
            ws.log(&format!("open_picture {name}"));
            Ok(describe_shown(ws))
        })
        .await??;
    #[derive(Serialize)]
    struct Opened {
        document_id: &'static str,
        revision: u64,
        undoable: bool,
        document: DocumentInfo,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        notes: Vec<String>,
    }
    Ok(success_result(&Opened {
        document_id: info.document_id,
        revision: info.revision,
        undoable: false,
        document: info,
        notes,
    }))
}

// ---------------------------------------------------------------------------------------
// Import

/// Largest decoded `data` of `import_image`.
pub const MAX_IMPORT_DATA: usize = 12 * 1024 * 1024;
/// Colour 0 of a new palette when the picture has transparent pixels (ADR-9).
pub const TRANSPARENT_PINK: Rgb8 = Rgb8::new(255, 0, 255);

/// What the import needs from the document, taken in one job.
#[derive(Clone, Debug)]
struct ImportContext {
    revision: u64,
    slot: Slot,
    target: Target,
    page: (u32, u32),
    palette: Palette,
    bg: u8,
}

/// The page a target imports into and the pixel shape to fit with.
fn import_area(
    kind: ImportTargetArg,
    ctx: &ImportContext,
    image: (u32, u32),
) -> ((u32, u32), (u32, u32)) {
    match kind {
        // A brush keeps the picture's own pixels; a bigger one is reduced to the page.
        ImportTargetArg::Brush => {
            let (w, h) = image;
            let size = if w <= ctx.page.0 && h <= ctx.page.1 {
                (w, h)
            } else {
                import::fitted_size(w, h, ctx.page, (1, 1))
            };
            (size, (1, 1))
        }
        _ => (ctx.page, ctx.target.mode.pixel_shape()),
    }
}

async fn import_image(ctx: &ToolCtx, a: ImportImageArgs) -> Result<CallToolResult, ToolError> {
    let (name, bytes) = match (&a.path, &a.data) {
        (Some(p), None) => {
            let access = ctx.state.settings().access;
            let (resolved, bytes) = read_file(&access, p, pictures::MAX_FILE_BYTES).await?;
            (file_name(&resolved), bytes)
        }
        (None, Some(d)) => {
            if d.len() > MAX_IMPORT_DATA / 3 * 4 + 1024 {
                return Err(ToolError::invalid(format!(
                    "data: more than {MAX_IMPORT_DATA} bytes once decoded"
                )));
            }
            let bytes = view::from_base64(d)?;
            if bytes.len() > MAX_IMPORT_DATA {
                return Err(ToolError::invalid(format!(
                    "data: {} bytes exceed the {MAX_IMPORT_DATA} byte limit",
                    bytes.len()
                )));
            }
            ("image".to_string(), bytes)
        }
        _ => {
            return Err(ToolError::invalid(
                "give either path (a file in an approved directory) or data (base64), not both",
            ));
        }
    };
    let palette_choice = match (a.target, a.palette) {
        (ImportTargetArg::Picture, p) => p.unwrap_or(PaletteArg::New),
        (_, None | Some(PaletteArg::Current)) => PaletteArg::Current,
        (_, Some(PaletteArg::New)) => {
            return Err(ToolError::invalid(
                "the spare page and the brush use the picture's palette: give palette=current",
            ));
        }
    };
    let n = name.clone();
    let (rgba, _colors, mut notes) = blocking(move || pictures::decode_rgba(&n, &bytes)).await??;
    let ic = ctx
        .exec
        .run(|ws| {
            let doc = ws.engine().document();
            ImportContext {
                revision: doc.revision(),
                slot: ws.shown(),
                target: doc.target(),
                page: (doc.page().width(), doc.page().height()),
                palette: doc.palette().clone(),
                bg: ws.bg_color(),
            }
        })
        .await?;
    let max = ic.target.mode.max_colors(ic.target.chipset).unwrap_or(32);
    let colors = match a.colors {
        Some(c) if c > max || c < 2 || !c.is_power_of_two() => {
            return Err(ToolError::new(
                "PALETTE_RANGE_INVALID",
                format!(
                    "{c} colours: {} {} allows a power of two from 2 to {max}",
                    chipset_name(ic.target.chipset),
                    ic.target.mode.name()
                ),
            ));
        }
        Some(c) => c,
        None => import::color_count(
            (ic.palette.len().clamp(2, 256) as u16).next_power_of_two(),
            max,
        ),
    };
    let opts = ImportOptions {
        position: match a.position {
            PositionArg::Centre => Position::Centre,
            PositionArg::TopLeft => Position::TopLeft,
        },
        palette: match palette_choice {
            PaletteArg::New => PaletteChoice::New,
            PaletteArg::Current => PaletteChoice::Current,
        },
        colors,
        dither: match a.dither {
            DitherArg::None => Dither::None,
            DitherArg::FloydSteinberg => Dither::FloydSteinberg,
            DitherArg::Ordered => Dither::Ordered,
        },
    };
    let (page, pixel_shape) = import_area(a.target, &ic, (rgba.width(), rgba.height()));
    let ic2 = ic.clone();
    let imported = blocking(move || {
        let t = ImportTarget {
            page,
            pixel_shape,
            depth: match ic2.target.chipset {
                Chipset::Ocs => PaletteDepth::Rgb4,
                Chipset::Aga => PaletteDepth::Rgb8,
            },
            palette: &ic2.palette,
            bg: ic2.bg,
            transparent: TRANSPARENT_PINK,
        };
        import::import(&rgba, &opts, &t)
    })
    .await?;
    if imported.reduction == Reduction::Rounded {
        notes.push("colours rounded to 12-bit OCS values".into());
    }
    if imported.transparent_color.is_some() {
        notes.push("transparent pixels became colour 0".into());
    }
    let kind = a.target;
    let result_palette = imported.palette.len();
    let (at, size) = (imported.at, imported.size);
    let (pw, ph) = (imported.page.width(), imported.page.height());
    let log_name = name.clone();
    let (slot, revision) = ctx
        .exec
        .run(move |ws| -> Result<_, ToolError> {
            if ws.engine().document().revision() != ic.revision || ws.shown() != ic.slot {
                return Err(ToolError::new(
                    "REVISION_CONFLICT",
                    "the picture changed while the image was converted; call import_image again",
                ));
            }
            match kind {
                ImportTargetArg::Picture => {
                    let doc = Document::from_picture(imported.page, imported.palette, ic.target);
                    ws.replace_picture(
                        doc,
                        PictureInfo {
                            name: Some(log_name.clone()),
                            extras: dpx_formats::project::ProjectExtras {
                                transparent_color: imported.transparent_color.map(u16::from),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    )?;
                }
                ImportTargetArg::Spare => ws.set_spare_page(imported.page)?,
                ImportTargetArg::Brush => ws.set_brush(Brush {
                    image: imported.page,
                    transparent: imported.transparent_color,
                })?,
            }
            ws.log(&format!("import_image {log_name} to {kind:?}").to_lowercase());
            Ok((ws.shown(), ws.engine().document().revision()))
        })
        .await??;
    #[derive(Serialize)]
    struct Imported {
        document_id: &'static str,
        revision: u64,
        target: ImportTargetArg,
        width: u32,
        height: u32,
        /// Where the fitted picture lies and its size there.
        at: [u32; 2],
        size: [u32; 2],
        palette: PaletteArg,
        colors: usize,
        undoable: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        notes: Vec<String>,
    }
    Ok(success_result(&Imported {
        document_id: slot.id(),
        revision,
        target: kind,
        width: pw,
        height: ph,
        at: [at.0, at.1],
        size: [size.0, size.1],
        palette: palette_choice,
        colors: result_palette,
        undoable: kind != ImportTargetArg::Picture,
        notes,
    }))
}

// ---------------------------------------------------------------------------------------
// Resources

pub const RESOURCE_INFO: &str = "dpx://document/info";
pub const RESOURCE_PALETTE: &str = "dpx://document/palette";
pub const RESOURCE_PREVIEW: &str = "dpx://document/preview.png";

/// The resources (also listed by the bridge).
pub fn resources() -> Vec<Resource> {
    vec![
        Resource::new(RESOURCE_INFO, "document-info")
            .with_title("Active document")
            .with_description("Size, mode, chipset, colours and revision of the active document")
            .with_mime_type("application/json"),
        Resource::new(RESOURCE_PALETTE, "palette")
            .with_title("Palette")
            .with_description("The active document's palette as #RRGGBB strings")
            .with_mime_type("application/json"),
        Resource::new(RESOURCE_PREVIEW, "preview")
            .with_title("Preview")
            .with_description("The picture as a PNG (scaled like get_preview's default)")
            .with_mime_type("image/png"),
    ]
}

/// Reads resource `uri`.
pub async fn read_resource(ctx: &ToolCtx, uri: &str) -> Result<ReadResourceResult, ToolError> {
    permission(Needs::Read, ctx)?;
    ctx.state.activity().tool_call();
    let contents = match uri {
        RESOURCE_INFO => {
            let info = ctx.exec.run(|ws| describe_shown(ws)).await?;
            ResourceContents::text(serde_json::to_string(&info).expect("serialisable"), uri)
                .with_mime_type("application/json")
        }
        RESOURCE_PALETTE => {
            let p = ctx.exec.run(|ws| palette_info(ws)).await?;
            ResourceContents::text(serde_json::to_string(&p).expect("serialisable"), uri)
                .with_mime_type("application/json")
        }
        RESOURCE_PREVIEW => {
            let (png, _) = render_preview(ctx, PreviewArgs::default()).await?;
            ResourceContents::blob(png, uri).with_mime_type("image/png")
        }
        other => {
            return Err(ToolError::new(
                "RESOURCE_NOT_FOUND",
                format!("no resource {other}"),
            ));
        }
    };
    Ok(ReadResourceResult::new(vec![contents]))
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
