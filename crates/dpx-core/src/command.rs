//! Document commands: the single mutation API shared by the GUI, the CLI and MCP (ADR-2).
//!
//! Commands are plain serialisable values. [`crate::Engine`] validates them, applies them and
//! records undo information.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::anim_brush::{self, AnimBrush};
use crate::document::{Document, check_frame_count};
use crate::gradient::FillType;
use crate::image::{IndexedImage, Rect};
use crate::mode::{Chipset, DisplayMode, Target};
use crate::paint::{BrushImage, DrawMode, PaintFx};
use crate::palette::{Palette, Rgb8};
use crate::shapes::{Shape, Symmetry};

/// Most points one stroke may carry.
pub const MAX_STROKE_POINTS: usize = 100_000;
/// Most pixels a brush may have (DeluxePaint brushes go up to 100×100 when resized).
pub const MAX_BRUSH_PIXELS: usize = 100 * 100;
/// Largest absolute brush offset.
pub const MAX_BRUSH_OFFSET: i32 = 1_000;
/// Largest absolute stroke coordinate; points may lie outside the page but not arbitrarily far.
pub const MAX_COORD: i32 = 1_000_000;
/// Largest page width or height.
pub const MAX_PAGE_SIDE: u32 = 4096;
/// Largest custom brush width or height. One limit for every brush: strokes and shapes
/// accept it, and the brush operations ([`crate::brush_ops`]) clamp or refuse at it, so a
/// transformed brush can always be painted. It is the largest page, so every pick-up fits.
pub const MAX_BRUSH_SIDE: u32 = MAX_PAGE_SIDE;
/// Most work one Stroke or Shape may cost (validation refuses more), in work units: per
/// pixel a brush stamp may plot on the page and per pixel of a filled shape's bounding box on
/// the page [`crate::paint::pixel_work`] units (1 for Matte, about 9 for Blend on 32
/// colours), plus one unit per path point and per mask pixel, and [`POLYGON_CROSSING_WORK`]
/// per row crossing of a filled shape's edge. Stamps and rows off the page cost nothing (they
/// are clipped away). The whole budget takes about 2 to 5 seconds in a release build on the
/// development machine (`timing_of_commands_at_the_work_budget`); realistic gestures stay
/// far below it (a 100×100 brush dragged across a 1008×1024 page 50 times is about 2^29).
pub const MAX_PAINT_WORK: u64 = 1 << 30;
/// Work units of one edge crossing of a filled shape's row (computed, then sorted).
pub const POLYGON_CROSSING_WORK: u64 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// Mirror left <-> right.
    Horizontal,
    /// Mirror top <-> bottom.
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Freehand stroke (Dotted / Continuous Freehand): the brush is stamped at every point and,
    /// if `continuous`, along Bresenham lines between consecutive points.
    ///
    /// Strokes and shapes paint every pixel through [`crate::paint::plot`] (ADR-18): `custom`,
    /// `mode`, `range`, `stencil` and `right` are optional in JSON and left out when unset.
    Stroke {
        /// Built-in brush pixels as offsets from its hotspot (may be empty with `custom`).
        #[serde(default)]
        brush: Vec<(i32, i32)>,
        /// FG, or BG with the right button.
        color: u8,
        points: Vec<(i32, i32)>,
        continuous: bool,
        /// Custom brush (Brush Selector); replaces `brush`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom: Option<BrushImage>,
        /// Drawing mode (Mode menu).
        #[serde(default, skip_serializing_if = "is_default")]
        mode: DrawMode,
        /// Colour range `low..=high` for Shade and Cycle.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<(u8, u8)>,
        /// Stencil: 256 entries, `true` protects that colour on the page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stencil: Option<Vec<bool>>,
        /// Painted with the right mouse button.
        #[serde(default, skip_serializing_if = "is_default")]
        right: bool,
        /// Effect menu: Translucency, Process, AntiAlias (ADR-31).
        #[serde(default, skip_serializing_if = "is_default")]
        fx: PaintFx,
    },
    /// A drawing-tool shape (ADR-14): the brush stamped along the outline, or with `filled` a
    /// solid shape in `color` (the brush is not used). With `symmetry`, rotated copies too.
    /// The paint fields are those of [`Command::Stroke`].
    Shape {
        shape: Shape,
        #[serde(default)]
        brush: Vec<(i32, i32)>,
        color: u8,
        #[serde(default)]
        filled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        symmetry: Option<Symmetry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom: Option<BrushImage>,
        #[serde(default, skip_serializing_if = "is_default")]
        mode: DrawMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<(u8, u8)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stencil: Option<Vec<bool>>,
        #[serde(default, skip_serializing_if = "is_default")]
        right: bool,
        /// Effect menu: Translucency, Process, AntiAlias (ADR-31).
        #[serde(default, skip_serializing_if = "is_default")]
        fx: PaintFx,
    },
    /// Fill tool: the 4-connected region of the colour at (`x`, `y`) becomes `color`, or with
    /// a gradient `fill` (ADR-20) steps through its colour range.
    FloodFill {
        x: i32,
        y: i32,
        color: u8,
        #[serde(default, skip_serializing_if = "FillType::is_solid")]
        fill: FillType,
    },
    /// Fills the whole page with one colour (CLR).
    Clear { color: u8 },
    /// Mirrors the page (Picture ▸ Flip).
    Flip { axis: Axis },
    /// Recolours every pixel of `from` to `to` (Color ▸ BG -> FG).
    ReplaceColor { from: u8, to: u8 },
    /// Exchanges two colours in every pixel (Color ▸ BG <-> FG).
    SwapColors { a: u8, b: u8 },
    /// Replaces all palette entries; the number of colours must stay the same. On OCS every
    /// entry it changes must be a 12-bit colour (ADR-46); an entry it keeps may stay as it is
    /// (a project saved before ADR-46 can hold colours that are not 12-bit).
    SetPalette { colors: Vec<Rgb8> },
    /// Picture ▸ Screen Format: display mode, palette size and page size. With `retain` the
    /// picture is kept (top-left aligned, cut off if the page shrinks), otherwise the page is
    /// cleared to colour 0. New palette entries get a grey ramp; dropped entries must be
    /// unused when `retain` is set. Mode, colours and screen are checked against the
    /// resulting chipset. On OCS every palette entry becomes a 12-bit colour: the grey ramp,
    /// and kept entries of an AGA palette when the chipset changes to OCS, are rounded to the
    /// nearest 12-bit value (ADR-46), and the outcome counts the kept entries rounded
    /// (`Outcome::rounded_colors`). Undo restores the whole previous target.
    SetFormat {
        mode: DisplayMode,
        colors: u16,
        width: u32,
        height: u32,
        retain: bool,
        /// Screen size; `None` = the mode's standard size (ADR-11).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screen: Option<(u32, u32)>,
        /// The chipset (ADR-46); `None` keeps the current one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chipset: Option<Chipset>,
    },
    /// Picture ▸ Page Size: the picture stays top-left; new area gets `fill` (the BG colour),
    /// a smaller page crops (ADR-10).
    SetPageSize { width: u32, height: u32, fill: u8 },
    /// Replaces the whole page with `pixels` (`width`×`height`, row by row), e.g. Picture ▸
    /// Spare ▸ Delete This Page or CLR on a fixed background (ADR-15). Another size must fit the
    /// target's page limits.
    SetPage {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    /// Picture ▸ Spare ▸ Merge in Front / Back: combines the page with another page
    /// (`width`×`height`, top-left aligned, clipped to the page). `front`: the other page lies
    /// on top and its `transparent` pixels let the page show through; otherwise the page lies
    /// on top and its own `transparent` pixels show the other page (ADR-15). With layers
    /// (ADR-44) the active layer is painted; in back the other page goes only where the frame
    /// as it shows (all visible layers) and the active layer are both `transparent`, so it
    /// lies behind the whole picture.
    MergePage {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        front: bool,
        transparent: u8,
    },
    // ---- Animation frames (ADR-28). Frame numbers start at 0 (the interface shows them
    // from 1). Every page command above acts on the current frame; SetFormat and
    // SetPageSize change all frames. Frame commands are one undo step each; GotoFrame and
    // SetFrameRate are settings that change no pixels and are no undo step.
    /// Anim ▸ Frames ▸ Set #..: more frames are copies of the current frame inserted after
    /// it; fewer frames delete from the current frame on (DeluxePaint, AGA p.236).
    SetFrameCount { count: u32 },
    /// Anim ▸ Frames ▸ Add Frames..: `count` copies of the current frame after it; the last
    /// copy becomes the current frame (AGA p.237).
    AddFrames { count: u32 },
    /// Anim ▸ Frames ▸ Copy Frames.. "Current Frame to": frames `first..=last` become copies
    /// of the current frame (all frames: Copy to All).
    CopyFrame { first: u32, last: u32 },
    /// Copy Frames.. "Range … Insert Before Frame": copies of frames `first..=last` go in
    /// before frame `before` (the frame count = at the end); the current picture stays current.
    InsertFrameCopies { first: u32, last: u32, before: u32 },
    /// Anim ▸ Frames ▸ Delete Frames..: removes frames `first..=last`; the frame after them
    /// becomes current if the current one goes (the one before at the end, AGA p.237).
    /// Deleting every frame leaves one frame in the `fill` (BG) colour.
    DeleteFrames {
        first: u32,
        last: u32,
        #[serde(default)]
        fill: u8,
    },
    /// Anim ▸ Control ▸ Previous / Next / Goto..: makes `frame` the current frame.
    GotoFrame { frame: u32 },
    /// Anim ▸ Control ▸ Set Rate..: playback rate, 1 to 60 frames per second.
    SetFrameRate { fps: u8 },
    /// Color ▸ Palette ▸ Arrange (ADR-31): reorders the palette, `order[new] = old` (a
    /// permutation of all entries), and renumbers the pixels so the picture looks the same.
    ArrangePalette { order: Vec<u8> },
    /// Color ▸ Remap (ADR-31): every picture of every frame and layer repainted with the
    /// colours of `from` (1 to 256 entries: the palette its pixels were drawn for) looked up
    /// in the current palette, nearest colour; colours beyond `from` stay. Above the bottom
    /// layer colour 0 stays transparent and no other colour becomes 0 (ADR-44).
    RemapColors { from: Vec<Rgb8> },
    /// Painting with an AnimBrush (ADR-36): `paint`, a Stroke or Shape, stamps the cels of
    /// `brush` in turn instead of its own custom brush. Stamp `k` along the path (and of
    /// every symmetric copy) paints the cel the brush reaches `k` stamps after its clock;
    /// [`Command::stamps`] tells how far the clock moves.
    AnimBrushPaint {
        brush: AnimBrush,
        paint: Box<Command>,
    },
    // ---- ADR-38: Anim ▸ Move.. Draw and Trails.
    /// The brush (`custom`, else the built-in `brush` in `color`) moved and rotated along
    /// `path` from `start` (page x, y and depth z), one stamp per frame, frames from the
    /// current one ([`crate::anim_move::plan`]); the frame where the move ends becomes
    /// current. One undo step for every frame it paints. The paint fields are those of
    /// [`Command::Stroke`].
    MoveBrush {
        start: (i32, i32, i32),
        path: crate::anim_move::MovePath,
        #[serde(default)]
        brush: Vec<(i32, i32)>,
        color: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom: Option<BrushImage>,
        #[serde(default, skip_serializing_if = "is_default")]
        mode: DrawMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<(u8, u8)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stencil: Option<Vec<bool>>,
        #[serde(default, skip_serializing_if = "is_default")]
        right: bool,
        #[serde(default, skip_serializing_if = "is_default")]
        fx: PaintFx,
        /// The area stencil and the frame it belongs to (ADR-39): only the stamps on that
        /// frame spare its pixels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area: Option<crate::anim_move::FrameArea>,
        /// The brush's orientation at `start` and the observer (Effect ▸ Perspective,
        /// ADR-37); `None`: unturned, seen from [`crate::anim_move::EYE`] pixels in front of
        /// the page centre.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view: Option<Box<crate::anim_move::MoveView>>,
    },
    /// An area stencil over a painting command (ADR-39; Effect ▸ Stencil ▸ Paint, Background ▸
    /// Lock FG): `command` (Stroke, Shape, AnimBrushPaint, FloodFill, Clear, or a SetPage of
    /// the page's size, such as CLR on a fixed background) paints as usual, but the pixels
    /// `area` protects keep their colour. `area` has the page's size.
    Stenciled {
        area: crate::stencil::AreaStencil,
        command: Box<Command>,
    },
    // ---- Layers (ADR-44). Layer numbers start at 0, the bottom layer; every frame holds one
    // picture per layer. On every layer above the bottom one colour 0 is transparent. Every
    // page command above paints the active layer of the current frame. Each layer command
    // is one undo step, except SelectLayer, a setting like GotoFrame.
    /// A new transparent layer (colour 0 everywhere, in every frame) above the active one;
    /// it becomes active. Without a name it is called "Layer N" (the smallest N unused).
    AddLayer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// ADR-45: a generated picture's "New layer" target. Like [`Command::AddLayer`] (above
    /// the active layer, which it becomes; "Layer N" without a name), but the new layer
    /// holds `pixels` (the page's size, row by row, in the palette) in the current frame and
    /// is transparent (colour 0) in every other frame. One undo step.
    AddPictureLayer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        pixels: Vec<u8>,
    },
    /// A copy of layer `layer` (every frame, name + " copy", same visibility) above it; the
    /// copy becomes active.
    DuplicateLayer { layer: u32 },
    /// Removes layer `layer` from every frame; the last layer cannot go. The layer below
    /// becomes active if the active one goes (the new bottom layer when it was the bottom).
    DeleteLayer { layer: u32 },
    /// Moves layer `from` to position `to` in every frame (the others close up); the active
    /// layer stays the same layer. A layer moved to the bottom becomes opaque.
    MoveLayer { from: u32, to: u32 },
    /// Makes layer `layer` the active one: a setting, no undo step (as GotoFrame).
    SelectLayer { layer: u32 },
    /// Shows or hides layer `layer`; hidden layers do not show and are not exported.
    SetLayerVisible { layer: u32, visible: bool },
    /// Renames layer `layer` (at most 255 characters).
    RenameLayer { layer: u32, name: String },
    /// Merges layer `layer` into the one below it in every frame: its pixels other than
    /// colour 0 cover the lower layer, whether shown or hidden; the result keeps the lower
    /// layer's name and visibility.
    MergeLayerDown { layer: u32 },
    /// Flattens every frame into one layer showing what the frame shows (hidden layers
    /// go): it keeps the bottom layer's name and is visible.
    FlattenLayers,
    /// Replaces the whole picture (Picture ▸ Spare ▸ Copy to Spare and Delete This Page on
    /// layered pages): every frame and the layer stack become `frames` and `layers`
    /// (`frames[f][l]` holds layer `l` of frame `f`, bottom layer first, `width`×`height`
    /// pixels row by row); frame `current` and layer `active` become the page. Palette,
    /// target and rate stay. Another page size must fit the target's page limits; layers and
    /// frames keep the limits of `add_layer` and the frame commands.
    /// [`Command::set_picture`] makes one from a document.
    SetPicture {
        width: u32,
        height: u32,
        layers: Vec<crate::Layer>,
        frames: Vec<Vec<Vec<u8>>>,
        #[serde(default)]
        current: u32,
        #[serde(default)]
        active: u32,
    },
    /// ADR-45: replaces the whole picture (a generated picture's "Picture" target): the
    /// target (chipset, mode and screen), the palette and one `width`×`height` page (row by
    /// row). Every other frame goes; the frame rate stays. One undo step brings the old
    /// document back. The target's limits are checked: the mode must exist on the chipset,
    /// the palette has a power of two of 2 up to the mode's maximum entries (12-bit values on
    /// OCS, unless `stored`), the screen fits the mode and the page the target's page limits.
    ReplacePicture {
        target: Target,
        palette: Vec<Rgb8>,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        /// The palette is a page's own, copied as it is stored ([`Command::replace_with_page`]:
        /// Copy to Spare and Delete This Page between pages of different formats). On OCS it
        /// may then keep colours that are not 12-bit, as a project saved before ADR-46 holds
        /// them (wave-8 integration; every IFF export still refuses them), so a copy never
        /// changes colours. Not part of the JSON form: a command from MCP or a script always
        /// gives 12-bit colours on OCS.
        #[serde(skip)]
        stored: bool,
    },
}

