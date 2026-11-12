//! Drawing-tool interaction (ADR-14): pointer gestures on the page become exactly one
//! undoable [`Command`] each. While a gesture is in progress its candidate command is painted
//! into a copy of the page with the same dpx-core code the engine uses, so the preview is the
//! committed result. Also the toolbox toggles (Grid, Symmetry, Magnify, Zoom), the tool keys
//! and `--commands`.
//!
//! Custom brushes (ADR-18): the Brush Selector picks up part of the page; every drawing tool
//! then stamps it, in the Mode menu's drawing mode, and it follows the pointer as a preview.

use std::borrow::Cow;

use dpx_core::AnimBrush;
use dpx_core::brush_ops;
use dpx_core::gradient::range_for;
use dpx_core::shapes::MAX_SHAPE_POINTS;
use dpx_core::{
    BrushImage, Command, DrawMode, Engine, FillType, IndexedImage, Palette, Rect, Shape, Symmetry,
    raster,
};
use dpx_ui::font::{self, Font};
use dpx_ui::{
    FillSettings, GridSettings, PaintMode, Pens, Pref, Prefs, SpacingMode, SpacingSettings,
    SymmetrySettings, Tool, UiState, toolbox,
};
use eframe::egui;

use crate::{PainterApp, View};

/// Default grid spacing in page pixels (the Gridding requester changes it).
pub const GRID: i32 = 16;
/// Default cyclic symmetry order (the Symmetry requester changes it).
pub const SYMMETRY_ORDER: u8 = 6;
/// Default airbrush spray radius in page pixels (horizontally; Be Square adjusts the height).
pub const AIRBRUSH_RADIUS: i32 = 10;
/// Default count of the Spacing requester (stamps, pixels or sprays per pixel).
pub const SPACING_N: i32 = 8;
/// Brush stamps the airbrush sprays per frame while the button is held.
pub const SPRAYS_PER_FRAME: usize = 4;
/// Magnify factors the Zoom tool steps through.
pub const ZOOMS: [i32; 8] = [2, 3, 4, 6, 8, 12, 16, 20];
/// 4× when Magnify is switched on the first time.
const DEFAULT_ZOOM: usize = 2;
/// A polygon closes when clicked within this many pixels of its first corner.
const CLOSE_DISTANCE: i32 = 2;
/// Width of the bar between the normal and the magnified half, in UI pixels.
const DIVIDER: i32 = 3;
/// Text tool line pitch in page pixels.
const TEXT_LINE: i32 = font::GLYPH_H + 2;

/// Toolbox toggles and tool settings; not part of the document.
#[derive(Clone, Debug)]
pub struct ToolState {
    pub grid: bool,
    /// Gridding requester: distance between grid points.
    pub grid_size: GridSettings,
    pub symmetry: bool,
    /// Symmetry requester: cyclic or mirror, number of points.
    pub symmetry_settings: SymmetrySettings,
    /// Spacing requester: how brush stamps are laid along a path, and the airbrush radius.
    pub spacing: SpacingSettings,
    /// Fill Type requester (the Fill tool).
    pub fill: FillSettings,
    /// Effect ▸ Stencil: the stencil while it is switched off (On/Off keeps it).
    pub stencil_off: Option<Vec<bool>>,
    /// Magnify on: the page pixel at the centre of the magnified half.
    pub magnify: Option<(i32, i32)>,
    /// Index into [`ZOOMS`].
    pub zoom: usize,
    /// Shift held this frame (constrains lines, squares rectangles).
    pub shift: bool,
    /// Airbrush random state (xorshift); the sprayed points end up in the command.
    rng: u32,
    /// The tool (and its filled half) to go back to after a Brush Selector pick-up.
    pick_return: Option<(Tool, bool)>,
    /// Brush menu gesture waiting for or in a drag (Stretch, Any Angle, Shear, Bend, Place;
    /// ADR-30).
    xform: Option<Xform>,
    /// After Rotate > Any Angle: the brush it made and the one it was made from, so that
    /// a second use starts again from the original orientation.
    rotate_base: Option<(BrushImage, BrushImage)>,
    /// The hidden spare brush (Brush > Spare, ADR-30).
    pub spare_brush: Option<BrushImage>,
    /// The page palette when the custom brush was picked up or loaded: what its colour
    /// indices meant then. Brush > Remap fits them to the current palette (ADR-30).
    pub brush_palette: Option<Palette>,
    /// Counts the brushes picked up or loaded (a new one ends perspective mode, ADR-37);
    /// changes to the brush itself (Arrange, Use Brush Palette, transforms) do not count.
    pub brush_gen: u64,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            grid: false,
            grid_size: GridSettings { x: GRID, y: GRID },
            symmetry: false,
            symmetry_settings: SymmetrySettings {
                mirror: false,
                order: SYMMETRY_ORDER,
            },
            spacing: SpacingSettings {
                mode: SpacingMode::Continuous,
                n: SPACING_N,
                airbrush_radius: AIRBRUSH_RADIUS,
            },
            fill: FillSettings {
                gradient: None,
                dither: false,
            },
            stencil_off: None,
            magnify: None,
            zoom: DEFAULT_ZOOM,
            shift: false,
            rng: 0x2545_F491,
            pick_return: None,
            xform: None,
            rotate_base: None,
            spare_brush: None,
            brush_palette: None,
            brush_gen: 0,
        }
    }
}

impl ToolState {
    fn random(&mut self, range: i32) -> i32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % (2 * range as u32 + 1)) as i32 - range
    }
}

/// What a gesture paints with (ADR-18): the fields its commands carry besides the geometry.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ink {
    /// FG, or BG with the right button.
    color: u8,
    /// The built-in brush (unused while `custom` is set).
    brush: Vec<(i32, i32)>,
    custom: Option<BrushImage>,
    mode: DrawMode,
    range: Option<(u8, u8)>,
    stencil: Option<Vec<bool>>,
    right: bool,
    /// Effect menu: Translucency, Process, AntiAlias (ADR-31).
    fx: dpx_core::PaintFx,
    /// AnimBrush (ADR-36): its cels, clock at the start of the gesture.
    cels: Option<AnimBrush>,
    /// Stamps already painted in parts (freehand, airbrush) since the gesture started.
    parts: u64,
    /// The area stencil (ADR-39): every command of the gesture is wrapped in it, so the
    /// preview spares the same pixels as the engine.
    area: Option<dpx_core::AreaStencil>,
}

impl Ink {
    /// `cmd` under the area stencil, if there is one.
    fn stenciled(&self, cmd: Command) -> Command {
        match &self.area {
            Some(area) => Command::Stenciled {
                area: area.clone(),
                command: Box::new(cmd),
            },
            None => cmd,
        }
    }

    /// `cmd` stamping the AnimBrush's cels from `offset` stamps after the gesture's start,
    /// when the gesture paints with an AnimBrush and `cmd` stamps the custom brush (ADR-36).
    fn with_cels(&self, cmd: Command, offset: u64) -> Command {
        match &self.cels {
            Some(b)
                if matches!(
                    cmd,
                    Command::Stroke {
                        custom: Some(_),
                        ..
                    } | Command::Shape {
                        custom: Some(_),
                        ..
                    }
                ) =>
            {
                let mut brush = b.clone();
                brush.advance(offset);
                Command::AnimBrushPaint {
                    brush,
                    paint: Box::new(cmd),
                }
            }
            _ => cmd,
        }
    }

    /// Whether parts of a stroke may be painted on top of each other: the modes that only set
    /// colours. The others depend on the order and motion of every stamp, so the preview is
    /// painted again as a whole.
    fn paints_in_parts(&self) -> bool {
        matches!(
            self.mode,
            DrawMode::Matte | DrawMode::Color | DrawMode::Replace
        )
    }
}

/// A gesture in progress (named after the freehand stroke it started as).
pub struct Stroke {
    /// The page with the candidate command painted on it.
    pub preview: IndexedImage,
    /// The part of `preview` that may differ from the document page.
    drawn: Rect,
    ink: Ink,
    filled: bool,
    symmetry: Option<Symmetry>,
    /// Spacing requester settings at the start of the gesture.
    spacing: SpacingSettings,
    /// Displayed pixel shape (Be Square), for the airbrush spacing.
    aspect: (i32, i32),
    /// The button that started the gesture is still held.
    held: bool,
    kind: Kind,
    /// Brush ▸ Handle ▸ Place: the handle's page pixel, marked over the picture as shown.
    mark: Option<(i32, i32)>,
}

enum Kind {
    /// Dotted or continuous freehand, painted incrementally.
    Freehand {
        points: Vec<(i32, i32)>,
        continuous: bool,
    },
    /// Filled freehand: a 1-pixel outline while drawing, filled on release.
    Lasso {
        points: Vec<(i32, i32)>,
    },
    Line {
        from: (i32, i32),
        to: (i32, i32),
    },
    /// First the chord is dragged; after the release `through` follows the pointer until a
    /// click fixes it.
    Curve {
        from: (i32, i32),
        to: (i32, i32),
        through: Option<(i32, i32)>,
    },
    Rect {
        from: (i32, i32),
        to: (i32, i32),
    },
    Circle {
        center: (i32, i32),
        to: (i32, i32),
        aspect: (i32, i32),
    },
    Ellipse {
        center: (i32, i32),
        to: (i32, i32),
    },
    /// Corners so far; `cursor` = the rubber band's end.
    Polygon {
        points: Vec<(i32, i32)>,
        cursor: Option<(i32, i32)>,
        closed: bool,
    },
    Airbrush {
        points: Vec<(i32, i32)>,
        at: (i32, i32),
    },
    /// Typed text with its top-left at `at`.
    Text {
        at: (i32, i32),
        lines: Vec<String>,
    },
    /// Brush Selector: the frame from `from` to `to` (corners inclusive) while dragging; the
    /// right button cuts.
    Pick {
        from: (i32, i32),
        to: (i32, i32),
    },
    /// No button held: the custom brush shown at the pointer, never committed.
    Hover {
        at: (i32, i32),
    },
}

/// A key the tools react to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKey {
    Char(char),
    Space,
    Enter,
    Escape,
    Backspace,
    Arrow(i32, i32),
    N,
}

fn cheb(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// Horizontal or vertical from `from`, whichever is closer to `p` on screen.
fn constrain_hv(from: (i32, i32), p: (i32, i32), (fx, fy): (i32, i32)) -> (i32, i32) {
    if (p.0 - from.0).abs() * fx >= (p.1 - from.1).abs() * fy {
        (p.0, from.1)
    } else {
        (from.0, p.1)
    }
}

/// The corner opposite `from` of the square (as displayed) towards `p`.
fn square(from: (i32, i32), p: (i32, i32), (fx, fy): (i32, i32)) -> (i32, i32) {
    let (dx, dy) = (p.0 - from.0, p.1 - from.1);
    let side = (dx.abs() * fx).max(dy.abs() * fy);
    let sign = |d: i32| if d < 0 { -1 } else { 1 };
    (
        from.0 + sign(dx) * ((side + fx / 2) / fx),
        from.1 + sign(dy) * ((side + fy / 2) / fy),
    )
}

/// Radii of the circle around `c` through `p` (as displayed: round on non-square pixels).
fn circle_radius(c: (i32, i32), p: (i32, i32), (fx, fy): (i32, i32)) -> (i32, i32) {
    let dx = f64::from((p.0 - c.0) * fx);
    let dy = f64::from((p.1 - c.1) * fy);
    let r = dx.hypot(dy);
    (
        (r / f64::from(fx)).round() as i32,
        (r / f64::from(fy)).round() as i32,
    )
}

/// The pixels of `lines` set in the UI font, top-left at `at`.
fn text_pixels(at: (i32, i32), lines: &[String]) -> Vec<(i32, i32)> {
    let font = Font::builtin();
    let w = lines.iter().map(|l| font.text_width(l)).max().unwrap_or(0) + 8;
    let h = lines.len() as i32 * TEXT_LINE;
    let mut img = IndexedImage::new(w.max(1) as u32, h.max(1) as u32, 0);
    for (i, line) in lines.iter().enumerate() {
        font.draw(&mut img, 0, i as i32 * TEXT_LINE, line, 1);
    }
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if img.get(x, y) == Some(1) {
                out.push((at.0 + x, at.1 + y));
            }
        }
    }
    out
}

/// Every pixel of the path through `points` (joined by Bresenham lines; `closed` also joins
/// the last point to the first), without repeating the joints.
fn dense_path(points: &[(i32, i32)], closed: bool) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    let mut join = |a: (i32, i32), b: (i32, i32)| {
        let line = raster::line_points(a, b);
        out.extend(line.into_iter().skip(usize::from(!out.is_empty())));
    };
    for pair in points.windows(2) {
        join(pair[0], pair[1]);
    }
    if closed && points.len() > 2 {
        join(points[points.len() - 1], points[0]);
    }
    if out.is_empty() {
        out.extend(points.first().copied());
    }
    out
}

/// A pseudo-random value in `0..range` from `a`, `b`, `c` alone, so a gesture that is redrawn
/// from its points sprays the same pixels every time.
fn hash_range(a: u64, b: u64, c: u64, range: u64) -> u64 {
    let mut x = a
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add(c.wrapping_mul(0x1656_67B1_9E37_79F9));
    x ^= x >> 31;
    x = x.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^= x >> 29;
    x % range.max(1)
}

