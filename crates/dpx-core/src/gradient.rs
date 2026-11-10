//! Fill types of the Fill tool (ADR-20): solid, or a gradient through a colour range.
//!
//! A gradient steps through the palette indices `from..=to` (descending when `from > to`,
//! so a reversed colour range works as well). The colour of a pixel depends on its position
//! inside an area: the bounding box of the filled region (Vertical, Horizontal) or the whole
//! page (the "Line" variants, which give the same gradient whatever shape is filled). With
//! dither, a 4×4 ordered pattern mixes two neighbouring colours instead of hard bands.

use serde::{Deserialize, Serialize};

use crate::image::{IndexedImage, Rect};
use crate::raster;

/// Which way the colour changes, and what the gradient is stretched over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Colour changes from the top to the bottom of the filled region.
    Vertical,
    /// Colour changes from the left to the right of the filled region.
    Horizontal,
    /// Like `Vertical`, but over the whole page, independent of the region.
    VerticalLine,
    /// Like `Horizontal`, but over the whole page, independent of the region.
    HorizontalLine,
}

impl Direction {
    pub const ALL: [Direction; 4] = [
        Direction::Vertical,
        Direction::Horizontal,
        Direction::VerticalLine,
        Direction::HorizontalLine,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Direction::Vertical => "Vertical",
            Direction::Horizontal => "Horizontal",
            Direction::VerticalLine => "Vertical Line",
            Direction::HorizontalLine => "Horizontal Line",
        }
    }

    fn is_vertical(self) -> bool {
        matches!(self, Direction::Vertical | Direction::VerticalLine)
    }

    fn over_page(self) -> bool {
        matches!(self, Direction::VerticalLine | Direction::HorizontalLine)
    }
}

/// How the Fill tool paints a region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FillType {
    /// One colour (the command's `color`).
    #[default]
    Solid,
    /// The colour range `from..=to` stretched over the region or page.
    Gradient {
        direction: Direction,
        from: u8,
        to: u8,
        #[serde(default)]
        dither: bool,
    },
}

impl FillType {
    pub fn is_solid(&self) -> bool {
        matches!(self, FillType::Solid)
    }
}

/// 4×4 Bayer thresholds, 0..16.
const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// The palette index at step `step` of a gradient of `n` colours from `from` towards `to`.
fn nth_color(from: u8, to: u8, step: usize) -> u8 {
    if from <= to {
        from + step as u8
    } else {
        from - step as u8
    }
}

/// The colour of pixel (`x`, `y`) of a gradient. `area` is the rect the gradient is stretched
/// over (a region's bounding box, or the page).
pub fn gradient_color(
    direction: Direction,
    from: u8,
    to: u8,
    dither: bool,
    area: Rect,
    (x, y): (i32, i32),
) -> u8 {
    let n = usize::from(from.abs_diff(to)) + 1;
    if n == 1 {
        return from;
    }
    let (pos, len) = if direction.is_vertical() {
        (y - area.y, area.h)
    } else {
        (x - area.x, area.w)
    };
    let len = i64::from(len.max(1));
    let pos = i64::from(pos).clamp(0, len - 1);
    let n64 = n as i64;
    if !dither {
        // n equal bands.
        let step = (pos * n64 / len).min(n64 - 1) as usize;
        return nth_color(from, to, step);
    }
    // Sample the pixel centre on the continuous scale 0..=(n-1), in 1/16 steps; the Bayer
    // threshold decides between the lower colour and the next one.
    let scaled = (2 * pos + 1) * (n64 - 1) * 16 / (2 * len);
    let (base, frac) = ((scaled / 16) as usize, (scaled % 16) as u8);
    let threshold = BAYER[y.rem_euclid(4) as usize][x.rem_euclid(4) as usize];
    let step = if frac > threshold {
        (base + 1).min(n - 1)
    } else {
        base
    };
    nth_color(from, to, step)
}

/// The 4-connected region of the colour at (`x`, `y`) as horizontal spans `(left, right, y)`
/// (inclusive); `None` if the point is outside the image.
fn region_spans(img: &IndexedImage, x: i32, y: i32) -> Option<Vec<(i32, i32, i32)>> {
    let target = img.get(x, y)?;
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut seen = vec![false; (w * h) as usize];
    let mut spans = Vec::new();
    let mut stack = vec![(x, y)];
    while let Some((x, y)) = stack.pop() {
        if seen[(y * w + x) as usize] || img.get(x, y) != Some(target) {
            continue;
        }
        let (mut l, mut r) = (x, x);
        while l > 0 && img.get(l - 1, y) == Some(target) && !seen[(y * w + l - 1) as usize] {
            l -= 1;
        }
        while r < w - 1 && img.get(r + 1, y) == Some(target) && !seen[(y * w + r + 1) as usize] {
            r += 1;
        }
        for i in l..=r {
            seen[(y * w + i) as usize] = true;
        }
        spans.push((l, r, y));
        for ny in [y - 1, y + 1] {
            if !(0..h).contains(&ny) {
                continue;
            }
            for i in l..=r {
                if !seen[(ny * w + i) as usize] && img.get(i, ny) == Some(target) {
                    stack.push((i, ny));
                }
            }
        }
    }
    Some(spans)
}

