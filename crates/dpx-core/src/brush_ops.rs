//! Brush menu operations as pure functions on [`BrushImage`] (ADR-19).
//!
//! Every function returns a new brush; the handle follows the pixels it points at. Size and
//! shape operations keep the transparent colour. No function touches the page.
//!
//! No operation makes a brush wider or taller than [`MAX_SIDE`], the limit strokes and shapes
//! accept, so every result can be painted: Stretch, Shear and Bend clamp their amount; Double,
//! Outline and Any Angle return the brush unchanged when the result would not fit.

use crate::paint::BrushImage;

/// Whether a `width`×`height` brush is within [`MAX_SIDE`].
fn fits(width: u32, height: u32) -> bool {
    width <= MAX_SIDE && height <= MAX_SIDE
}

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
/// and/or high. Unchanged when the result would exceed [`MAX_SIDE`].
pub fn double(b: &BrushImage, horizontal: bool, vertical: bool) -> BrushImage {
    let (fx, fy) = (1 + u32::from(horizontal), 1 + u32::from(vertical));
    if !fits(b.width.saturating_mul(fx), b.height.saturating_mul(fy)) {
        return b.clone();
    }
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
/// new ring is painted. Unchanged when the result would exceed [`MAX_SIDE`].
pub fn outline(b: &BrushImage, color: u8) -> BrushImage {
    if !fits(b.width.saturating_add(2), b.height.saturating_add(2)) {
        return b.clone();
    }
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

// ---- Interactive transforms (ADR-30) -------------------------------------------------------

/// Largest side any brush operation produces: [`crate::command::MAX_BRUSH_SIDE`], the limit
/// strokes and shapes accept (it also keeps a wild drag from allocating without bound).
pub const MAX_SIDE: u32 = crate::command::MAX_BRUSH_SIDE;

/// The colour that stands for "nothing" in a transformed brush: its transparent colour, else
/// the lowest colour below `colors` that no pixel uses, else 0 (then colour 0 pixels turn
/// transparent; only a brush that uses every palette colour gets here).
pub fn fill_colour(b: &BrushImage, colors: usize) -> u8 {
    if let Some(t) = b.transparent {
        return t;
    }
    let mut used = [false; 256];
    for &p in &b.pixels {
        used[usize::from(p)] = true;
    }
    (0..colors.clamp(1, 256))
        .find(|&c| !used[c])
        .map_or(0, |c| c as u8)
}

/// Brush ▸ Size ▸ Stretch: the brush resized to `width` x `height` (nearest neighbour,
/// each side 1..=[`MAX_SIDE`]). The handle keeps its relative position.
pub fn stretch(b: &BrushImage, width: u32, height: u32) -> BrushImage {
    let (w, h) = (width.clamp(1, MAX_SIDE), height.clamp(1, MAX_SIDE));
    let handle = (
        (b.handle.0 * w as i32 / b.width as i32).clamp(0, w as i32 - 1),
        (b.handle.1 * h as i32 / b.height as i32).clamp(0, h as i32 - 1),
    );
    let mut out = new_brush(w, h, 0, b, handle);
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = (x * b.width / w, y * b.height / h);
            put(&mut out, x, y, b.pixels[(sy * b.width + sx) as usize]);
        }
    }
    out
}

/// `(sin, cos)` of `degrees`, exact for multiples of 90.
fn sin_cos(degrees: f64) -> (f64, f64) {
    let d = degrees.rem_euclid(360.0);
    if d == 0.0 {
        (0.0, 1.0)
    } else if d == 90.0 {
        (1.0, 0.0)
    } else if d == 180.0 {
        (0.0, -1.0)
    } else if d == 270.0 {
        (-1.0, 0.0)
    } else {
        d.to_radians().sin_cos()
    }
}

/// Where the point (`x`, `y`) of the brush (pixel corner coordinates: the brush covers 0..w,
/// 0..h) ends up in the brush [`rotate_any`] makes of it.
pub fn rotated_point(b: &BrushImage, degrees: f64, (x, y): (f64, f64)) -> (f64, f64) {
    let (s, c) = sin_cos(degrees);
    let (w, h) = (f64::from(b.width), f64::from(b.height));
    let (dw, dh) = rotated_size(b, degrees);
    let (rx, ry) = (x - w / 2.0, y - h / 2.0);
    (
        rx * c - ry * s + f64::from(dw) / 2.0,
        rx * s + ry * c + f64::from(dh) / 2.0,
    )
}