impl Command {
    /// True for the commands of ADR-28 that work on frames.
    fn is_frame_command(&self) -> bool {
        matches!(
            self,
            Command::SetFrameCount { .. }
                | Command::AddFrames { .. }
                | Command::CopyFrame { .. }
                | Command::InsertFrameCopies { .. }
                | Command::DeleteFrames { .. }
                | Command::GotoFrame { .. }
                | Command::SetFrameRate { .. }
        )
    }
}

/// What a command changes; decides what the engine snapshots for undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Footprint {
    /// Page pixels inside the rect (already clipped to the page; may be empty).
    Pixels(Rect),
    Palette,
    /// Page size, palette and target may all change.
    Document,
    /// The frames (number, order, contents, current one) change; size and palette stay.
    Frames,
    /// A setting (current frame, rate, active layer): applied without an undo step (ADR-28).
    Setting,
    /// The layer stack and every frame's layer pictures change (ADR-44); size and palette
    /// stay.
    Layers,
    /// Only layer names or visibility change (ADR-44).
    LayerInfo,
}

impl Command {
    /// Checks a page-pixel command (Stroke, Shape, FloodFill) exactly as
    /// [`crate::Engine::execute`] does on a document with this page and palette, without
    /// painting anything; other commands are refused. Front-ends use it before previewing a
    /// candidate command, so nothing is shown that the engine would refuse on release.
    pub fn check_paint(&self, page: &IndexedImage, palette: &Palette) -> Result<(), CommandError> {
        let colors = palette.len();
        let color = |c: u8| check_color(c, colors);
        let result = match self {
            Command::Stroke {
                brush,
                color: c,
                points,
                custom,
                range,
                stencil,
                ..
            } => {
                color(*c)?;
                validate_paint(colors, custom.as_ref(), *range, stencil.as_deref())?;
                if (brush.is_empty() && custom.is_none()) || brush.len() > MAX_BRUSH_PIXELS {
                    return Err(CommandError::invalid(format!(
                        "brush must have 1..={MAX_BRUSH_PIXELS} pixels, got {}",
                        brush.len()
                    )));
                }
                if brush
                    .iter()
                    .any(|&(x, y)| x.abs() > MAX_BRUSH_OFFSET || y.abs() > MAX_BRUSH_OFFSET)
                {
                    return Err(CommandError::invalid(format!(
                        "brush offsets must lie within ±{MAX_BRUSH_OFFSET}"
                    )));
                }
                if points.is_empty() || points.len() > MAX_STROKE_POINTS {
                    return Err(CommandError::invalid(format!(
                        "stroke must have 1..={MAX_STROKE_POINTS} points, got {}",
                        points.len()
                    )));
                }
                if points
                    .iter()
                    .any(|&(x, y)| x.abs() > MAX_COORD || y.abs() > MAX_COORD)
                {
                    return Err(CommandError::invalid(format!(
                        "stroke points must lie within ±{MAX_COORD}"
                    )));
                }
                Ok(())
            }
            Command::Shape {
                shape,
                brush,
                color: c,
                filled,
                symmetry,
                custom,
                range,
                stencil,
                ..
            } => {
                color(*c)?;
                validate_paint(colors, custom.as_ref(), *range, stencil.as_deref())?;
                shape.validate(brush, custom.is_some(), *filled, symmetry.as_ref())
            }
            Command::AnimBrushPaint { brush, paint } => {
                if !matches!(**paint, Command::Stroke { .. } | Command::Shape { .. }) {
                    return Err(CommandError::invalid(
                        "an AnimBrush paints with a stroke or a shape".into(),
                    ));
                }
                validate_anim_brush(colors, brush)?;
                paint.check_paint(page, palette)
            }
            Command::FloodFill {
                x,
                y,
                color: c,
                fill,
            } => {
                color(*c)?;
                if let FillType::Gradient { from, to, .. } = fill {
                    color(*from)?;
                    color(*to)?;
                }
                if x.abs() > MAX_COORD || y.abs() > MAX_COORD {
                    return Err(CommandError::invalid(format!(
                        "fill point must lie within ±{MAX_COORD}"
                    )));
                }
                return Ok(());
            }
            Command::Stenciled { area, command } => {
                check_area(area, page)?;
                return match **command {
                    Command::Stroke { .. }
                    | Command::Shape { .. }
                    | Command::FloodFill { .. }
                    | Command::AnimBrushPaint { .. } => command.check_paint(page, palette),
                    _ => Err(CommandError::invalid(
                        "only strokes, shapes and fills paint page pixels".into(),
                    )),
                };
            }
            _ => {
                return Err(CommandError::invalid(
                    "only strokes, shapes and fills paint page pixels".into(),
                ));
            }
        };
        result?;
        let (w, h) = (page.width(), page.height());
        let work = self.paint_work(w, h, colors);
        if work > MAX_PAINT_WORK {
            return Err(CommandError::invalid(format!(
                "too much work for one command: at least {} million units on this {w}x{h} \
                 page, at most {} million (use fewer points, a smaller brush or fewer \
                 symmetry copies)",
                work / 1_000_000,
                MAX_PAINT_WORK / 1_000_000
            )));
        }
        Ok(())
    }