/// Where the brush is stamped along a path for the Spacing requester (ADR-20). `Continuous`
/// is not handled here (the stamps then form an unbroken line); the other modes give the
/// stamp points: exactly `n` evenly spread, one every `n` pixels, or `n` sprays per pixel
/// within the airbrush radius (`aspect` keeps the spray round on non-square pixels).
fn spaced_points(
    points: &[(i32, i32)],
    closed: bool,
    s: &SpacingSettings,
    aspect: (i32, i32),
) -> Vec<(i32, i32)> {
    let path = dense_path(points, closed);
    let n = s.n.max(1) as usize;
    let mut out = match s.mode {
        SpacingMode::Continuous => path,
        SpacingMode::NTotal if path.len() <= 1 || n == 1 => path.into_iter().take(1).collect(),
        SpacingMode::NTotal => {
            let last = path.len() - 1;
            (0..n).map(|i| path[i * last / (n - 1)]).collect()
        }
        SpacingMode::EveryN => path.into_iter().step_by(n).collect(),
        SpacingMode::Airbrush => {
            let (fx, fy) = aspect;
            let rx = s.airbrush_radius.max(1);
            let ry = (rx * fx / fy.max(1)).max(1);
            let mut out = Vec::with_capacity(path.len() * n);
            for (i, &(x, y)) in path.iter().enumerate() {
                for k in 0..n {
                    // Rejection sampling inside the ellipse; a few tries, then the centre.
                    let (mut dx, mut dy) = (0, 0);
                    for t in 0..8u64 {
                        let seed = (i * n + k) as u64;
                        let ddx = hash_range(seed, t, 1, 2 * rx as u64 + 1) as i32 - rx;
                        let ddy = hash_range(seed, t, 2, 2 * ry as u64 + 1) as i32 - ry;
                        if ddx * ddx * ry * ry + ddy * ddy * rx * rx <= rx * rx * ry * ry {
                            (dx, dy) = (ddx, ddy);
                            break;
                        }
                    }
                    out.push((x + dx, y + dy));
                }
            }
            out
        }
    };
    out.truncate(MAX_SHAPE_POINTS);
    out
}

/// The Mode menu's mode as the drawing mode the commands carry.
pub fn draw_mode(mode: PaintMode) -> DrawMode {
    match mode {
        PaintMode::Matte => DrawMode::Matte,
        PaintMode::Color => DrawMode::Color,
        PaintMode::Replace => DrawMode::Replace,
        PaintMode::Smear => DrawMode::Smear,
        PaintMode::Shade => DrawMode::Shade,
        PaintMode::Blend => DrawMode::Blend,
        PaintMode::Cycle => DrawMode::Cycle,
        PaintMode::Smooth => DrawMode::Smooth,
        PaintMode::Mix => DrawMode::Mix,
        PaintMode::HBrite => DrawMode::HBrite,
    }
}

/// The rectangle with the opposite corners `a` and `b` (both inclusive).
fn rect_between(a: (i32, i32), b: (i32, i32)) -> Rect {
    Rect::new(
        a.0.min(b.0),
        a.1.min(b.1),
        (a.0 - b.0).abs() + 1,
        (a.1 - b.1).abs() + 1,
    )
}

/// The pixel at (`x`, `y`) of `img` in its complementary colour (XOR style: with an odd
/// number of colours it always differs), if it is inside.
fn flip_pixel(img: &mut IndexedImage, x: i32, y: i32, colors: usize) {
    let top = (colors.clamp(1, 256) - 1) as u8;
    if let Some(p) = img.get(x, y) {
        img.set(x, y, top.saturating_sub(p));
    }
}

/// The Brush Selector's frame: the outline of `r` in the complementary colour of each pixel
/// of `img` (XOR style, visible on any picture), clipped to `img`. `img` is the picture as
/// it shows (ADR-44: all visible layers), so the frame differs from what lies under it.
fn draw_frame(img: &mut IndexedImage, r: Rect, colors: usize) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut flip = |x: i32, y: i32| flip_pixel(img, x, y, colors);
    let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
    let (cx0, cx1) = (r.x.max(0), x1.min(w - 1));
    let (cy0, cy1) = (r.y.max(0), y1.min(h - 1));
    for x in cx0..=cx1 {
        flip(x, r.y);
        if y1 != r.y {
            flip(x, y1);
        }
    }
    for y in (cy0..=cy1).filter(|&y| y != r.y && y != y1) {
        flip(r.x, y);
        if x1 != r.x {
            flip(x1, y);
        }
    }
}

/// The transparent colour of a brush picked up as `img`: the BG colour, or with Prefs ▸
/// AutoTransp the colour all four corners share (manual: "else BG").
pub(crate) fn pick_transparent(img: &IndexedImage, prefs: Prefs, bg: u8) -> u8 {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let corners = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)].map(|(x, y)| img.get(x, y));
    match corners[0] {
        Some(c) if prefs.get(Pref::AutoTransp) && corners.iter().all(|&k| k == Some(c)) => c,
        _ => bg,
    }
}

impl Stroke {
    /// Spacing other than Continuous applies to this gesture.
    fn spaced(&self) -> bool {
        self.spacing.mode != SpacingMode::Continuous
            && matches!(
                self.kind,
                Kind::Freehand { .. }
                    | Kind::Line { .. }
                    | Kind::Curve { .. }
                    | Kind::Rect { .. }
                    | Kind::Circle { .. }
                    | Kind::Ellipse { .. }
                    | Kind::Polygon { .. }
            )
            && !(self.filled && !matches!(self.kind, Kind::Freehand { .. }))
    }

    /// The path of an outline gesture for [`spaced_points`], and whether it is closed.
    fn outline_path(&self, commit: bool) -> Option<(Vec<(i32, i32)>, bool)> {
        Some(match &self.kind {
            Kind::Freehand { points, .. } => (points.clone(), false),
            Kind::Line { from, to }
            | Kind::Curve {
                from,
                to,
                through: None,
            } => (vec![*from, *to], false),
            Kind::Curve {
                from,
                to,
                through: Some(t),
            } => (raster::curve_points(*from, *t, *to), false),
            Kind::Rect { from, to } => (vec![*from, (to.0, from.1), *to, (from.0, to.1)], true),
            Kind::Circle { center, to, aspect } => {
                let (rx, ry) = circle_radius(*center, *to, *aspect);
                (raster::ellipse_points(*center, rx, ry), true)
            }
            Kind::Ellipse { center, to } => (
                raster::ellipse_points(*center, (to.0 - center.0).abs(), (to.1 - center.1).abs()),
                true,
            ),
            Kind::Polygon {
                points,
                cursor,
                closed,
            } => {
                let mut pts = points.clone();
                if !commit && let Some(c) = cursor {
                    pts.push(*c);
                }
                (pts, *closed)
            }
            _ => return None,
        })
    }

    fn shape(&self, shape: Shape, filled: bool) -> Command {
        let solid = filled && !matches!(shape, Shape::Dots { .. });
        let ink = &self.ink;
        Command::Shape {
            shape,
            brush: if solid || ink.custom.is_some() {
                Vec::new()
            } else {
                ink.brush.clone()
            },
            color: ink.color,
            filled: solid,
            symmetry: self.symmetry,
            custom: if solid { None } else { ink.custom.clone() },
            mode: ink.mode,
            range: ink.range,
            stencil: ink.stencil.clone(),
            right: ink.right,
            fx: ink.fx,
        }
    }

    /// Freehand points as a command: a plain [`Command::Stroke`] without symmetry.
    fn freehand(&self, points: Vec<(i32, i32)>, continuous: bool) -> Command {
        let ink = &self.ink;
        match (self.symmetry, continuous) {
            (None, _) => Command::Stroke {
                brush: if ink.custom.is_some() {
                    Vec::new()
                } else {
                    ink.brush.clone()
                },
                color: ink.color,
                points,
                continuous,
                custom: ink.custom.clone(),
                mode: ink.mode,
                range: ink.range,
                stencil: ink.stencil.clone(),
                right: ink.right,
                fx: ink.fx,
            },
            (Some(_), true) => self.shape(
                Shape::Polygon {
                    points,
                    closed: false,
                },
                false,
            ),
            (Some(_), false) => self.shape(Shape::Dots { points }, false),
        }
    }

    /// A 1-pixel outline in plain colour (feedback for filled freehand and polygons while they
    /// are drawn).
    fn outline(&self, points: Vec<(i32, i32)>) -> Command {
        Command::Stroke {
            brush: vec![(0, 0)],
            color: self.ink.color,
            points,
            continuous: true,
            custom: None,
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    /// The command this gesture stands for: as shown while drawing, or (`commit`) as
    /// committed. They differ only for filled freehand and polygons (outline, then filled)
    /// and the polygon's rubber band (dropped). With an AnimBrush the stamps go through its
    /// cels (ADR-36); under the area stencil (ADR-39).
    fn candidate(&self, commit: bool) -> Option<Command> {
        self.bare_candidate(commit).map(|c| self.ink.stenciled(c))
    }

    /// The command the gesture shows now (ADR-39: Stencil ▸ Paint paints it into the mask
    /// view); `None` for the Brush Selector's frame.
    pub(crate) fn preview_command(&self) -> Option<Command> {
        self.candidate(false)
    }

    /// The picture of `doc` as it shows during this gesture: its visible layers with the
    /// preview in the active layer's place, and over all of them the Brush Selector's frame
    /// or the Place handle mark, each pixel in the complement of what shows there (ADR-44;
    /// on one layer the same as drawing them into the page). Without frame or mark the
    /// preview of a single layer is shown as it is, not copied.
    pub(crate) fn shown<'a>(&'a self, doc: &'a dpx_core::Document) -> Cow<'a, IndexedImage> {
        let shown = doc.composite_with(&self.preview);
        let colors = doc.palette().len();
        match (&self.kind, self.mark) {
            (Kind::Pick { from, to }, _) => {
                let mut img = shown.into_owned();
                draw_frame(&mut img, rect_between(*from, *to), colors);
                Cow::Owned(img)
            }
            (_, Some((x, y))) => {
                let mut img = shown.into_owned();
                flip_pixel(&mut img, x, y, colors);
                Cow::Owned(img)
            }
            _ => shown,
        }
    }

    /// [`Self::candidate`] without the area stencil.
    fn bare_candidate(&self, commit: bool) -> Option<Command> {
        self.plain_candidate(commit)
            .map(|c| self.ink.with_cels(c, 0))
    }

    fn plain_candidate(&self, commit: bool) -> Option<Command> {
        if self.spaced()
            && let Some((path, closed)) = self.outline_path(commit)
        {
            let points = spaced_points(&path, closed, &self.spacing, self.aspect);
            return Some(self.shape(Shape::Dots { points }, false));
        }
        let filled = self.filled;
        Some(match &self.kind {
            Kind::Freehand { points, continuous } => self.freehand(points.clone(), *continuous),
            Kind::Lasso { points } if commit => self.shape(
                Shape::Polygon {
                    points: points.clone(),
                    closed: true,
                },
                true,
            ),
            Kind::Lasso { points } => self.outline(points.clone()),
            Kind::Line { from, to }
            | Kind::Curve {
                from,
                to,
                through: None,
            } => self.shape(
                Shape::Line {
                    from: *from,
                    to: *to,
                },
                false,
            ),
            Kind::Curve {
                from,
                to,
                through: Some(t),
            } => self.shape(
                Shape::Curve {
                    from: *from,
                    to: *to,
                    through: *t,
                },
                false,
            ),
            Kind::Rect { from, to } => self.shape(
                Shape::Rect {
                    from: *from,
                    to: *to,
                },
                filled,
            ),
            Kind::Circle { center, to, aspect } => self.shape(
                Shape::Ellipse {
                    center: *center,
                    radius: circle_radius(*center, *to, *aspect),
                },
                filled,
            ),
            Kind::Ellipse { center, to } => self.shape(
                Shape::Ellipse {
                    center: *center,
                    radius: ((to.0 - center.0).abs(), (to.1 - center.1).abs()),
                },
                filled,
            ),
            Kind::Polygon {
                points,
                cursor,
                closed,
            } => {
                let mut pts = points.clone();
                if !commit && let Some(c) = cursor {
                    pts.push(*c);
                }
                if filled && !commit {
                    self.outline(pts)
                } else {
                    self.shape(
                        Shape::Polygon {
                            points: pts,
                            closed: *closed,
                        },
                        filled,
                    )
                }
            }
            Kind::Airbrush { points, .. } if points.is_empty() => return None,
            Kind::Airbrush { points, .. } => self.shape(
                Shape::Dots {
                    points: points.clone(),
                },
                false,
            ),
            Kind::Text { at, lines } => {
                let points = text_pixels(*at, lines);
                if points.is_empty() {
                    return None;
                }
                // Text is set in the font's pixels, never with the brush.
                Command::Shape {
                    shape: Shape::Dots { points },
                    brush: vec![(0, 0)],
                    color: self.ink.color,
                    filled: false,
                    symmetry: None,
                    custom: None,
                    mode: self.ink.mode,
                    range: self.ink.range,
                    stencil: self.ink.stencil.clone(),
                    right: self.ink.right,
                    // Translucency does not work with Text (manual AGA p.278).
                    fx: Default::default(),
                }
            }
            // The pick-up is no page command; its frame is drawn by `redraw`.
            Kind::Pick { .. } => return None,
            Kind::Hover { .. } if commit => return None,
            Kind::Hover { at } => self.shape(Shape::Dots { points: vec![*at] }, false),
        })
    }

    /// Paints part of the gesture on top of the preview (freehand, airbrush). `page` is the
    /// document page (the page before the gesture). A part the engine would refuse is not
    /// shown, so nothing appears that vanishes on release.
    fn paint_more(&mut self, cmd: &Command, page: &IndexedImage, palette: &Palette) {
        let cmd = &self.ink.stenciled(cmd.clone());
        if cmd.check_paint(page, palette).is_err() {
            return;
        }
        if let Some(r) = cmd.paint_bounds(&self.preview) {
            self.drawn = self.drawn.union(r);
        }
        cmd.paint(&mut self.preview, page, palette);
    }

    /// A new part of a freehand stroke or spray: painted on top, or in the modes that need the
    /// whole stroke ([`Ink::paints_in_parts`]) the whole preview again.
    fn add_part(&mut self, part: &Command, page: &IndexedImage, palette: &Palette) {
        if self.ink.paints_in_parts() && self.ink.cels.is_some() {
            // ADR-36: an AnimBrush part goes on from the stamps painted so far.
            let part = self.ink.with_cels(part.clone(), self.ink.parts);
            self.ink.parts += part.stamps();
            self.paint_more(&part, page, palette);
        } else if self.ink.paints_in_parts() {
            self.paint_more(part, page, palette);
        } else {
            self.redraw(page, palette);
        }
    }

    /// Repaints the preview from the document page and the current candidate.
    fn redraw(&mut self, page: &IndexedImage, palette: &Palette) {
        if !self.drawn.is_empty() {
            self.preview
                .blit(&page.crop(self.drawn), self.drawn.x, self.drawn.y);
        }
        self.drawn = Rect::default();
        if let Kind::Pick { .. } = self.kind {
            // The frame goes over the picture as shown, not into the layer ([`Self::shown`]).
            return;
        }
        if let Some(cmd) = self.candidate(false)
            && cmd.check_paint(page, palette).is_ok()
        {
            self.drawn = cmd.paint_bounds(page).unwrap_or_default();
            cmd.paint(&mut self.preview, page, palette);
        }
    }
}

/// The Brush menu gestures (ADR-30).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XformKind {
    Stretch,
    Rotate,
    Shear,
    BendH,
    BendV,
    Place,
}

/// A Brush menu gesture: armed by the menu item, started by a press on the page, ended by
/// the release (one brush change).
#[derive(Clone, Debug)]
pub struct Xform {
    kind: XformKind,
    /// Where the button went down; `None` while the gesture only waits for it.
    origin: Option<(i32, i32)>,
    /// The pointer.
    cur: (i32, i32),
    shift: bool,
    /// The brush that is transformed (for Any Angle: the original orientation).
    base: BrushImage,
}

impl Xform {
    pub(crate) fn kind(&self) -> XformKind {
        self.kind
    }