fn rotated_size(b: &BrushImage, degrees: f64) -> (u32, u32) {
    let (s, c) = sin_cos(degrees);
    let (w, h) = (f64::from(b.width), f64::from(b.height));
    // Not clamped: `rotate_any` refuses a result beyond MAX_SIDE instead of cropping it.
    let fit = |v: f64| (v - 1e-6).ceil().clamp(1.0, f64::from(u32::MAX)) as u32;
    (
        fit(w * c.abs() + h * s.abs()),
        fit(w * s.abs() + h * c.abs()),
    )
}

/// Brush ▸ Rotate ▸ Any Angle: rotated clockwise by `degrees` (nearest neighbour). The new
/// area is transparent: the brush's transparent colour, or one picked by [`fill_colour`]
/// from the `colors` of the palette. The handle turns with the pixels. Multiples of 90
/// degrees give the same pixels as [`rotate_90`] and the flips. Unchanged when the rotated
/// brush would exceed [`MAX_SIDE`] (it is never cropped).
pub fn rotate_any(b: &BrushImage, degrees: f64, colors: usize) -> BrushImage {
    let (dw, dh) = rotated_size(b, degrees);
    if !fits(dw, dh) {
        return b.clone();
    }
    let fill = fill_colour(b, colors);
    let (s, c) = sin_cos(degrees);
    let (hx, hy) = rotated_point(
        b,
        degrees,
        (f64::from(b.handle.0) + 0.5, f64::from(b.handle.1) + 0.5),
    );
    let handle = (
        (hx.floor() as i32).clamp(0, dw as i32 - 1),
        (hy.floor() as i32).clamp(0, dh as i32 - 1),
    );
    let mut out = new_brush(dw, dh, fill, b, handle);
    out.transparent = Some(fill);
    let (w, h) = (f64::from(b.width), f64::from(b.height));
    for y in 0..dh {
        for x in 0..dw {
            let (rx, ry) = (
                f64::from(x) + 0.5 - f64::from(dw) / 2.0,
                f64::from(y) + 0.5 - f64::from(dh) / 2.0,
            );
            // Inverse rotation back into the source brush.
            let sx = (rx * c + ry * s + w / 2.0 + 1e-9).floor();
            let sy = (-rx * s + ry * c + h / 2.0 + 1e-9).floor();
            if let Some(p) = b.get(sx as i32, sy as i32) {
                put(&mut out, x, y, p);
            }
        }
    }
    out
}

/// Moves every row (`rows` true) or column of the brush by its own offset along the row or
/// column; rows move sideways, columns up and down. The new area is transparent.
fn displace(b: &BrushImage, offsets: &[i32], rows: bool, colors: usize) -> BrushImage {
    let fill = fill_colour(b, colors);
    let (min, max) = (
        offsets.iter().copied().min().unwrap_or(0).min(0),
        offsets.iter().copied().max().unwrap_or(0).max(0),
    );
    let grow = (max - min) as u32;
    let (w, h) = if rows {
        (b.width + grow, b.height)
    } else {
        (b.width, b.height + grow)
    };
    let handle = if rows {
        (b.handle.0 + offsets[b.handle.1 as usize] - min, b.handle.1)
    } else {
        (b.handle.0, b.handle.1 + offsets[b.handle.0 as usize] - min)
    };
    let mut out = new_brush(w, h, fill, b, handle);
    out.transparent = Some(fill);
    for y in 0..b.height {
        for x in 0..b.width {
            let p = b.pixels[(y * b.width + x) as usize];
            if rows {
                put(
                    &mut out,
                    (x as i32 + offsets[y as usize] - min) as u32,
                    y,
                    p,
                );
            } else {
                put(
                    &mut out,
                    x,
                    (y as i32 + offsets[x as usize] - min) as u32,
                    p,
                );
            }
        }
    }
    out
}