    /// Checks the command against the document without changing anything.
    pub(crate) fn validate(&self, doc: &Document) -> Result<(), CommandError> {
        if self.is_frame_command() {
            return crate::frame_ops::validate(self, doc);
        }
        if let Command::MoveBrush { .. } = self {
            return crate::anim_move::validate(self, doc);
        }
        if self.is_layer_command() {
            return crate::layer::validate(self, doc);
        }
        let colors = doc.palette.len();
        let color = |c: u8| check_color(c, colors);
        match self {
            Command::Stroke { .. }
            | Command::Shape { .. }
            | Command::FloodFill { .. }
            | Command::AnimBrushPaint { .. } => self.check_paint(&doc.page, &doc.palette),
            Command::Clear { color: c } => color(*c),
            Command::Flip { .. } => Ok(()),
            Command::ReplaceColor { from, to } => color(*from).and(color(*to)),
            Command::SwapColors { a, b } => color(*a).and(color(*b)),
            Command::SetFormat {
                mode,
                colors: n,
                width,
                height,
                retain,
                screen,
                chipset,
            } => {
                let chipset = chipset.unwrap_or(doc.target.chipset);
                let max = mode.max_colors(chipset).ok_or_else(|| {
                    CommandError::new(
                        ErrorCode::UnsupportedMode,
                        format!("{} is not available on {chipset:?}", mode.name()),
                    )
                })?;
                if !n.is_power_of_two() || *n < 2 || *n > max {
                    return Err(CommandError::new(
                        ErrorCode::PaletteRangeInvalid,
                        format!(
                            "{n} colours: {} allows 2..={max} (powers of two)",
                            mode.name()
                        ),
                    ));
                }
                if !(1..=MAX_PAGE_SIDE).contains(width) || !(1..=MAX_PAGE_SIDE).contains(height) {
                    return Err(CommandError::invalid(format!(
                        "page {width}x{height} is outside 1..={MAX_PAGE_SIDE}"
                    )));
                }
                let (sw, sh) = screen.unwrap_or(mode.size());
                if !mode.screen_fits(sw, sh) {
                    let ((a, b), (c, d)) = mode.screen_limits();
                    return Err(CommandError::invalid(format!(
                        "screen {sw}x{sh}: {} allows {a}x{b} to {c}x{d} in whole UI pixels",
                        mode.name()
                    )));
                }
                if *width < sw || *height < sh {
                    return Err(CommandError::invalid(format!(
                        "page {width}x{height} is smaller than the {sw}x{sh} screen"
                    )));
                }
                // All frames take the new size (ADR-28); without `retain` one layer is left
                // (ADR-44), with it every layer takes the new size.
                let layers = if *retain { doc.layer_count() } else { 1 };
                check_frame_count(doc.frame_count(), layers, *width, *height)?;
                if *retain {
                    let kept = Rect::new(0, 0, *width as i32, *height as i32);
                    let max_used = doc
                        .cels()
                        .filter_map(|f| f.crop(kept).pixels().iter().max().copied())
                        .max();
                    if let Some(max_used) = max_used
                        && u16::from(max_used) >= *n
                    {
                        let need = (u16::from(max_used) + 1).next_power_of_two();
                        let message = if need > max {
                            format!(
                                "the picture uses colour {max_used}; {} on {chipset:?} has at \
                                 most {max} colours (turn Retain Picture off)",
                                mode.name()
                            )
                        } else {
                            format!(
                                "the picture uses colour {max_used}; keep at least {need} colours"
                            )
                        };
                        return Err(CommandError::new(ErrorCode::PaletteRangeInvalid, message));
                    }
                }
                Ok(())
            }
            Command::SetPageSize {
                width,
                height,
                fill,
            } => {
                color(*fill)?;
                check_frame_count(doc.frame_count(), doc.layer_count(), *width, *height)?;
                if doc.target.page_fits(*width, *height) {
                    Ok(())
                } else {
                    let ((min_w, min_h), (max_w, max_h)) = doc.target.page_limits();
                    Err(CommandError::invalid(format!(
                        "Page must be {min_w}x{min_h} to {max_w}x{max_h}"
                    )))
                }
            }
            Command::SetPage {
                width,
                height,
                pixels,
            } => {
                other_page(*width, *height, pixels, colors)?;
                let same = (*width, *height) == (doc.page.width(), doc.page.height());
                if !same && doc.frame_count() > 1 {
                    // All frames share one size (ADR-28); Page Size changes them together.
                    return Err(CommandError::invalid(format!(
                        "page {width}x{height}: all {} frames are {}x{}",
                        doc.frame_count(),
                        doc.page.width(),
                        doc.page.height()
                    )));
                }
                if !same && doc.layer_count() > 1 {
                    // So do all layers (ADR-44).
                    return Err(CommandError::invalid(format!(
                        "page {width}x{height}: all {} layers are {}x{}",
                        doc.layer_count(),
                        doc.page.width(),
                        doc.page.height()
                    )));
                }
                if same || doc.target.page_fits(*width, *height) {
                    Ok(())
                } else {
                    let ((min_w, min_h), (max_w, max_h)) = doc.target.page_limits();
                    Err(CommandError::invalid(format!(
                        "Page must be {min_w}x{min_h} to {max_w}x{max_h}"
                    )))
                }
            }
            Command::MergePage {
                width,
                height,
                pixels,
                transparent,
                ..
            } => {
                color(*transparent)?;
                other_page(*width, *height, pixels, colors)
            }
            Command::SetPalette { colors: new } => {
                if new.len() != colors {
                    return Err(CommandError::invalid(format!(
                        "palette must keep {colors} colours, got {}",
                        new.len()
                    )));
                }
                // OCS colours are 12-bit, a document rule (ADR-46). Only the entries this
                // changes must be: an older project's colours that are not 12-bit stay
                // editable entry by entry (wave-8 integration).
                let old = doc.palette.colors();
                let changed: Vec<Rgb8> = new
                    .iter()
                    .zip(old)
                    .map(|(&c, &o)| if c == o { Rgb8::BLACK } else { c })
                    .collect();
                match crate::hardware::first_unshowable(doc.target.chipset, &changed) {
                    Some((i, c)) => Err(CommandError::new(
                        ErrorCode::PaletteRangeInvalid,
                        format!(
                            "colour {i} {c} is not a 12-bit OCS colour (each channel a \
                             multiple of 0x11, e.g. {})",
                            crate::hardware::snap(Chipset::Ocs, c)
                        ),
                    )),
                    None => Ok(()),
                }
            }
            Command::SetFrameCount { .. }
            | Command::AddFrames { .. }
            | Command::CopyFrame { .. }
            | Command::InsertFrameCopies { .. }
            | Command::DeleteFrames { .. }
            | Command::GotoFrame { .. }
            | Command::SetFrameRate { .. }
            | Command::MoveBrush { .. } => unreachable!("frame commands are checked above"),
            Command::AddLayer { .. }
            | Command::AddPictureLayer { .. }
            | Command::DuplicateLayer { .. }
            | Command::DeleteLayer { .. }
            | Command::MoveLayer { .. }
            | Command::SelectLayer { .. }
            | Command::SetLayerVisible { .. }
            | Command::RenameLayer { .. }
            | Command::MergeLayerDown { .. }
            | Command::FlattenLayers
            | Command::SetPicture { .. } => unreachable!("layer commands are checked above"),
            Command::Stenciled { area, command } => {
                check_area(area, &doc.page)?;
                let same = |w: u32, h: u32| (w, h) == (doc.page.width(), doc.page.height());
                match **command {
                    Command::Stroke { .. }
                    | Command::Shape { .. }
                    | Command::AnimBrushPaint { .. }
                    | Command::FloodFill { .. }
                    | Command::Clear { .. } => {}
                    Command::SetPage { width, height, .. } if same(width, height) => {}
                    _ => {
                        return Err(CommandError::invalid(
                            "an area stencil covers strokes, shapes, fills, CLR and a page of \
                             the same size only"
                                .into(),
                        ));
                    }
                }
                command.validate(doc)
            }
            Command::ReplacePicture {
                target,
                palette,
                width,
                height,
                pixels,
                stored,
            } => validate_replace_picture(*target, palette, *width, *height, pixels, *stored),
            Command::RemapColors { from } => {
                if (1..=256).contains(&from.len()) {
                    Ok(())
                } else {
                    Err(CommandError::invalid(format!(
                        "remap needs 1 to 256 colours, got {}",
                        from.len()
                    )))
                }
            }
            Command::ArrangePalette { order } => {
                if !crate::remap::is_permutation(order, colors) {
                    return Err(CommandError::new(
                        ErrorCode::PaletteRangeInvalid,
                        format!("order must be a permutation of the {colors} palette entries"),
                    ));
                }
                // ADR-44: above the bottom layer colour 0 stays transparent, so no other
                // colour there may become entry 0.
                let to_zero = order[0];
                if to_zero != 0 && upper_layers_use(doc, to_zero) {
                    return Err(CommandError::new(
                        ErrorCode::PaletteRangeInvalid,
                        format!(
                            "colour {to_zero} would become colour 0, which is transparent on \
                             the upper layers that use it"
                        ),
                    ));
                }
                Ok(())
            }
        }
    }