    /// The same gesture done on brush `b` (another AnimBrush cel, ADR-36).
    pub(crate) fn apply_to(&self, b: &BrushImage, colors: usize) -> BrushImage {
        Xform {
            base: b.clone(),
            ..self.clone()
        }
        .result(colors)
        .0
    }

    /// The new brush and where the top-left of its image lies on the page, for the pointer
    /// at `self.cur` and the button down at `origin`.
    fn result(&self, colors: usize) -> (BrushImage, (i32, i32)) {
        let b = &self.base;
        let Some(o) = self.origin else {
            // Waiting: the brush as it is, its handle (Place) or top-left at the pointer.
            let at = if self.kind == XformKind::Place {
                (self.cur.0 - b.handle.0, self.cur.1 - b.handle.1)
            } else {
                self.cur
            };
            return (b.clone(), at);
        };
        let (dx, dy) = (self.cur.0 - o.0, self.cur.1 - o.1);
        match self.kind {
            XformKind::Stretch => {
                let (mut w, mut h) = (f64::from((dx + 1).max(1)), f64::from((dy + 1).max(1)));
                if self.shift {
                    let s = (w / f64::from(b.width)).max(h / f64::from(b.height));
                    (w, h) = (s * f64::from(b.width), s * f64::from(b.height));
                }
                (brush_ops::stretch(b, w.round() as u32, h.round() as u32), o)
            }
            XformKind::Rotate => {
                // The bottom edge follows the line from the press to the pointer.
                let mut angle = if dx == 0 && dy == 0 {
                    0.0
                } else {
                    f64::from(dy).atan2(f64::from(dx)).to_degrees()
                };
                if self.shift {
                    angle = (angle / 15.0).round() * 15.0;
                }
                let r = brush_ops::rotate_any(b, angle, colors);
                let (ax, ay) = brush_ops::rotated_point(b, angle, (0.0, f64::from(b.height)));
                (r, (o.0 - ax.round() as i32, o.1 - ay.round() as i32))
            }
            XformKind::Shear => (brush_ops::shear(b, dx, colors), (o.0 + dx.min(0), o.1)),
            XformKind::BendH => (
                brush_ops::bend(b, true, dx, dy, colors),
                (o.0 + dx.min(0), o.1),
            ),
            XformKind::BendV => (
                brush_ops::bend(b, false, dy, dx, colors),
                (o.0, o.1 + dy.min(0)),
            ),
            XformKind::Place => (brush_ops::handle_place(b, dx, dy), o),
        }
    }
}

impl PainterApp {
    /// Arms a Brush menu gesture on the custom brush: the brush follows the pointer until the
    /// button goes down.
    pub(crate) fn arm_xform(&mut self, kind: XformKind) {
        let Some(current) = self.custom_brush.clone() else {
            return;
        };
        let base = match (&self.tools.rotate_base, kind) {
            (Some((result, original)), XformKind::Rotate) if *result == current => original.clone(),
            _ => current,
        };
        let cur = self.pointer_page.unwrap_or((0, 0));
        self.tools.xform = Some(Xform {
            kind,
            origin: None,
            cur,
            shift: false,
            base,
        });
        self.xform_preview();
    }

    /// Ends an armed or running Brush menu gesture without changing the brush.
    fn drop_xform(&mut self) {
        if self.tools.xform.take().is_some() {
            self.stroke = None;
            self.dirty = true;
        }
    }

    fn xform_press(&mut self, x: i32, y: i32) {
        let shift = self.tools.shift;
        let Some(g) = &mut self.tools.xform else {
            return;
        };
        if g.origin.is_some() {
            return;
        }
        g.cur = (x, y);
        // Place: the brush stays where it was shown; the pointer then moves the handle.
        g.origin = Some(if g.kind == XformKind::Place {
            (x - g.base.handle.0, y - g.base.handle.1)
        } else {
            (x, y)
        });
        g.shift = shift;
        self.xform_preview();
    }

    fn xform_move(&mut self, x: i32, y: i32) {
        let shift = self.tools.shift;
        let Some(g) = &mut self.tools.xform else {
            return;
        };
        if g.cur == (x, y) && g.shift == shift {
            return;
        }
        g.cur = (x, y);
        g.shift = shift;
        self.xform_preview();
    }

    /// The button went up: the gesture's brush becomes the custom brush. A click without a
    /// drag changes nothing, except for Place.
    fn xform_commit(&mut self) {
        let Some(g) = self.tools.xform.take() else {
            return;
        };
        self.stroke = None;
        self.dirty = true;
        if g.origin == Some(g.cur) && g.kind != XformKind::Place {
            return;
        }
        let colors = self.engine.document().palette().len();
        // ADR-36: on an AnimBrush the gesture changes every cel.
        if self.animbrush_xform(&g, colors) {
            return;
        }
        let (brush, _) = g.result(colors);
        if g.kind == XformKind::Rotate {
            self.tools.rotate_base = Some((brush.clone(), g.base));
        }
        self.custom_brush = Some(brush);
    }

    /// Shows the gesture's brush in the page preview, in its own colours.
    fn xform_preview(&mut self) {
        let Some(g) = &self.tools.xform else {
            return;
        };
        let doc = self.engine.document();
        let (mut brush, at) = g.result(doc.palette().len());
        let handle = brush.handle;
        let mark = g.kind == XformKind::Place;
        brush.handle = (0, 0);
        let mut ink = self.ink(false);
        ink.custom = Some(brush);
        ink.mode = DrawMode::Matte;
        ink.range = None;
        ink.stencil = None;
        ink.area = None;
        let mut s = Stroke {
            preview: doc.page().clone(),
            drawn: Rect::default(),
            ink,
            filled: false,
            symmetry: None,
            spacing: self.tools.spacing,
            aspect: (1, 1),
            held: false,
            kind: Kind::Hover { at },
            mark: None,
        };
        s.redraw(doc.page(), doc.palette());
        if mark {
            // The handle: its pixel in the complementary colour of what shows there.
            s.mark = Some((at.0 + handle.0, at.1 + handle.1));
        }
        self.stroke = Some(s);
        self.dirty = true;
    }
}

/// Where the halves of the Magnify screen are, in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MagnifyLayout {
    /// The normal view ends (and the divider starts) here.
    pub split: i32,
    /// The magnified view starts here …
    pub start: i32,
    /// … and ends here (the toolbox or the screen edge).
    pub end: i32,
    pub zoom: i32,
    /// The page pixels shown magnified.
    pub source: Rect,
}

impl PainterApp {
    /// Be Square: the displayed shape of a page pixel, else square.
    fn aspect(&self) -> (i32, i32) {
        if self.ui.prefs.get(Pref::BeSquare) {
            let (fx, fy) = self.engine.document().target().mode.pixel_shape();
            (fx as i32, fy as i32)
        } else {
            (1, 1)
        }
    }

    /// Grid: tools that obey it (as listed in the manual) snap to the nearest grid point.
    pub(crate) fn snap(&self, p: (i32, i32)) -> (i32, i32) {
        let obeys = matches!(
            self.ui.tool,
            Tool::DottedFreehand
                | Tool::Line
                | Tool::Rectangle
                | Tool::Circle
                | Tool::Ellipse
                | Tool::BrushSelect
                | Tool::Text
        );
        if self.tools.grid && obeys {
            let GridSettings { x: gx, y: gy } = self.tools.grid_size;
            (
                (p.0 + gx / 2).div_euclid(gx) * gx,
                (p.1 + gy / 2).div_euclid(gy) * gy,
            )
        } else {
            p
        }
    }

    /// The Fill tool's fill: solid, or a gradient through the colour range that holds FG
    /// (ranges of the picture), else from FG to BG (ADR-20).
    pub(crate) fn fill_type(&self) -> FillType {
        let Some(direction) = self.tools.fill.gradient else {
            return FillType::Solid;
        };
        let colors = self.palette().len();
        let ranges: Vec<(u8, u8)> = self
            .extras
            .ranges
            .iter()
            .filter(|r| usize::from(r.high) < colors)
            .map(|r| (r.low, r.high))
            .collect();
        let (from, to) = range_for(&ranges, self.ui.fg, self.ui.bg);
        FillType::Gradient {
            direction,
            from,
            to,
            dither: self.tools.fill.dither,
        }
    }

    /// Symmetry for a new gesture: around the centre of the screen.
    pub(crate) fn symmetry(&self) -> Option<Symmetry> {
        if !self.tools.symmetry
            || matches!(self.ui.tool, Tool::Text | Tool::Fill | Tool::BrushSelect)
        {
            return None;
        }
        let v = self.view();
        let (fx, fy) = self.aspect();
        Some(Symmetry {
            center: (
                self.scroll.0 + v.screen_w / 2,
                self.scroll.1 + v.screen_h / 2,
            ),
            order: self.tools.symmetry_settings.order,
            aspect: (fx as u8, fy as u8),
            mirror: self.tools.symmetry_settings.mirror,
        })
    }

    /// The colour range (CRNG, loaded with the picture or a palette) that holds the FG colour,
    /// for Shade and Cycle; ranges outside the palette are skipped.
    fn fg_range(&self) -> Option<(u8, u8)> {
        let colors = self.engine.document().palette().len();
        let fg = self.ui.fg;
        self.extras
            .ranges
            .iter()
            .find(|r| r.low <= fg && fg <= r.high && usize::from(r.high) < colors)
            .map(|r| (r.low, r.high))
    }

    /// What a new gesture paints with: FG (BG with `right`), the custom brush if there is one
    /// (else the built-in brush), the Mode menu's mode, the range holding FG and the stencil.
    /// A custom brush with colours the palette no longer has (after a Screen Format change)
    /// is not used.
    fn ink(&self, right: bool) -> Ink {
        let colors = self.engine.document().palette().len();
        let fits = |c: u8| usize::from(c) < colors;
        Ink {
            color: if right { self.ui.bg } else { self.ui.fg },
            brush: toolbox::BUILTIN_BRUSHES[self.ui.brush].footprint(),
            custom: self
                .custom_brush
                .clone()
                .filter(|b| b.pixels.iter().all(|&c| fits(c)) && b.transparent.is_none_or(fits)),
            mode: draw_mode(self.ui.mode),
            range: self.fg_range(),
            stencil: self.stencil.clone(),
            right,
            fx: self.menu_data.effects.fx(),
            cels: None,
            parts: 0,
            area: self.active_area().cloned(),
        }
    }

    /// One stamp of `brush` (its handle at the top-left) at `at`, painted like the tools
    /// paint: the Mode menu's mode (Matte with `matte`), the range holding FG, the stencil and
    /// the effects, in FG or with `right` in BG (Effect > Perspective, ADR-37).
    pub(crate) fn brush_stamp(
        &self,
        brush: BrushImage,
        at: (i32, i32),
        right: bool,
        matte: bool,
    ) -> Command {
        let ink = self.ink(right);
        Command::Stroke {
            brush: Vec::new(),
            color: ink.color,
            points: vec![at],
            continuous: false,
            custom: Some(brush),
            mode: if matte { DrawMode::Matte } else { ink.mode },
            range: ink.range,
            stencil: ink.stencil,
            right,
            fx: ink.fx,
        }
    }

