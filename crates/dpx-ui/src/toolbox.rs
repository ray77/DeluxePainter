//! The DeluxePaint toolbox: tools, built-in brushes and their icons.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::bitmap::{self, Bitmap};

/// Toolbox tools, in DeluxePaint IV order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    DottedFreehand,
    ContinuousFreehand,
    Line,
    Curve,
    Fill,
    Airbrush,
    Rectangle,
    Circle,
    Ellipse,
    Polygon,
    BrushSelect,
    Text,
    Grid,
    Symmetry,
    Magnify,
    Zoom,
    Undo,
    Clear,
}

/// Rows of the toolbox below the brush block: `[left, right]`.
pub const TOOL_ROWS: [[Tool; 2]; 9] = [
    [Tool::DottedFreehand, Tool::ContinuousFreehand],
    [Tool::Line, Tool::Curve],
    [Tool::Fill, Tool::Airbrush],
    [Tool::Rectangle, Tool::Circle],
    [Tool::Ellipse, Tool::Polygon],
    [Tool::BrushSelect, Tool::Text],
    [Tool::Grid, Tool::Symmetry],
    [Tool::Magnify, Tool::Zoom],
    [Tool::Undo, Tool::Clear],
];

impl Tool {
    /// Tools whose button is split diagonally: upper-left = outline / continuous,
    /// lower-right = filled.
    pub fn is_split(self) -> bool {
        matches!(
            self,
            Tool::ContinuousFreehand
                | Tool::Rectangle
                | Tool::Circle
                | Tool::Ellipse
                | Tool::Polygon
        )
    }

    /// Shape tools whose icons are generated from geometry.
    fn is_shape(self) -> bool {
        matches!(
            self,
            Tool::Rectangle | Tool::Circle | Tool::Ellipse | Tool::Polygon
        )
    }

    /// One-shot actions that do not stay selected.
    pub fn is_action(self) -> bool {
        matches!(self, Tool::Undo | Tool::Clear)
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::DottedFreehand => "Dotted Freehand",
            Tool::ContinuousFreehand => "Continuous Freehand",
            Tool::Line => "Straight Line",
            Tool::Curve => "Curve",
            Tool::Fill => "Fill",
            Tool::Airbrush => "Airbrush",
            Tool::Rectangle => "Rectangle",
            Tool::Circle => "Circle",
            Tool::Ellipse => "Ellipse",
            Tool::Polygon => "Polygon",
            Tool::BrushSelect => "Brush Selector",
            Tool::Text => "Text",
            Tool::Grid => "Grid",
            Tool::Symmetry => "Symmetry",
            Tool::Magnify => "Magnify",
            Tool::Zoom => "Zoom",
            Tool::Undo => "Undo",
            Tool::Clear => "Clear",
        }
    }

    fn icon_name(self) -> &'static str {
        match self {
            Tool::DottedFreehand => "dotted_freehand",
            Tool::ContinuousFreehand => "continuous_freehand",
            Tool::Line => "line",
            Tool::Curve => "curve",
            Tool::Fill => "fill",
            Tool::Airbrush => "airbrush",
            Tool::BrushSelect => "brush_select",
            Tool::Text => "text",
            Tool::Grid => "grid",
            Tool::Symmetry => "symmetry",
            Tool::Magnify => "magnify",
            Tool::Zoom => "zoom",
            Tool::Undo => "undo",
            Tool::Clear => "clr",
            Tool::Rectangle | Tool::Circle | Tool::Ellipse | Tool::Polygon => "",
        }
    }
}

pub const ICON: usize = 9;

/// Whether icon pixel (`x`, `y`) belongs to the filled (lower-right) half of a split icon.
pub fn in_filled_half(x: usize, y: usize) -> bool {
    x + y >= ICON - 1
}

/// How a toolbox button icon is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconState {
    /// Not selected: the full (split) icon.
    Normal,
    /// Selected: only the chosen variant, drawn inverted by the renderer.
    /// `filled` picks the lower-right variant of split buttons.
    Selected { filled: bool },
}

