//! Basic raster algorithms on integer pixel coordinates.
//!
//! These will move to the `dpx-raster` crate once more tools exist (see ARCHITECTURE.md).
//! The drawing commands stamp their brush with a [`Pen`], which sends every pixel through
//! [`paint::plot`] so the drawing mode and the stencil apply (ADR-18).

use crate::image::{IndexedImage, Rect};
use crate::paint::{self, BrushImage, DrawMode, PaintContext};

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
    extent_bounds(Rect::from_points(brush)?, points)
}

/// Unclipped bounding rect of a brush whose pixels cover `extent` (relative to its hotspot)
/// stamped at every one of `points`; `None` if `points` is empty.
pub fn extent_bounds(extent: Rect, points: &[(i32, i32)]) -> Option<Rect> {
    let p = Rect::from_points(points)?;
    Some(Rect::new(
        p.x + extent.x,
        p.y + extent.y,
        p.w + extent.w - 1,
        p.h + extent.h - 1,
    ))
}

/// The pixels of a custom brush relative to its handle; `None` for an empty brush.
pub fn brush_extent(brush: &BrushImage) -> Option<Rect> {
    (brush.width > 0 && brush.height > 0).then(|| {
        Rect::new(
            -brush.handle.0,
            -brush.handle.1,
            brush.width as i32,
            brush.height as i32,
        )
    })
}

/// The colour custom brush pixel (`x`, `y`) paints, `None` where it paints nothing (ADR-18).
/// `color` is FG, or BG when `right`.
///
/// - Color: the brush's shape (its non-transparent pixels) in `color`.
/// - Replace: every pixel in its own colour, transparent ones too (they hold the BG colour of
///   the pick-up); with the right button the whole rectangle in `color`.
/// - Matte and every other mode: the brush's own colours, transparent pixels skipped; with the
///   right button the shape in `color`.
pub fn custom_color(
    brush: &BrushImage,
    x: i32,
    y: i32,
    mode: DrawMode,
    color: u8,
    right: bool,
) -> Option<u8> {
    let own = brush.get(x, y)?;
    let opaque = Some(own) != brush.transparent;
    match mode {
        DrawMode::Color => opaque.then_some(color),
        DrawMode::Replace => Some(if right { color } else { own }),
        _ => opaque.then_some(if right { color } else { own }),
    }
}

/// How a drawing command paints (ADR-18): the brush, the colour and the drawing mode. Every
/// pixel goes through [`paint::plot`] with [`Pen::ctx`], so the stencil applies everywhere.
#[derive(Clone, Copy, Debug)]
pub struct Pen<'a> {
    /// Built-in brush: pixel offsets from its hotspot, painted in `color`. Not used when
    /// `custom` is set.
    pub offsets: &'a [(i32, i32)],
    /// Custom brush, painted as [`custom_color`] says.
    pub custom: Option<&'a BrushImage>,
    /// FG, or BG with the right button (`ctx.right`).
    pub color: u8,
    pub mode: DrawMode,
    /// Palette, range, stencil, the page before the command and the button; `motion` is
    /// set for each stamp.
    pub ctx: PaintContext<'a>,
    /// The Effect menu: Translucency, Process and the AntiAlias of custom brush edges
    /// (ADR-31).
    pub fx: paint::PaintFx,
}

