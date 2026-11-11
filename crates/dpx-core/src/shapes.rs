//! Drawing-tool shapes (ADR-14): the geometry of [`Command::Shape`], cyclic symmetry, and
//! [`Command::paint`], which draws a pixel command into any image. Front-ends preview a shape
//! by painting the candidate command into a copy of the page, so the preview is exactly what
//! the engine commits. Every pixel is painted through [`crate::paint::plot`] (ADR-18).

use serde::{Deserialize, Serialize};

use crate::anim_brush::AnimBrush;
use crate::command::{
    Command, CommandError, MAX_BRUSH_OFFSET, MAX_BRUSH_PIXELS, MAX_PAINT_WORK,
    POLYGON_CROSSING_WORK,
};
use crate::gradient;
use crate::image::{IndexedImage, Rect};
use crate::paint::PaintContext;
use crate::palette::Palette;
use crate::raster::{self, Pen, Stamper};

/// Largest absolute shape coordinate (points, centres) and radius.
pub const MAX_SHAPE_COORD: i32 = 16_384;
/// Most points of a polygon, freehand outline or dot set.
pub const MAX_SHAPE_POINTS: usize = 100_000;
/// Largest symmetry order (number of copies).
pub const MAX_SYMMETRY_ORDER: u8 = 64;
/// Largest pixel-shape component of [`Symmetry::aspect`].
pub const MAX_ASPECT: u8 = 4;

/// The geometry of a drawing-tool shape, in page pixels.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// Straight Line.
    Line { from: (i32, i32), to: (i32, i32) },
    /// Curve from `from` to `to` that passes through `through` (a quadratic curve, bent
    /// halfway through that point).
    Curve {
        from: (i32, i32),
        to: (i32, i32),
        through: (i32, i32),
    },
    /// Rectangle with the opposite corners `from` and `to` (both inclusive).
    Rect { from: (i32, i32), to: (i32, i32) },
    /// Ellipse around `center` with horizontal and vertical radii; equal radii = circle.
    Ellipse {
        center: (i32, i32),
        radius: (i32, i32),
    },
    /// Polygon or freehand outline through `points`; `closed` also joins the last point to
    /// the first. Filled shapes are always closed.
    Polygon {
        points: Vec<(i32, i32)>,
        #[serde(default)]
        closed: bool,
    },
    /// The brush stamped once at every point (airbrush spray, symmetric dotted freehand,
    /// text pixels); never filled.
    Dots { points: Vec<(i32, i32)> },
}

/// Point symmetry (the Symmetry tool). Cyclic (the default): `order` copies of the shape,
/// evenly rotated around `center`. With `mirror` (ADR-20): `order` rounded down to an even
/// number of copies (at least 2), half of them rotated and the other half their mirror images
/// (left <-> right), so order 2 is a plain mirror and order 4 mirrors both ways. Copies are
/// made in displayed space, so with Be Square they stay round on non-square pixels: `aspect`
/// is the displayed (width, height) of one pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symmetry {
    pub center: (i32, i32),
    pub order: u8,
    #[serde(default = "square_pixels")]
    pub aspect: (u8, u8),
    #[serde(default, skip_serializing_if = "is_false")]
    pub mirror: bool,
}

