//! Drawing-tool interaction (ADR-14): pointer gestures on the page become exactly one
//! undoable [`Command`] each. While a gesture is in progress its candidate command is painted
//! into a copy of the page with the same dpx-core code the engine uses, so the preview is the
//! committed result. Also the toolbox toggles (Grid, Symmetry, Magnify, Zoom), the tool keys
//! and `--commands`.

use dpx_core::shapes::MAX_SHAPE_POINTS;
use dpx_core::{Command, Engine, IndexedImage, Rect, Shape, Symmetry, raster};
use dpx_ui::font::{self, Font};
use dpx_ui::{Pens, Pref, Tool, toolbox};
use eframe::egui;

use crate::{PainterApp, View};

/// Grid spacing in page pixels (the Gridding requester is not implemented yet).
pub const GRID: i32 = 16;
/// Cyclic symmetry order (the Symmetry requester is not implemented yet).
pub const SYMMETRY_ORDER: u8 = 6;
/// Airbrush spray radius in page pixels (horizontally; Be Square adjusts the height).
pub const AIRBRUSH_RADIUS: i32 = 10;
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
    pub symmetry: bool,
    /// Magnify on: the page pixel at the centre of the magnified half.
    pub magnify: Option<(i32, i32)>,
    /// Index into [`ZOOMS`].
    pub zoom: usize,
    /// Shift held this frame (constrains lines, squares rectangles).
    pub shift: bool,
    /// Airbrush random state (xorshift); the sprayed points end up in the command.
    rng: u32,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            grid: false,
            symmetry: false,
            magnify: None,
            zoom: DEFAULT_ZOOM,
            shift: false,
            rng: 0x2545_F491,
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

/// A gesture in progress (named after the freehand stroke it started as).
pub struct Stroke {
    /// The page with the candidate command painted on it.
    pub preview: IndexedImage,
    /// The part of `preview` that may differ from the document page.
    drawn: Rect,
    color: u8,
    brush: Vec<(i32, i32)>,
    filled: bool,
    symmetry: Option<Symmetry>,
    /// The button that started the gesture is still held.
    held: bool,
    kind: Kind,
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

impl Stroke {
    fn shape(&self, shape: Shape, filled: bool) -> Command {
        let solid = filled && !matches!(shape, Shape::Dots { .. });
        Command::Shape {
            shape,
            brush: if solid {
                Vec::new()
            } else {
                self.brush.clone()
            },
            color: self.color,
            filled: solid,
            symmetry: self.symmetry,
        }
    }

    /// Freehand points as a command: a plain [`Command::Stroke`] without symmetry.
    fn freehand(&self, points: Vec<(i32, i32)>, continuous: bool) -> Command {
        match (self.symmetry, continuous) {
            (None, _) => Command::Stroke {
                brush: self.brush.clone(),
                color: self.color,
                points,
                continuous,
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

    /// A 1-pixel outline (filled freehand and polygons while they are drawn).
    fn outline(&self, points: Vec<(i32, i32)>) -> Command {
        Command::Stroke {
            brush: vec![(0, 0)],
            color: self.color,
            points,
            continuous: true,
        }
    }

    /// The command this gesture stands for: as shown while drawing, or (`commit`) as
    /// committed. They differ only for filled freehand and polygons (outline, then filled)
    /// and the polygon's rubber band (dropped).
    fn candidate(&self, commit: bool) -> Option<Command> {
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
                Command::Shape {
                    shape: Shape::Dots { points },
                    brush: vec![(0, 0)],
                    color: self.color,
                    filled: false,
                    symmetry: None,
                }
            }
        })
    }

    /// Paints part of the gesture on top of the preview (freehand, airbrush).
    fn paint_more(&mut self, cmd: &Command) {
        if let Some(r) = cmd.paint_bounds(&self.preview) {
            self.drawn = self.drawn.union(r);
        }
        cmd.paint(&mut self.preview);
    }

