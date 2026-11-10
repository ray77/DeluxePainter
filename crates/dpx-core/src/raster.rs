//! Basic raster algorithms on integer pixel coordinates.
//!
//! These will move to the `dpx-raster` crate once more tools exist (see ARCHITECTURE.md).

use crate::image::{IndexedImage, Rect};

/// All points of a Bresenham line from `a` to `b`, inclusive.
pub fn line_points(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x, mut y) = a;
    let dx = (b.0 - a.0).abs();
    let dy = -(b.1 - a.1).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut out = Vec::with_capacity((dx - dy + 1) as usize);
    loop {
        out.push((x, y));
        if (x, y) == b {
            return out;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Stamps `brush` (pixel offsets relative to the hotspot) at (`x`, `y`) in `color`, clipped.
pub fn stamp(img: &mut IndexedImage, brush: &[(i32, i32)], x: i32, y: i32, color: u8) {
    for &(dx, dy) in brush {
        img.set(x + dx, y + dy, color);
    }
}

/// Paints a freehand stroke: stamps the brush at every point; with `continuous`, each pair of
/// consecutive points is joined with a Bresenham line and the brush is stamped on every line
/// pixel. Points may lie outside the image (clipped).
pub fn stroke(
    img: &mut IndexedImage,
    brush: &[(i32, i32)],
    color: u8,
    points: &[(i32, i32)],
    continuous: bool,
) {
    if continuous && points.len() > 1 {
        for pair in points.windows(2) {
            for (x, y) in line_points(pair[0], pair[1]) {
                stamp(img, brush, x, y, color);
            }
        }
    } else {
        for &(x, y) in points {
            stamp(img, brush, x, y, color);
        }
    }
}

/// Unclipped bounding rect of every pixel `stroke` can touch; `None` if `points` or `brush` is empty.
pub fn stroke_bounds(brush: &[(i32, i32)], points: &[(i32, i32)]) -> Option<Rect> {
    let p = Rect::from_points(points)?;
    let b = Rect::from_points(brush)?;
    Some(Rect::new(
        p.x + b.x,
        p.y + b.y,
        p.w + b.w - 1,
        p.h + b.h - 1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endpoints_and_continuity() {
        let pts = line_points((0, 0), (7, 3));
        assert_eq!(pts.first(), Some(&(0, 0)));
        assert_eq!(pts.last(), Some(&(7, 3)));
        assert_eq!(pts.len(), 8);
        for w in pts.windows(2) {
            assert!((w[1].0 - w[0].0).abs() <= 1 && (w[1].1 - w[0].1).abs() <= 1);
        }
        assert_eq!(line_points((3, 3), (3, 3)), vec![(3, 3)]);
    }

    fn changed(img: &IndexedImage) -> Vec<(i32, i32)> {
        let mut v = Vec::new();
        for y in 0..img.height() as i32 {
            for x in 0..img.width() as i32 {
                if img.get(x, y) != Some(0) {
                    v.push((x, y));
                }
            }
        }
        v
    }

    fn sorted(mut v: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
        v.sort_by_key(|&(x, y)| (y, x));
        v.dedup();
        v
    }

    const BIG: &[(i32, i32)] = &[(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (2, 2)];

    #[test]
    fn stamp_clips_all_edges() {
        let mut img = IndexedImage::new(4, 4, 0);
        for (x, y) in [(-1, 2), (4, 2), (2, -1), (2, 4), (-5, -5), (9, 9)] {
            stamp(&mut img, BIG, x, y, 1);
        }
        assert!(!changed(&img).is_empty());
        stamp(&mut img, &[], 1, 1, 1);
    }

    #[test]
    fn dotted_and_continuous_pixels() {
        let mut img = IndexedImage::new(8, 8, 0);
        let pts = [(1, 1), (6, 2), (3, 7)];
        stroke(&mut img, &[(0, 0)], 1, &pts, false);
        assert_eq!(changed(&img), sorted(pts.to_vec()));

        let mut img = IndexedImage::new(8, 8, 0);
        stroke(&mut img, &[(0, 0)], 1, &[(0, 0), (5, 2)], true);
        assert_eq!(changed(&img), sorted(line_points((0, 0), (5, 2))));
    }

    #[test]
    fn incremental_equivalence() {
        let pts = [(2, 2), (9, 4), (5, 12), (-3, 6), (14, 14), (7, 0)];
        for continuous in [false, true] {
            let mut full = IndexedImage::new(16, 16, 0);
            stroke(&mut full, BIG, 3, &pts, continuous);
            for k in 0..pts.len() {
                let mut inc = IndexedImage::new(16, 16, 0);
                stroke(&mut inc, BIG, 3, &pts[..=k], continuous);
                stroke(&mut inc, BIG, 3, &pts[k..], continuous);
                assert_eq!(inc, full, "continuous={continuous} k={k}");
            }
        }
    }

    #[test]
    fn bounds_cover_changes() {
        let brushes: [&[(i32, i32)]; 3] = [&[(0, 0)], BIG, &[(-2, 0), (3, -4)]];
        let sets: [&[(i32, i32)]; 3] = [
            &[(0, 0)],
            &[(-5, -3), (4, 2), (-1, 6)],
            &[(3, 3), (3, 3), (-7, 1)],
        ];
        let (ox, oy) = (30, 30);
        for brush in brushes {
            for pts in sets {
                for continuous in [false, true] {
                    let shifted: Vec<_> = pts.iter().map(|&(x, y)| (x + ox, y + oy)).collect();
                    let mut img = IndexedImage::new(64, 64, 0);
                    stroke(&mut img, brush, 1, &shifted, continuous);
                    let r = stroke_bounds(brush, &shifted).unwrap();
                    let logical = stroke_bounds(brush, pts).unwrap();
                    assert_eq!(logical, Rect::new(r.x - ox, r.y - oy, r.w, r.h));
                    for (x, y) in changed(&img) {
                        assert!(r.contains(x, y), "({x},{y}) outside {r:?}");
                    }
                }
            }
        }
        assert_eq!(stroke_bounds(&[], &[(0, 0)]), None);
        assert_eq!(stroke_bounds(&[(0, 0)], &[]), None);
    }
}