impl Pen<'_> {
    /// The brush's pixels relative to its hotspot; `None` for an empty brush.
    pub fn extent(&self) -> Option<Rect> {
        match self.custom {
            Some(b) => brush_extent(b),
            None => Rect::from_points(self.offsets),
        }
    }

    /// Stamps the brush with its hotspot at (`x`, `y`); `motion` is the movement since the
    /// previous stamp of the same path.
    pub fn stamp(&self, img: &mut IndexedImage, x: i32, y: i32, motion: (i32, i32)) {
        let ctx = PaintContext { motion, ..self.ctx };
        let Some(b) = self.custom else {
            for &(dx, dy) in self.offsets {
                paint::plot_fx(
                    img,
                    x + dx,
                    y + dy,
                    self.color,
                    self.mode,
                    &ctx,
                    &self.fx,
                    100,
                );
            }
            return;
        };
        // Only the brush rows and columns that land on the image.
        let (ox, oy) = (x - b.handle.0, y - b.handle.1);
        let (w, h) = (b.width as i32, b.height as i32);
        let (x0, x1) = ((-ox).max(0), (img.width() as i32 - ox).min(w));
        let (y0, y1) = ((-oy).max(0), (img.height() as i32 - oy).min(h));
        for by in y0..y1 {
            for bx in x0..x1 {
                if let Some(c) = custom_color(b, bx, by, self.mode, self.color, ctx.right) {
                    let cover = paint::edge_cover(b, bx, by, self.fx.antialias);
                    paint::plot_fx(img, ox + bx, oy + by, c, self.mode, &ctx, &self.fx, cover);
                }
            }
        }
    }

    /// One pixel in the pen's colour (filled shapes).
    pub fn dot(&self, img: &mut IndexedImage, x: i32, y: i32) {
        let ctx = PaintContext {
            motion: (0, 0),
            ..self.ctx
        };
        paint::plot_fx(img, x, y, self.color, self.mode, &ctx, &self.fx, 100);
    }
}

/// Stamps a [`Pen`] along a path and remembers the previous stamp, for each stamp's motion.
pub struct Stamper<'a> {
    pen: Pen<'a>,
    last: Option<(i32, i32)>,
}

impl<'a> Stamper<'a> {
    pub fn new(pen: Pen<'a>) -> Self {
        Self { pen, last: None }
    }

    /// One stamp at `p`.
    pub fn at(&mut self, img: &mut IndexedImage, p: (i32, i32)) {
        let motion = self.last.map_or((0, 0), |l| (p.0 - l.0, p.1 - l.1));
        self.last = Some(p);
        self.pen.stamp(img, p.0, p.1, motion);
    }

    /// A stamp on every pixel of the Bresenham line from `a` to `b`, both ends included.
    pub fn line(&mut self, img: &mut IndexedImage, a: (i32, i32), b: (i32, i32)) {
        for p in line_points(a, b) {
            self.at(img, p);
        }
    }
}

/// [`stroke`] with a [`Pen`]: the brush stamped at every point, or with `continuous` on every
/// pixel of the lines between consecutive points. Shared ends are stamped twice, as in
/// [`stroke`], so a stroke painted in parts gives the same stamps in the same order.
pub fn stroke_pen(img: &mut IndexedImage, pen: &Pen, points: &[(i32, i32)], continuous: bool) {
    let mut s = Stamper::new(*pen);
    if continuous && points.len() > 1 {
        for pair in points.windows(2) {
            s.line(img, pair[0], pair[1]);
        }
    } else {
        for &p in points {
            s.at(img, p);
        }
    }
}

/// Rounds `num / den` to the nearest integer (halves up); `den` must be positive.
fn div_round(num: i64, den: i64) -> i64 {
    (2 * num + den).div_euclid(2 * den)
}

/// Removes consecutive duplicates.
fn push_point(out: &mut Vec<(i32, i32)>, p: (i32, i32)) {
    if out.last() != Some(&p) {
        out.push(p);
    }
}