    /// Repaints the preview from the document page and the current candidate.
    fn redraw(&mut self, page: &IndexedImage) {
        if !self.drawn.is_empty() {
            self.preview
                .blit(&page.crop(self.drawn), self.drawn.x, self.drawn.y);
        }
        self.drawn = Rect::default();
        if let Some(cmd) = self.candidate(false) {
            self.drawn = cmd.paint_bounds(page).unwrap_or_default();
            cmd.paint(&mut self.preview);
        }
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
    fn snap(&self, p: (i32, i32)) -> (i32, i32) {
        let obeys = matches!(
            self.ui.tool,
            Tool::DottedFreehand
                | Tool::Line
                | Tool::Rectangle
                | Tool::Circle
                | Tool::Ellipse
                | Tool::Text
        );
        if self.tools.grid && obeys {
            let s = |v: i32| (v + GRID / 2).div_euclid(GRID) * GRID;
            (s(p.0), s(p.1))
        } else {
            p
        }
    }

    /// Symmetry for a new gesture: around the centre of the screen.
    fn symmetry(&self) -> Option<Symmetry> {
        if !self.tools.symmetry || matches!(self.ui.tool, Tool::Text | Tool::Fill) {
            return None;
        }
        let v = self.view();
        let (fx, fy) = self.aspect();
        Some(Symmetry {
            center: (
                self.scroll.0 + v.screen_w / 2,
                self.scroll.1 + v.screen_h / 2,
            ),
            order: SYMMETRY_ORDER,
            aspect: (fx as u8, fy as u8),
        })
    }

    /// A press on the page (`right` = BG colour).
    pub(crate) fn press_page(&mut self, x: i32, y: i32, right: bool) {
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
                    s.redraw(self.engine.document().page());
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
            Tool::Fill => {
                self.run(Command::FloodFill { x, y, color });
                return;
            }
            _ => return,
        };
        let page = self.engine.document().page();
        let mut s = Stroke {
            preview: page.clone(),
            drawn: Rect::default(),
            color,
            brush: dpx_ui::toolbox::BUILTIN_BRUSHES[self.ui.brush].footprint(),
            filled: self.ui.filled && self.ui.tool.is_split(),
            symmetry: self.symmetry(),
            held: true,
            kind,
        };
        match &s.kind {
            Kind::Freehand { continuous, .. } => {
                let first = s.freehand(vec![p], *continuous);
                s.paint_more(&first);
            }
            Kind::Lasso { .. } => {
                let first = s.outline(vec![p]);
                s.paint_more(&first);
            }
            _ => s.redraw(page),
        }
        self.stroke = Some(s);
        if self.ui.tool == Tool::Airbrush {
            self.spray(SPRAYS_PER_FRAME);
        }
        self.dirty = true;
    }