/// The 9×9 icon of a tool in the given state. Set pixels are drawn in the dark pen
/// (`Normal`) or the face pen on dark (`Selected`).
pub fn icon(tool: Tool, state: IconState) -> &'static Bitmap {
    static ICONS: OnceLock<HashMap<(Tool, IconState), Bitmap>> = OnceLock::new();
    let icons = ICONS.get_or_init(|| {
        let parsed: HashMap<String, Bitmap> =
            bitmap::parse(include_str!("../../../assets/ui/toolbox-icons-lores.txt"))
                .expect("built-in toolbox icons are valid")
                .into_iter()
                .collect();
        let named = |name: &str| -> Bitmap {
            parsed
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("missing toolbox icon '{name}'"))
        };
        let mut map = HashMap::new();
        for &t in TOOL_ROWS.iter().flatten() {
            let states = [
                IconState::Normal,
                IconState::Selected { filled: false },
                IconState::Selected { filled: true },
            ];
            for s in states {
                let bmp = match (t.is_shape(), s) {
                    (true, IconState::Normal) => split_icon(t),
                    (true, IconState::Selected { filled }) => variant_icon(t, filled),
                    (false, IconState::Normal) if t.is_split() => {
                        // Split freehand: the stroke plus the corner ticks of the diagonal.
                        let mut b = named(t.icon_name());
                        for (x, y) in [(8, 0), (7, 1), (1, 7), (0, 8)] {
                            b.set(x, y, true);
                        }
                        b
                    }
                    (false, _) => named(t.icon_name()),
                };
                map.insert((t, s), bmp);
            }
        }
        map
    });
    &icons[&(tool, state)]
}

/// Whether (`x`, `y`) lies inside a shape tool's icon geometry.
fn shape_inside(tool: Tool, x: i32, y: i32) -> bool {
    let c = (ICON / 2) as i32; // centre = 4
    match tool {
        Tool::Rectangle => (1..=7).contains(&x) && (1..=7).contains(&y),
        Tool::Circle => {
            let (dx, dy) = (x - c, y - c);
            dx * dx + dy * dy <= 4 * 4 + 2
        }
        Tool::Ellipse => {
            let half = match (y - c).abs() {
                0 => 4,
                1 => 3,
                2 => 2,
                _ => return false,
            };
            (x - c).abs() <= half
        }
        Tool::Polygon => (x - c).abs() + (y - c).abs() <= c,
        _ => false,
    }
}

/// Icon of the selected variant of a shape tool: the outline, or the solid shape.
/// The filled circle is a 7×7 disc (measured from the reference); other shapes use their
/// split-icon geometry (to be calibrated).
fn variant_icon(tool: Tool, filled: bool) -> Bitmap {
    let n = ICON as i32;
    let inside = |x: i32, y: i32| -> bool {
        if tool == Tool::Circle {
            let (dx, dy) = (x - 4, y - 4);
            dx * dx + dy * dy <= 3 * 3 + 2
        } else {
            shape_inside(tool, x, y)
        }
    };
    let mut bmp = Bitmap::new(ICON, ICON);
    for y in 0..n {
        for x in 0..n {
            let edge = [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|&(dx, dy)| !inside(x + dx, y + dy));
            bmp.set(x as usize, y as usize, inside(x, y) && (filled || edge));
        }
    }
    bmp
}

/// Split shape icons follow one rule: the shape outline, the anti-diagonal from the lower-left
/// to the upper-right corner where it lies outside the shape, and the shape filled where
/// `x + y >= 8`.
fn split_icon(tool: Tool) -> Bitmap {
    let n = ICON as i32;
    let inside = |x: i32, y: i32| shape_inside(tool, x, y);
    let outline = |x: i32, y: i32| -> bool {
        inside(x, y)
            && [(-1, 0), (1, 0), (0, -1), (0, 1)]
                .iter()
                .any(|&(dx, dy)| !inside(x + dx, y + dy))
    };
    let mut bmp = Bitmap::new(ICON, ICON);
    for y in 0..n {
        for x in 0..n {
            let on = outline(x, y)
                || (inside(x, y) && in_filled_half(x as usize, y as usize))
                || (!inside(x, y) && x + y == n - 1);
            bmp.set(x as usize, y as usize, on);
        }
    }
    bmp
}