    /// A Brush menu gesture (ADR-30) is armed or running (Effect > Perspective, ADR-37).
    pub(crate) fn xform_armed(&self) -> bool {
        self.tools.xform.is_some()
    }

    /// A gesture is under way (not just the custom brush following the pointer), or a Brush
    /// menu gesture is armed.
    pub(crate) fn gesture_in_progress(&self) -> bool {
        self.tools.xform.is_some()
            || self
                .stroke
                .as_ref()
                .is_some_and(|s| !matches!(s.kind, Kind::Hover { .. }))
    }

    /// Whether the selected tool stamps the brush (outlines, freehand, airbrush).
    fn stamps_brush(&self) -> bool {
        match self.ui.tool {
            Tool::DottedFreehand | Tool::Line | Tool::Curve | Tool::Airbrush => true,
            Tool::ContinuousFreehand
            | Tool::Rectangle
            | Tool::Circle
            | Tool::Ellipse
            | Tool::Polygon => !self.ui.filled,
            _ => false,
        }
    }

    /// The custom brush at the pointer (`at`; `None` = off the page) while a tool that stamps
    /// it is selected and no gesture is in progress. Shown in the preview, never committed.
    pub(crate) fn hover_brush(&mut self, at: Option<(i32, i32)>) {
        if self.tools.xform.is_some() || self.perspective.busy() {
            return;
        }
        let idle = self.stroke.is_none() && self.custom_brush.is_none();
        if idle
            || self
                .stroke
                .as_ref()
                .is_some_and(|s| !matches!(s.kind, Kind::Hover { .. }))
        {
            return;
        }
        let ink = self.ink(false);
        let at = at
            .filter(|_| ink.custom.is_some() && self.stamps_brush())
            .map(|p| self.snap(p));
        let Some(at) = at else {
            if self.stroke.take().is_some() {
                self.dirty = true;
            }
            return;
        };
        let doc = self.engine.document();
        if let Some(s) = &mut self.stroke {
            if matches!(s.kind, Kind::Hover { at: a } if a == at) && s.ink == ink {
                return;
            }
            s.kind = Kind::Hover { at };
            s.ink = ink;
            s.mark = None;
            s.redraw(doc.page(), doc.palette());
        } else {
            let mut s = Stroke {
                preview: doc.page().clone(),
                drawn: Rect::default(),
                ink,
                filled: false,
                symmetry: None,
                spacing: self.tools.spacing,
                aspect: (1, 1),
                held: false,
                kind: Kind::Hover { at },
                mark: None,
            };
            s.redraw(doc.page(), doc.palette());
            self.stroke = Some(s);
        }
        self.dirty = true;
    }

    /// The Brush Selector was released: the framed part of the page becomes the custom brush
    /// (BG colour transparent, handle in the centre) and the mode Matte. A right-button drag
    /// cuts: the part is filled with BG in one undoable command. Then the tool used before
    /// the Brush Selector comes back.
    fn pick_up(&mut self) {
        let Some(Stroke {
            kind: Kind::Pick { from, to },
            ink,
            ..
        }) = self.stroke.take()
        else {
            return;
        };
        self.dirty = true;
        let page = self.engine.document().page();
        let mut r = rect_between(from, to);
        // ExclBrush: with the grid on, leave out the right and bottom border.
        if self.tools.grid && self.ui.prefs.get(Pref::ExclBrush) {
            r.w = (r.w - 1).max(1);
            r.h = (r.h - 1).max(1);
        }
        let r = r.intersect(page.bounds());
        if r.is_empty() {
            return;
        }
        // ADR-36: Alt, or Anim ▸ AnimBrush ▸ Pick up.., takes the frame for an AnimBrush.
        if self.animbrush_take_pick(r) {
            let back = self.tools.pick_return.take();
            let (tool, filled) = back.unwrap_or((UiState::default().tool, false));
            self.select_tool(tool, filled);
            return;
        }
        let img = self.engine.document().page().crop(r);
        let transparent = pick_transparent(&img, self.ui.prefs, self.ui.bg);
        let brush = BrushImage::from_image(&img, Some(transparent));
        if ink.right {
            self.execute(Command::Shape {
                shape: Shape::Rect {
                    from: (r.x, r.y),
                    to: (r.x + r.w - 1, r.y + r.h - 1),
                },
                brush: Vec::new(),
                color: self.ui.bg,
                filled: true,
                symmetry: None,
                custom: None,
                mode: DrawMode::Matte,
                range: None,
                stencil: None,
                right: true,
                fx: Default::default(),
            });
        }
        self.custom_brush = Some(brush.clone());
        self.picked_brush = Some(brush);
        self.tools.brush_gen += 1;
        self.leave_anim_brush(); // ADR-36
        self.tools.brush_palette = Some(self.engine.document().palette().clone());
        self.ui.mode = PaintMode::Matte;
        let (tool, filled) = self
            .tools
            .pick_return
            .take()
            .unwrap_or((UiState::default().tool, false));
        self.select_tool(tool, filled);
    }

    /// `B` (Brush ▸ Restore): the brush last picked up becomes the brush again.
    pub(crate) fn restore_custom_brush(&mut self) {
        // ADR-36: B brings back the AnimBrush when it was made after the last brush.
        if self.animbrush_restore() {
            return;
        }
        if let Some(b) = &self.picked_brush {
            self.custom_brush = Some(b.clone());
            self.leave_anim_brush();
            self.dirty = true;
        }
    }

    /// Right-click on the Brush Selector: back to the last custom brush, or (while it is the
    /// brush) to the built-in brush.
    pub(crate) fn toggle_custom_brush(&mut self) {
        // ADR-36: with an AnimBrush it toggles between the AnimBrush and the custom brush.
        if self.animbrush_toggle() {
            return;
        }
        if self.custom_brush.take().is_none() {
            self.restore_custom_brush();
        }
        self.dirty = true;
    }

    /// A built-in brush was clicked: it replaces the custom brush.
    pub(crate) fn select_builtin_brush(&mut self, i: usize) {
        self.ui.brush = i.min(toolbox::BUILTIN_BRUSHES.len() - 1);
        self.custom_brush = None;
        self.leave_anim_brush(); // ADR-36
        self.dirty = true;
    }

    /// A press on the page (`right` = BG colour).
    pub(crate) fn press_page(&mut self, x: i32, y: i32, right: bool) {
        if self.tools.xform.is_some() {
            return self.xform_press(x, y);
        }
        // A pending curve bend or polygon takes the click.
        if let Some(s) = &mut self.stroke
            && !s.held
        {
            match &mut s.kind {
                Kind::Curve {
                    through: Some(t), ..
                } => {
                    *t = (x, y);
                    self.finish_stroke();
                    return;
                }
                Kind::Polygon {
                    points,
                    cursor,
                    closed,
                } => {
                    if points.len() >= 2 && cheb(points[0], (x, y)) <= CLOSE_DISTANCE {
                        *closed = true;
                        self.finish_stroke();
                        return;
                    }
                    if points.len() < MAX_SHAPE_POINTS {
                        points.push((x, y));
                    }
                    *cursor = None;
                    s.held = true;
                    let doc = self.engine.document();
                    s.redraw(doc.page(), doc.palette());
                    self.dirty = true;
                    return;
                }
                _ => {}
            }
        }
        self.finish_stroke();
        let color = if right { self.ui.bg } else { self.ui.fg };
        let p = self.snap((x, y));
        let kind = match self.ui.tool {
            Tool::DottedFreehand => Kind::Freehand {
                points: vec![p],
                continuous: false,
            },
            Tool::ContinuousFreehand if self.ui.filled => Kind::Lasso { points: vec![p] },
            Tool::ContinuousFreehand => Kind::Freehand {
                points: vec![p],
                continuous: true,
            },
            Tool::Line => Kind::Line { from: p, to: p },
            Tool::Curve => Kind::Curve {
                from: p,
                to: p,
                through: None,
            },
            Tool::Rectangle => Kind::Rect { from: p, to: p },
            Tool::Circle => Kind::Circle {
                center: p,
                to: p,
                aspect: self.aspect(),
            },
            Tool::Ellipse => Kind::Ellipse { center: p, to: p },
            Tool::Polygon => Kind::Polygon {
                points: vec![p],
                cursor: None,
                closed: false,
            },
            Tool::Airbrush => Kind::Airbrush {
                points: Vec::new(),
                at: p,
            },
            Tool::Text => Kind::Text {
                at: p,
                lines: vec![String::new()],
            },
            Tool::BrushSelect => Kind::Pick { from: p, to: p },
            Tool::Fill => {
                let fill = self.fill_type();
                self.run(Command::FloodFill { x, y, color, fill });
                return;
            }
            _ => return,
        };
        let doc = self.engine.document();
        let (page, palette) = (doc.page(), doc.palette());
        let mut s = Stroke {
            preview: page.clone(),
            drawn: Rect::default(),
            ink: self.ink(right),
            filled: self.ui.filled && self.ui.tool.is_split(),
            symmetry: self.symmetry(),
            spacing: self.tools.spacing,
            aspect: self.aspect(),
            held: true,
            kind,
            mark: None,
        };
        s.ink.cels = self.animbrush_cels();
        match &s.kind {
            Kind::Freehand { continuous, .. } if !s.spaced() => {
                let continuous = *continuous;
                let first = s.freehand(vec![p], continuous);
                s.add_part(&first, page, palette);
                if continuous {
                    // ADR-36: the first line stamps this point again, from the same cel.
                    s.ink.parts = 0;
                }
            }
            Kind::Lasso { .. } => {
                let first = s.outline(vec![p]);
                s.paint_more(&first, page, palette);
            }
            _ => s.redraw(page, palette),
        }
        self.stroke = Some(s);
        if self.ui.tool == Tool::Airbrush {
            self.spray(SPRAYS_PER_FRAME);
        }
        self.dirty = true;
    }

    /// The pointer moved to page pixel (`x`, `y`) with a button held.
    pub(crate) fn handle_drag(&mut self, x: i32, y: i32) {
        if self.tools.xform.is_some() {
            return self.xform_move(x, y);
        }
        if self.perspective.busy() {
            return self.perspective_move(x, y);
        }
        let p = self.snap((x, y));
        let aspect = self.aspect();
        let shift = self.tools.shift;
        let doc = self.engine.document();
        let (page, palette) = (doc.page(), doc.palette());
        let Some(s) = &mut self.stroke else {
            return;
        };
        if !s.held {
            return;
        }
        match &mut s.kind {
            Kind::Freehand { points, continuous } => {
                let last = *points.last().expect("a stroke starts with one point");
                if last == p || points.len() >= MAX_SHAPE_POINTS {
                    return;
                }
                points.push(p);
                let continuous = *continuous;
                if s.spaced() {
                    // Stamps depend on the whole path: draw it again.
                    s.redraw(page, palette);
                } else {
                    let part = if continuous { vec![last, p] } else { vec![p] };
                    let cmd = s.freehand(part, continuous);
                    s.add_part(&cmd, page, palette);
                }
            }
            Kind::Lasso { points } => {
                let last = *points.last().expect("a stroke starts with one point");
                if last == p || points.len() >= MAX_SHAPE_POINTS {
                    return;
                }
                points.push(p);
                let cmd = s.outline(vec![last, p]);
                s.paint_more(&cmd, page, palette);
            }
            Kind::Line { from, to }
            | Kind::Curve {
                from,
                to,
                through: None,
            } => {
                *to = if shift {
                    constrain_hv(*from, p, aspect)
                } else {
                    p
                };
                s.redraw(page, palette);
            }
            Kind::Rect { from, to } | Kind::Pick { from, to } => {
                *to = if shift { square(*from, p, aspect) } else { p };
                s.redraw(page, palette);
            }
            Kind::Circle { to, .. } | Kind::Ellipse { to, .. } => {
                *to = p;
                s.redraw(page, palette);
            }
            Kind::Polygon { cursor, .. } => {
                *cursor = Some((x, y));
                s.redraw(page, palette);
            }
            Kind::Airbrush { at, .. } => *at = p,
            Kind::Curve { .. } | Kind::Text { .. } | Kind::Hover { .. } => return,
        }
        self.dirty = true;
    }

    /// The pointer moved over the page with no button held: the curve's bend and the
    /// polygon's rubber band follow it, else a custom brush does ([`Self::hover_brush`]).
    pub(crate) fn handle_hover(&mut self, x: i32, y: i32) {
        if self.tools.xform.is_some() {
            return self.xform_move(x, y);
        }
        if self.perspective.busy() {
            return self.perspective_move(x, y);
        }
        if self
            .stroke
            .as_ref()
            .is_none_or(|s| matches!(s.kind, Kind::Hover { .. }))
        {
            self.hover_brush(Some((x, y)));
            return;
        }
        let doc = self.engine.document();
        let (page, palette) = (doc.page(), doc.palette());
        let Some(s) = &mut self.stroke else {
            return;
        };
        if s.held {
            return;
        }
        match &mut s.kind {
            Kind::Curve {
                through: Some(t), ..
            } if *t != (x, y) => *t = (x, y),
            Kind::Polygon { cursor, .. } if *cursor != Some((x, y)) => *cursor = Some((x, y)),
            _ => return,
        }
        s.redraw(page, palette);
        self.dirty = true;
    }