/// Brush ▸ Rotate ▸ Shear: the top row stays, the bottom row moves `dx` pixels sideways
/// (negative = left), rows in between in proportion. The brush grows to hold the result;
/// `dx` is clamped so it stays within [`MAX_SIDE`].
pub fn shear(b: &BrushImage, dx: i32, colors: usize) -> BrushImage {
    let room = MAX_SIDE.saturating_sub(b.width) as i32;
    let dx = dx.clamp(-room, room);
    let span = (b.height as i32 - 1).max(1);
    let offsets: Vec<i32> = (0..b.height as i32)
        .map(|y| (f64::from(dx) * f64::from(y) / f64::from(span)).round() as i32)
        .collect();
    displace(b, &offsets, true, colors)
}

/// Brush ▸ Bend ▸ Horiz (`horizontal`) or Vert: a quadratic bend. The two edges stay; the
/// row (column) at `line` moves `amount` pixels sideways (up or down), the others along
/// the parabola through those three points. `line` is clamped to the inside of the brush,
/// `amount` so the brush stays within [`MAX_SIDE`].
pub fn bend(b: &BrushImage, horizontal: bool, amount: i32, line: i32, colors: usize) -> BrushImage {
    let n = if horizontal { b.height } else { b.width } as i32;
    if n < 3 {
        return b.clone();
    }
    let line = line.clamp(1, n - 2);
    let denom = f64::from(line) * f64::from(n - 1 - line);
    // The middle row (column) moves `peak` times `amount`: more than the bend line itself when
    // that lies near an edge. The brush grows by the largest move.
    let middle = (n - 1) / 2;
    let peak = f64::from(middle) * f64::from(n - 1 - middle) / denom;
    let room = MAX_SIDE.saturating_sub(if horizontal { b.width } else { b.height });
    let limit = (f64::from(room) / peak).floor() as i32;
    let amount = amount.clamp(-limit, limit);
    let offsets: Vec<i32> = (0..n)
        .map(|i| (f64::from(amount) * f64::from(i) * f64::from(n - 1 - i) / denom).round() as i32)
        .collect();
    displace(b, &offsets, horizontal, colors)
}

/// Brush ▸ Handle ▸ Place: the handle at brush coordinates (`x`, `y`), kept inside the brush.
pub fn handle_place(b: &BrushImage, x: i32, y: i32) -> BrushImage {
    BrushImage {
        handle: (
            x.clamp(0, b.width as i32 - 1),
            y.clamp(0, b.height as i32 - 1),
        ),
        ..b.clone()
    }
}

