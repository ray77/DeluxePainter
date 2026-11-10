//! Brush menu operations as pure functions on [`BrushImage`] (ADR-19).
//!
//! Every function returns a new brush; the handle follows the pixels it points at. Size and
//! shape operations keep the transparent colour. No function touches the page.

use crate::paint::BrushImage;

fn new_brush(
    width: u32,
    height: u32,
    fill: u8,
    from: &BrushImage,
    handle: (i32, i32),
) -> BrushImage {
    BrushImage {
        width,
        height,
        pixels: vec![fill; width as usize * height as usize],
        handle,
        transparent: from.transparent,
    }
}

fn put(b: &mut BrushImage, x: u32, y: u32, c: u8) {
    let i = y as usize * b.width as usize + x as usize;
    b.pixels[i] = c;
}

/// Brush ▸ Flip ▸ Horiz: mirrored about the vertical axis.
pub fn flip_horizontal(b: &BrushImage) -> BrushImage {
    let mut out = new_brush(
        b.width,
        b.height,
        0,
        b,
        (b.width as i32 - 1 - b.handle.0, b.handle.1),
    );
    for y in 0..b.height {
        for x in 0..b.width {
            put(
                &mut out,
                b.width - 1 - x,
                y,
                b.pixels[(y * b.width + x) as usize],
            );
        }
    }
    out
}

/// Brush ▸ Flip ▸ Vert: mirrored about the horizontal axis.
pub fn flip_vertical(b: &BrushImage) -> BrushImage {
    let mut out = new_brush(
        b.width,
        b.height,
        0,
        b,
        (b.handle.0, b.height as i32 - 1 - b.handle.1),
    );
    for y in 0..b.height {
        for x in 0..b.width {
            put(
                &mut out,
                x,
                b.height - 1 - y,
                b.pixels[(y * b.width + x) as usize],
            );
        }
    }
    out
}

/// Brush ▸ Rotate ▸ 90 Degrees: clockwise; width and height swap.
pub fn rotate_90(b: &BrushImage) -> BrushImage {
    let handle = (b.height as i32 - 1 - b.handle.1, b.handle.0);
    let mut out = new_brush(b.height, b.width, 0, b, handle);
    for y in 0..b.height {
        for x in 0..b.width {
            put(
                &mut out,
                b.height - 1 - y,
                x,
                b.pixels[(y * b.width + x) as usize],
            );
        }
    }
    out
}

/// Brush ▸ Size ▸ Double / Double Horiz / Double Vert: every pixel becomes 2 pixels wide
/// and/or high.
pub fn double(b: &BrushImage, horizontal: bool, vertical: bool) -> BrushImage {
    let (fx, fy) = (1 + u32::from(horizontal), 1 + u32::from(vertical));
    let handle = (b.handle.0 * fx as i32, b.handle.1 * fy as i32);
    let mut out = new_brush(b.width * fx, b.height * fy, 0, b, handle);
    for y in 0..b.height * fy {
        for x in 0..b.width * fx {
            put(
                &mut out,
                x,
                y,
                b.pixels[((y / fy) * b.width + x / fx) as usize],
            );
        }
    }
    out
}

/// Brush ▸ Size ▸ Halve: 50 % in both directions (at least 1x1). Each 2x2 block becomes its
/// first opaque pixel, so thin shapes survive; a block with none stays transparent.
pub fn halve(b: &BrushImage) -> BrushImage {
    let (w, h) = ((b.width / 2).max(1), (b.height / 2).max(1));
    let handle = (b.handle.0 / 2, b.handle.1 / 2);
    let mut out = new_brush(w, h, b.transparent.unwrap_or(0), b, handle);
    for y in 0..h {
        for x in 0..w {
            let mut pick = None;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (sx, sy) = ((x * 2 + dx) as i32, (y * 2 + dy) as i32);
                if let Some(c) = b.get(sx, sy) {
                    if pick.is_none() {
                        pick = Some(c);
                    }
                    if b.is_opaque(sx, sy) {
                        pick = Some(c);
                        break;
                    }
                }
            }
            if let Some(c) = pick {
                put(&mut out, x, y, c);
            }
        }
    }
    out
}