/// The Fill tool: fills the region at (`x`, `y`) with `color` (solid) or the gradient.
/// Returns the bounding rect of the changed pixels; `None` if nothing changed.
pub fn flood_fill(
    img: &mut IndexedImage,
    x: i32,
    y: i32,
    color: u8,
    fill: &FillType,
) -> Option<Rect> {
    let FillType::Gradient {
        direction,
        from,
        to,
        dither,
    } = *fill
    else {
        return raster::flood_fill(img, x, y, color);
    };
    let spans = region_spans(img, x, y)?;
    let bounds = spans.iter().fold(Rect::default(), |r, &(l, rt, y)| {
        r.union(Rect::new(l, y, rt - l + 1, 1))
    });
    let area = if direction.over_page() {
        img.bounds()
    } else {
        bounds
    };
    let mut changed = Rect::default();
    for (l, r, y) in spans {
        for x in l..=r {
            let c = gradient_color(direction, from, to, dither, area, (x, y));
            if img.get(x, y) != Some(c) {
                img.set(x, y, c);
                changed = changed.union(Rect::new(x, y, 1, 1));
            }
        }
    }
    (!changed.is_empty()).then_some(changed)
}

/// The colour range a gradient should use for foreground colour `fg`: the first colour-cycling
/// range (`low..=high`) that holds it, else `fg` to `bg`.
pub fn range_for(ranges: &[(u8, u8)], fg: u8, bg: u8) -> (u8, u8) {
    ranges
        .iter()
        .copied()
        .find(|&(lo, hi)| lo < hi && (lo..=hi).contains(&fg))
        .unwrap_or((fg, bg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grad(d: Direction, from: u8, to: u8, dither: bool) -> FillType {
        FillType::Gradient {
            direction: d,
            from,
            to,
            dither,
        }
    }

    #[test]
    fn bands_are_equal_and_cover_the_whole_range() {
        let area = Rect::new(0, 0, 8, 8);
        let col = |y| gradient_color(Direction::Vertical, 2, 5, false, area, (0, y));
        let got: Vec<u8> = (0..8).map(col).collect();
        assert_eq!(got, [2, 2, 3, 3, 4, 4, 5, 5]);
        // Horizontal uses x, and a descending range reverses the order.
        let col = |x| gradient_color(Direction::Horizontal, 5, 2, false, area, (x, 3));
        let got: Vec<u8> = (0..8).map(col).collect();
        assert_eq!(got, [5, 5, 4, 4, 3, 3, 2, 2]);
    }

    #[test]
    fn single_colour_range_and_tiny_areas() {
        let area = Rect::new(4, 4, 1, 1);
        assert_eq!(
            gradient_color(Direction::Vertical, 7, 7, true, area, (4, 4)),
            7
        );
        // One pixel of a four-colour range gets the first colour; never out of range.
        assert_eq!(
            gradient_color(Direction::Vertical, 1, 4, false, area, (4, 4)),
            1
        );
        for y in -3..20 {
            let c = gradient_color(
                Direction::Vertical,
                1,
                4,
                true,
                Rect::new(0, 0, 3, 9),
                (0, y),
            );
            assert!((1..=4).contains(&c));
        }
    }

    #[test]
    fn dither_mixes_neighbouring_colours_only() {
        let area = Rect::new(0, 0, 16, 16);
        let mut seen = std::collections::BTreeSet::new();
        let mut mixed_rows = 0;
        for y in 0..16 {
            let row: std::collections::BTreeSet<u8> = (0..16)
                .map(|x| gradient_color(Direction::Vertical, 10, 13, true, area, (x, y)))
                .collect();
            if row.len() > 1 {
                mixed_rows += 1;
                let (lo, hi) = (*row.first().unwrap(), *row.last().unwrap());
                assert_eq!(hi - lo, 1, "only adjacent colours mix");
            }
            seen.extend(row);
        }
        assert!(mixed_rows >= 8, "dither blends most rows, got {mixed_rows}");
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), [10, 11, 12, 13]);
        // Without dither a row is one colour.
        let plain: std::collections::BTreeSet<u8> = (0..16)
            .map(|x| gradient_color(Direction::Vertical, 10, 13, false, area, (x, 5)))
            .collect();
        assert_eq!(plain.len(), 1);
    }

    #[test]
    fn region_gradient_follows_its_bounding_box() {
        // A 6x4 room (colour 0) inside a wall of colour 9.
        let mut img = IndexedImage::new(10, 8, 9);
        img.fill_rect(2, 2, 6, 4, 0);
        let r = flood_fill(
            &mut img,
            3,
            3,
            1,
            &grad(Direction::Horizontal, 20, 25, false),
        );
        assert_eq!(r, Some(Rect::new(2, 2, 6, 4)));
        let row: Vec<u8> = (2..8).map(|x| img.get(x, 3).unwrap()).collect();
        assert_eq!(row, [20, 21, 22, 23, 24, 25]);
        assert_eq!(img.get(1, 3), Some(9), "wall untouched");
        assert_eq!(img.get(8, 3), Some(9));
    }

    #[test]
    fn line_variants_use_the_page_not_the_region() {
        let mut img = IndexedImage::new(8, 8, 9);
        img.fill_rect(0, 6, 8, 2, 0);
        // Region is only the bottom two rows; Vertical Line spans the page height (8 rows
        // over 4 colours), so rows 6 and 7 get the last colour; plain Vertical would not.
        flood_fill(
            &mut img,
            0,
            7,
            1,
            &grad(Direction::VerticalLine, 1, 4, false),
        );
        assert_eq!((img.get(0, 6), img.get(0, 7)), (Some(4), Some(4)));
        let mut img2 = IndexedImage::new(8, 8, 9);
        img2.fill_rect(0, 6, 8, 2, 0);
        flood_fill(&mut img2, 0, 7, 1, &grad(Direction::Vertical, 1, 4, false));
        assert_eq!((img2.get(0, 6), img2.get(0, 7)), (Some(1), Some(3)));
    }

    #[test]
    fn solid_fill_is_unchanged_and_gradient_fill_works_on_the_same_colour() {
        let mut img = IndexedImage::new(4, 4, 3);
        assert_eq!(flood_fill(&mut img, 0, 0, 3, &FillType::Solid), None);
        assert!(flood_fill(&mut img, 0, 0, 1, &FillType::Solid).is_some());
        assert_eq!(img.get(3, 3), Some(1));
        // A gradient may start with the colour the region already has.
        let mut img = IndexedImage::new(4, 4, 1);
        let r = flood_fill(&mut img, 0, 0, 0, &grad(Direction::Vertical, 1, 2, false));
        assert_eq!(r, Some(Rect::new(0, 2, 4, 2)));
        assert_eq!(flood_fill(&mut img, 9, 9, 0, &FillType::Solid), None);
        assert_eq!(
            flood_fill(&mut img, 9, 9, 0, &grad(Direction::Vertical, 1, 2, false)),
            None
        );
    }

    #[test]
    fn range_prefers_the_cycling_range_that_holds_fg() {
        let ranges = [(2, 6), (10, 20), (30, 30)];
        assert_eq!(range_for(&ranges, 12, 0), (10, 20));
        assert_eq!(range_for(&ranges, 3, 0), (2, 6));
        assert_eq!(range_for(&ranges, 8, 1), (8, 1), "no range: FG to BG");
        assert_eq!(
            range_for(&ranges, 30, 0),
            (30, 0),
            "one-colour range is none"
        );
    }

    #[test]
    fn flood_fill_command_runs_validates_and_undoes() {
        use crate::{Command, Document, Engine, ErrorCode, Palette};
        let mut e = Engine::new(Document::new(8, 8, Palette::dpaint_default_32()));
        let fill = grad(Direction::Vertical, 4, 7, false);
        e.execute(Command::FloodFill {
            x: 1,
            y: 1,
            color: 0,
            fill,
        })
        .unwrap();
        let col: Vec<u8> = (0..8)
            .map(|y| e.document().page().get(3, y).unwrap())
            .collect();
        assert_eq!(col, [4, 4, 5, 5, 6, 6, 7, 7]);
        e.undo().unwrap();
        assert_eq!(e.document().page().get(3, 7), Some(0));
        // A gradient colour outside the palette is refused and changes nothing.
        let err = e
            .execute(Command::FloodFill {
                x: 1,
                y: 1,
                color: 0,
                fill: grad(Direction::Vertical, 4, 40, false),
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::PaletteRangeInvalid);
        // JSON: solid fills keep their old form, gradients carry the fill.
        let json = serde_json::to_string(&Command::FloodFill {
            x: 1,
            y: 2,
            color: 3,
            fill,
        })
        .unwrap();
        assert!(json.contains(r#""fill":{"type":"gradient""#), "{json}");
        let back: Command = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, Command::FloodFill { fill: f, .. } if f == fill));
    }

    #[test]
    fn fill_type_serde_defaults() {
        let f: FillType = serde_json::from_str(r#"{"type":"solid"}"#).unwrap();
        assert!(f.is_solid());
        let g: FillType = serde_json::from_str(
            r#"{"type":"gradient","direction":"vertical_line","from":1,"to":4}"#,
        )
        .unwrap();
        assert_eq!(g, grad(Direction::VerticalLine, 1, 4, false));
    }
}