    /// No button is held (any more): a held gesture ends. Curves go on to their bend,
    /// polygons to the next corner, text to typing, the Brush Selector picks up; everything
    /// else is committed.
    pub(crate) fn release(&mut self) {
        if self
            .tools
            .xform
            .as_ref()
            .is_some_and(|x| x.origin.is_some())
        {
            return self.xform_commit();
        }
        let doc = self.engine.document();
        let (page, palette) = (doc.page(), doc.palette());
        let Some(s) = &mut self.stroke else {
            return;
        };
        if !s.held {
            return;
        }
        s.held = false;
        match &mut s.kind {
            Kind::Curve {
                from,
                to,
                through: through @ None,
            } => {
                if from == to {
                    // A click without a drag draws nothing.
                    self.stroke = None;
                } else {
                    *through = Some(((from.0 + to.0) / 2, (from.1 + to.1) / 2));
                    s.redraw(page, palette);
                }
            }
            Kind::Polygon { points, cursor, .. } => {
                if let Some(c) = cursor.take()
                    && points.last() != Some(&c)
                    && points.len() < MAX_SHAPE_POINTS
                {
                    points.push(c);
                }
                s.redraw(page, palette);
            }
            Kind::Text { .. } => {}
            Kind::Pick { .. } => self.pick_up(),
            _ => self.finish_stroke(),
        }
        self.dirty = true;
    }

    /// Commits the gesture in progress as one command (what it shows, without a polygon's
    /// rubber band).
    pub(crate) fn finish_stroke(&mut self) {
        self.drop_xform();
        if let Some(s) = self.stroke.take() {
            if let Some(cmd) = s.candidate(true) {
                self.execute_painted(cmd);
            }
            self.dirty = true;
        }
    }

    /// Drops the gesture in progress.
    fn cancel_stroke(&mut self) {
        self.drop_xform();
        if self.stroke.take().is_some() {
            self.dirty = true;
        }
    }

    /// Sprays `n` brush stamps at random points around the airbrush.
    pub(crate) fn spray(&mut self, n: usize) {
        let (fx, fy) = self.aspect();
        let Some(Stroke {
            kind: Kind::Airbrush { at, points },
            ..
        }) = &self.stroke
        else {
            return;
        };
        if points.len() + n > MAX_SHAPE_POINTS {
            return;
        }
        let at = *at;
        let radius = self.tools.spacing.airbrush_radius;
        let (rx, ry) = (radius, (radius * fx / fy).max(1));
        let mut new = Vec::with_capacity(n);
        while new.len() < n {
            let (dx, dy) = (self.tools.random(rx), self.tools.random(ry));
            if dx * dx * ry * ry + dy * dy * rx * rx <= rx * rx * ry * ry {
                new.push((at.0 + dx, at.1 + dy));
            }
        }
        let doc = self.engine.document();
        let Some(s) = &mut self.stroke else {
            return;
        };
        if let Kind::Airbrush { points, .. } = &mut s.kind {
            points.extend_from_slice(&new);
        }
        let cmd = s.shape(Shape::Dots { points: new }, false);
        s.add_part(&cmd, doc.page(), doc.palette());
        self.dirty = true;
    }

    /// Once per frame: the airbrush keeps spraying while its button is held.
    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
        if let Some(Stroke {
            held: true,
            kind: Kind::Airbrush { .. },
            ..
        }) = &self.stroke
        {
            self.spray(SPRAYS_PER_FRAME);
            ctx.request_repaint();
        }
    }

    /// A toolbox button (or its key).
    pub(crate) fn select_tool(&mut self, tool: Tool, filled_half: bool) {
        match tool {
            Tool::Undo => self.undo(),
            Tool::Clear => self.run(Command::Clear { color: self.ui.bg }),
            Tool::Grid => self.tools.grid = !self.tools.grid,
            Tool::Symmetry => {
                self.finish_stroke();
                self.tools.symmetry = !self.tools.symmetry;
            }
            Tool::Magnify => self.toggle_magnify(None),
            Tool::Zoom => self.zoom(1),
            _ => {
                self.finish_stroke();
                if tool == Tool::BrushSelect && self.ui.tool != Tool::BrushSelect {
                    self.tools.pick_return = Some((self.ui.tool, self.ui.filled));
                }
                self.ui.tool = tool;
                self.ui.filled = filled_half && tool.is_split();
            }
        }
        self.dirty = true;
    }

    /// Magnify on (centred on `at`, else on the screen's centre) or off.
    pub(crate) fn toggle_magnify(&mut self, at: Option<(i32, i32)>) {
        if self.tools.magnify.take().is_none() {
            let v = self.view();
            let c = at.unwrap_or((
                self.scroll.0 + v.screen_w / 2,
                self.scroll.1 + v.screen_h / 2,
            ));
            self.magnify_at(c);
        }
        self.dirty = true;
    }

    fn magnify_at(&mut self, (x, y): (i32, i32)) {
        let page = self.engine.document().page();
        let c = (
            x.clamp(0, page.width() as i32 - 1),
            y.clamp(0, page.height() as i32 - 1),
        );
        self.tools.magnify = Some(c);
        // The normal half follows, so the magnified part stays in view.
        if let Some(m) = self.magnify_layout(self.view()) {
            let s = m.source;
            let (vw, vh) = (m.split, self.view().screen_h);
            let seen = Rect::new(self.scroll.0, self.scroll.1, vw, vh);
            if s.intersect(seen) != s.intersect(page.bounds()) {
                self.scroll_to(c.0 - vw / 2, c.1 - vh / 2);
            }
        }
        self.dirty = true;
    }

    /// Zoom: a bigger (`+1`) or smaller (`-1`) magnify factor.
    pub(crate) fn zoom(&mut self, step: i32) {
        let z = (self.tools.zoom as i32 + step).clamp(0, ZOOMS.len() as i32 - 1);
        self.tools.zoom = z as usize;
        if let Some(c) = self.tools.magnify {
            self.magnify_at(c);
        }
        self.dirty = true;
    }

    /// The halves of the screen while Magnify is on.
    pub(crate) fn magnify_layout(&self, view: View) -> Option<MagnifyLayout> {
        let (cx, cy) = self.tools.magnify?;
        let toolbox = if self.ui.toolbox_visible {
            dpx_ui::screen::geom::TOOLBOX_W * view.ux
        } else {
            0
        };
        let end = view.screen_w - toolbox;
        let split = end / 3 / view.ux * view.ux;
        let start = split + DIVIDER * view.ux;
        let zoom = ZOOMS[self.tools.zoom];
        let (w, h) = (
            (end - start + zoom - 1) / zoom,
            (view.screen_h + zoom - 1) / zoom,
        );
        let page = self.engine.document().page();
        let x = (cx - w / 2).clamp(0, (page.width() as i32 - w).max(0));
        let y = (cy - h / 2).clamp(0, (page.height() as i32 - h).max(0));
        Some(MagnifyLayout {
            split,
            start,
            end,
            zoom,
            source: Rect::new(x, y, w, h),
        })
    }

    /// The page pixel under screen pixel (`sx`, `sy`); `None` on the Magnify divider.
    pub(crate) fn screen_to_page(&self, view: View, (sx, sy): (i32, i32)) -> Option<(i32, i32)> {
        match self.magnify_layout(view) {
            Some(m) if (m.split..m.start).contains(&sx) => None,
            Some(m) if (m.start..m.end).contains(&sx) => Some((
                m.source.x + (sx - m.start) / m.zoom,
                m.source.y + sy / m.zoom,
            )),
            _ => Some((sx + self.scroll.0, sy + self.scroll.1)),
        }
    }

    /// The screen as shown: the page (normal or split for Magnify) with the text cursor, the
    /// interface over them (the tooltip last, ADR-41) and the toolbox toggles that are on.
    pub(crate) fn compose_screen(&self) -> IndexedImage {
        let view = self.view();
        let palette = self.palette();
        let pens = Pens::from_palette(palette);
        let (w, h) = (view.screen_w, view.screen_h);
        let magnify = self.magnify_layout(view);
        // The picture as shown (all visible layers, ADR-44), made once for both halves.
        let page = self.page();
        let mut shown = match magnify {
            None => page.crop(Rect::new(self.scroll.0, self.scroll.1, w, h)),
            Some(m) => {
                let mut shown = IndexedImage::new(w as u32, h as u32, pens.dark);
                shown.blit(
                    &page.crop(Rect::new(self.scroll.0, self.scroll.1, m.split, h)),
                    0,
                    0,
                );
                shown.fill_rect(m.split, 0, m.start - m.split, h, pens.face);
                shown.fill_rect(m.split, 0, view.ux, h, pens.shine);
                shown.fill_rect(m.start - view.ux, 0, view.ux, h, pens.dark);
                let big = raster::magnified(&page, m.source, m.zoom, pens.dark);
                shown.blit(&big.crop(Rect::new(0, 0, m.end - m.start, h)), m.start, 0);
                shown
            }
        };
        // The text cursor belongs to the page, like the text it follows: the interface lies
        // over it (toolbox, menus, requesters and the tooltip, ADR-41).
        if let Some(Stroke {
            kind: Kind::Text { at, lines },
            ink: Ink { color, .. },
            ..
        }) = &self.stroke
        {
            let last = lines.last().map_or("", String::as_str);
            let x = at.0 + Font::builtin().text_width(last) + 1;
            let y = at.1 + (lines.len() as i32 - 1) * TEXT_LINE;
            // Normal view only; in the magnified half the typed text itself shows.
            let (sx, sy) = (x - self.scroll.0, y - self.scroll.1);
            let limit = magnify.map_or(w, |m| m.split);
            if (0..limit).contains(&sx) {
                shown.fill_rect(sx, sy, 1, font::GLYPH_H, *color);
            }
        }
        let mut screen = view
            .ui_screen()
            .render_scaled(&shown, palette, &self.ui, view.ux, view.uy);
        if self.ui.toolbox_visible {
            let x0 = view.ui_screen().toolbox_x();
            let on = [
                (Tool::Grid, self.tools.grid),
                (Tool::Symmetry, self.tools.symmetry),
                (Tool::Magnify, self.tools.magnify.is_some()),
            ];
            // ADR-42: the Import requester covers the toolbox; the marks stay under it.
            crate::import::under_import_requester(
                &mut screen,
                &self.ui,
                (view.ux, view.uy),
                |screen| {
                    for (tool, _) in on.iter().filter(|(_, on)| *on) {
                        toolbox::draw_toggle_on(screen, x0, *tool, &pens, view.ux, view.uy);
                    }
                    // While a custom brush is the brush, no built-in brush is selected.
                    if self.custom_brush.is_some() {
                        toolbox::draw_brush_unselected(
                            screen,
                            x0,
                            self.ui.brush,
                            &pens,
                            view.ux,
                            view.uy,
                        );
                    }
                },
            );
        }
        screen
    }

    /// Whether the Text tool is taking the keyboard.
    pub(crate) fn typing(&self) -> bool {
        matches!(
            self.stroke,
            Some(Stroke {
                kind: Kind::Text { .. },
                ..
            })
        )
    }

    /// Tool keys from this frame's events. Returns whether the keyboard belongs to the tools
    /// (text typing, Magnify scrolling), so the page scroll keys and `u` must not act.
    pub(crate) fn tool_keys(&mut self, ctx: &egui::Context) -> bool {
        let keys: Vec<ToolKey> = ctx.input(|i| {
            let mut keys = Vec::new();
            for event in &i.events {
                match event {
                    egui::Event::Text(text)
                        if !(i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt) =>
                    {
                        keys.extend(text.chars().map(ToolKey::Char));
                    }
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => keys.extend(match key {
                        egui::Key::Space => Some(ToolKey::Space),
                        egui::Key::Enter => Some(ToolKey::Enter),
                        egui::Key::Escape => Some(ToolKey::Escape),
                        egui::Key::Backspace => Some(ToolKey::Backspace),
                        egui::Key::ArrowLeft => Some(ToolKey::Arrow(-1, 0)),
                        egui::Key::ArrowRight => Some(ToolKey::Arrow(1, 0)),
                        egui::Key::ArrowUp => Some(ToolKey::Arrow(0, -1)),
                        egui::Key::ArrowDown => Some(ToolKey::Arrow(0, 1)),
                        egui::Key::N if !i.modifiers.any() => Some(ToolKey::N),
                        _ => None,
                    }),
                    _ => {}
                }
            }
            keys
        });
        let typing = self.typing();
        for key in keys {
            self.tool_key(key);
        }
        typing || self.typing() || self.tools.magnify.is_some()
    }

    /// One tool key: the toolbox keys (manual keyboard table), polygon and text keys.
    pub(crate) fn tool_key(&mut self, key: ToolKey) {
        if let Some(Stroke {
            kind: Kind::Text { lines, .. },
            ..
        }) = &mut self.stroke
        {
            match key {
                ToolKey::Char(c) if (' '..='~').contains(&c) => {
                    lines.last_mut().expect("text has a line").push(c);
                }
                ToolKey::Space => {}
                ToolKey::Backspace => {
                    let last = lines.last_mut().expect("text has a line");
                    if last.pop().is_none() && lines.len() > 1 {
                        lines.pop();
                    }
                }
                ToolKey::Enter => lines.push(String::new()),
                ToolKey::Escape => {
                    self.finish_stroke();
                    return;
                }
                _ => return,
            }
            let doc = self.engine.document();
            if let Some(s) = &mut self.stroke {
                s.redraw(doc.page(), doc.palette());
            }
            self.dirty = true;
            return;
        }
        let tool = |t, filled| Some((t, filled));
        let pick = match key {
            ToolKey::Char('s') => tool(Tool::DottedFreehand, false),
            ToolKey::Char('d') => tool(Tool::ContinuousFreehand, false),
            ToolKey::Char('D') => tool(Tool::ContinuousFreehand, true),
            ToolKey::Char('v') => tool(Tool::Line, false),
            ToolKey::Char('q') => tool(Tool::Curve, false),
            ToolKey::Char('f') => tool(Tool::Fill, false),
            ToolKey::Char('r') => tool(Tool::Rectangle, false),
            ToolKey::Char('R') => tool(Tool::Rectangle, true),
            ToolKey::Char('c') => tool(Tool::Circle, false),
            ToolKey::Char('C') => tool(Tool::Circle, true),
            ToolKey::Char('e') => tool(Tool::Ellipse, false),
            ToolKey::Char('E') => tool(Tool::Ellipse, true),
            ToolKey::Char('t') => tool(Tool::Text, false),
            ToolKey::Char('b') => tool(Tool::BrushSelect, false),
            ToolKey::Char('g' | 'G') => tool(Tool::Grid, false),
            ToolKey::Char('/') => tool(Tool::Symmetry, false),
            ToolKey::Char('K') => tool(Tool::Clear, false),
            _ => None,
        };
        if let Some((t, filled)) = pick {
            self.select_tool(t, filled);
            return;
        }
        match key {
            // Fill Type and Spacing requesters (manual keyboard table).
            ToolKey::Char('F') => {
                self.open_tool_requester(Tool::Fill);
            }
            ToolKey::Char('V') => {
                self.open_tool_requester(Tool::Line);
            }
            ToolKey::Char('B') => self.restore_custom_brush(),
            ToolKey::Char('m') => self.toggle_magnify(self.pointer_page),
            ToolKey::Char('>') => self.zoom(1),
            ToolKey::Char('<') => self.zoom(-1),
            // Space/Return close a polygon (filled) or let go of its rubber band.
            ToolKey::Space | ToolKey::Enter => {
                if self.stroke.as_ref().is_some_and(|s| !s.held) {
                    self.finish_stroke();
                }
            }
            ToolKey::Escape => self.cancel_stroke(),
            ToolKey::Arrow(dx, dy) => {
                if let (Some(c), Some(m)) = (self.tools.magnify, self.magnify_layout(self.view())) {
                    let step = (m.source.w / 4).max(1);
                    self.magnify_at((c.0 + dx * step, c.1 + dy * step));
                }
            }
            ToolKey::N => {
                if self.tools.magnify.is_some()
                    && let Some(p) = self.pointer_page
                {
                    self.magnify_at(p);
                }
            }
            _ => {}
        }
    }
}