fn square_pixels() -> (u8, u8) {
    (1, 1)
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Symmetry {
    /// Number of copies including the original.
    pub fn count(&self) -> u8 {
        if self.mirror {
            (self.order & !1).max(2)
        } else {
            self.order.max(1)
        }
    }

    /// Copy `k` (0 = the original, unchanged) of point `p`.
    pub fn copy(&self, k: u8, p: (i32, i32)) -> (i32, i32) {
        let n = self.count();
        if k == 0 || n <= 1 {
            return p;
        }
        let (ax, ay) = (f64::from(self.aspect.0), f64::from(self.aspect.1));
        let mut dx = f64::from(p.0 - self.center.0) * ax;
        let dy = f64::from(p.1 - self.center.1) * ay;
        // Mirror: odd copies are reflected first; the rotation step is that of n/2 copies.
        let (turn, steps) = if self.mirror {
            if k % 2 == 1 {
                dx = -dx;
            }
            (k / 2, n / 2)
        } else {
            (k, n)
        };
        let angle = std::f64::consts::TAU * f64::from(turn) / f64::from(steps);
        let (s, c) = angle.sin_cos();
        (
            self.center.0 + ((dx * c - dy * s) / ax).round() as i32,
            self.center.1 + ((dx * s + dy * c) / ay).round() as i32,
        )
    }

    fn copies(sym: Option<&Symmetry>) -> u8 {
        sym.map_or(1, Symmetry::count)
    }
}

/// An outline as drawn: consecutive points joined by lines (or stamped one by one).
struct Path {
    points: Vec<(i32, i32)>,
    closed: bool,
    dots: bool,
}

impl Shape {
    fn path(&self, filled: bool) -> Path {
        let open = |points| Path {
            points,
            closed: filled,
            dots: false,
        };
        match self {
            Shape::Line { from, to } => open(vec![*from, *to]),
            Shape::Curve { from, to, through } => open(raster::curve_points(*from, *through, *to)),
            Shape::Rect { from, to } => Path {
                points: vec![*from, (to.0, from.1), *to, (from.0, to.1)],
                closed: true,
                dots: false,
            },
            Shape::Ellipse { center, radius } => Path {
                points: raster::ellipse_points(*center, radius.0, radius.1),
                closed: true,
                dots: false,
            },
            Shape::Polygon { points, closed } => Path {
                points: points.clone(),
                closed: *closed || filled,
                dots: false,
            },
            Shape::Dots { points } => Path {
                points: points.clone(),
                closed: false,
                dots: true,
            },
        }
    }

    /// Every coordinate the shape is defined by, for validation.
    fn coords(&self) -> Vec<(i32, i32)> {
        match self {
            Shape::Line { from, to } | Shape::Rect { from, to } => vec![*from, *to],
            Shape::Curve { from, to, through } => vec![*from, *to, *through],
            Shape::Ellipse { center, radius } => vec![*center, *radius],
            Shape::Polygon { points, .. } | Shape::Dots { points } => points.clone(),
        }
    }

    /// Whether the outline uses the brush (filled shapes are solid).
    fn uses_brush(&self, filled: bool) -> bool {
        !filled || matches!(self, Shape::Dots { .. })
    }

    /// `custom`: a custom brush replaces `brush`, which may then be empty.
    pub(crate) fn validate(
        &self,
        brush: &[(i32, i32)],
        custom: bool,
        filled: bool,
        symmetry: Option<&Symmetry>,
    ) -> Result<(), CommandError> {
        let invalid = |m: String| Err(CommandError::invalid(m));
        if let Shape::Polygon { points, .. } | Shape::Dots { points } = self
            && (points.is_empty() || points.len() > MAX_SHAPE_POINTS)
        {
            return invalid(format!(
                "a shape must have 1..={MAX_SHAPE_POINTS} points, got {}",
                points.len()
            ));
        }
        if let Shape::Ellipse { radius, .. } = self
            && (radius.0 < 0 || radius.1 < 0)
        {
            return invalid("ellipse radii must not be negative".into());
        }
        if self
            .coords()
            .iter()
            .any(|&(x, y)| x.abs() > MAX_SHAPE_COORD || y.abs() > MAX_SHAPE_COORD)
        {
            return invalid(format!(
                "shape coordinates must lie within ±{MAX_SHAPE_COORD}"
            ));
        }
        let need_brush = self.uses_brush(filled) && !custom;
        if (need_brush && brush.is_empty()) || brush.len() > MAX_BRUSH_PIXELS {
            return invalid(format!(
                "brush must have 1..={MAX_BRUSH_PIXELS} pixels, got {}",
                brush.len()
            ));
        }
        if brush
            .iter()
            .any(|&(x, y)| x.abs() > MAX_BRUSH_OFFSET || y.abs() > MAX_BRUSH_OFFSET)
        {
            return invalid(format!("brush offsets must lie within ±{MAX_BRUSH_OFFSET}"));
        }
        if let Some(s) = symmetry {
            if !(1..=MAX_SYMMETRY_ORDER).contains(&s.order) {
                return invalid(format!(
                    "symmetry order must be 1..={MAX_SYMMETRY_ORDER}, got {}",
                    s.order
                ));
            }
            if !(1..=MAX_ASPECT).contains(&s.aspect.0) || !(1..=MAX_ASPECT).contains(&s.aspect.1) {
                return invalid(format!("symmetry aspect must be 1..={MAX_ASPECT}"));
            }
            if s.center.0.abs() > MAX_SHAPE_COORD || s.center.1.abs() > MAX_SHAPE_COORD {
                return invalid(format!(
                    "symmetry centre must lie within ±{MAX_SHAPE_COORD}"
                ));
            }
        }
        Ok(())
    }

    /// The outline's points for each symmetric copy (copy 0 = the shape itself).
    fn copies(&self, filled: bool, symmetry: Option<&Symmetry>) -> (Path, Vec<Vec<(i32, i32)>>) {
        let path = self.path(filled);
        let copies = (0..Symmetry::copies(symmetry))
            .map(|k| match symmetry {
                Some(s) if k > 0 => path.points.iter().map(|&p| s.copy(k, p)).collect(),
                _ => path.points.clone(),
            })
            .collect();
        (path, copies)
    }

    /// Draws the shape (and its symmetric copies) into `img` with `pen`. Outlines and dots
    /// visit the copies point by point (each copy keeps its own stamp motion), so a shape
    /// painted in parts gives the same stamps in the same order as the whole shape.
    fn draw(&self, img: &mut IndexedImage, pen: &Pen, filled: bool, symmetry: Option<&Symmetry>) {
        let (path, copies) = self.copies(filled, symmetry);
        if filled && !path.dots {
            for points in &copies {
                raster::fill_polygon_pen(img, points, pen);
            }
            return;
        }
        let mut stampers: Vec<Stamper> = copies.iter().map(|_| Stamper::new(*pen)).collect();
        let n = path.points.len();
        if path.dots || n == 1 {
            for i in 0..n {
                for (s, points) in stampers.iter_mut().zip(&copies) {
                    s.set_count(i as u64);
                    s.at(img, points[i]);
                }
            }
            return;
        }
        // Lines between consecutive points, then back to the first when closed. Every copy
        // starts each line at the original's stamp count (an AnimBrush cel, ADR-36), so the
        // copies show the original's cels and a shape painted in parts stays the same.
        let close = (path.closed && n > 2).then_some((n - 1, 0));
        let mut count = 0;
        for (a, b) in (1..n).map(|i| (i - 1, i)).chain(close) {
            for (s, points) in stampers.iter_mut().zip(&copies) {
                s.set_count(count);
                s.line(img, points[a], points[b]);
            }
            count += u64::from(raster::line_steps(path.points[a], path.points[b])) + 1;
        }
    }

    /// Brush stamps [`Self::draw`] makes along the shape itself (not its symmetric copies),
    /// those off the page included: the stamps an AnimBrush moves on by (ADR-36). 0 for a
    /// filled shape, which uses no brush.
    fn stamps(&self, filled: bool) -> u64 {
        let path = self.path(filled);
        let n = path.points.len();
        if filled && !path.dots {
            return 0;
        }
        if path.dots || n == 1 {
            return n as u64;
        }
        let close = (path.closed && n > 2).then_some((n - 1, 0));
        (1..n)
            .map(|i| (i - 1, i))
            .chain(close)
            .map(|(a, b)| u64::from(raster::line_steps(path.points[a], path.points[b])) + 1)
            .sum()
    }

    /// What [`Self::draw`] costs on a `size` page at most, in work units (see
    /// [`MAX_PAINT_WORK`]); `per_pixel` = the work of one plotted pixel, `per_stamp` = that of
    /// one brush stamp, `reach` = where a stamp can touch the page. Copies are visited one at
    /// a time; the count stops early once it passes [`MAX_PAINT_WORK`].
    fn work(
        &self,
        filled: bool,
        symmetry: Option<&Symmetry>,
        (per_pixel, per_stamp): (u64, u64),
        reach: Rect,
        size: (u32, u32),
    ) -> u64 {
        let path = self.path(filled);
        let n = path.points.len() as u64;
        let mut copy = Vec::with_capacity(path.points.len());
        let mut total = 0u64;
        for k in 0..Symmetry::copies(symmetry) {
            copy.clear();
            match symmetry {
                Some(s) if k > 0 => copy.extend(path.points.iter().map(|&p| s.copy(k, p))),
                _ => copy.extend_from_slice(&path.points),
            }
            let work = if filled && !path.dots {
                raster::polygon_work(&copy, size, POLYGON_CROSSING_WORK, per_pixel)
            } else {
                raster::path_stamps(&copy, !path.dots, path.closed, reach)
                    .saturating_mul(per_stamp)
                    .saturating_add(n)
            };
            total = total.saturating_add(work);
            if total > MAX_PAINT_WORK {
                break;
            }
        }
        total
    }

    /// Unclipped bounding rect of every pixel [`Self::draw`] can touch; `extent` = the
    /// brush's pixels relative to its hotspot.
    fn bounds(&self, extent: Option<Rect>, filled: bool, symmetry: Option<&Symmetry>) -> Rect {
        let (_, copies) = self.copies(filled, symmetry);
        let mut all = Rect::default();
        for points in &copies {
            let r = if self.uses_brush(filled) {
                extent.and_then(|e| raster::extent_bounds(e, points))
            } else {
                Rect::from_points(points)
            };
            all = all.union(r.unwrap_or_default());
        }
        all
    }
}

impl Command {
    /// The Stroke or Shape that paints: the command itself, or the one an
    /// [`Command::AnimBrushPaint`] wraps.
    fn painting(&self) -> &Command {
        match self {
            Command::AnimBrushPaint { paint, .. } => paint,
            other => other,
        }
    }

    /// The cels of an [`Command::AnimBrushPaint`].
    fn cels(&self) -> Option<&AnimBrush> {
        match self {
            Command::AnimBrushPaint { brush, .. } => Some(brush),
            _ => None,
        }
    }

    /// Brush stamps a Stroke or Shape makes along its path (not counting symmetric copies),
    /// those off the page included; for an [`Command::AnimBrushPaint`] those of the command
    /// it wraps. A front-end moves its AnimBrush on by this many stamps after the command
    /// (ADR-36). 0 for other commands and filled shapes.
    pub fn stamps(&self) -> u64 {
        match self.painting() {
            Command::Stroke {
                points, continuous, ..
            } => {
                if *continuous && points.len() > 1 {
                    points
                        .windows(2)
                        .map(|w| u64::from(raster::line_steps(w[0], w[1])) + 1)
                        .sum()
                } else {
                    points.len() as u64
                }
            }
            Command::Shape { shape, filled, .. } => shape.stamps(*filled),
            _ => 0,
        }
    }

    /// The pen a Stroke or Shape paints with (ADR-18); `before` is the page before the
    /// command. For an [`Command::AnimBrushPaint`] the wrapped command's pen with the cels
    /// (ADR-36). `None` for other commands.
    pub fn pen<'a>(&'a self, before: &'a IndexedImage, palette: &'a Palette) -> Option<Pen<'a>> {
        if let Command::AnimBrushPaint { brush, paint } = self {
            let pen = paint.pen(before, palette)?;
            return Some(Pen {
                cels: Some(brush),
                ..pen
            });
        }
        let (brush, color, custom, mode, range, stencil, right, fx) = match self {
            Command::Stroke {
                brush,
                color,
                custom,
                mode,
                range,
                stencil,
                right,
                fx,
                ..
            }
            | Command::Shape {
                brush,
                color,
                custom,
                mode,
                range,
                stencil,
                right,
                fx,
                ..
            } => (brush, color, custom, mode, range, stencil, right, fx),
            _ => return None,
        };
        Some(Pen {
            offsets: brush,
            custom: custom.as_ref(),
            color: *color,
            mode: *mode,
            ctx: PaintContext {
                palette,
                range: *range,
                stencil: stencil.as_deref(),
                before,
                motion: (0, 0),
                right: *right,
            },
            fx: *fx,
            cels: None,
        })
    }

    /// The pixels of a Stroke's or Shape's brush relative to its hotspot (the custom brush if
    /// there is one, all cels of an AnimBrush); `None` for an empty brush and other commands.
    fn brush_extent(&self) -> Option<Rect> {
        if let Some(c) = self.cels() {
            return c.extent();
        }
        match self {
            Command::Stroke { brush, custom, .. } | Command::Shape { brush, custom, .. } => {
                match custom {
                    Some(b) => raster::brush_extent(b),
                    None => Rect::from_points(brush),
                }
            }
            _ => None,
        }
    }

    /// What painting this Stroke or Shape on a `width`×`height` page with a palette of
    /// `colors` entries costs at most, in work units (about one per brush pixel plotted; see
    /// [`MAX_PAINT_WORK`]). Stamps that cannot touch the page cost nothing. 0 for other
    /// commands.
    pub(crate) fn paint_work(&self, width: u32, height: u32, colors: usize) -> u64 {
        let (brush, custom, mode, fx) = match self.painting() {
            Command::Stroke {
                brush,
                custom,
                mode,
                fx,
                ..
            }
            | Command::Shape {
                brush,
                custom,
                mode,
                fx,
                ..
            } => (brush, custom.as_ref(), *mode, fx),
            _ => return 0,
        };
        let per_pixel = crate::paint::pixel_work(mode, fx, colors);
        // An AnimBrush costs what its largest cel costs per stamp (ADR-36).
        let stamp = match self.cels() {
            Some(c) => c
                .cels
                .iter()
                .map(|b| raster::stamp_cost(brush, Some(b), width, height))
                .max()
                .unwrap_or(0),
            None => raster::stamp_cost(brush, custom, width, height),
        };
        let per_stamp = stamp.saturating_mul(per_pixel);
        let reach = self
            .brush_extent()
            .map_or_else(Rect::default, |e| raster::reach(e, width, height));
        match self.painting() {
            Command::Stroke {
                points, continuous, ..
            } => raster::path_stamps(points, *continuous, false, reach)
                .saturating_mul(per_stamp)
                .saturating_add(points.len() as u64),
            Command::Shape {
                shape,
                filled,
                symmetry,
                ..
            } => shape.work(
                *filled,
                symmetry.as_ref(),
                (per_pixel, per_stamp),
                reach,
                (width, height),
            ),
            _ => unreachable!("only strokes and shapes have a brush"),
        }
    }

    /// Draws a page-pixel command (Stroke, Shape, FloodFill) into `img` exactly as the engine
    /// applies it; returns `false` (and does nothing) for other commands. Front-ends use it
    /// to preview a tool's candidate command on a copy of the page. `before` is the page
    /// before the command (the drawing modes and the stencil read it); a gesture previewed in
    /// parts passes the page before the gesture. `palette` is the document's. The command
    /// should be valid for the document (see [`crate::Engine::execute`]).
    pub fn paint(&self, img: &mut IndexedImage, before: &IndexedImage, palette: &Palette) -> bool {
        if let Some(pen) = self.pen(before, palette) {
            match self.painting() {
                Command::Stroke {
                    points, continuous, ..
                } => raster::stroke_pen(img, &pen, points, *continuous),
                Command::Shape {
                    shape,
                    filled,
                    symmetry,
                    ..
                } => shape.draw(img, &pen, *filled, symmetry.as_ref()),
                _ => unreachable!("only strokes and shapes have a pen"),
            }
            return true;
        }
        match self {
            Command::FloodFill { x, y, color, fill } => {
                gradient::flood_fill(img, *x, *y, *color, fill);
            }
            _ => return false,
        }
        true
    }

    /// The pixels of `page` that [`Self::paint`] can change, clipped to the page (empty if
    /// none); `None` for commands that are not page-pixel commands.
    pub fn paint_bounds(&self, page: &IndexedImage) -> Option<Rect> {
        let r = match self.painting() {
            Command::Stroke { points, .. } => self
                .brush_extent()
                .and_then(|e| raster::extent_bounds(e, points))
                .unwrap_or_default(),
            Command::Shape {
                shape,
                filled,
                symmetry,
                ..
            } => shape.bounds(self.brush_extent(), *filled, symmetry.as_ref()),
            Command::FloodFill { x, y, color, fill } => {
                gradient::flood_fill(&mut page.clone(), *x, *y, *color, fill).unwrap_or_default()
            }
            _ => return None,
        };
        Some(r.intersect(page.bounds()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::{BrushImage, DrawMode};

    /// The image as rows of `#` (colour 1) and `.` (colour 0); other colours as digits.
    fn rows(img: &IndexedImage) -> Vec<String> {
        (0..img.height() as i32)
            .map(|y| {
                (0..img.width() as i32)
                    .map(|x| match img.get(x, y) {
                        Some(0) => '.',
                        Some(1) => '#',
                        Some(c) => char::from_digit(u32::from(c), 36).unwrap_or('?'),
                        None => '?',
                    })
                    .collect()
            })
            .collect()
    }

    fn shape(shape: Shape, filled: bool) -> Command {
        Command::Shape {
            shape,
            brush: vec![(0, 0)],
            color: 1,
            filled,
            symmetry: None,
            custom: None,
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    /// [`Command::paint`] on `img`, with `img` as the page before and the default palette.
    fn paint(cmd: &Command, img: &mut IndexedImage) -> bool {
        let before = img.clone();
        cmd.paint(img, &before, &Palette::dpaint_default_32())
    }

    fn painted(w: u32, h: u32, cmd: &Command) -> Vec<String> {
        let mut img = IndexedImage::new(w, h, 0);
        assert!(paint(cmd, &mut img));
        // The bounds cover every changed pixel.
        let r = cmd.paint_bounds(&IndexedImage::new(w, h, 0)).unwrap();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                if img.get(x, y) != Some(0) {
                    assert!(r.contains(x, y), "({x},{y}) outside {r:?} for {cmd:?}");
                }
            }
        }
        rows(&img)
    }

    #[test]
    fn line_and_rect_pixels() {
        let line = shape(
            Shape::Line {
                from: (0, 0),
                to: (5, 2),
            },
            false,
        );
        assert_eq!(
            painted(6, 3, &line),
            ["##....", "..##..", "....##"].map(String::from)
        );
        let rect = |filled| {
            shape(
                Shape::Rect {
                    from: (4, 3),
                    to: (1, 1),
                },
                filled,
            )
        };
        assert_eq!(
            painted(6, 5, &rect(false)),
            ["......", ".####.", ".#..#.", ".####.", "......"].map(String::from)
        );
        assert_eq!(
            painted(6, 5, &rect(true)),
            ["......", ".####.", ".####.", ".####.", "......"].map(String::from)
        );
    }

    #[test]
    fn circle_and_ellipse_pixels() {
        let circle = |r, filled| {
            shape(
                Shape::Ellipse {
                    center: (3, 3),
                    radius: (r, r),
                },
                filled,
            )
        };
        assert_eq!(
            painted(7, 7, &circle(1, false)),
            [
                ".......", ".......", "...#...", "..#.#..", "...#...", ".......", "......."
            ]
            .map(String::from)
        );
        assert_eq!(
            painted(7, 7, &circle(3, false)),
            [
                "..###..", ".#...#.", "#.....#", "#.....#", "#.....#", ".#...#.", "..###.."
            ]
            .map(String::from)
        );
        assert_eq!(
            painted(7, 7, &circle(3, true)),
            [
                "..###..", ".#####.", "#######", "#######", "#######", ".#####.", "..###.."
            ]
            .map(String::from)
        );
        let ellipse = shape(
            Shape::Ellipse {
                center: (4, 2),
                radius: (4, 2),
            },
            false,
        );
        assert_eq!(
            painted(9, 5, &ellipse),
            [
                "..#####..",
                ".#.....#.",
                "#.......#",
                ".#.....#.",
                "..#####.."
            ]
            .map(String::from)
        );
        // A zero radius is a line.
        let flat = shape(
            Shape::Ellipse {
                center: (2, 1),
                radius: (2, 0),
            },
            true,
        );
        assert_eq!(
            painted(5, 3, &flat),
            [".....", "#####", "....."].map(String::from)
        );
    }

    #[test]
    fn polygon_outline_open_closed_and_filled() {
        let tri = |closed, filled| {
            shape(
                Shape::Polygon {
                    points: vec![(0, 0), (4, 0), (0, 4)],
                    closed,
                },
                filled,
            )
        };
        assert_eq!(
            painted(5, 5, &tri(false, false)),
            ["#####", "...#.", "..#..", ".#...", "#...."].map(String::from)
        );
        assert_eq!(
            painted(5, 5, &tri(true, false)),
            ["#####", "#..#.", "#.#..", "##...", "#...."].map(String::from)
        );
        // Filled ignores `closed` and includes the edges.
        assert_eq!(
            painted(5, 5, &tri(false, true)),
            ["#####", "####.", "###..", "##...", "#...."].map(String::from)
        );
        // Even-odd: the inner square of a doubled outline stays empty.
        let ring = shape(
            Shape::Polygon {
                points: vec![
                    (0, 0),
                    (6, 0),
                    (6, 6),
                    (0, 6),
                    (0, 0),
                    (2, 2),
                    (2, 4),
                    (4, 4),
                    (4, 2),
                    (2, 2),
                ],
                closed: true,
            },
            true,
        );
        assert_eq!(painted(7, 7, &ring)[3], "###.###");
    }

    #[test]
    fn curve_passes_through_its_bend_point() {
        let pts = raster::curve_points((0, 6), (4, 0), (8, 6));
        assert_eq!(pts.first(), Some(&(0, 6)));
        assert_eq!(pts.last(), Some(&(8, 6)));
        assert!(pts.contains(&(4, 0)), "{pts:?}");
        let curve = shape(
            Shape::Curve {
                from: (0, 6),
                to: (8, 6),
                through: (4, 0),
            },
            false,
        );
        assert_eq!(
            painted(9, 7, &curve),
            [
                "....#....",
                "...#.#...",
                "..#...#..",
                ".#.....#.",
                ".#.....#.",
                "#.......#",
                "#.......#"
            ]
            .map(String::from)
        );
        // A bend point on the chord gives the straight line.
        let straight = raster::curve_points((0, 0), (3, 1), (6, 2));
        let mut line = IndexedImage::new(7, 3, 0);
        raster::stroke(&mut line, &[(0, 0)], 1, &straight, true);
        let mut expect = IndexedImage::new(7, 3, 0);
        raster::stroke(&mut expect, &[(0, 0)], 1, &[(0, 0), (6, 2)], true);
        assert_eq!(rows(&line), rows(&expect));
    }

    #[test]
    fn flood_fill_is_four_connected() {
        let mut img = IndexedImage::new(5, 5, 0);
        // A closed box plus a diagonal gap at the bottom-right corner.
        for (x, y) in [
            (1, 1),
            (2, 1),
            (3, 1),
            (1, 2),
            (3, 2),
            (1, 3),
            (2, 3),
            (3, 3),
        ] {
            img.set(x, y, 1);
        }
        let cmd = Command::FloodFill {
            x: 2,
            y: 2,
            color: 2,
            fill: crate::gradient::FillType::Solid,
        };
        assert_eq!(cmd.paint_bounds(&img), Some(Rect::new(2, 2, 1, 1)));
        paint(&cmd, &mut img);
        assert_eq!(
            rows(&img),
            [".....", ".###.", ".#2#.", ".###.", "....."].map(String::from)
        );
        // The outside region flows around the box but not through its corner diagonals.
        assert_eq!(raster::flood_fill(&mut img, 0, 0, 3), Some(img.bounds()));
        assert_eq!(rows(&img)[2], "3#2#3");
        assert_eq!(raster::flood_fill(&mut img, 0, 0, 3), None, "same colour");
        assert_eq!(raster::flood_fill(&mut img, 9, 0, 3), None, "outside");
    }

    #[test]
    fn symmetry_draws_rotated_copies() {
        let sym = Symmetry {
            center: (4, 4),
            order: 4,
            aspect: (1, 1),
            mirror: false,
        };
        assert_eq!(sym.copy(0, (6, 3)), (6, 3));
        assert_eq!(sym.copy(1, (6, 4)), (4, 6));
        assert_eq!(sym.copy(2, (6, 4)), (2, 4));
        assert_eq!(sym.copy(3, (6, 4)), (4, 2));
        let mut cmd = shape(
            Shape::Dots {
                points: vec![(7, 4)],
            },
            false,
        );
        if let Command::Shape { symmetry, .. } = &mut cmd {
            *symmetry = Some(sym);
        }
        assert_eq!(
            painted(9, 9, &cmd),
            [
                ".........",
                "....#....",
                ".........",
                ".........",
                ".#.....#.",
                ".........",
                ".........",
                "....#....",
                "........."
            ]
            .map(String::from)
        );
        // Tall pixels (HiRes): a quarter turn of 2 pixels right is 1 pixel down.
        let tall = Symmetry {
            aspect: (1, 2),
            ..sym
        };
        assert_eq!(tall.copy(1, (6, 4)), (4, 5));
    }

    #[test]
    fn mirror_symmetry_reflects_and_rotates() {
        let m = |order| Symmetry {
            center: (10, 10),
            order,
            aspect: (1, 1),
            mirror: true,
        };
        // Order 2: a left <-> right mirror only.
        let two = m(2);
        assert_eq!(two.count(), 2);
        assert_eq!(two.copy(1, (14, 3)), (6, 3));
        // Order 4: mirror both ways (copy 2 = half turn, copy 3 = up <-> down).
        let four = m(4);
        assert_eq!(four.copy(1, (14, 3)), (6, 3));
        assert_eq!(four.copy(2, (14, 3)), (6, 17));
        assert_eq!(four.copy(3, (14, 3)), (14, 17));
        // Odd orders round down to an even number of copies; at least two.
        assert_eq!(m(5).count(), 4);
        assert_eq!(m(1).count(), 2);
        // Cyclic is unchanged and `mirror` stays out of the JSON unless set.
        let cyc = Symmetry {
            mirror: false,
            ..four
        };
        assert_eq!(cyc.count(), 4);
        assert_eq!(cyc.copy(1, (14, 10)), (10, 14));
        assert!(!serde_json::to_string(&cyc).unwrap().contains("mirror"));
        let back: Symmetry = serde_json::from_str(&serde_json::to_string(&four).unwrap()).unwrap();
        assert_eq!(back, four);
        // A mirrored shape draws every copy.
        let cmd = Command::Shape {
            shape: Shape::Dots {
                points: vec![(13, 8)],
            },
            brush: vec![(0, 0)],
            color: 1,
            filled: false,
            symmetry: Some(four),
            custom: None,
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        };
        let mut img = IndexedImage::new(21, 21, 0);
        let before = img.clone();
        cmd.paint(&mut img, &before, &Palette::dpaint_default_32());
        for p in [(13, 8), (7, 8), (7, 12), (13, 12)] {
            assert_eq!(img.get(p.0, p.1), Some(1), "{p:?}");
        }
        assert_eq!(img.pixels().iter().filter(|&&p| p == 1).count(), 4);
    }

    #[test]
    fn json_shape_is_stable() {
        let cmd: Command = serde_json::from_str(
            r#"{"op":"shape","shape":{"kind":"rect","from":[1,2],"to":[3,4]},"color":5,"filled":true}"#,
        )
        .unwrap();
        assert_eq!(
            cmd,
            Command::Shape {
                shape: Shape::Rect {
                    from: (1, 2),
                    to: (3, 4)
                },
                brush: vec![],
                color: 5,
                filled: true,
                symmetry: None,
                custom: None,
                mode: DrawMode::Matte,
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            }
        );
        assert_eq!(
            serde_json::to_string(&cmd).unwrap(),
            r#"{"op":"shape","shape":{"kind":"rect","from":[1,2],"to":[3,4]},"brush":[],"color":5,"filled":true}"#,
            "unset paint fields are left out"
        );
        let fill = Command::FloodFill {
            x: 1,
            y: 2,
            color: 3,
            fill: crate::gradient::FillType::Solid,
        };
        assert_eq!(
            serde_json::to_string(&fill).unwrap(),
            r#"{"op":"flood_fill","x":1,"y":2,"color":3}"#
        );
        let sym: Symmetry = serde_json::from_str(r#"{"center":[160,128],"order":6}"#).unwrap();
        assert_eq!(sym.aspect, (1, 1));
    }

    /// A 3×2 custom brush with colour 0 transparent: `.5.` over `66.`; handle (1, 1).
    fn brush() -> BrushImage {
        BrushImage::from_image(
            &IndexedImage::from_pixels(3, 2, vec![0, 5, 0, 6, 6, 0]).unwrap(),
            Some(0),
        )
    }

    /// One stamp of `brush()` at (2, 2) on a 5×4 page of colour 9.
    fn stamped(mode: DrawMode, color: u8, right: bool, stencil: Option<Vec<bool>>) -> Vec<String> {
        let mut cmd = shape(
            Shape::Dots {
                points: vec![(2, 2)],
            },
            false,
        );
        if let Command::Shape {
            brush: offsets,
            color: c,
            custom,
            mode: m,
            stencil: s,
            right: r,
            ..
        } = &mut cmd
        {
            offsets.clear();
            *c = color;
            *custom = Some(brush());
            *m = mode;
            *s = stencil;
            *r = right;
        }
        let mut img = IndexedImage::new(5, 4, 9);
        img.set(3, 1, 1);
        assert!(paint(&cmd, &mut img));
        rows(&img)
    }

    #[test]
    fn custom_brush_matte_color_and_replace() {
        // Matte: the brush's own colours; its transparent colour 0 lets the page show.
        assert_eq!(
            stamped(DrawMode::Matte, 3, false, None),
            ["99999", "995#9", "96699", "99999"].map(String::from)
        );
        // Color: the brush's shape in the given colour (FG).
        assert_eq!(
            stamped(DrawMode::Color, 3, false, None),
            ["99999", "993#9", "93399", "99999"].map(String::from)
        );
        // Replace: the whole rectangle, transparent pixels in their own colour (the BG of
        // the pick-up).
        assert_eq!(
            stamped(DrawMode::Replace, 3, false, None),
            ["99999", "9.5.9", "966.9", "99999"].map(String::from)
        );
        // Right button (colour = BG 4): Matte and Color paint the shape in BG, Replace the
        // whole rectangle.
        for mode in [DrawMode::Matte, DrawMode::Color] {
            assert_eq!(
                stamped(mode, 4, true, None),
                ["99999", "994#9", "94499", "99999"].map(String::from),
                "{mode:?}"
            );
        }
        assert_eq!(
            stamped(DrawMode::Replace, 4, true, None),
            ["99999", "94449", "94449", "99999"].map(String::from)
        );
    }

    #[test]
    fn the_stencil_protects_colours_from_every_path() {
        let mut stencil = vec![false; 256];
        stencil[9] = true;
        // Only the pixel of colour 1 is not protected.
        assert_eq!(
            stamped(DrawMode::Replace, 3, false, Some(stencil.clone())),
            ["99999", "999.9", "99999", "99999"].map(String::from)
        );
        // Filled shapes and outlines with a built-in brush too.
        for filled in [true, false] {
            let mut cmd = shape(
                Shape::Rect {
                    from: (0, 0),
                    to: (4, 3),
                },
                filled,
            );
            if let Command::Shape { stencil: s, .. } = &mut cmd {
                *s = Some(stencil.clone());
            }
            let mut img = IndexedImage::new(5, 4, 9);
            img.fill_rect(0, 0, 2, 4, 2);
            paint(&cmd, &mut img);
            assert_eq!(img.get(4, 0), Some(9), "protected");
            assert_eq!(img.get(0, 0), Some(1), "painted");
            assert_eq!(img.get(1, 1), Some(if filled { 1 } else { 2 }));
        }
    }

    #[test]
    fn a_custom_brush_painted_in_parts_equals_the_whole_shape() {
        let sym = Symmetry {
            center: (16, 16),
            order: 5,
            aspect: (1, 1),
            mirror: false,
        };
        let pts = vec![(20, 18), (26, 10), (12, 24), (30, 30), (21, 5)];
        let with_brush = |outline: Shape| {
            let mut cmd = shape(outline, false);
            if let Command::Shape {
                custom, symmetry, ..
            } = &mut cmd
            {
                *custom = Some(brush());
                *symmetry = Some(sym);
            }
            cmd
        };
        for dots in [true, false] {
            let make = |points: Vec<(i32, i32)>| {
                with_brush(if dots {
                    Shape::Dots { points }
                } else {
                    Shape::Polygon {
                        points,
                        closed: false,
                    }
                })
            };
            let page = IndexedImage::new(32, 32, 0);
            let mut whole = page.clone();
            make(pts.clone()).paint(&mut whole, &page, &Palette::dpaint_default_32());
            let mut parts = page.clone();
            for k in 0..pts.len() {
                let part = if dots {
                    vec![pts[k]]
                } else {
                    pts[k.saturating_sub(1)..=k].to_vec()
                };
                make(part).paint(&mut parts, &page, &Palette::dpaint_default_32());
            }
            assert_eq!(rows(&parts), rows(&whole), "dots={dots}");
            // The bounds cover every stamp of every copy.
            let r = make(pts.clone()).paint_bounds(&page).unwrap();
            for y in 0..32 {
                for x in 0..32 {
                    if whole.get(x, y) != Some(0) {
                        assert!(r.contains(x, y), "({x},{y}) outside {r:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn custom_brush_json_round_trips() {
        let mut cmd = shape(
            Shape::Dots {
                points: vec![(1, 1)],
            },
            false,
        );
        if let Command::Shape {
            custom,
            mode,
            range,
            right,
            ..
        } = &mut cmd
        {
            *custom = Some(brush());
            *mode = DrawMode::Replace;
            *range = Some((2, 7));
            *right = true;
        }
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(
            json.contains(r#""mode":"replace""#) && json.contains(r#""right":true"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
    }

    #[test]
    fn magnified_view() {
        let mut img = IndexedImage::new(2, 1, 0);
        img.set(1, 0, 1);
        let m = raster::magnified(&img, Rect::new(0, 0, 3, 1), 2, 7);
        assert_eq!(rows(&m), ["..##77", "..##77"].map(String::from));
    }
}