    /// The part of the document this command changes. Call only after [`Self::validate`].
    pub(crate) fn footprint(&self, doc: &Document) -> Footprint {
        let page = doc.page.bounds();
        match self {
            Command::Stroke { .. } => {
                Footprint::Pixels(self.paint_bounds(&doc.page).unwrap_or_default())
            }
            Command::Shape { .. } | Command::FloodFill { .. } | Command::AnimBrushPaint { .. } => {
                Footprint::Pixels(self.paint_bounds(&doc.page).unwrap_or_default())
            }
            Command::Clear { .. }
            | Command::Flip { .. }
            | Command::ReplaceColor { .. }
            | Command::SwapColors { .. } => Footprint::Pixels(page),
            Command::SetPalette { .. } => Footprint::Palette,
            Command::SetFormat { .. } | Command::SetPageSize { .. } => Footprint::Document,
            Command::SetPage { width, height, .. } => {
                if (*width, *height) == (doc.page.width(), doc.page.height()) {
                    Footprint::Pixels(page)
                } else {
                    Footprint::Document
                }
            }
            Command::MergePage { width, height, .. } => {
                Footprint::Pixels(page.intersect(Rect::new(0, 0, *width as i32, *height as i32)))
            }
            Command::SetFrameCount { .. }
            | Command::AddFrames { .. }
            | Command::CopyFrame { .. }
            | Command::InsertFrameCopies { .. }
            | Command::DeleteFrames { .. } => Footprint::Frames,
            Command::GotoFrame { .. }
            | Command::SetFrameRate { .. }
            | Command::SelectLayer { .. } => Footprint::Setting,
            Command::AddLayer { .. }
            | Command::AddPictureLayer { .. }
            | Command::DuplicateLayer { .. }
            | Command::DeleteLayer { .. }
            | Command::MoveLayer { .. }
            | Command::MergeLayerDown { .. }
            | Command::FlattenLayers => Footprint::Layers,
            Command::SetPicture { width, height, .. } => {
                if (*width, *height) == (doc.page.width(), doc.page.height()) {
                    Footprint::Layers
                } else {
                    Footprint::Document
                }
            }
            Command::SetLayerVisible { .. } | Command::RenameLayer { .. } => Footprint::LayerInfo,
            Command::ArrangePalette { .. } | Command::ReplacePicture { .. } => Footprint::Document,
            // Every frame's pictures change; size and palette stay.
            Command::RemapColors { .. } => Footprint::Frames,
            // Several frames and the current one change (ADR-38).
            Command::MoveBrush { .. } => Footprint::Frames,
            Command::Stenciled { command, .. } => command.footprint(doc),
        }
    }