/// Color ▸ Brush ▸ Remap: every colour index `from` of the brush's own palette becomes the
/// nearest colour of `to` (the transparent colour keeps its index). Indices `from` does not
/// have stay. Returns the brush and how many distinct colours changed.
pub fn remap(b: &BrushImage, from: &crate::Palette, to: &crate::Palette) -> (BrushImage, usize) {
    let mut map: [Option<u8>; 256] = [None; 256];
    let mut changed = 0;
    let mut out = b.clone();
    for p in &mut out.pixels {
        let slot = &mut map[usize::from(*p)];
        let new = *slot.get_or_insert_with(|| match from.get(*p) {
            Some(c) if Some(*p) != b.transparent && to.get(*p) != Some(c) => {
                let n = to.nearest(c);
                changed += usize::from(n != *p);
                n
            }
            _ => *p,
        });
        *p = new;
    }
    (out, changed)
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

    #[test]
    fn stretch_resizes_nearest_neighbour_and_scales_the_handle() {
        let b = stretch(&sample(), 6, 4);
        assert_eq!((b.width, b.height), (6, 4));
        assert_eq!(&b.pixels[0..6], &[0, 0, 5, 5, 6, 6]);
        assert_eq!(&b.pixels[18..24], &[7, 7, 0, 0, 8, 8]);
        assert_eq!(b.handle, (4, 0));
        assert_eq!(b.transparent, Some(0));
        let s = stretch(&sample(), 1, 1);
        assert_eq!(s.pixels, vec![0]);
        assert_eq!(stretch(&sample(), 0, 5000).height, MAX_SIDE);
    }

    #[test]
    fn rotate_any_is_exact_for_quarter_turns() {
        let b = sample();
        assert_eq!(rotate_any(&b, 0.0, 32), b);
        assert_eq!(rotate_any(&b, 90.0, 32), rotate_90(&b));
        assert_eq!(
            rotate_any(&b, 180.0, 32),
            flip_vertical(&flip_horizontal(&b))
        );
        assert_eq!(
            rotate_any(&b, -90.0, 32),
            rotate_90(&rotate_90(&rotate_90(&b)))
        );
        assert_eq!(rotate_any(&b, 450.0, 32), rotate_90(&b));
    }

    #[test]
    fn rotate_any_grows_the_brush_and_fills_with_transparent() {
        let b = BrushImage {
            width: 4,
            height: 4,
            pixels: vec![5; 16],
            handle: (0, 0),
            transparent: None,
        };
        let r = rotate_any(&b, 45.0, 32);
        assert!(r.width >= 5 && r.height >= 5, "{}x{}", r.width, r.height);
        // No transparent colour before: an unused one is picked (colour 0 here).
        assert_eq!(r.transparent, Some(0));
        assert_eq!(r.get(0, 0), Some(0), "corners are empty");
        let (cx, cy) = (r.width as i32 / 2, r.height as i32 / 2);
        assert_eq!(r.get(cx, cy), Some(5));
        // The handle (top-left corner pixel) is rotated to the top middle.
        assert!(
            r.handle.1 < cy && (r.handle.0 - cx).abs() <= 2,
            "{:?}",
            r.handle
        );
        // A used colour 0 is skipped as the fill colour.
        let mut c = b.clone();
        c.pixels[0] = 0;
        assert_eq!(fill_colour(&c, 32), 1);
    }

    #[test]
    fn rotated_point_follows_the_rotation() {
        let b = sample();
        // Top-left corner (0,0) of a 3x2 brush turned 90 degrees clockwise: top-right of 2x3.
        assert_eq!(rotated_point(&b, 90.0, (0.0, 0.0)), (2.0, 0.0));
        let (x, y) = rotated_point(&b, 90.0, (0.0, 2.0));
        assert!((x - 0.0).abs() < 1e-9 && (y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn shear_moves_rows_in_proportion() {
        let b = BrushImage {
            width: 2,
            height: 3,
            pixels: vec![1, 2, 3, 4, 5, 6],
            handle: (0, 2),
            transparent: Some(0),
        };
        let s = shear(&b, 4, 32);
        assert_eq!((s.width, s.height), (6, 3));
        assert_eq!(&s.pixels[0..6], &[1, 2, 0, 0, 0, 0]);
        assert_eq!(&s.pixels[6..12], &[0, 0, 3, 4, 0, 0]);
        assert_eq!(&s.pixels[12..18], &[0, 0, 0, 0, 5, 6]);
        assert_eq!(s.handle, (4, 2), "the handle stays on its pixel");
        let l = shear(&b, -2, 32);
        assert_eq!(l.width, 4);
        assert_eq!(&l.pixels[0..4], &[0, 0, 1, 2]);
        assert_eq!(&l.pixels[8..12], &[5, 6, 0, 0]);
        assert_eq!(l.handle, (0, 2));
        assert_eq!(shear(&b, 0, 32), b);
    }

    #[test]
    fn bend_is_a_parabola_through_edges_and_line() {
        let b = BrushImage {
            width: 1,
            height: 5,
            pixels: vec![1, 2, 3, 4, 5],
            handle: (0, 0),
            transparent: Some(0),
        };
        // Bend line at row 2 by 4: offsets 0, 3, 4, 3, 0.
        let r = bend(&b, true, 4, 2, 32);
        assert_eq!((r.width, r.height), (5, 5));
        for (y, x) in [(0, 0), (1, 3), (2, 4), (3, 3), (4, 0)] {
            assert_eq!(r.get(x, y), Some(y as u8 + 1), "row {y}");
        }
        // Negative amounts bend the other way and keep every pixel.
        let n = bend(&b, true, -4, 2, 32);
        assert_eq!(n.get(4, 0), Some(1));
        assert_eq!(n.get(0, 2), Some(3));
        assert_eq!(n.pixels.iter().filter(|&&p| p != 0).count(), 5);
        // Vertical bend works on columns.
        let v = bend(&rotate_90(&b), false, 4, 2, 32);
        assert_eq!((v.width, v.height), (5, 5));
        assert_eq!(bend(&b, true, 0, 2, 32), b);
        // Too small to bend.
        let tiny = stretch(&b, 1, 2);
        assert_eq!(bend(&tiny, true, 3, 1, 32), tiny);
    }

    /// A solid brush of colour 5 (0 transparent), handle top-left.
    fn solid(width: u32, height: u32) -> BrushImage {
        BrushImage {
            width,
            height,
            pixels: vec![5; width as usize * height as usize],
            handle: (0, 0),
            transparent: Some(0),
        }
    }

    #[test]
    fn growing_operations_stay_within_the_shared_limit() {
        assert_eq!(MAX_SIDE, crate::command::MAX_BRUSH_SIDE);
        // Double and Outline refuse (the brush stays) instead of cropping.
        let half = solid(MAX_SIDE / 2 + 1, 3);
        assert_eq!(double(&half, true, false), half);
        assert_eq!(
            double(&half, false, true).height,
            6,
            "the other side may double"
        );
        let full = solid(MAX_SIDE - 1, 2);
        assert_eq!(outline(&full, 9), full);
        assert_eq!(outline(&solid(MAX_SIDE - 2, 2), 9).width, MAX_SIDE);
        // Shear clamps the move to the room left.
        let s = shear(&solid(MAX_SIDE - 10, 4), 1_000, 32);
        assert_eq!(s.width, MAX_SIDE);
        assert_eq!(
            s.pixels.iter().filter(|&&p| p == 5).count(),
            4 * (MAX_SIDE - 10) as usize
        );
        // Bend near an edge moves the middle much more than the bend line; clamped to fit.
        let b = bend(&solid(600, 4), false, 100_000, 1, 32);
        assert!(b.height <= MAX_SIDE, "{}", b.height);
        assert_eq!(b.pixels.iter().filter(|&&p| p == 5).count(), 2400);
        let small = bend(&solid(600, 4), false, 2, 1, 32);
        assert!(
            small.height > 4 + 2,
            "the middle moves more than the bend line"
        );
        // Any Angle refuses a result beyond the limit (3000 px turned 45 degrees is 4243 px)
        // and never crops.
        let big = solid(3000, 3000);
        assert!(rotate_any(&big, 45.0, 32) == big, "refused, not cropped");
        let long = solid(MAX_SIDE, 8);
        let turned = rotate_any(&long, 90.0, 32);
        assert_eq!((turned.width, turned.height), (8, MAX_SIDE));
        let diagonal = rotate_any(&long, 45.0, 32);
        assert_eq!((diagonal.width, diagonal.height), (2902, 2902));
        assert_eq!(rotated_size(&big, 45.0), (4243, 4243));
    }

    #[test]
    fn handle_place_stays_inside() {
        let b = handle_place(&sample(), 1, 1);
        assert_eq!(b.handle, (1, 1));
        assert_eq!(handle_place(&sample(), -4, 9).handle, (0, 1));
        assert_eq!(handle_place(&sample(), 1, 1).pixels, sample().pixels);
    }

    #[test]
    fn remap_fits_colours_to_the_new_palette() {
        use crate::Rgb8;
        let from = crate::Palette::dpaint_default_32();
        let mut colors = from.colors().to_vec();
        colors[5] = Rgb8::new(1, 2, 3);
        let to = crate::Palette::new(colors).unwrap();
        let (r, changed) = remap(&sample(), &from, &to);
        assert_eq!(r.transparent, Some(0));
        assert_eq!(r.pixels[0], 0);
        let want = to.nearest(from.get(5).unwrap());
        assert_ne!(want, 5);
        assert_eq!(r.pixels[1], want);
        assert_eq!(changed, 1);
        let (same, none) = remap(&sample(), &from, &from);
        assert_eq!((same, none), (sample(), 0));
    }
}