    /// The pointer moved to page pixel (`x`, `y`) with a button held.
    pub(crate) fn handle_drag(&mut self, x: i32, y: i32) {
        let p = self.snap((x, y));
        let aspect = self.aspect();
        let shift = self.tools.shift;
        let page = self.engine.document().page();
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
                let part = if continuous { vec![last, p] } else { vec![p] };
                let cmd = s.freehand(part, continuous);
                s.paint_more(&cmd);
            }
            Kind::Lasso { points } => {
                let last = *points.last().expect("a stroke starts with one point");
                if last == p || points.len() >= MAX_SHAPE_POINTS {
                    return;
                }
                points.push(p);
                let cmd = s.outline(vec![last, p]);
                s.paint_more(&cmd);
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
                s.redraw(page);
            }
            Kind::Rect { from, to } => {
                *to = if shift { square(*from, p, aspect) } else { p };
                s.redraw(page);
            }
            Kind::Circle { to, .. } | Kind::Ellipse { to, .. } => {
                *to = p;
                s.redraw(page);
            }
            Kind::Polygon { cursor, .. } => {
                *cursor = Some((x, y));
                s.redraw(page);
            }
            Kind::Airbrush { at, .. } => *at = p,
            Kind::Curve { .. } | Kind::Text { .. } => return,
        }
        self.dirty = true;
    }

    /// The pointer moved over the page with no button held: the curve's bend and the
    /// polygon's rubber band follow it.
    pub(crate) fn handle_hover(&mut self, x: i32, y: i32) {
        let page = self.engine.document().page();
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
        s.redraw(page);
        self.dirty = true;
    }

    /// No button is held (any more): a held gesture ends. Curves go on to their bend,
    /// polygons to the next corner, text to typing; everything else is committed.
    pub(crate) fn release(&mut self) {
        let page = self.engine.document().page();
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
                    s.redraw(page);
                }
            }
            Kind::Polygon { points, cursor, .. } => {
                if let Some(c) = cursor.take()
                    && points.last() != Some(&c)
                    && points.len() < MAX_SHAPE_POINTS
                {
                    points.push(c);
                }
                s.redraw(page);
            }
            Kind::Text { .. } => {}
            _ => self.finish_stroke(),
        }
        self.dirty = true;
    }

    /// Commits the gesture in progress as one command (what it shows, without a polygon's
    /// rubber band).
    pub(crate) fn finish_stroke(&mut self) {
        if let Some(s) = self.stroke.take() {
            if let Some(cmd) = s.candidate(true) {
                self.execute(cmd);
            }
            self.dirty = true;
        }
    }

    /// Drops the gesture in progress.
    fn cancel_stroke(&mut self) {
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
        let (rx, ry) = (AIRBRUSH_RADIUS, (AIRBRUSH_RADIUS * fx / fy).max(1));
        let mut new = Vec::with_capacity(n);
        while new.len() < n {
            let (dx, dy) = (self.tools.random(rx), self.tools.random(ry));
            if dx * dx * ry * ry + dy * dy * rx * rx <= rx * rx * ry * ry {
                new.push((at.0 + dx, at.1 + dy));
            }
        }
        let Some(s) = &mut self.stroke else {
            return;
        };
        if let Kind::Airbrush { points, .. } = &mut s.kind {
            points.extend_from_slice(&new);
        }
        let cmd = s.shape(Shape::Dots { points: new }, false);
        s.paint_more(&cmd);
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

    /// The screen as shown: the page (normal or split for Magnify), the interface, the
    /// toolbox toggles that are on and the text cursor.
    pub(crate) fn compose_screen(&self) -> IndexedImage {
        let view = self.view();
        let palette = self.palette();
        let pens = Pens::from_palette(palette);
        let mut screen = match self.magnify_layout(view) {
            None => view.compose(self.page(), self.scroll, palette, &self.ui),
            Some(m) => {
                let (w, h) = (view.screen_w, view.screen_h);
                let mut shown = IndexedImage::new(w as u32, h as u32, pens.dark);
                shown.blit(
                    &self
                        .page()
                        .crop(Rect::new(self.scroll.0, self.scroll.1, m.split, h)),
                    0,
                    0,
                );
                shown.fill_rect(m.split, 0, m.start - m.split, h, pens.face);
                shown.fill_rect(m.split, 0, view.ux, h, pens.shine);
                shown.fill_rect(m.start - view.ux, 0, view.ux, h, pens.dark);
                let big = raster::magnified(self.page(), m.source, m.zoom, pens.dark);
                shown.blit(&big.crop(Rect::new(0, 0, m.end - m.start, h)), m.start, 0);
                view.ui_screen()
                    .render_scaled(&shown, palette, &self.ui, view.ux, view.uy)
            }
        };
        if self.ui.toolbox_visible {
            let x0 = view.ui_screen().toolbox_x();
            let on = [
                (Tool::Grid, self.tools.grid),
                (Tool::Symmetry, self.tools.symmetry),
                (Tool::Magnify, self.tools.magnify.is_some()),
            ];
            for (tool, _) in on.iter().filter(|(_, on)| *on) {
                toolbox::draw_toggle_on(&mut screen, x0, *tool, &pens, view.ux, view.uy);
            }
        }
        if let Some(Stroke {
            kind: Kind::Text { at, lines },
            color,
            ..
        }) = &self.stroke
        {
            let last = lines.last().map_or("", String::as_str);
            let x = at.0 + Font::builtin().text_width(last) + 1;
            let y = at.1 + (lines.len() as i32 - 1) * TEXT_LINE;
            // Normal view only; in the magnified half the typed text itself shows.
            let (sx, sy) = (x - self.scroll.0, y - self.scroll.1);
            let limit = self.magnify_layout(view).map_or(view.screen_w, |m| m.split);
            if (0..limit).contains(&sx) {
                screen.fill_rect(sx, sy, 1, font::GLYPH_H, *color);
            }
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
            let page = self.engine.document().page();
            if let Some(s) = &mut self.stroke {
                s.redraw(page);
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
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let commands: Vec<Command> = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    for (i, cmd) in commands.iter().enumerate() {
        engine
            .execute(cmd.clone())
            .map_err(|e| format!("{path}: command {i}: {e}"))?;
    }
    Ok(commands.len())
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
        assert_eq!(app.page(), &IndexedImage::new(320, 256, 0), "undo restores");
        app.redo();
        assert_eq!(app.page(), &painted, "redo repeats exactly");
        painted
    }

    #[test]
    fn line_rubber_band_is_the_committed_line() {
        let mut app = app(Tool::Line, false);
        press(&mut app, 10, 10);
        app.handle_drag(40, 30);
        app.handle_drag(50, 20);
        assert_eq!(app.page().get(40, 30), Some(0), "old rubber band erased");
        let preview = app.page().clone();
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
            let preview = app.page().clone();
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
        let preview = app.page().clone();
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
        let page = app.page().clone();
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
        let preview = app.page().clone();
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
        let preview = app.page().clone();
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
        let preview = app.page().clone();
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
}