/// The ten built-in brushes in the block at the top of the toolbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinBrush {
    /// Pixels relative to the toolbox origin (left edge, screen y).
    pub pixels: &'static [(i32, i32)],
    /// Highlight box when selected: x, y, w, h relative to the toolbox origin.
    pub select_box: (i32, i32, i32, i32),
}

const ROUND_1: &[(i32, i32)] = &[(2, 15)];
const ROUND_3: &[(i32, i32)] = &[(6, 14), (5, 15), (6, 15), (7, 15), (6, 16)];
const ROUND_5: &[(i32, i32)] = &[
    (10, 13),
    (11, 13),
    (12, 13),
    (9, 14),
    (10, 14),
    (11, 14),
    (12, 14),
    (13, 14),
    (9, 15),
    (10, 15),
    (11, 15),
    (12, 15),
    (13, 15),
    (9, 16),
    (10, 16),
    (11, 16),
    (12, 16),
    (13, 16),
    (10, 17),
    (11, 17),
    (12, 17),
];
const ROUND_7: &[(i32, i32)] = &[
    (17, 12),
    (18, 12),
    (19, 12),
    (16, 13),
    (17, 13),
    (18, 13),
    (19, 13),
    (20, 13),
    (15, 14),
    (16, 14),
    (17, 14),
    (18, 14),
    (19, 14),
    (20, 14),
    (21, 14),
    (15, 15),
    (16, 15),
    (17, 15),
    (18, 15),
    (19, 15),
    (20, 15),
    (21, 15),
    (15, 16),
    (16, 16),
    (17, 16),
    (18, 16),
    (19, 16),
    (20, 16),
    (21, 16),
    (16, 17),
    (17, 17),
    (18, 17),
    (19, 17),
    (20, 17),
    (17, 18),
    (18, 18),
    (19, 18),
];
const SQUARE_5: &[(i32, i32)] = &[
    (1, 20),
    (2, 20),
    (3, 20),
    (4, 20),
    (5, 20),
    (1, 21),
    (2, 21),
    (3, 21),
    (4, 21),
    (5, 21),
    (1, 22),
    (2, 22),
    (3, 22),
    (4, 22),
    (5, 22),
    (1, 23),
    (2, 23),
    (3, 23),
    (4, 23),
    (5, 23),
    (1, 24),
    (2, 24),
    (3, 24),
    (4, 24),
    (5, 24),
];
const SQUARE_4: &[(i32, i32)] = &[
    (8, 20),
    (9, 20),
    (10, 20),
    (11, 20),
    (8, 21),
    (9, 21),
    (10, 21),
    (11, 21),
    (8, 22),
    (9, 22),
    (10, 22),
    (11, 22),
    (8, 23),
    (9, 23),
    (10, 23),
    (11, 23),
];
const SQUARE_3: &[(i32, i32)] = &[
    (14, 20),
    (15, 20),
    (16, 20),
    (14, 21),
    (15, 21),
    (16, 21),
    (14, 22),
    (15, 22),
    (16, 22),
];
const SQUARE_2: &[(i32, i32)] = &[(19, 20), (20, 20), (19, 21), (20, 21)];
const SCATTER_SMALL: &[(i32, i32)] = &[(6, 26), (4, 27), (6, 28)];
const SCATTER_LARGE: &[(i32, i32)] = &[(15, 26), (18, 27), (21, 27), (18, 29)];

/// Built-in brushes; index 0 (single pixel) is the start-up brush.
pub const BUILTIN_BRUSHES: [BuiltinBrush; 10] = [
    BuiltinBrush {
        pixels: ROUND_1,
        select_box: (1, 12, 3, 6),
    },
    BuiltinBrush {
        pixels: ROUND_3,
        select_box: (4, 12, 5, 6),
    },
    BuiltinBrush {
        pixels: ROUND_5,
        select_box: (8, 12, 7, 7),
    },
    BuiltinBrush {
        pixels: ROUND_7,
        select_box: (14, 11, 9, 9),
    },
    BuiltinBrush {
        pixels: SQUARE_5,
        select_box: (1, 19, 6, 7),
    },
    BuiltinBrush {
        pixels: SQUARE_4,
        select_box: (7, 19, 6, 6),
    },
    BuiltinBrush {
        pixels: SQUARE_3,
        select_box: (13, 19, 5, 5),
    },
    BuiltinBrush {
        pixels: SQUARE_2,
        select_box: (18, 19, 4, 4),
    },
    BuiltinBrush {
        pixels: SCATTER_SMALL,
        select_box: (3, 25, 5, 5),
    },
    BuiltinBrush {
        pixels: SCATTER_LARGE,
        select_box: (14, 25, 9, 6),
    },
];

