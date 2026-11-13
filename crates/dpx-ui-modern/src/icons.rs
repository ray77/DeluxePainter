//! The Modern skin's own vector icons (ADR-47): path data on a 24-unit grid, drawn with
//! round-capped strokes [`STROKE`] units wide, with filled parts where a tool has a filled
//! variant. Every icon is designed here from what its tool does; none is traced from the
//! classic 9×9 bitmaps, the original program or its manuals.
//!
//! Split tools (outline in the upper-left half of the button, filled in the lower-right half,
//! as in DeluxePaint) show the outline of their shape with the lower-right half filled.

use dpx_ui::Tool;
use dpx_ui::toolbox::BUILTIN_BRUSHES;

use crate::paint::Path;

/// Side of the icon grid.
pub const GRID: f32 = 24.0;
/// Stroke width in grid units.
pub const STROKE: f32 = 1.75;

/// How one part of an icon is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// A round-capped line [`STROKE`] wide.
    Stroke,
    /// A filled area.
    Fill,
}

/// One part of an icon, in grid units.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub path: Path,
    pub ink: Ink,
}

/// An icon: its parts in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub parts: Vec<Part>,
}

impl Icon {
    fn new() -> Self {
        Icon { parts: Vec::new() }
    }

    fn stroke(mut self, path: Path) -> Self {
        self.parts.push(Part {
            path,
            ink: Ink::Stroke,
        });
        self
    }

    fn fill(mut self, path: Path) -> Self {
        self.parts.push(Part {
            path,
            ink: Ink::Fill,
        });
        self
    }
}

/// The part of a convex or concave polygon on the lower-right side of the anti-diagonal
/// through the grid's centre (`x + y >= 24`), the filled half of a split button.
fn lower_right(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let side = |(x, y): (f32, f32)| x + y - GRID;
    let mut out = Vec::new();
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            out.push(a);
        }
        // A corner on the diagonal itself is kept above and is no crossing.
        if sa * sb < 0.0 {
            let t = sa / (sa - sb);
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    out
}

/// The half of an ellipse around the grid's centre that lies on the lower-right side of the
/// anti-diagonal.
fn lower_right_ellipse(rx: f32, ry: f32) -> Path {
    // The point of the ellipse in the direction of the upper-right corner.
    let start = (-rx).atan2(ry).to_degrees();
    Path::new()
        .arc(GRID / 2.0, GRID / 2.0, rx, ry, start, 180.0)
        .close()
}

/// A point of the cubic Bézier `p` at parameter `t`.
fn cubic_at(p: [(f32, f32); 4], t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    (
        (0..4).map(|i| w[i] * p[i].0).sum(),
        (0..4).map(|i| w[i] * p[i].1).sum(),
    )
}