/// `--commands FILE`: executes a JSON array of commands; returns how many.
pub fn run_command_file(engine: &mut Engine, path: &str) -> Result<usize, String> {
    run_commands(engine, path, None)
}

/// `--brush-commands FILE`: like [`run_command_file`], but every stroke and shape outline
/// without a custom brush of its own paints with `brush`.
pub fn run_brush_command_file(
    engine: &mut Engine,
    path: &str,
    brush: &BrushImage,
) -> Result<usize, String> {
    run_commands(engine, path, Some(brush))
}

fn run_commands(
    engine: &mut Engine,
    path: &str,
    brush: Option<&BrushImage>,
) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let commands: Vec<Command> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    for (i, mut cmd) in commands.iter().cloned().enumerate() {
        if let (
            Some(b),
            Command::Stroke { custom, .. }
            | Command::Shape {
                custom,
                filled: false,
                ..
            },
        ) = (brush, &mut cmd)
            && custom.is_none()
        {
            *custom = Some(b.clone());
        }
        engine
            .execute(cmd)
            .map_err(|e| format!("{path}: command {i}: {e}"))?;
    }
    Ok(commands.len())
}

/// `--brush-xforms SPEC` (snapshots, ADR-30): `OP[:ARGS]@X,Y` entries separated by `;`. Each
/// transforms `brush` with the Brush menu's pure operation (`stretch:W,H`, `rotate:DEGREES`,
/// `shear:DX`, `bend_h:AMOUNT,LINE`, `bend_v:AMOUNT,LINE`) and stamps it with its top-left
/// at X,Y in Matte mode, as one command each.
pub fn run_xform_spec(engine: &mut Engine, brush: &BrushImage, spec: &str) -> Result<(), String> {
    let colors = engine.document().palette().len();
    for entry in spec.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let bad = || format!("'{entry}': expected OP[:ARGS]@X,Y");
        let (op, at) = entry.split_once('@').ok_or_else(bad)?;
        let nums = |s: &str| -> Result<Vec<f64>, String> {
            s.split(',')
                .map(|n| n.trim().parse::<f64>().map_err(|_| bad()))
                .collect()
        };
        let at = nums(at)?;
        let (name, args) = op.split_once(':').unwrap_or((op, ""));
        let args = if args.is_empty() { vec![] } else { nums(args)? };
        let arg = |i: usize| args.get(i).copied().ok_or_else(bad);
        let mut b = match name {
            "stretch" => brush_ops::stretch(brush, arg(0)? as u32, arg(1)? as u32),
            "rotate" => brush_ops::rotate_any(brush, arg(0)?, colors),
            "shear" => brush_ops::shear(brush, arg(0)? as i32, colors),
            "bend_h" => brush_ops::bend(brush, true, arg(0)? as i32, arg(1)? as i32, colors),
            "bend_v" => brush_ops::bend(brush, false, arg(0)? as i32, arg(1)? as i32, colors),
            _ => return Err(format!("'{name}': unknown brush operation")),
        };
        b.handle = (0, 0);
        let &[x, y] = at.as_slice() else {
            return Err(bad());
        };
        engine
            .execute(Command::Shape {
                shape: Shape::Dots {
                    points: vec![(x as i32, y as i32)],
                },
                brush: Vec::new(),
                color: 0,
                filled: false,
                symmetry: None,
                custom: Some(b),
                mode: DrawMode::Matte,
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            })
            .map_err(|e| format!("'{entry}': {e}"))?;
    }
    Ok(())
}