/// The outline of an ellipse around `c` with radii `rx`, `ry` (midpoint algorithm), as a
/// closed ring of 8-connected points in drawing order. Equal radii give a circle; a zero
/// radius gives the straight line between the two ends.
pub fn ellipse_points(c: (i32, i32), rx: i32, ry: i32) -> Vec<(i32, i32)> {
    let (rx, ry) = (rx.abs(), ry.abs());
    if rx == 0 || ry == 0 {
        return vec![(c.0 - rx, c.1 - ry), (c.0 + rx, c.1 + ry)];
    }
    // First quadrant from (0, ry) to (rx, 0); all terms are scaled by 4 to stay integral.
    let (a2, b2) = (i64::from(rx) * i64::from(rx), i64::from(ry) * i64::from(ry));
    let (mut x, mut y) = (0i64, i64::from(ry));
    let mut quad = Vec::new();
    let mut d = 4 * b2 - 4 * a2 * y + a2;
    while b2 * x < a2 * y {
        quad.push((x as i32, y as i32));
        if d >= 0 {
            y -= 1;
            d -= 8 * a2 * y;
        }
        x += 1;
        d += 4 * b2 * (2 * x + 1);
    }
    let mut d = b2 * (2 * x + 1) * (2 * x + 1) + 4 * a2 * (y - 1) * (y - 1) - 4 * a2 * b2;
    while y >= 0 {
        quad.push((x as i32, y as i32));
        if d <= 0 {
            x += 1;
            d += 8 * b2 * x;
        }
        y -= 1;
        d += 4 * a2 * (1 - 2 * y);
    }
    let mut ring = Vec::with_capacity(quad.len() * 4);
    for &(x, y) in &quad {
        push_point(&mut ring, (c.0 + x, c.1 + y));
    }
    for &(x, y) in quad.iter().rev() {
        push_point(&mut ring, (c.0 + x, c.1 - y));
    }
    for &(x, y) in &quad {
        push_point(&mut ring, (c.0 - x, c.1 - y));
    }
    for &(x, y) in quad.iter().rev() {
        push_point(&mut ring, (c.0 - x, c.1 + y));
    }
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    ring
}

/// Points along the quadratic curve from `a` to `b` that passes through `through` halfway
/// (the Curve tool's bend point), sampled densely enough that joining consecutive points with
/// lines gives a smooth curve. The ends are exact.
pub fn curve_points(a: (i32, i32), through: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let cheb = |p: (i32, i32), q: (i32, i32)| (p.0 - q.0).abs().max((p.1 - q.1).abs());
    let n = i64::from((cheb(a, through) + cheb(through, b)).max(1)) * 2;
    // B(t) = (1-t)² a + 2t(1-t) c + t² b with the control point c = 2·through − (a+b)/2,
    // so with t = i/n: n²·B = (n-i)² a + i(n-i)(4·through − a − b) + i² b.
    let axis = |a: i32, t: i32, b: i32, i: i64| -> i32 {
        let (a, t, b) = (i64::from(a), i64::from(t), i64::from(b));
        let num = (n - i) * (n - i) * a + i * (n - i) * (4 * t - a - b) + i * i * b;
        div_round(num, n * n) as i32
    };
    let mut out: Vec<(i32, i32)> = Vec::new();
    for i in 0..=n {
        let p = (axis(a.0, through.0, b.0, i), axis(a.1, through.1, b.1, i));
        // Drop staircase corners: a point whose neighbours already touch is redundant.
        // The bend point itself always stays.
        while out.len() >= 2 && out[out.len() - 1] != through && cheb(out[out.len() - 2], p) <= 1 {
            out.pop();
        }
        push_point(&mut out, p);
    }
    out
}

/// Fills the polygon through `points` (closed; even-odd rule, sampled at pixel centres) and
/// also sets every pixel on its edges, so the outline drawn with a 1-pixel brush always lies
/// inside the filled shape. Clipped to the image.
pub fn fill_polygon(img: &mut IndexedImage, points: &[(i32, i32)], color: u8) {
    let (w, h) = (img.width(), img.height());
    polygon_pixels(points, w, h, |x, y| img.set(x, y, color));
}

/// [`fill_polygon`] with a [`Pen`]: every pixel of the shape is plotted once, in the pen's
/// colour and mode.
pub fn fill_polygon_pen(img: &mut IndexedImage, points: &[(i32, i32)], pen: &Pen) {
    let (w, h) = (img.width(), img.height());
    polygon_pixels(points, w, h, |x, y| pen.dot(img, x, y));
}