    /// How many kept palette entries this command rounds to colours its chipset shows
    /// (ADR-46): a SetFormat to OCS rounds the entries of an AGA palette that are not 12-bit
    /// (a change of the colours, reported in `Outcome::rounded_colors`, never silent). New
    /// grey-ramp entries are not counted; every other command rounds nothing. Call only after
    /// [`Self::validate`], before [`Self::apply`].
    pub(crate) fn rounded_colors(&self, doc: &Document) -> u16 {
        let Command::SetFormat {
            colors, chipset, ..
        } = self
        else {
            return 0;
        };
        let chipset = chipset.unwrap_or(doc.target.chipset);
        let kept = usize::from(*colors).min(doc.palette.len());
        let rounded = doc.palette.colors()[..kept]
            .iter()
            .filter(|&&c| crate::hardware::snap(chipset, c) != c)
            .count();
        rounded as u16
    }

    /// Applies the command. Call only after [`Self::validate`].
    pub(crate) fn apply(&self, doc: &mut Document) {
        if self.is_frame_command() {
            return crate::frame_ops::apply(self, doc);
        }
        if let Command::MoveBrush { .. } = self {
            return crate::anim_move::apply(self, doc);
        }
        if self.is_layer_command() {
            return crate::layer::apply(self, doc);
        }
        let page = &mut doc.page;
        match self {
            Command::Stroke { .. }
            | Command::Shape { .. }
            | Command::FloodFill { .. }
            | Command::AnimBrushPaint { .. }
            | Command::Stenciled { .. } => {
                let before = page.clone();
                self.paint(page, &before, &doc.palette);
            }
            Command::Clear { color } => {
                let (w, h) = (page.width() as i32, page.height() as i32);
                page.fill_rect(0, 0, w, h, *color);
            }
            Command::Flip {
                axis: Axis::Horizontal,
            } => page.flip_horizontal(),
            Command::Flip {
                axis: Axis::Vertical,
            } => page.flip_vertical(),
            Command::ReplaceColor { from, to } => page.replace_index(*from, *to),
            Command::SwapColors { a, b } => page.swap_indices(*a, *b),
            Command::SetPalette { colors } => {
                doc.palette = Palette::new(colors.clone()).expect("size checked by validate");
            }
            Command::SetFormat {
                mode,
                colors,
                width,
                height,
                retain,
                screen,
                chipset,
            } => {
                if *retain {
                    doc.for_each_cel(|_, cel| {
                        let mut new_page = IndexedImage::new(*width, *height, 0);
                        new_page.blit(cel, 0, 0);
                        *cel = new_page;
                    });
                } else {
                    // A cleared page has one layer, "Background" (ADR-44).
                    let blank = std::sync::Arc::new(IndexedImage::new(*width, *height, 0));
                    let frames = vec![vec![blank]; doc.frame_count()];
                    let current = doc.current;
                    doc.set_layered(frames, vec![crate::Layer::background()], current, 0);
                }
                let chipset = chipset.unwrap_or(doc.target.chipset);
                // The kept entries this rounds are counted in `Outcome::rounded_colors`
                // ([`Self::rounded_colors`]), so no front-end changes colours silently.
                doc.palette = resized_palette(&doc.palette, usize::from(*colors), chipset);
                doc.target.chipset = chipset;
                doc.target.mode = *mode;
                doc.target.screen = screen.unwrap_or(mode.size());
            }
            Command::SetPageSize {
                width,
                height,
                fill,
            } => {
                // New area: the fill (BG) colour on the bottom layer, transparent above it.
                doc.for_each_cel(|layer, cel| {
                    let fill = if layer == 0 { *fill } else { 0 };
                    let mut new_page = IndexedImage::new(*width, *height, fill);
                    new_page.blit(cel, 0, 0);
                    *cel = new_page;
                });
            }
            Command::SetPage {
                width,
                height,
                pixels,
            } => {
                doc.page = IndexedImage::from_pixels(*width, *height, pixels.clone())
                    .expect("size checked by validate");
            }
            Command::MergePage {
                width,
                height,
                pixels,
                front,
                transparent,
            } => {
                // In back, the frame as it shows decides (ADR-44): a pixel of the active layer
                // that is transparent but lies over another layer's colour is no hole in the
                // picture. With one visible layer that is the page itself.
                let shown = (!*front).then(|| doc.composite().into_owned());
                let page = &mut doc.page;
                let w = (*width).min(page.width()) as usize;
                let h = (*height).min(page.height()) as usize;
                let pw = page.width() as usize;
                let mut merged = page.pixels().to_vec();
                for y in 0..h {
                    let other = &pixels[y * *width as usize..][..w];
                    for (x, &o) in other.iter().enumerate() {
                        let i = y * pw + x;
                        let take = match &shown {
                            None => o != *transparent,
                            Some(shown) => {
                                merged[i] == *transparent && shown.pixels()[i] == *transparent
                            }
                        };
                        if take {
                            merged[i] = o;
                        }
                    }
                }
                *page = IndexedImage::from_pixels(page.width(), page.height(), merged)
                    .expect("same size");
            }
            Command::SetFrameCount { .. }
            | Command::AddFrames { .. }
            | Command::CopyFrame { .. }
            | Command::InsertFrameCopies { .. }
            | Command::DeleteFrames { .. }
            | Command::GotoFrame { .. }
            | Command::SetFrameRate { .. }
            | Command::MoveBrush { .. } => unreachable!("frame commands are applied above"),
            Command::AddLayer { .. }
            | Command::AddPictureLayer { .. }
            | Command::DuplicateLayer { .. }
            | Command::DeleteLayer { .. }
            | Command::MoveLayer { .. }
            | Command::SelectLayer { .. }
            | Command::SetLayerVisible { .. }
            | Command::RenameLayer { .. }
            | Command::MergeLayerDown { .. }
            | Command::FlattenLayers
            | Command::SetPicture { .. } => unreachable!("layer commands are applied above"),
            Command::RemapColors { from } => {
                let lower = crate::remap::nearest_lut(from, &doc.palette);
                let upper = crate::remap::nearest_lut_upper(from, &doc.palette);
                doc.for_each_cel(|layer, cel| {
                    *cel = crate::remap::remap_with(cel, if layer == 0 { &lower } else { &upper });
                });
            }
            Command::ReplacePicture {
                target,
                palette,
                width,
                height,
                pixels,
                ..
            } => {
                // One frame with one layer, "Background" (ADR-44): the generated picture.
                let page = IndexedImage::from_pixels(*width, *height, pixels.clone())
                    .expect("size checked by validate");
                doc.set_layered(
                    vec![vec![std::sync::Arc::new(page)]],
                    vec![crate::Layer::background()],
                    0,
                    0,
                );
                doc.palette = Palette::new(palette.clone()).expect("size checked by validate");
                doc.target = *target;
            }
            Command::ArrangePalette { order } => {
                // Every animation frame and layer shares the palette (ADR-28, ADR-44), so
                // all are renumbered; above the bottom layer colour 0 stays transparent.
                doc.for_each_cel(|layer, cel| {
                    *cel = crate::remap::arranged_picture(cel, order, layer > 0);
                });
                doc.palette =
                    Palette::new(crate::remap::arranged_palette(doc.palette.colors(), order))
                        .expect("same number of colours");
            }
        }
    }
}