impl BuiltinBrush {
    /// The brush footprint centred on (0, 0), for painting.
    pub fn footprint(&self) -> Vec<(i32, i32)> {
        let (minx, maxx) = self
            .pixels
            .iter()
            .fold((i32::MAX, i32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
        let (miny, maxy) = self
            .pixels
            .iter()
            .fold((i32::MAX, i32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        let (cx, cy) = ((minx + maxx) / 2, (miny + maxy) / 2);
        self.pixels.iter().map(|&(x, y)| (x - cx, y - cy)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(b: &Bitmap) -> Vec<String> {
        (0..b.height)
            .map(|y| {
                (0..b.width)
                    .map(|x| if b.get(x, y) { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn all_icons_load() {
        for t in TOOL_ROWS.iter().flatten() {
            for s in [
                IconState::Normal,
                IconState::Selected { filled: false },
                IconState::Selected { filled: true },
            ] {
                let i = icon(*t, s);
                assert_eq!((i.width, i.height), (ICON, ICON), "{t:?}");
            }
        }
    }

    #[test]
    fn split_rectangle_matches_reference() {
        assert_eq!(
            rows(icon(Tool::Rectangle, IconState::Normal)),
            [
                "........#",
                ".#######.",
                ".#....##.",
                ".#...###.",
                ".#..####.",
                ".#.#####.",
                ".#######.",
                ".#######.",
                "#........",
            ]
        );
    }

    #[test]
    fn split_polygon_matches_reference() {
        assert_eq!(
            rows(icon(Tool::Polygon, IconState::Normal)),
            [
                "....#...#",
                "...#.#.#.",
                "..#...#..",
                ".#...###.",
                "#...#####",
                ".#.#####.",
                "..#####..",
                ".#.###...",
                "#...#....",
            ]
        );
    }

    #[test]
    fn split_circle_and_ellipse_match_reference() {
        assert_eq!(
            rows(icon(Tool::Circle, IconState::Normal)),
            [
                "...###..#",
                ".##...##.",
                ".#....##.",
                "#....####",
                "#...#####",
                "#..######",
                ".#######.",
                ".#######.",
                "#..###...",
            ]
        );
        assert_eq!(
            rows(icon(Tool::Ellipse, IconState::Normal)),
            [
                "........#",
                ".......#.",
                "..#####..",
                ".#...###.",
                "#...#####",
                ".#.#####.",
                "..#####..",
                ".#.......",
                "#........",
            ]
        );
    }

    #[test]
    fn freehand_and_selected_circle_match_reference() {
        // Reference (Inapp_gfx_04): unselected split freehand button.
        assert_eq!(
            rows(icon(Tool::ContinuousFreehand, IconState::Normal)),
            [
                "........#",
                ".##....#.",
                "#..#.....",
                "....#....",
                "....#....",
                "....#....",
                "....#...#",
                ".#..#..#.",
                "#....##..",
            ]
        );
        // Reference (Inapp_gfx_04): filled circle selected = 7x7 disc.
        assert_eq!(
            rows(icon(Tool::Circle, IconState::Selected { filled: true })),
            [
                ".........",
                "...###...",
                "..#####..",
                ".#######.",
                ".#######.",
                ".#######.",
                "..#####..",
                "...###...",
                ".........",
            ]
        );
    }

    #[test]
    fn brush_footprints_are_centred() {
        assert_eq!(BUILTIN_BRUSHES[0].footprint(), vec![(0, 0)]);
        assert_eq!(BUILTIN_BRUSHES[3].footprint().len(), 37);
    }
}