/// Brush ▸ Edge ▸ Outline: grows the brush by one pixel on every side and paints `color` on
/// the new pixels next to opaque ones (8 neighbours). Without a transparent colour the whole
/// new ring is painted.
pub fn outline(b: &BrushImage, color: u8) -> BrushImage {
    let handle = (b.handle.0 + 1, b.handle.1 + 1);
    let mut out = new_brush(b.width + 2, b.height + 2, color, b, handle);
    for y in 0..out.height as i32 {
        for x in 0..out.width as i32 {
            let (sx, sy) = (x - 1, y - 1);
            let new = if b.is_opaque(sx, sy) {
                b.get(sx, sy).unwrap_or(color)
            } else if (-1..=1).any(|dy| (-1..=1).any(|dx| b.is_opaque(sx + dx, sy + dy))) {
                color
            } else {
                b.transparent.unwrap_or(color)
            };
            put(&mut out, x as u32, y as u32, new);
        }
    }
    out
}

/// Brush ▸ Edge ▸ Trim: removes the outermost layer of opaque pixels (those touching a
/// transparent pixel or the brush edge) and crops to what is left. The inverse of
/// [`outline`]. `None` when nothing would be left.
pub fn trim(b: &BrushImage) -> Option<BrushImage> {
    let keep = |x: i32, y: i32| {
        b.is_opaque(x, y) && (-1..=1).all(|dy| (-1..=1).all(|dx| b.is_opaque(x + dx, y + dy)))
    };
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..b.height as i32 {
        for x in 0..b.width as i32 {
            if keep(x, y) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 {
        return None;
    }
    let handle = (
        (b.handle.0 - x0).clamp(0, x1 - x0),
        (b.handle.1 - y0).clamp(0, y1 - y0),
    );
    let mut out = new_brush((x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32, 0, b, handle);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let c = if keep(x, y) {
                b.get(x, y).unwrap_or(0)
            } else {
                b.transparent.unwrap_or(0)
            };
            put(&mut out, (x - x0) as u32, (y - y0) as u32, c);
        }
    }
    Some(out)
}

/// Brush ▸ Color ▸ BG -> FG: opaque pixels of colour `from` take colour `to` (transparent
/// pixels stay transparent).
pub fn replace_color(b: &BrushImage, from: u8, to: u8) -> BrushImage {
    let mut out = b.clone();
    for p in &mut out.pixels {
        if *p == from && Some(*p) != b.transparent {
            *p = to;
        }
    }
    out
}

/// Brush ▸ Color ▸ BG <-> FG: the two colours trade places, the transparent colour with them.
pub fn swap_colors(b: &BrushImage, a: u8, c: u8) -> BrushImage {
    let map = |p: u8| {
        if p == a {
            c
        } else if p == c {
            a
        } else {
            p
        }
    };
    let mut out = b.clone();
    for p in &mut out.pixels {
        *p = map(*p);
    }
    out.transparent = b.transparent.map(map);
    out
}

/// Brush ▸ Color ▸ Change Transparency: the transparent colour becomes `bg`.
pub fn set_transparent(b: &BrushImage, bg: u8) -> BrushImage {
    BrushImage {
        transparent: Some(bg),
        ..b.clone()
    }
}

/// The handle positions Brush ▸ Handle ▸ Center / Corner step through.
pub fn handle_center(b: &BrushImage) -> BrushImage {
    BrushImage {
        handle: (b.width as i32 / 2, b.height as i32 / 2),
        ..b.clone()
    }
}

/// Brush ▸ Handle ▸ Corner: cycles centre, top left, top right, bottom right, bottom left,
/// centre ... (AGA addendum: five positions). A handle anywhere else goes to the top left.
pub fn handle_next_corner(b: &BrushImage) -> BrushImage {
    let (r, bt) = (b.width as i32 - 1, b.height as i32 - 1);
    let stops = [
        (b.width as i32 / 2, b.height as i32 / 2),
        (0, 0),
        (r, 0),
        (r, bt),
        (0, bt),
    ];
    let next = stops
        .iter()
        .position(|&s| s == b.handle)
        // The centre may coincide with a corner on tiny brushes; take the first match.
        .map_or(1, |i| (i + 1) % stops.len());
    BrushImage {
        handle: stops[next],
        ..b.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3x2 brush, 0 transparent:
    /// ```text
    /// 0 5 6
    /// 7 0 8
    /// ```
    fn sample() -> BrushImage {
        BrushImage {
            width: 3,
            height: 2,
            pixels: vec![0, 5, 6, 7, 0, 8],
            handle: (2, 0),
            transparent: Some(0),
        }
    }

    #[test]
    fn flips_mirror_pixels_and_handle() {
        let h = flip_horizontal(&sample());
        assert_eq!(h.pixels, vec![6, 5, 0, 8, 0, 7]);
        assert_eq!(h.handle, (0, 0));
        let v = flip_vertical(&sample());
        assert_eq!(v.pixels, vec![7, 0, 8, 0, 5, 6]);
        assert_eq!(v.handle, (2, 1));
        assert_eq!(flip_horizontal(&h), sample());
        assert_eq!(flip_vertical(&v), sample());
    }

    #[test]
    fn rotate_90_is_clockwise_and_four_times_is_identity() {
        let r = rotate_90(&sample());
        assert_eq!((r.width, r.height), (2, 3));
        // Left column of the original (0, 7) becomes the top row, read right to left: 7 0.
        assert_eq!(r.pixels, vec![7, 0, 0, 5, 8, 6]);
        // Handle was the top-right pixel (6); it is the bottom-right one now.
        assert_eq!(r.handle, (1, 2));
        assert_eq!(r.get(r.handle.0, r.handle.1), Some(6));
        let mut b = sample();
        for _ in 0..4 {
            b = rotate_90(&b);
        }
        assert_eq!(b, sample());
    }

    #[test]
    fn double_and_halve() {
        let d = double(&sample(), true, true);
        assert_eq!((d.width, d.height), (6, 4));
        assert_eq!(d.handle, (4, 0));
        assert_eq!(d.get(2, 0), Some(5));
        assert_eq!(d.get(3, 1), Some(5));
        let dh = double(&sample(), true, false);
        assert_eq!((dh.width, dh.height, dh.handle), (6, 2, (4, 0)));
        let dv = double(&sample(), false, true);
        assert_eq!((dv.width, dv.height, dv.handle), (3, 4, (2, 0)));
        let back = halve(&d);
        assert_eq!(back.pixels, vec![0, 5, 6, 7, 0, 8]);
        assert_eq!((back.width, back.height, back.handle), (3, 2, (2, 0)));
        let tiny = BrushImage {
            width: 1,
            height: 1,
            pixels: vec![4],
            handle: (0, 0),
            transparent: None,
        };
        let h = halve(&tiny);
        assert_eq!((h.width, h.height, h.pixels.clone()), (1, 1, vec![4]));
    }

    #[test]
    fn halve_keeps_thin_lines() {
        let line = BrushImage {
            width: 4,
            height: 2,
            pixels: vec![0, 0, 3, 0, 0, 0, 0, 0],
            handle: (0, 0),
            transparent: Some(0),
        };
        assert_eq!(halve(&line).pixels, vec![0, 3]);
    }

    #[test]
    fn outline_grows_by_one_and_trim_undoes_it() {
        let o = outline(&sample(), 9);
        assert_eq!((o.width, o.height, o.handle), (5, 4, (3, 1)));
        assert_eq!(o.get(0, 0), Some(0), "corner far from opaque pixels");
        assert_eq!(o.get(1, 0), Some(9), "ring pixel next to an opaque one");
        assert_eq!(o.get(2, 1), Some(5), "old pixel kept");
        assert_eq!(
            o.get(1, 1),
            Some(9),
            "old transparent pixel touching opaque ones"
        );
        assert_eq!(o.get(4, 3), Some(9));
        let t = trim(&o).unwrap();
        assert_eq!((t.width, t.height), (3, 2));
        assert_eq!(t.handle, (2, 0));
        // A hole next to opaque pixels was painted too (9); the corner hole is eroded again.
        assert_eq!(t.pixels, vec![0, 5, 6, 7, 9, 8]);
    }

    #[test]
    fn outline_of_an_opaque_brush_adds_a_ring_and_trim_removes_it() {
        let b = BrushImage {
            width: 2,
            height: 2,
            pixels: vec![1; 4],
            handle: (1, 1),
            transparent: None,
        };
        let o = outline(&b, 7);
        assert_eq!((o.width, o.height), (4, 4));
        assert_eq!(o.get(0, 0), Some(7));
        assert_eq!(o.get(1, 1), Some(1));
        assert_eq!(
            trim(&o).unwrap(),
            BrushImage {
                handle: (1, 1),
                ..b
            }
        );
    }

    #[test]
    fn outline_leaves_far_pixels_transparent() {
        let b = BrushImage {
            width: 1,
            height: 1,
            pixels: vec![5],
            handle: (0, 0),
            transparent: Some(0),
        };
        let o = outline(&b, 9);
        assert_eq!(o.pixels, vec![9, 9, 9, 9, 5, 9, 9, 9, 9]);
        let wide = BrushImage {
            width: 3,
            height: 1,
            pixels: vec![5, 0, 0],
            handle: (0, 0),
            transparent: Some(0),
        };
        let o = outline(&wide, 9);
        assert_eq!(o.get(4, 1), Some(0), "two pixels away from the opaque one");
        assert_eq!(o.get(0, 1), Some(9));
    }

    #[test]
    fn trim_of_a_solid_one_pixel_brush_is_nothing() {
        let b = BrushImage {
            width: 1,
            height: 1,
            pixels: vec![5],
            handle: (0, 0),
            transparent: Some(0),
        };
        assert!(trim(&b).is_none());
    }

    #[test]
    fn colour_operations() {
        let r = replace_color(&sample(), 5, 3);
        assert_eq!(r.pixels, vec![0, 3, 6, 7, 0, 8]);
        let r = replace_color(&sample(), 0, 3);
        assert_eq!(r.pixels, sample().pixels, "transparent pixels stay");
        let s = swap_colors(&sample(), 5, 6);
        assert_eq!(s.pixels, vec![0, 6, 5, 7, 0, 8]);
        let s = swap_colors(&sample(), 0, 7);
        assert_eq!(s.transparent, Some(7));
        assert_eq!(s.pixels, vec![7, 5, 6, 0, 7, 8]);
        let t = set_transparent(&sample(), 5);
        assert_eq!(t.transparent, Some(5));
        assert_eq!(t.pixels, sample().pixels);
    }

    #[test]
    fn handle_positions_cycle() {
        let b = sample();
        let c = handle_center(&b);
        assert_eq!(c.handle, (1, 1));
        let mut h = c;
        let mut seen = Vec::new();
        for _ in 0..5 {
            h = handle_next_corner(&h);
            seen.push(h.handle);
        }
        assert_eq!(seen, vec![(0, 0), (2, 0), (2, 1), (0, 1), (1, 1)]);
        let odd = BrushImage {
            handle: (2, 1),
            ..b.clone()
        };
        assert_eq!(handle_next_corner(&odd).handle, (0, 1));
        let lost = BrushImage {
            handle: (1, 0),
            ..b
        };
        assert_eq!(handle_next_corner(&lost).handle, (0, 0));
    }
}