/// `skip_serializing_if` for fields that are left out when they have their default value.
fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// Largest absolute brush handle coordinate (the handle may lie outside the brush).
pub const MAX_HANDLE: i32 = 2 * MAX_PAGE_SIDE as i32;

/// `c` must be a colour of a palette of `colors` entries.
fn check_color(c: u8, colors: usize) -> Result<(), CommandError> {
    if usize::from(c) < colors {
        Ok(())
    } else {
        Err(CommandError::new(
            ErrorCode::PaletteRangeInvalid,
            format!("colour {c} is outside the palette (0..{colors})"),
        ))
    }
}

/// Checks an AnimBrush (ADR-36): 1 to [`anim_brush::MAX_CELS`] cels, each a valid custom
/// brush for a palette of `colors` entries, at most [`anim_brush::MAX_CEL_BYTES`] of pixels,
/// a Duration of 1 to [`anim_brush::MAX_DURATION`].
fn validate_anim_brush(colors: usize, brush: &AnimBrush) -> Result<(), CommandError> {
    let n = brush.cels.len();
    if !(1..=anim_brush::MAX_CELS).contains(&n) {
        return Err(CommandError::invalid(format!(
            "an AnimBrush has 1..={} cels, got {n}",
            anim_brush::MAX_CELS
        )));
    }
    if !(1..=anim_brush::MAX_DURATION).contains(&brush.duration) {
        return Err(CommandError::invalid(format!(
            "AnimBrush duration must be 1..={}, got {}",
            anim_brush::MAX_DURATION,
            brush.duration
        )));
    }
    if brush.bytes() > anim_brush::MAX_CEL_BYTES {
        return Err(CommandError::invalid(format!(
            "AnimBrush cels hold {} bytes, at most {}",
            brush.bytes(),
            anim_brush::MAX_CEL_BYTES
        )));
    }
    for cel in &brush.cels {
        validate_paint(colors, Some(cel), None, None)?;
    }
    Ok(())
}