/// `count` points at equal distances along the chain of cubic Béziers `curves`, from its
/// start to its end.
fn evenly_spaced(curves: &[[(f32, f32); 4]], count: usize) -> Vec<(f32, f32)> {
    const STEPS: usize = 48;
    let line: Vec<(f32, f32)> = curves
        .iter()
        .flat_map(|c| (0..=STEPS).map(move |i| cubic_at(*c, i as f32 / STEPS as f32)))
        .collect();
    let dist = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    // The distance walked up to each point of the polyline.
    let mut walked = vec![0.0f32];
    for w in line.windows(2) {
        walked.push(walked[walked.len() - 1] + dist(w[0], w[1]));
    }
    let total = walked[walked.len() - 1];
    (0..count)
        .map(|k| {
            let at = total * k as f32 / (count.max(2) - 1) as f32;
            let i = walked
                .windows(2)
                .position(|w| at <= w[1])
                .unwrap_or(line.len() - 2);
            let len = walked[i + 1] - walked[i];
            let t = if len > 0.0 {
                ((at - walked[i]) / len).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (a, b) = (line[i], line[i + 1]);
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        })
        .collect()
}

/// The corners of the pentagon of the Polygon tool.
fn pentagon() -> Vec<(f32, f32)> {
    (0..5)
        .map(|i| {
            let a = (-90.0 + 72.0 * i as f32).to_radians();
            (12.0 + 9.0 * a.cos(), 12.7 + 9.0 * a.sin())
        })
        .collect()
}

/// The icon of `tool`.
pub fn tool(tool: Tool) -> Icon {
    match tool {
        // A wave made of separate dots, evenly spaced along it.
        Tool::DottedFreehand => {
            let wave = [
                [(4.0, 15.5), (6.5, 5.0), (10.0, 6.0), (12.0, 12.0)],
                [(12.0, 12.0), (14.0, 18.0), (17.5, 19.0), (20.0, 8.5)],
            ];
            let dots = evenly_spaced(&wave, 8)
                .into_iter()
                .fold(Path::new(), |p, (x, y)| p.circle(x, y, 1.25));
            Icon::new().fill(dots)
        }
        // A wave drawn in one go in the outline half, a filled freehand shape in the filled
        // half.
        Tool::ContinuousFreehand => Icon::new()
            .stroke(
                Path::new()
                    .move_to(3.5, 13.0)
                    .cubic_to(4.0, 7.0, 7.5, 5.5, 9.0, 9.0)
                    .cubic_to(10.5, 12.5, 13.5, 11.0, 14.5, 4.5),
            )
            .fill(
                Path::new()
                    .move_to(12.5, 17.6)
                    .cubic_to(12.5, 14.6, 15.0, 12.9, 17.6, 13.1)
                    .cubic_to(20.3, 13.3, 21.7, 15.3, 21.3, 17.7)
                    .cubic_to(20.9, 20.1, 18.7, 21.4, 16.1, 21.1)
                    .cubic_to(13.9, 20.8, 12.5, 19.6, 12.5, 17.6)
                    .close(),
            ),
        Tool::Line => Icon::new().stroke(Path::new().segment(5.0, 19.0, 19.0, 5.0)),
        // An arc between two marked end points.
        Tool::Curve => Icon::new()
            .stroke(
                Path::new()
                    .move_to(5.0, 19.0)
                    .cubic_to(5.5, 10.0, 10.5, 5.5, 19.0, 5.0),
            )
            .fill(Path::new().circle(5.0, 19.0, 2.2).circle(19.0, 5.0, 2.2)),
        // A tilted paint pot, half full, with a drop falling from its lip.
        Tool::Fill => Icon::new()
            .fill(Path::new().polygon(&[(5.2, 13.2), (17.8, 13.2), (11.5, 19.5)]))
            .stroke(Path::new().polygon(&[(10.5, 4.5), (18.5, 12.5), (11.5, 19.5), (3.5, 11.5)]))
            .stroke(Path::new().segment(10.5, 4.5, 8.2, 2.2))
            .fill(
                Path::new()
                    .move_to(20.2, 14.8)
                    .cubic_to(21.7, 17.0, 22.3, 18.2, 22.3, 19.2)
                    .cubic_to(22.3, 20.4, 21.4, 21.3, 20.2, 21.3)
                    .cubic_to(19.0, 21.3, 18.1, 20.4, 18.1, 19.2)
                    .cubic_to(18.1, 18.2, 18.7, 17.0, 20.2, 14.8)
                    .close(),
            ),
        // A spray can and the cone of paint it throws.
        Tool::Airbrush => Icon::new()
            .stroke(Path::new().round_rect(crate::paint::Rect::new(3.5, 10.5, 8.0, 10.5), 1.8))
            .stroke(
                Path::new()
                    .move_to(5.7, 10.5)
                    .line_to(5.7, 7.0)
                    .line_to(9.3, 7.0)
                    .line_to(9.3, 10.5),
            )
            .fill(
                [
                    (13.2, 7.4),
                    (16.4, 4.8),
                    (16.6, 9.4),
                    (19.8, 2.8),
                    (20.2, 7.0),
                    (20.0, 11.4),
                ]
                .into_iter()
                .fold(Path::new(), |p, (x, y)| p.circle(x, y, 1.05)),
            ),
        Tool::Rectangle => {
            let corners = [(4.5, 4.5), (19.5, 4.5), (19.5, 19.5), (4.5, 19.5)];
            Icon::new()
                .fill(Path::new().polygon(&lower_right(&corners)))
                .stroke(Path::new().polygon(&corners))
        }
        Tool::Circle => Icon::new()
            .fill(lower_right_ellipse(8.0, 8.0))
            .stroke(Path::new().circle(12.0, 12.0, 8.0)),
        Tool::Ellipse => Icon::new()
            .fill(lower_right_ellipse(9.0, 5.8))
            .stroke(Path::new().ellipse(12.0, 12.0, 9.0, 5.8)),
        Tool::Polygon => {
            let corners = pentagon();
            Icon::new()
                .fill(Path::new().polygon(&lower_right(&corners)))
                .stroke(Path::new().polygon(&corners))
        }
        // The dashed frame that picks up a brush.
        Tool::BrushSelect => {
            let (a, b, c) = (4.0, 20.0, 3.6);
            Icon::new().stroke(
                Path::new()
                    .move_to(a, a + c)
                    .line_to(a, a)
                    .line_to(a + c, a)
                    .move_to(b - c, a)
                    .line_to(b, a)
                    .line_to(b, a + c)
                    .move_to(b, b - c)
                    .line_to(b, b)
                    .line_to(b - c, b)
                    .move_to(a + c, b)
                    .line_to(a, b)
                    .line_to(a, b - c)
                    .segment(10.4, a, 13.6, a)
                    .segment(10.4, b, 13.6, b)
                    .segment(a, 10.4, a, 13.6)
                    .segment(b, 10.4, b, 13.6),
            )
        }
        // A capital letter.
        Tool::Text => Icon::new().stroke(
            Path::new()
                .move_to(5.5, 19.5)
                .line_to(12.0, 4.5)
                .line_to(18.5, 19.5)
                .segment(8.1, 14.0, 15.9, 14.0),
        ),
        Tool::Grid => Icon::new().stroke(
            Path::new()
                .segment(9.0, 4.0, 9.0, 20.0)
                .segment(15.0, 4.0, 15.0, 20.0)
                .segment(4.0, 9.0, 20.0, 9.0)
                .segment(4.0, 15.0, 20.0, 15.0),
        ),
        // A shape and its mirror image on either side of a dashed axis.
        Tool::Symmetry => Icon::new()
            .stroke(
                Path::new()
                    .segment(12.0, 3.0, 12.0, 5.5)
                    .segment(12.0, 10.5, 12.0, 13.5)
                    .segment(12.0, 18.5, 12.0, 21.0),
            )
            .stroke(Path::new().polygon(&[(3.5, 7.0), (8.8, 12.0), (3.5, 17.0)]))
            .fill(Path::new().polygon(&[(20.5, 7.0), (15.2, 12.0), (20.5, 17.0)]))
            .stroke(Path::new().polygon(&[(20.5, 7.0), (15.2, 12.0), (20.5, 17.0)])),
        Tool::Magnify => Icon::new().stroke(
            Path::new()
                .circle(10.5, 10.5, 6.0)
                .segment(15.0, 15.0, 20.0, 20.0),
        ),
        // More or less magnification: the left button zooms in, the right one out.
        Tool::Zoom => Icon::new().stroke(
            Path::new()
                .segment(3.8, 8.0, 10.2, 8.0)
                .segment(7.0, 4.8, 7.0, 11.2)
                .segment(14.2, 16.8, 20.6, 16.8)
                .segment(15.8, 5.0, 8.6, 19.0),
        ),
        // An arrow that turns back.
        Tool::Undo => Icon::new().stroke(
            Path::new()
                .move_to(6.0, 9.5)
                .line_to(14.0, 9.5)
                .arc(14.0, 14.5, 5.0, 5.0, -90.0, 180.0)
                .line_to(9.0, 19.5)
                .move_to(9.5, 5.5)
                .line_to(5.5, 9.5)
                .line_to(9.5, 13.5),
        ),
        // The letters CLR, as on the DeluxePaint button, in our own strokes.
        Tool::Clear => Icon::new().stroke(
            Path::new()
                .arc(6.0, 12.0, 3.0, 4.5, -48.0, -264.0)
                .move_to(11.2, 7.5)
                .line_to(11.2, 16.5)
                .line_to(14.6, 16.5)
                .move_to(17.0, 16.5)
                .line_to(17.0, 7.5)
                .line_to(18.9, 7.5)
                .arc(18.9, 9.75, 2.25, 2.25, -90.0, 180.0)
                .line_to(17.0, 12.0)
                .segment(18.9, 12.0, 21.0, 16.5),
        ),
    }
}

/// The filled shape of built-in brush `index` (toolbox order: four round brushes, four
/// squares, two scatter brushes), centred on (`cx`, `cy`); one brush pixel is `unit` points.
/// The size comes from the brush's real footprint, so the dock shows the brushes in their
/// true proportions. `None` for an index without a brush.
pub fn brush(index: usize, cx: f32, cy: f32, unit: f32) -> Option<Path> {
    let footprint = BUILTIN_BRUSHES.get(index)?.footprint();
    let extent = |f: fn(&(i32, i32)) -> i32| {
        let (min, max) = footprint
            .iter()
            .map(f)
            .fold((i32::MAX, i32::MIN), |(a, b), v| (a.min(v), b.max(v)));
        (max - min + 1) as f32
    };
    let (w, h) = (extent(|p| p.0), extent(|p| p.1));
    // A single pixel would vanish at this size: nothing is drawn smaller than this.
    let least = 2.4;
    Some(match index {
        0..=3 => Path::new().circle(cx, cy, (w * unit).max(least) / 2.0),
        4..=7 => {
            let (w, h) = ((w * unit).max(least), (h * unit).max(least));
            Path::new().round_rect(
                crate::paint::Rect::new(cx - w / 2.0, cy - h / 2.0, w, h),
                0.6,
            )
        }
        _ => {
            // Scatter brushes: one dot per pixel, spread a little so the dots stay apart.
            let (mx, my) = (
                footprint.iter().map(|p| p.0).sum::<i32>() as f32 / footprint.len() as f32,
                footprint.iter().map(|p| p.1).sum::<i32>() as f32 / footprint.len() as f32,
            );
            footprint.iter().fold(Path::new(), |p, &(x, y)| {
                p.circle(
                    cx + (x as f32 - mx) * unit * 1.25,
                    cy + (y as f32 - my) * unit * 1.25,
                    least / 2.0,
                )
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_ui::toolbox::TOOL_ROWS;

    #[test]
    fn every_tool_has_an_icon_inside_the_grid() {
        for &t in TOOL_ROWS.iter().flatten() {
            let icon = tool(t);
            assert!(!icon.parts.is_empty(), "{t:?}");
            for part in &icon.parts {
                let b = part
                    .path
                    .bounds()
                    .unwrap_or_else(|| panic!("{t:?}: empty part"));
                // Strokes reach half their width past the outline.
                let margin = if part.ink == Ink::Stroke {
                    STROKE / 2.0
                } else {
                    0.0
                };
                assert!(
                    b.x - margin >= 0.0
                        && b.y - margin >= 0.0
                        && b.right() + margin <= GRID
                        && b.bottom() + margin <= GRID,
                    "{t:?}: {b:?} leaves the grid"
                );
            }
        }
    }

    #[test]
    fn no_two_tools_share_an_icon() {
        let icons: Vec<_> = TOOL_ROWS.iter().flatten().map(|&t| (t, tool(t))).collect();
        for (i, (a, ia)) in icons.iter().enumerate() {
            for (b, ib) in &icons[i + 1..] {
                assert_ne!(ia, ib, "{a:?} and {b:?}");
            }
        }
    }

    /// The filled part of a split tool lies in the lower-right half, where a click selects
    /// the filled variant; only split tools of a shape have one on the diagonal.
    #[test]
    fn split_tools_fill_their_lower_right_half() {
        for &t in TOOL_ROWS.iter().flatten().filter(|t| t.is_split()) {
            let icon = tool(t);
            let fill = icon
                .parts
                .iter()
                .find(|p| p.ink == Ink::Fill)
                .unwrap_or_else(|| panic!("{t:?} has no filled part"));
            assert!(icon.parts.iter().any(|p| p.ink == Ink::Stroke), "{t:?}");
            for seg in &fill.path.0 {
                let end = match *seg {
                    crate::paint::Seg::Move(x, y) | crate::paint::Seg::Line(x, y) => Some((x, y)),
                    crate::paint::Seg::Cubic(.., x, y) | crate::paint::Seg::Quad(.., x, y) => {
                        Some((x, y))
                    }
                    crate::paint::Seg::Close => None,
                };
                if let Some((x, y)) = end {
                    assert!(
                        x + y >= GRID - 1e-3,
                        "{t:?}: ({x}, {y}) is in the outline half"
                    );
                }
            }
        }
    }

    #[test]
    fn cutting_a_square_on_the_diagonal_leaves_a_triangle() {
        let half = lower_right(&[(4.0, 4.0), (20.0, 4.0), (20.0, 20.0), (4.0, 20.0)]);
        assert_eq!(half, vec![(20.0, 4.0), (20.0, 20.0), (4.0, 20.0)]);
    }

    #[test]
    fn dots_along_a_curve_are_evenly_spaced_from_end_to_end() {
        let curve = [[(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)]];
        let dots = evenly_spaced(&curve, 6);
        assert_eq!(dots.len(), 6);
        assert_eq!(dots[0], (0.0, 0.0));
        let last = dots[5];
        assert!(
            (last.0 - 10.0).abs() < 1e-3 && last.1.abs() < 1e-3,
            "{last:?}"
        );
        let gaps: Vec<f32> = dots
            .windows(2)
            .map(|w| ((w[0].0 - w[1].0).powi(2) + (w[0].1 - w[1].1).powi(2)).sqrt())
            .collect();
        let (min, max) = gaps
            .iter()
            .fold((f32::MAX, 0.0f32), |(a, b), g| (a.min(*g), b.max(*g)));
        // Chords of equal arcs differ a little where the curve bends more.
        assert!(max - min < 0.6, "{gaps:?}");
    }

    #[test]
    fn brushes_grow_with_their_footprint() {
        let width = |i: usize| {
            brush(i, 0.0, 0.0, 1.5)
                .and_then(|p| p.bounds())
                .expect("brush")
                .w
        };
        // Round 1, 3, 5, 7 and square 5, 4, 3, 2.
        assert!(width(0) < width(1) && width(1) < width(2) && width(2) < width(3));
        assert!(width(4) > width(5) && width(5) > width(6) && width(6) > width(7));
        assert!(
            (width(3) - 10.5).abs() < 0.01,
            "seven pixels at 1.5 pt: {}",
            width(3)
        );
        assert!(
            (width(4) - 7.5).abs() < 0.01,
            "five pixels at 1.5 pt: {}",
            width(4)
        );
        // The scatter brushes are three and five separate dots.
        let dots = |i: usize| {
            let p = brush(i, 0.0, 0.0, 1.5).expect("brush");
            p.0.iter()
                .filter(|s| matches!(s, crate::paint::Seg::Close))
                .count()
        };
        assert_eq!((dots(8), dots(9)), (3, 5));
        assert!(brush(10, 0.0, 0.0, 1.5).is_none());
    }
}
