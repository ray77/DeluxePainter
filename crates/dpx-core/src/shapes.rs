//! Drawing-tool shapes (ADR-14): the geometry of [`Command::Shape`], cyclic symmetry, and
//! [`Command::paint`], which draws a pixel command into any image. Front-ends preview a shape
//! by painting the candidate command into a copy of the page, so the preview is exactly what
//! the engine commits.

use serde::{Deserialize, Serialize};

use crate::command::{Command, CommandError, MAX_BRUSH_OFFSET, MAX_BRUSH_PIXELS};
use crate::image::{IndexedImage, Rect};
use crate::raster;

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

/// Cyclic point symmetry (the Symmetry tool): `order` copies of the shape, evenly rotated
/// around `center`. Copies are rotated in displayed space, so with Be Square they stay round
/// on non-square pixels: `aspect` is the displayed (width, height) of one pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symmetry {
    pub center: (i32, i32),
    pub order: u8,
    #[serde(default = "square_pixels")]
    pub aspect: (u8, u8),
}

fn square_pixels() -> (u8, u8) {
    (1, 1)
}

impl Symmetry {
    /// Copy `k` (0 = the original, unchanged) of point `p`.
    pub fn copy(&self, k: u8, p: (i32, i32)) -> (i32, i32) {
        if k == 0 || self.order <= 1 {
            return p;
        }
        let (ax, ay) = (f64::from(self.aspect.0), f64::from(self.aspect.1));
        let angle = std::f64::consts::TAU * f64::from(k) / f64::from(self.order);
        let (s, c) = angle.sin_cos();
        let dx = f64::from(p.0 - self.center.0) * ax;
        let dy = f64::from(p.1 - self.center.1) * ay;
        (
            self.center.0 + ((dx * c - dy * s) / ax).round() as i32,
            self.center.1 + ((dx * s + dy * c) / ay).round() as i32,
        )
    }

    fn copies(sym: Option<&Symmetry>) -> u8 {
        sym.map_or(1, |s| s.order.max(1))
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

    pub(crate) fn validate(
        &self,
        brush: &[(i32, i32)],
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
        let need_brush = self.uses_brush(filled);
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

    /// Draws the shape (and its symmetric copies) into `img`.
    fn draw(
        &self,
        img: &mut IndexedImage,
        brush: &[(i32, i32)],
        color: u8,
        filled: bool,
        symmetry: Option<&Symmetry>,
    ) {
        let path = self.path(filled);
        for k in 0..Symmetry::copies(symmetry) {
            let points: Vec<(i32, i32)> = match symmetry {
                Some(s) if k > 0 => path.points.iter().map(|&p| s.copy(k, p)).collect(),
                _ => path.points.clone(),
            };
            if path.dots {
                for &(x, y) in &points {
                    raster::stamp(img, brush, x, y, color);
                }
            } else if filled {
                raster::fill_polygon(img, &points, color);
            } else {
                stroke_path(img, brush, color, &points, path.closed);
            }
        }
    }

    /// Unclipped bounding rect of every pixel [`Self::draw`] can touch.
    fn bounds(&self, brush: &[(i32, i32)], filled: bool, symmetry: Option<&Symmetry>) -> Rect {
        let path = self.path(filled);
        let mut all = Rect::default();
        for k in 0..Symmetry::copies(symmetry) {
            let points: Vec<(i32, i32)> = match symmetry {
                Some(s) if k > 0 => path.points.iter().map(|&p| s.copy(k, p)).collect(),
                _ => path.points.clone(),
            };
            let r = if self.uses_brush(filled) {
                raster::stroke_bounds(brush, &points)
            } else {
                Rect::from_points(&points)
            };
            all = all.union(r.unwrap_or_default());
        }
        all
    }
}

/// Stamps `brush` along the lines through `points` (and back to the first when `closed`).
fn stroke_path(
    img: &mut IndexedImage,
    brush: &[(i32, i32)],
    color: u8,
    points: &[(i32, i32)],
    closed: bool,
) {
    raster::stroke(img, brush, color, points, true);
    if closed && points.len() > 2 {
        raster::stroke(
            img,
            brush,
            color,
            &[points[points.len() - 1], points[0]],
            true,
        );
    }
}

impl Command {
    /// Draws a page-pixel command (Stroke, Shape, FloodFill) into `img` exactly as the engine
    /// applies it; returns `false` (and does nothing) for other commands. Front-ends use it
    /// to preview a tool's candidate command on a copy of the page. The command should be
    /// valid for the document (see [`crate::Engine::execute`]).
    pub fn paint(&self, img: &mut IndexedImage) -> bool {
        match self {
            Command::Stroke {
                brush,
                color,
                points,
                continuous,
            } => raster::stroke(img, brush, *color, points, *continuous),
            Command::Shape {
                shape,
                brush,
                color,
                filled,
                symmetry,
            } => shape.draw(img, brush, *color, *filled, symmetry.as_ref()),
            Command::FloodFill { x, y, color } => {
                raster::flood_fill(img, *x, *y, *color);
            }
            _ => return false,
        }
        true
    }

    /// The pixels of `page` that [`Self::paint`] can change, clipped to the page (empty if
    /// none); `None` for commands that are not page-pixel commands.
    pub fn paint_bounds(&self, page: &IndexedImage) -> Option<Rect> {
        let r = match self {
            Command::Stroke { brush, points, .. } => {
                raster::stroke_bounds(brush, points).unwrap_or_default()
            }
            Command::Shape {
                shape,
                brush,
                filled,
                symmetry,
                ..
            } => shape.bounds(brush, *filled, symmetry.as_ref()),
            Command::FloodFill { x, y, color } => {
                raster::flood_fill(&mut page.clone(), *x, *y, *color).unwrap_or_default()
            }
            _ => return None,
        };
        Some(r.intersect(page.bounds()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }

    fn painted(w: u32, h: u32, cmd: &Command) -> Vec<String> {
        let mut img = IndexedImage::new(w, h, 0);
        assert!(cmd.paint(&mut img));
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
        };
        assert_eq!(cmd.paint_bounds(&img), Some(Rect::new(2, 2, 1, 1)));
        cmd.paint(&mut img);
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
        };
        assert_eq!(sym.copy(0, (6, 3)), (6, 3));
        assert_eq!(sym.copy(1, (6, 4)), (4, 6));
        assert_eq!(sym.copy(2, (6, 4)), (2, 4));
        assert_eq!(sym.copy(3, (6, 4)), (4, 2));
        let cmd = Command::Shape {
            shape: Shape::Dots {
                points: vec![(7, 4)],
            },
            brush: vec![(0, 0)],
            color: 1,
            filled: false,
            symmetry: Some(sym),
        };
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
            }
        );
        let fill = Command::FloodFill {
            x: 1,
            y: 2,
            color: 3,
        };
        assert_eq!(
            serde_json::to_string(&fill).unwrap(),
            r#"{"op":"flood_fill","x":1,"y":2,"color":3}"#
        );
        let sym: Symmetry = serde_json::from_str(r#"{"center":[160,128],"order":6}"#).unwrap();
        assert_eq!(sym.aspect, (1, 1));
    }

    #[test]
    fn magnified_view() {
        let mut img = IndexedImage::new(2, 1, 0);
        img.set(1, 0, 1);
        let m = raster::magnified(&img, Rect::new(0, 0, 3, 1), 2, 7);
        assert_eq!(rows(&m), ["..##77", "..##77"].map(String::from));
    }
}