/// Checks the paint fields of a Stroke or Shape (ADR-18): a custom brush whose pixel count
/// matches its size, each side at most [`MAX_BRUSH_SIDE`], colours inside the palette of
/// `colors` entries; a colour range inside the palette; a stencil with one entry per possible
/// colour (256).
pub(crate) fn validate_paint(
    colors: usize,
    custom: Option<&BrushImage>,
    range: Option<(u8, u8)>,
    stencil: Option<&[bool]>,
) -> Result<(), CommandError> {
    let outside = |c: u8, what: &str| {
        CommandError::new(
            ErrorCode::PaletteRangeInvalid,
            format!("{what} colour {c} is outside the palette (0..{colors})"),
        )
    };
    if let Some(b) = custom {
        let max = MAX_BRUSH_SIDE;
        if !(1..=max).contains(&b.width) || !(1..=max).contains(&b.height) {
            return Err(CommandError::invalid(format!(
                "brush {}x{} must be 1x1 to {max}x{max}",
                b.width, b.height
            )));
        }
        let n = b.width as usize * b.height as usize;
        if b.pixels.len() != n {
            return Err(CommandError::invalid(format!(
                "brush {}x{} needs {n} pixels, got {}",
                b.width,
                b.height,
                b.pixels.len()
            )));
        }
        if b.handle.0.abs() > MAX_HANDLE || b.handle.1.abs() > MAX_HANDLE {
            return Err(CommandError::invalid(format!(
                "brush handle must lie within ±{MAX_HANDLE}"
            )));
        }
        if let Some(&max) = b.pixels.iter().max()
            && usize::from(max) >= colors
        {
            return Err(outside(max, "brush"));
        }
        if let Some(t) = b.transparent
            && usize::from(t) >= colors
        {
            return Err(outside(t, "transparent"));
        }
    }
    if let Some((low, high)) = range {
        if low > high {
            return Err(CommandError::invalid(format!(
                "colour range {low}..={high} is empty"
            )));
        }
        if usize::from(high) >= colors {
            return Err(outside(high, "range"));
        }
    }
    if let Some(s) = stencil
        && s.len() != 256
    {
        return Err(CommandError::invalid(format!(
            "stencil must have 256 entries, got {}",
            s.len()
        )));
    }
    Ok(())
}

/// An area stencil (ADR-39) must have the page's size.
pub(crate) fn check_area(
    area: &crate::stencil::AreaStencil,
    page: &IndexedImage,
) -> Result<(), CommandError> {
    if area.fits(page) {
        Ok(())
    } else {
        Err(CommandError::invalid(format!(
            "area stencil {}x{} does not fit the {}x{} page",
            area.width(),
            area.height(),
            page.width(),
            page.height()
        )))
    }
}

/// Checks a page passed in a command: `width`×`height` pixels, all inside the palette.
pub(crate) fn other_page(
    width: u32,
    height: u32,
    pixels: &[u8],
    colors: usize,
) -> Result<(), CommandError> {
    if !(1..=MAX_PAGE_SIDE).contains(&width) || !(1..=MAX_PAGE_SIDE).contains(&height) {
        return Err(CommandError::invalid(format!(
            "page {width}x{height} is outside 1..={MAX_PAGE_SIDE}"
        )));
    }
    let n = width as usize * height as usize;
    if pixels.len() != n {
        return Err(CommandError::invalid(format!(
            "page {width}x{height} needs {n} pixels, got {}",
            pixels.len()
        )));
    }
    match pixels.iter().max() {
        Some(&max) if usize::from(max) >= colors => Err(CommandError::new(
            ErrorCode::PaletteRangeInvalid,
            format!("colour {max} is outside the palette (0..{colors})"),
        )),
        _ => Ok(()),
    }
}

/// True if a layer above the bottom one of any frame uses colour `c` (ADR-44).
fn upper_layers_use(doc: &Document, c: u8) -> bool {
    (1..doc.layer_count()).any(|l| {
        (0..doc.frame_count()).any(|f| doc.cel(f, l).is_some_and(|p| p.pixels().contains(&c)))
    })
}

impl Command {
    /// The [`Command::ReplacePicture`] that makes a document `doc`'s page ([`Document::page`],
    /// the active layer of the current frame; a layered picture follows with
    /// [`Command::set_picture`]) with `doc`'s target and its palette as stored (`stored`: on OCS
    /// also colours that are not 12-bit, as an older project's are; ADR-46, wave-8
    /// integration). Copy to Spare and Delete This Page between pages of different formats.
    pub fn replace_with_page(doc: &Document) -> Command {
        let page = doc.page();
        Command::ReplacePicture {
            target: doc.target(),
            palette: doc.palette().colors().to_vec(),
            width: page.width(),
            height: page.height(),
            pixels: page.pixels().to_vec(),
            stored: true,
        }
    }
}