/// `--pick X,Y,W,H`: that part of `page` as a custom brush, picked up like the Brush
/// Selector does (colour 0, the default BG, transparent; handle in the centre).
pub fn pick_spec(page: &IndexedImage, spec: &str) -> Result<BrushImage, String> {
    let nums: Option<Vec<i32>> = spec.split(',').map(|s| s.trim().parse().ok()).collect();
    let Some(&[x, y, w, h]) = nums.as_deref() else {
        return Err(format!("'{spec}': expected X,Y,W,H"));
    };
    let r = Rect::new(x, y, w, h).intersect(page.bounds());
    if w <= 0 || h <= 0 || r.is_empty() {
        return Err(format!("'{spec}' lies outside the page"));
    }
    Ok(BrushImage::from_image(&page.crop(r), Some(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_ui::Hit;

    fn app(tool: Tool, filled: bool) -> PainterApp {
        let mut app = PainterApp::new();
        app.select_tool(tool, filled);
        app.ui.fg = 2;
        app
    }

    fn press(app: &mut PainterApp, x: i32, y: i32) {
        app.handle_press(Hit::Page { x, y }, false);
    }

    /// The gesture ended as exactly one command; undo restores, redo repeats it.
    fn committed_once(app: &mut PainterApp) -> IndexedImage {
        assert!(app.stroke.is_none(), "gesture finished");
        assert_eq!(app.engine.document().revision(), 1, "exactly one command");
        let painted = app.engine.document().page().clone();
        app.undo();
        assert_eq!(
            &*app.page(),
            &IndexedImage::new(320, 256, 0),
            "undo restores"
        );
        app.redo();
        assert_eq!(&*app.page(), &painted, "redo repeats exactly");
        painted
    }

    #[test]
    fn a_command_the_engine_refuses_is_not_previewed() {
        // A brush wider than any brush may be (e.g. loaded from a file): the engine refuses
        // every stroke with it.
        let mut app = app(Tool::Line, false);
        let width = dpx_core::command::MAX_BRUSH_SIDE + 1;
        app.custom_brush = Some(BrushImage {
            width,
            height: 1,
            pixels: vec![3; width as usize],
            handle: (0, 0),
            transparent: None,
        });
        press(&mut app, 10, 10);
        app.handle_drag(40, 30);
        assert_eq!(&*app.page(), &IndexedImage::new(320, 256, 0), "no preview");
        app.release();
        assert_eq!(app.engine.document().revision(), 0, "nothing committed");
        // The same gesture with a valid brush is previewed.
        let mut app = self::app(Tool::Line, false);
        press(&mut app, 10, 10);
        app.handle_drag(40, 30);
        assert_ne!(&*app.page(), &IndexedImage::new(320, 256, 0));
    }

    #[test]
    fn line_rubber_band_is_the_committed_line() {
        let mut app = app(Tool::Line, false);
        press(&mut app, 10, 10);
        app.handle_drag(40, 30);
        app.handle_drag(50, 20);
        assert_eq!(app.page().get(40, 30), Some(0), "old rubber band erased");
        let preview = app.page().into_owned();
        assert_eq!(app.engine.document().revision(), 0, "nothing committed yet");
        app.release();
        assert_eq!(committed_once(&mut app), preview);
        assert_eq!(preview.get(50, 20), Some(2));
        // Shift constrains to horizontal / vertical.
        let mut app = self::app(Tool::Line, false);
        app.tools.shift = true;
        press(&mut app, 10, 10);
        app.handle_drag(40, 14);
        app.release();
        assert_eq!(app.page().get(40, 10), Some(2));
        assert_eq!(app.page().get(40, 14), Some(0));
    }

    #[test]
    fn filled_rectangle_circle_and_ellipse_are_one_command_each() {
        for (tool, inside, outside) in [
            (Tool::Rectangle, (25, 15), (31, 15)),
            (Tool::Circle, (20, 23), (29, 29)),
            (Tool::Ellipse, (20, 18), (20, 21)),
        ] {
            let mut app = app(tool, true);
            assert!(app.ui.filled);
            press(&mut app, 20, 10);
            app.handle_drag(30, 20);
            let preview = app.page().into_owned();
            app.release();
            let page = committed_once(&mut app);
            assert_eq!(page, preview, "{tool:?}: preview = result");
            assert_eq!(page.get(inside.0, inside.1), Some(2), "{tool:?} inside");
            assert_eq!(page.get(outside.0, outside.1), Some(0), "{tool:?} outside");
        }
    }

    #[test]
    fn be_square_circles_are_round_on_tall_hires_pixels() {
        use dpx_core::{DisplayMode, Resolution, Video};
        let mut app = app(Tool::Circle, false);
        let hires = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        app.engine
            .execute(Command::SetFormat {
                mode: hires,
                colors: 16,
                width: 640,
                height: 256,
                retain: false,
                screen: None,
                chipset: None,
            })
            .unwrap();
        let radius = |app: &mut PainterApp| {
            press(app, 100, 100);
            app.handle_drag(120, 100);
            let cmd = app.stroke.as_ref().and_then(|s| s.candidate(true));
            app.cancel_stroke();
            match cmd {
                Some(Command::Shape {
                    shape: Shape::Ellipse { radius, .. },
                    ..
                }) => radius,
                other => panic!("a circle is an ellipse shape: {other:?}"),
            }
        };
        assert_eq!(radius(&mut app), (20, 10), "half as many rows");
        app.ui.prefs.toggle(Pref::BeSquare);
        assert_eq!(radius(&mut app), (20, 20), "Be Square off");
    }

    #[test]
    fn polygon_clicks_close_on_the_first_corner() {
        let mut app = app(Tool::Polygon, true);
        for (x, y) in [(10, 10), (60, 10), (30, 50)] {
            press(&mut app, x, y);
            app.release();
            app.handle_hover(x + 3, y + 3);
        }
        assert!(app.stroke.is_some(), "still collecting corners");
        assert_eq!(app.engine.document().revision(), 0);
        press(&mut app, 11, 11);
        let page = committed_once(&mut app);
        assert_eq!(page.get(30, 20), Some(2), "filled");
        assert_eq!(page.get(33, 53), Some(0), "rubber band dropped");

        // Unfilled: Space lets go of the rubber band and keeps the open outline.
        let mut app = self::app(Tool::Polygon, false);
        for (x, y) in [(10, 10), (60, 10), (60, 40)] {
            press(&mut app, x, y);
            app.release();
        }
        app.handle_hover(5, 40);
        app.tool_key(ToolKey::Space);
        let page = committed_once(&mut app);
        assert_eq!(page.get(35, 10), Some(2));
        assert_eq!(page.get(35, 25), Some(0), "not closed");
        assert_eq!(page.get(30, 40), Some(0), "rubber band dropped");
    }

    #[test]
    fn curve_drag_then_bend_then_click() {
        let mut app = app(Tool::Curve, false);
        press(&mut app, 10, 50);
        app.handle_drag(90, 50);
        app.release();
        assert!(app.stroke.is_some(), "waiting for the bend");
        app.handle_hover(50, 10);
        let preview = app.page().into_owned();
        assert_eq!(preview.get(50, 10), Some(2), "passes through the pointer");
        press(&mut app, 50, 10);
        assert_eq!(committed_once(&mut app), preview);
    }

    #[test]
    fn filled_freehand_closes_and_fills_on_release() {
        let mut app = app(Tool::ContinuousFreehand, true);
        press(&mut app, 10, 10);
        for p in [(50, 10), (50, 50), (10, 50)] {
            app.handle_drag(p.0, p.1);
        }
        assert_eq!(
            app.page().get(30, 30),
            Some(0),
            "outline only while drawing"
        );
        app.release();
        let page = committed_once(&mut app);
        assert_eq!(page.get(30, 30), Some(2));
        assert_eq!(page.get(10, 30), Some(2), "closed with a straight line");
    }

    #[test]
    fn fill_click_and_grid_snapping() {
        let mut app = app(Tool::Rectangle, false);
        app.tools.grid = true;
        press(&mut app, 7, 9);
        app.handle_drag(41, 40);
        app.release();
        let page = app.page().into_owned();
        assert_eq!(
            page.get(0, 16),
            Some(2),
            "corner snapped to the 16-pixel grid"
        );
        assert_eq!(page.get(48, 48), Some(2));
        assert_eq!(page.get(7, 9), Some(0));
        app.select_tool(Tool::Fill, false);
        app.ui.bg = 4;
        app.handle_press(Hit::Page { x: 20, y: 30 }, true);
        assert_eq!(app.page().get(20, 30), Some(4), "right = BG");
        app.ui.fg = 5;
        app.handle_press(Hit::Page { x: 20, y: 30 }, false);
        assert_eq!(app.page().get(47, 47), Some(5));
        assert_eq!(app.page().get(60, 60), Some(0), "only inside the box");
        assert_eq!(app.engine.document().revision(), 3);
        app.undo();
        assert_eq!(app.page().get(47, 47), Some(4));
    }

    #[test]
    fn airbrush_commits_its_sprayed_points_and_replays_exactly() {
        let mut app = app(Tool::Airbrush, false);
        press(&mut app, 100, 100);
        for _ in 0..10 {
            app.spray(SPRAYS_PER_FRAME);
        }
        app.handle_drag(120, 100);
        app.spray(SPRAYS_PER_FRAME);
        let preview = app.page().into_owned();
        let n = preview.pixels().iter().filter(|&&p| p == 2).count();
        assert!(n > 10, "sprayed {n} pixels");
        for (i, &p) in preview.pixels().iter().enumerate() {
            if p == 2 {
                let (x, y) = ((i % 320) as i32, (i / 320) as i32);
                assert!(
                    cheb((x, y), (100, 100)) <= AIRBRUSH_RADIUS
                        || cheb((x, y), (120, 100)) <= AIRBRUSH_RADIUS
                );
            }
        }
        app.release();
        assert_eq!(committed_once(&mut app), preview);
    }

    #[test]
    fn symmetry_draws_six_copies_around_the_screen_centre() {
        let mut app = app(Tool::Line, false);
        app.select_tool(Tool::Symmetry, false);
        assert!(app.tools.symmetry);
        assert_eq!(app.ui.tool, Tool::Line, "a toggle keeps the tool");
        press(&mut app, 200, 128);
        app.handle_drag(220, 128);
        let preview = app.page().into_owned();
        app.release();
        let page = committed_once(&mut app);
        assert_eq!(page, preview);
        // The copy rotated by 180 degrees.
        assert_eq!(page.get(110, 128), Some(2));
        assert_eq!(page.get(160, 128), Some(0), "centre untouched");
        // Freehand copies are drawn while painting.
        let mut app = self::app(Tool::ContinuousFreehand, false);
        app.tools.symmetry = true;
        press(&mut app, 200, 128);
        app.handle_drag(210, 128);
        assert_eq!(app.page().get(115, 128), Some(2));
        app.release();
        committed_once(&mut app);
    }

    #[test]
    fn text_is_typed_and_committed_as_pixels() {
        let mut app = app(Tool::Text, false);
        press(&mut app, 20, 20);
        app.release();
        for c in "Hi".chars() {
            app.tool_key(ToolKey::Char(c));
        }
        app.tool_key(ToolKey::Enter);
        app.tool_key(ToolKey::Char('x'));
        app.tool_key(ToolKey::Backspace);
        app.tool_key(ToolKey::Backspace);
        // Shortcut letters are typed, not tool switches.
        app.tool_key(ToolKey::Char('r'));
        assert_eq!(app.ui.tool, Tool::Text);
        assert!(app.typing());
        let preview = app.page().into_owned();
        assert!(preview.pixels().contains(&2));
        app.tool_key(ToolKey::Escape);
        assert!(!app.typing());
        assert_eq!(committed_once(&mut app), preview);
    }

    #[test]
    fn tool_keys_pick_tools_and_toggles() {
        let mut app = PainterApp::new();
        for (c, tool, filled) in [
            ('v', Tool::Line, false),
            ('q', Tool::Curve, false),
            ('R', Tool::Rectangle, true),
            ('c', Tool::Circle, false),
            ('E', Tool::Ellipse, true),
            ('D', Tool::ContinuousFreehand, true),
            ('s', Tool::DottedFreehand, false),
            ('f', Tool::Fill, false),
        ] {
            app.tool_key(ToolKey::Char(c));
            assert_eq!((app.ui.tool, app.ui.filled), (tool, filled), "key {c}");
        }
        app.tool_key(ToolKey::Char('g'));
        app.tool_key(ToolKey::Char('/'));
        assert!(app.tools.grid && app.tools.symmetry);
        app.tool_key(ToolKey::Char('g'));
        assert!(!app.tools.grid);
        app.tool_key(ToolKey::Char('>'));
        assert_eq!(app.tools.zoom, DEFAULT_ZOOM + 1);
        app.handle_press(
            Hit::Tool {
                tool: Tool::Zoom,
                filled_half: false,
            },
            true,
        );
        assert_eq!(app.tools.zoom, DEFAULT_ZOOM);
    }

    #[test]
    fn magnify_splits_the_screen_and_paints_in_both_halves() {
        let mut app = app(Tool::DottedFreehand, false);
        app.pointer_page = Some((100, 100));
        app.tool_key(ToolKey::Char('m'));
        let view = app.view();
        let m = app.magnify_layout(view).expect("magnify on");
        assert_eq!((m.split, m.start, m.end, m.zoom), (98, 101, 296, 4));
        assert_eq!(m.source, Rect::new(76, 68, 49, 64));
        // Normal half, divider, magnified half.
        assert_eq!(app.screen_to_page(view, (10, 20)), Some((10, 20)));
        assert_eq!(app.screen_to_page(view, (99, 20)), None);
        let p = app
            .screen_to_page(view, (101 + 4 * 14 + 3, 4 * 32))
            .unwrap();
        assert_eq!(p, (90, 100));
        press(&mut app, p.0, p.1);
        app.release();
        let screen = app.compose_screen();
        assert_eq!(screen.get(90, 100), Some(2), "normal half");
        for (x, y) in [(101 + 56, 128), (101 + 59, 131)] {
            assert_eq!(screen.get(x, y), Some(2), "magnified 4x at ({x},{y})");
        }
        assert_eq!(screen.get(101 + 60, 128), Some(0));
        assert_eq!(app.ui.tool, Tool::DottedFreehand, "the drawing tool stays");
        app.select_tool(Tool::Magnify, false);
        assert!(app.tools.magnify.is_none());
        assert_eq!(app.screen_to_page(view, (200, 20)), Some((200, 20)));
    }

    /// A page with a 10×5 block of colour 5 at (10, 10) holding one pixel of colour 7 at
    /// (12, 11); then the Brush Selector picks up (8, 8)–(21, 16) (14×9, a colour-0 border)
    /// with the given button, coming from `tool`.
    fn picked(tool: Tool, right: bool) -> PainterApp {
        let mut app = app(Tool::Rectangle, true);
        app.ui.fg = 5;
        press(&mut app, 10, 10);
        app.handle_drag(19, 14);
        app.release();
        app.select_tool(Tool::DottedFreehand, false);
        app.ui.fg = 7;
        press(&mut app, 12, 11);
        app.release();
        app.ui.fg = 2;
        app.select_tool(tool, false);
        app.tool_key(ToolKey::Char('b'));
        assert_eq!(app.ui.tool, Tool::BrushSelect);
        app.handle_press(Hit::Page { x: 8, y: 8 }, right);
        app.handle_drag(21, 16);
        app
    }

    #[test]
    fn the_brush_selector_picks_up_a_brush_and_returns_to_the_tool() {
        let mut app = picked(Tool::Line, false);
        // While dragging: a frame in the complementary colour, nothing committed.
        assert_eq!(app.page().get(8, 8), Some(31), "frame over colour 0");
        assert_eq!(app.page().get(10, 16), Some(31));
        assert_eq!(app.page().get(12, 11), Some(7), "inside unchanged");
        assert_eq!(app.engine.document().revision(), 2);
        app.release();
        assert!(app.stroke.is_none());
        assert_eq!(
            app.engine.document().revision(),
            2,
            "a copy changes nothing"
        );
        assert_eq!(app.page().get(8, 8), Some(0), "frame gone");
        let b = app.custom_brush.clone().expect("picked up");
        assert_eq!((b.width, b.height), (14, 9));
        assert_eq!(b.handle, (7, 4), "handle in the centre");
        assert_eq!(b.transparent, Some(0), "BG is transparent");
        assert_eq!(b.get(4, 3), Some(7));
        assert_eq!(app.picked_brush.as_ref(), Some(&b));
        assert_eq!(app.ui.mode, PaintMode::Matte);
        assert_eq!(app.ui.tool, Tool::Line, "back to the tool before");
        // Shift makes the frame square; a pick-up off the page picks nothing.
        let mut app = picked(Tool::DottedFreehand, false);
        app.cancel_stroke();
        app.tools.shift = true;
        app.select_tool(Tool::BrushSelect, false);
        press(&mut app, 8, 8);
        app.handle_drag(21, 12);
        app.release();
        let b = app.custom_brush.clone().unwrap();
        assert_eq!((b.width, b.height), (14, 14));
    }

    #[test]
    fn a_right_button_drag_cuts_in_one_undo_step() {
        let mut app = picked(Tool::DottedFreehand, true);
        let before = app.engine.document().page().clone();
        app.ui.bg = 4;
        app.release();
        assert_eq!(app.engine.document().revision(), 3, "one command");
        let page = app.engine.document().page().clone();
        assert_eq!(page.get(12, 11), Some(4), "BG left behind");
        assert_eq!(page.get(21, 16), Some(4));
        assert_eq!(page.get(22, 16), Some(0), "only the framed part");
        let b = app.custom_brush.clone().unwrap();
        assert_eq!(b.get(4, 3), Some(7), "the brush has the old pixels");
        assert_eq!(b.transparent, Some(4), "BG at pick-up time");
        app.undo();
        assert_eq!(
            app.engine.document().page(),
            &before,
            "undo restores the cut"
        );
        assert!(app.custom_brush.is_some(), "the brush stays");
    }

    #[test]
    fn auto_transp_uses_the_shared_corner_colour() {
        let img = IndexedImage::from_pixels(3, 2, vec![6, 1, 6, 6, 2, 6]).unwrap();
        let mut prefs = Prefs::default();
        assert_eq!(pick_transparent(&img, prefs, 0), 0, "off: BG");
        prefs.set(Pref::AutoTransp, true);
        assert_eq!(pick_transparent(&img, prefs, 0), 6);
        let mixed = IndexedImage::from_pixels(2, 1, vec![6, 1]).unwrap();
        assert_eq!(pick_transparent(&mixed, prefs, 0), 0, "corners differ: BG");
    }

    #[test]
    fn a_custom_brush_stamps_in_matte_color_and_replace() {
        let mut app = picked(Tool::DottedFreehand, false);
        app.release();
        app.run(Command::Clear { color: 3 });
        let rev = app.engine.document().revision();
        // Matte: own colours; the colour-0 border is transparent. Handle (7, 4).
        press(&mut app, 100, 100);
        app.release();
        let page = app.page().into_owned();
        assert_eq!(page.get(95, 98), Some(5));
        assert_eq!(page.get(97, 99), Some(7));
        assert_eq!(page.get(93, 96), Some(3), "transparent");
        // Color: the shape in FG.
        app.ui.mode = PaintMode::Color;
        press(&mut app, 200, 100);
        app.release();
        assert_eq!(app.page().get(195, 98), Some(2));
        assert_eq!(app.page().get(197, 99), Some(2));
        assert_eq!(app.page().get(193, 96), Some(3));
        // Replace: the whole rectangle, the border in its own colour 0.
        app.ui.mode = PaintMode::Replace;
        press(&mut app, 150, 150);
        app.release();
        assert_eq!(app.page().get(143, 146), Some(0));
        assert_eq!(app.page().get(145, 148), Some(5));
        assert_eq!(
            app.engine.document().revision(),
            rev + 3,
            "one command each"
        );
        // Outline tools stamp it too; the preview is the committed result.
        app.ui.mode = PaintMode::Matte;
        app.select_tool(Tool::Line, false);
        press(&mut app, 40, 200);
        app.handle_drag(90, 220);
        let preview = app.page().into_owned();
        app.release();
        assert_eq!(app.engine.document().page(), &preview);
        assert_eq!(preview.get(35, 198), Some(5), "brush at the start");
        assert_eq!(preview.get(85, 218), Some(5), "brush at the end");
        app.undo();
        assert_eq!(app.engine.document().page().get(85, 218), Some(3));
    }

    #[test]
    fn the_custom_brush_follows_the_pointer_without_being_committed() {
        let mut app = picked(Tool::DottedFreehand, false);
        app.release();
        app.run(Command::Clear { color: 3 });
        let rev = app.engine.document().revision();
        app.handle_hover(100, 100);
        assert_eq!(app.page().get(95, 98), Some(5), "shown at the pointer");
        assert_eq!(app.engine.document().page().get(95, 98), Some(3));
        app.handle_hover(120, 100);
        assert_eq!(app.page().get(95, 98), Some(3), "moved on");
        assert_eq!(app.page().get(115, 98), Some(5));
        // Leaving the page hides it; nothing was committed.
        app.hover_brush(None);
        assert!(app.stroke.is_none());
        assert_eq!(&*app.page(), app.engine.document().page());
        assert_eq!(app.engine.document().revision(), rev);
        // A click while it is shown paints exactly one stamp.
        app.handle_hover(100, 100);
        press(&mut app, 100, 100);
        app.release();
        assert_eq!(app.engine.document().revision(), rev + 1);
        assert_eq!(app.engine.document().page().get(95, 98), Some(5));
        // Tools that do not stamp the brush show nothing.
        for (tool, filled) in [
            (Tool::Fill, false),
            (Tool::Rectangle, true),
            (Tool::Text, false),
        ] {
            app.select_tool(tool, filled);
            app.handle_hover(60, 60);
            assert!(app.stroke.is_none(), "{tool:?}");
        }
    }

    #[test]
    fn a_built_in_brush_drops_the_custom_brush_and_b_restores_it() {
        let mut app = picked(Tool::DottedFreehand, false);
        app.release();
        let pens = Pens::from_palette(app.palette());
        let x0 = app.view().ui_screen().toolbox_x();
        assert_eq!(
            app.compose_screen().get(x0 + 1, 12),
            Some(pens.face),
            "no built-in brush selected"
        );
        app.handle_press(Hit::Brush(2), false);
        assert!(app.custom_brush.is_none());
        assert_eq!(app.ui.brush, 2);
        app.ui.fg = 2;
        press(&mut app, 100, 100);
        app.release();
        assert_eq!(app.page().get(100, 100), Some(2), "built-in brush in FG");
        assert_eq!(app.page().get(95, 98), Some(0));
        app.tool_key(ToolKey::Char('B'));
        assert_eq!(app.custom_brush, app.picked_brush);
        // Right-click on the Brush Selector toggles the custom brush.
        let right_click = Hit::Tool {
            tool: Tool::BrushSelect,
            filled_half: false,
        };
        app.handle_press(right_click, true);
        assert!(app.custom_brush.is_none());
        app.handle_press(right_click, true);
        assert!(app.custom_brush.is_some());
        assert_eq!(app.ui.tool, Tool::DottedFreehand, "the tool stays");
    }

    #[test]
    fn mode_range_and_stencil_reach_every_command() {
        let mut app = app(Tool::Line, false);
        app.extras.ranges = vec![
            dpx_iff::ColorRange {
                rate: 0,
                flags: 0,
                low: 8,
                high: 12,
            },
            dpx_iff::ColorRange {
                rate: 0,
                flags: 1,
                low: 1,
                high: 4,
            },
        ];
        app.ui.mode = PaintMode::Shade;
        let mut stencil = vec![false; 256];
        stencil[3] = true;
        app.stencil = Some(stencil.clone());
        app.ui.bg = 6;
        app.handle_press(Hit::Page { x: 10, y: 10 }, true);
        app.handle_drag(20, 20);
        let cmd = app.stroke.as_ref().and_then(|s| s.candidate(true));
        match cmd {
            Some(Command::Shape {
                color,
                mode,
                range,
                stencil: s,
                right,
                ..
            }) => {
                assert_eq!((color, mode, right), (6, DrawMode::Shade, true));
                assert_eq!(range, Some((1, 4)), "the range holding FG 2");
                assert_eq!(s, Some(stencil));
            }
            other => panic!("a line is a shape: {other:?}"),
        }
        app.release();
        // FG in no range: no range.
        app.ui.fg = 20;
        app.ui.mode = PaintMode::Matte;
        app.select_tool(Tool::DottedFreehand, false);
        press(&mut app, 5, 5);
        match app.stroke.as_ref().and_then(|s| s.candidate(true)) {
            Some(Command::Stroke {
                range, mode, right, ..
            }) => assert_eq!((range, mode, right), (None, DrawMode::Matte, false)),
            other => panic!("freehand is a stroke: {other:?}"),
        }
        assert_eq!(draw_mode(PaintMode::Replace), DrawMode::Replace);
        assert_eq!(draw_mode(PaintMode::HBrite), DrawMode::HBrite);
    }

    #[test]
    fn brush_command_files_paint_with_the_picked_brush() {
        let mut page = IndexedImage::new(8, 8, 0);
        page.fill_rect(2, 2, 3, 3, 5);
        let brush = pick_spec(&page, "1,1,5,5").unwrap();
        assert_eq!((brush.width, brush.height, brush.handle), (5, 5, (2, 2)));
        assert!(pick_spec(&page, "20,20,2,2").is_err());
        assert!(pick_spec(&page, "1,2,3").is_err());
        let dir = std::env::temp_dir().join(format!("dpx-brush-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stamps.json");
        std::fs::write(
            &path,
            r#"[{"op":"shape","shape":{"kind":"dots","points":[[10,10]]},"color":2,"mode":"color"},
                {"op":"shape","shape":{"kind":"rect","from":[0,0],"to":[1,1]},"color":3,"filled":true}]"#,
        )
        .unwrap();
        let mut engine = Engine::new(dpx_core::Document::new(
            32,
            32,
            dpx_core::Palette::dpaint_default_32(),
        ));
        let path = path.to_string_lossy().into_owned();
        assert_eq!(run_brush_command_file(&mut engine, &path, &brush), Ok(2));
        let p = engine.document().page();
        assert_eq!(p.get(10, 10), Some(2), "brush shape in FG (Color)");
        assert_eq!(p.get(8, 8), Some(0), "transparent corner");
        assert_eq!(p.get(1, 1), Some(3), "filled shapes are not brushes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn command_files_run_through_the_engine() {
        let dir = std::env::temp_dir().join(format!("dpx-tools-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cmds.json");
        std::fs::write(
            &path,
            r#"[{"op":"shape","shape":{"kind":"rect","from":[1,1],"to":[4,4]},"color":3,"filled":true},
                {"op":"flood_fill","x":0,"y":0,"color":2}]"#,
        )
        .unwrap();
        let mut engine = Engine::new(dpx_core::Document::new(
            8,
            8,
            dpx_core::Palette::dpaint_default_32(),
        ));
        let path = path.to_string_lossy().into_owned();
        assert_eq!(run_command_file(&mut engine, &path), Ok(2));
        assert_eq!(engine.document().page().get(2, 2), Some(3));
        assert_eq!(engine.document().page().get(7, 7), Some(2));
        std::fs::write(&path, r#"[{"op":"flood_fill","x":0,"y":0,"color":99}]"#).unwrap();
        let err = run_command_file(&mut engine, &path).unwrap_err();
        assert!(err.contains("command 0"), "{err}");
        std::fs::write(&path, "not json").unwrap();
        assert!(run_command_file(&mut engine, &path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The custom brush of `picked` (14x9, handle in the centre), the pointer at rest.
    fn with_brush() -> PainterApp {
        let mut app = picked(Tool::DottedFreehand, false);
        app.release();
        assert!(app.custom_brush.is_some());
        app
    }

    fn brush_size(app: &PainterApp) -> (u32, u32) {
        let b = app.custom_brush.as_ref().unwrap();
        (b.width, b.height)
    }

    #[test]
    fn stretch_drags_a_new_size_and_commits_one_brush_change() {
        let mut app = with_brush();
        let original = app.custom_brush.clone().unwrap();
        let rev = app.engine.document().revision();
        app.brush_action("brush.stretch");
        // Armed: the brush shows at the pointer, nothing changes.
        app.handle_hover(100, 100);
        assert!(app.stroke.is_some());
        assert_eq!(app.page().get(100, 100), original.get(0, 0));
        assert_eq!(app.custom_brush.as_ref(), Some(&original));
        press(&mut app, 100, 100);
        app.handle_drag(129, 119);
        assert_eq!(brush_size(&app), (14, 9), "only previewed while dragging");
        app.release();
        assert_eq!(brush_size(&app), (30, 20));
        assert!(app.stroke.is_none() && app.tools.xform.is_none());
        assert_eq!(app.engine.document().revision(), rev, "no page change");
        // Brush > Restore brings the picked brush back.
        app.tool_key(ToolKey::Char('B'));
        assert_eq!(app.custom_brush.as_ref(), Some(&original));
        // Shift keeps the aspect ratio (14:9): width wins.
        app.brush_action("brush.stretch");
        press(&mut app, 100, 100);
        app.tools.shift = true;
        app.handle_drag(139, 104);
        app.release();
        let (w, h) = brush_size(&app);
        assert_eq!(w, 40);
        assert_eq!(h, (9.0 * 40.0 / 14.0_f64).round() as u32);
    }

    #[test]
    fn a_click_without_a_drag_changes_nothing_and_place_sets_the_handle() {
        let mut app = with_brush();
        let original = app.custom_brush.clone().unwrap();
        app.brush_action("brush.rotate_any");
        press(&mut app, 50, 50);
        app.release();
        assert_eq!(app.custom_brush.as_ref(), Some(&original));
        // Place: the brush is shown with its handle at the pointer; the press fixes its place.
        app.brush_action("brush.handle_place");
        app.handle_hover(100, 100);
        assert_eq!(app.page().get(100 - 7, 100 - 4), original.get(0, 0));
        press(&mut app, 100, 100);
        app.handle_drag(96, 98);
        app.release();
        let b = app.custom_brush.clone().unwrap();
        assert_eq!(b.handle, (3, 2), "pointer minus the brush's top left");
        assert_eq!(b.pixels, original.pixels);
    }

    /// ADR-44: the Brush Selector's frame and the Place handle mark go over the picture as
    /// it shows, in the complement of what shows there, whichever layer is active (on an
    /// upper layer they used to be drawn into it, inverted from its own transparent pixels).
    #[test]
    fn frame_and_handle_mark_show_over_every_layer() {
        // CLR 31 (the top colour of 32: its complement is 0).
        let layered = |upper: bool| {
            let mut app = app(Tool::BrushSelect, false);
            app.engine.execute(Command::Clear { color: 31 }).unwrap();
            if upper {
                app.engine
                    .execute(Command::AddLayer { name: None })
                    .unwrap();
            }
            app
        };
        for upper in [false, true] {
            let mut app = layered(upper);
            let before: Vec<_> = app.engine.document().cels().cloned().collect();
            press(&mut app, 10, 10);
            app.handle_drag(60, 50);
            let shown = app.page();
            assert_eq!(shown.get(30, 50), Some(0), "bottom edge, upper {upper}");
            assert_eq!(shown.get(10, 30), Some(0), "left edge, upper {upper}");
            assert_eq!(shown.get(30, 30), Some(31), "inside");
            drop(shown);
            assert_eq!(app.compose_screen().get(30, 50), Some(0), "on the screen");
            let during: Vec<_> = app.engine.document().cels().cloned().collect();
            assert_eq!(before, during, "nothing goes into a layer");
            assert!(
                app.stroke.as_ref().unwrap().preview == *app.engine.document().page(),
                "not into the active layer's preview either"
            );
        }
        // Place on a layered page: the handle's pixel is the complement of what shows.
        let mut app = with_brush();
        app.engine
            .execute(Command::AddLayer { name: None })
            .unwrap();
        app.brush_action("brush.handle_place");
        app.handle_hover(100, 100);
        let shown = app.page();
        let doc = app.engine.document();
        let under = doc.composite_with(&app.stroke.as_ref().unwrap().preview);
        let top = (doc.palette().len() - 1) as u8;
        assert_eq!(
            shown.get(100, 100),
            Some(top - under.get(100, 100).unwrap())
        );
        assert_ne!(shown.get(100, 100), under.get(100, 100));
    }

    #[test]
    fn any_angle_restarts_from_the_original_orientation() {
        let mut app = with_brush();
        let original = app.custom_brush.clone().unwrap();
        app.brush_action("brush.rotate_any");
        press(&mut app, 60, 60);
        app.handle_drag(60, 80);
        app.release();
        assert_eq!(
            app.custom_brush.as_ref(),
            Some(&brush_ops::rotate_90(&original))
        );
        // The second use starts again from the original: pointing right is 0 degrees.
        app.brush_action("brush.rotate_any");
        press(&mut app, 60, 60);
        app.handle_drag(90, 60);
        app.release();
        assert_eq!(app.custom_brush.as_ref(), Some(&original));
    }

    #[test]
    fn shear_and_bend_grow_the_brush_and_escape_cancels() {
        let mut app = with_brush();
        app.brush_action("brush.shear");
        press(&mut app, 60, 60);
        app.handle_drag(70, 60);
        app.release();
        assert_eq!(brush_size(&app), (24, 9));
        app.tool_key(ToolKey::Char('B'));
        app.brush_action("brush.bend_horizontal");
        press(&mut app, 60, 60);
        app.handle_drag(66, 64);
        app.release();
        assert_eq!(brush_size(&app), (20, 9));
        app.tool_key(ToolKey::Char('B'));
        app.brush_action("brush.bend_vertical");
        press(&mut app, 60, 60);
        app.handle_drag(66, 66);
        app.release();
        assert_eq!(brush_size(&app), (14, 15));
        // Escape drops an armed gesture; the next press paints normally.
        app.tool_key(ToolKey::Char('B'));
        app.brush_action("brush.stretch");
        app.tool_key(ToolKey::Escape);
        assert!(app.tools.xform.is_none() && app.stroke.is_none());
        let rev = app.engine.document().revision();
        press(&mut app, 100, 100);
        app.release();
        assert_eq!(app.engine.document().revision(), rev + 1);
        // Choosing another tool drops it too.
        app.brush_action("brush.shear");
        app.select_tool(Tool::Line, false);
        assert!(app.tools.xform.is_none());
    }
}