/// Calls `f` once for every pixel [`fill_polygon`] paints in a `width`×`height` image, row by
/// row.
fn polygon_pixels(points: &[(i32, i32)], width: u32, height: u32, mut f: impl FnMut(i32, i32)) {
    let Some(bounds) = Rect::from_points(points) else {
        return;
    };
    let clip = bounds.intersect(Rect::new(0, 0, width as i32, height as i32));
    if clip.is_empty() {
        return;
    }
    let mut mask = vec![false; clip.w as usize * clip.h as usize];
    let mut mark = |x: i32, y: i32| {
        if clip.contains(x, y) {
            mask[(y - clip.y) as usize * clip.w as usize + (x - clip.x) as usize] = true;
        }
    };
    let (y0, y1) = (clip.y, clip.y + clip.h - 1);
    let mut rows: Vec<Vec<f64>> = vec![Vec::new(); (y1 - y0 + 1) as usize];
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        if a.1 == b.1 {
            continue;
        }
        let (top, bot) = if a.1 < b.1 { (a, b) } else { (b, a) };
        let (dx, dy) = (f64::from(bot.0 - top.0), f64::from(bot.1 - top.1));
        // Half-open in y, so a vertex shared by two edges is counted once.
        for y in top.1.max(y0)..bot.1.min(y1 + 1) {
            let x = f64::from(top.0) + f64::from(y - top.1) * dx / dy;
            rows[(y - y0) as usize].push(x);
        }
    }
    for (r, xs) in rows.iter_mut().enumerate() {
        xs.sort_by(f64::total_cmp);
        let y = y0 + r as i32;
        for &[a, b] in xs.as_chunks::<2>().0 {
            let xa = (a.ceil() as i32).max(clip.x);
            let xb = (b.floor() as i32).min(clip.x + clip.w - 1);
            for x in xa..=xb {
                mark(x, y);
            }
        }
    }
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        for (x, y) in line_points(a, b) {
            mark(x, y);
        }
    }
    let w = clip.w as usize;
    for (i, _) in mask.iter().enumerate().filter(|(_, on)| **on) {
        f(clip.x + (i % w) as i32, clip.y + (i / w) as i32);
    }
}

/// 4-connected flood fill (the Fill tool): the region of the colour at (`x`, `y`) becomes
/// `color`. Returns the bounding rect of the changed pixels, `None` if nothing changed
/// (outside the image, or the region already has the colour).
pub fn flood_fill(img: &mut IndexedImage, x: i32, y: i32, color: u8) -> Option<Rect> {
    let target = img.get(x, y)?;
    if target == color {
        return None;
    }
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut changed = Rect::default();
    let mut stack = vec![(x, y)];
    while let Some((x, y)) = stack.pop() {
        if img.get(x, y) != Some(target) {
            continue;
        }
        let (mut l, mut r) = (x, x);
        while l > 0 && img.get(l - 1, y) == Some(target) {
            l -= 1;
        }
        while r < w - 1 && img.get(r + 1, y) == Some(target) {
            r += 1;
        }
        img.hline(l, r, y, color);
        changed = changed.union(Rect::new(l, y, r - l + 1, 1));
        for ny in [y - 1, y + 1] {
            if !(0..h).contains(&ny) {
                continue;
            }
            let mut i = l;
            while i <= r {
                if img.get(i, ny) == Some(target) {
                    stack.push((i, ny));
                    while i <= r && img.get(i, ny) == Some(target) {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
    Some(changed)
}

/// The part `src` of `img` enlarged `factor` times (nearest neighbour), for the Magnify view.
/// Pixels of `src` outside the image become `outside`.
pub fn magnified(img: &IndexedImage, src: Rect, factor: i32, outside: u8) -> IndexedImage {
    let f = factor.max(1);
    let (w, h) = (src.w.max(0) * f, src.h.max(0) * f);
    let mut out = IndexedImage::new(w as u32, h as u32, outside);
    for y in 0..src.h.max(0) {
        for x in 0..src.w.max(0) {
            if let Some(p) = img.get(src.x + x, src.y + y) {
                out.fill_rect(x * f, y * f, f, f, p);
            }
        }
    }
    out
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