/// Checks [`Command::ReplacePicture`] against its own target (the document's is replaced).
/// A `stored` palette (a page's own, copied) skips the 12-bit rule of OCS; the size rules
/// still apply.
fn validate_replace_picture(
    target: Target,
    palette: &[Rgb8],
    width: u32,
    height: u32,
    pixels: &[u8],
    stored: bool,
) -> Result<(), CommandError> {
    let mode = target.mode;
    let max = mode.max_colors(target.chipset).ok_or_else(|| {
        CommandError::new(
            ErrorCode::UnsupportedMode,
            format!("{} is not available on {:?}", mode.name(), target.chipset),
        )
    })?;
    let n = palette.len();
    if !n.is_power_of_two() || n < 2 || n > usize::from(max) {
        return Err(CommandError::new(
            ErrorCode::PaletteRangeInvalid,
            format!(
                "{n} colours: {} on {:?} allows 2..={max} (powers of two)",
                mode.name(),
                target.chipset
            ),
        ));
    }
    if target.chipset == Chipset::Ocs
        && !stored
        && let Some(i) = palette
            .iter()
            .position(|&c| Rgb8::from_rgb4(c.to_rgb4()) != c)
    {
        return Err(CommandError::new(
            ErrorCode::PaletteRangeInvalid,
            format!(
                "palette entry {i} ({}) is not a 12-bit OCS colour",
                palette[i]
            ),
        ));
    }
    let (sw, sh) = target.screen;
    if !mode.screen_fits(sw, sh) {
        let ((a, b), (c, d)) = mode.screen_limits();
        return Err(CommandError::invalid(format!(
            "screen {sw}x{sh}: {} allows {a}x{b} to {c}x{d} in whole UI pixels",
            mode.name()
        )));
    }
    if !target.page_fits(width, height) {
        let ((min_w, min_h), (max_w, max_h)) = target.page_limits();
        return Err(CommandError::invalid(format!(
            "page {width}x{height}: this target allows {min_w}x{min_h} to {max_w}x{max_h}"
        )));
    }
    other_page(width, height, pixels, n)
}

/// Keeps the first `n` entries; missing ones become a grey ramp from black to white. Every
/// entry is snapped to what `chipset` shows (12-bit on OCS, [`crate::hardware::snap`]).
fn resized_palette(palette: &Palette, n: usize, chipset: Chipset) -> Palette {
    let mut colors: Vec<Rgb8> = palette.colors().iter().copied().take(n).collect();
    let start = colors.len();
    let extra = n - start;
    for i in 0..extra {
        let v = if extra > 1 {
            (i * 255 / (extra - 1)) as u8
        } else {
            255
        };
        colors.push(Rgb8::new(v, v, v));
    }
    crate::hardware::snap_colors(chipset, &mut colors);
    Palette::new(colors).expect("1..=256 entries")
}

/// Stable error codes, also used by the MCP interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    PaletteRangeInvalid,
    RevisionConflict,
    UnsupportedMode,
    NothingToUndo,
    NothingToRedo,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ErrorCode::PaletteRangeInvalid => "PALETTE_RANGE_INVALID",
            ErrorCode::RevisionConflict => "REVISION_CONFLICT",
            ErrorCode::UnsupportedMode => "UNSUPPORTED_MODE",
            ErrorCode::NothingToUndo => "NOTHING_TO_UNDO",
            ErrorCode::NothingToRedo => "NOTHING_TO_REDO",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: String) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CommandError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_is_stable() {
        let cmd = Command::Flip {
            axis: Axis::Horizontal,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"op":"flip","axis":"horizontal"}"#);
        let stroke: Command = serde_json::from_str(
            r#"{"op":"stroke","brush":[[0,0]],"color":1,"points":[[3,4],[5,6]],"continuous":true}"#,
        )
        .unwrap();
        assert_eq!(
            stroke,
            Command::Stroke {
                brush: vec![(0, 0)],
                color: 1,
                points: vec![(3, 4), (5, 6)],
                continuous: true,
                custom: None,
                mode: DrawMode::Matte,
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            }
        );
        // Without the paint fields set, the JSON stays as it was before ADR-18.
        assert_eq!(
            serde_json::to_string(&stroke).unwrap(),
            r#"{"op":"stroke","brush":[[0,0]],"color":1,"points":[[3,4],[5,6]],"continuous":true}"#
        );
        let pal = Command::SetPalette {
            colors: vec![Rgb8::new(1, 2, 3)],
        };
        let back: Command = serde_json::from_str(&serde_json::to_string(&pal).unwrap()).unwrap();
        assert_eq!(back, pal);
        assert_eq!(
            serde_json::to_string(&ErrorCode::PaletteRangeInvalid).unwrap(),
            format!("\"{}\"", ErrorCode::PaletteRangeInvalid.as_str())
        );
    }

    fn doc_4x2() -> Document {
        let page = IndexedImage::from_pixels(4, 2, vec![0, 1, 0, 2, 3, 0, 0, 0]).unwrap();
        Document::from_picture(
            page,
            Palette::dpaint_default_32(),
            crate::mode::Target::default(),
        )
    }

    #[test]
    fn merge_page_in_front_and_in_back() {
        // The other page is 3x3: clipped to the 4x2 page, top-left aligned.
        let other = vec![5, 0, 6, 7, 8, 0, 9, 9, 9];
        let merge = |front| Command::MergePage {
            width: 3,
            height: 3,
            pixels: other.clone(),
            front,
            transparent: 0,
        };
        let mut doc = doc_4x2();
        merge(true).validate(&doc).unwrap();
        assert_eq!(
            merge(true).footprint(&doc),
            Footprint::Pixels(Rect::new(0, 0, 3, 2))
        );
        merge(true).apply(&mut doc);
        // In front: the other page's colour-0 pixels let the page show through.
        assert_eq!(doc.page.pixels(), &[5, 1, 6, 2, 7, 8, 0, 0]);
        let mut doc = doc_4x2();
        merge(false).apply(&mut doc);
        // In back: the page's own colour-0 pixels show the other page.
        assert_eq!(doc.page.pixels(), &[5, 1, 6, 2, 3, 8, 0, 0]);
        // Colours outside the palette and wrong sizes are refused.
        let bad = Command::MergePage {
            width: 1,
            height: 1,
            pixels: vec![40],
            front: true,
            transparent: 0,
        };
        assert_eq!(
            bad.validate(&doc).unwrap_err().code,
            ErrorCode::PaletteRangeInvalid
        );
        let short = Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![0; 7],
        };
        assert_eq!(
            short.validate(&doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn set_page_replaces_the_pixels() {
        let mut doc = doc_4x2();
        let same = Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![1; 8],
        };
        same.validate(&doc).unwrap();
        assert_eq!(same.footprint(&doc), Footprint::Pixels(doc.page.bounds()));
        same.apply(&mut doc);
        assert_eq!(doc.page.pixels(), &[1; 8]);
        // Another size must be a valid page for the target (at least the 320x256 screen).
        let small = Command::SetPage {
            width: 2,
            height: 2,
            pixels: vec![0; 4],
        };
        assert!(small.validate(&doc).is_err());
        let big = Command::SetPage {
            width: 320,
            height: 300,
            pixels: vec![2; 320 * 300],
        };
        big.validate(&doc).unwrap();
        assert_eq!(big.footprint(&doc), Footprint::Document);
        let json = serde_json::to_string(&small).unwrap();
        assert!(json.starts_with(r#"{"op":"set_page","width":2"#), "{json}");
    }
}
