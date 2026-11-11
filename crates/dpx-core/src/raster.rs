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

/// Steps of the Bresenham line from `a` to `b` along its major axis: it has this many points
/// plus one.
pub fn line_steps(a: (i32, i32), b: (i32, i32)) -> u32 {
    let dx = (i64::from(b.0) - i64::from(a.0)).unsigned_abs();
    let dy = (i64::from(b.1) - i64::from(a.1)).unsigned_abs();
    u32::try_from(dx.max(dy)).unwrap_or(u32::MAX)
}

/// Point `i` (`0..=line_steps(a, b)`) of [`line_points`]`(a, b)`, computed directly, so a
/// clipped line can start anywhere along it.
pub fn line_point(a: (i32, i32), b: (i32, i32), i: u32) -> (i32, i32) {
    let dx = (i64::from(b.0) - i64::from(a.0)).abs();
    let dy = (i64::from(b.1) - i64::from(a.1)).abs();
    let sx = if a.0 < b.0 { 1 } else { -1 };
    let sy = if a.1 < b.1 { 1 } else { -1 };
    let i = i64::from(i);
    // The major axis moves every step; the minor axis steps where the line passes the middle
    // between two pixels (rounding half down), exactly as the error term of `line_points`
    // decides (checked against it for every slope in the tests).
    let (ox, oy) = if dx >= dy {
        if dx == 0 {
            return a;
        }
        (i, (2 * i * dy + dx) / (2 * dx))
    } else {
        ((2 * i * dx + dy) / (2 * dy), i)
    };
    (
        (i64::from(a.0) + sx * ox) as i32,
        (i64::from(a.1) + sy * oy) as i32,
    )
}

/// The steps `i0..=i1` of the line from `a` to `b` whose points may lie inside `r`, `None`
/// if none can. Conservative: a point just outside `r` may be included, a point inside never
/// left out. Lines far off the page cost nothing this way.
pub fn clip_steps(a: (i32, i32), b: (i32, i32), r: Rect) -> Option<(u32, u32)> {
    if r.is_empty() {
        return None;
    }
    let n = line_steps(a, b);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (s, e, lo, hi) in [
        (a.0, b.0, r.x, r.x + r.w - 1),
        (a.1, b.1, r.y, r.y + r.h - 1),
    ] {
        // Line points lie within half a pixel of the exact line; one pixel of margin.
        let (lo, hi) = (f64::from(lo) - 1.0, f64::from(hi) + 1.0);
        let (s, d) = (f64::from(s), f64::from(e) - f64::from(s));
        if d == 0.0 {
            if s < lo || s > hi {
                return None;
            }
            continue;
        }
        let (u0, u1) = ((lo - s) / d, (hi - s) / d);
        t0 = t0.max(u0.min(u1));
        t1 = t1.min(u0.max(u1));
    }
    if t0 > t1 {
        return None;
    }
    let n = f64::from(n);
    let i0 = (t0 * n).floor().clamp(0.0, n) as u32;
    let i1 = (t1 * n).ceil().clamp(0.0, n) as u32;
    (i0 <= i1).then_some((i0, i1))
}

/// Hotspot positions at which a brush covering `extent` (its pixels relative to the hotspot)
/// touches a `width`×`height` image: the image grown by the brush extent.
pub fn reach(extent: Rect, width: u32, height: u32) -> Rect {
    Rect::new(
        -(extent.x + extent.w - 1),
        -(extent.y + extent.h - 1),
        width as i32 + extent.w - 1,
        height as i32 + extent.h - 1,
    )
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

    /// Where custom brush `b` lands on `img` with its handle at (`x`, `y`): the page position
    /// of the brush's top-left pixel, and the rectangle of brush pixels (in brush
    /// coordinates) that fall on the image.
    fn span(b: &BrushImage, img: &IndexedImage, x: i32, y: i32) -> ((i32, i32), Rect) {
        let (ox, oy) = (x - b.handle.0, y - b.handle.1);
        let (w, h) = (b.width as i32, b.height as i32);
        let (x0, x1) = ((-ox).max(0), (img.width() as i32 - ox).min(w));
        let (y0, y1) = ((-oy).max(0), (img.height() as i32 - oy).min(h));
        ((ox, oy), Rect::new(x0, y0, x1 - x0, y1 - y0))
    }

    /// Stamps the brush with its hotspot at (`x`, `y`); `motion` is the movement since the
    /// previous stamp of the same path.
    pub fn stamp(&self, img: &mut IndexedImage, x: i32, y: i32, motion: (i32, i32)) {
        let ctx = PaintContext { motion, ..self.ctx };
        if self.mode == DrawMode::Smear {
            self.smear(img, x, y, &ctx);
            return;
        }
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
        let ((ox, oy), s) = Self::span(b, img, x, y);
        for by in s.y..s.y + s.h {
            for bx in s.x..s.x + s.w {
                if let Some(c) = custom_color(b, bx, by, self.mode, self.color, ctx.right) {
                    let cover = paint::edge_cover(b, bx, by, self.fx.antialias);
                    paint::plot_fx(img, ox + bx, oy + by, c, self.mode, &ctx, &self.fx, cover);
                }
            }
        }
    }

    /// Smear (ADR-19): every brush pixel takes the colour `motion` behind it on the page as it
    /// was before this stamp, so each stamp moves the colours by exactly `motion`, whatever
    /// the direction and the order of the brush pixels.
    fn smear(&self, img: &mut IndexedImage, x: i32, y: i32, ctx: &PaintContext) {
        let (mx, my) = ctx.motion;
        if (mx, my) == (0, 0) {
            // Every pixel would take its own colour.
            return;
        }
        let Some(b) = self.custom else {
            let source: Vec<Option<u8>> = self
                .offsets
                .iter()
                .map(|&(dx, dy)| img.get(x + dx - mx, y + dy - my))
                .collect();
            for (&(dx, dy), s) in self.offsets.iter().zip(source) {
                paint::smear_pixel(img, x + dx, y + dy, s, ctx);
            }
            return;
        };
        let ((ox, oy), s) = Self::span(b, img, x, y);
        if s.is_empty() {
            return;
        }
        // The stamped rectangle `motion` back, as it is before the stamp.
        let from = Rect::new(ox + s.x - mx, oy + s.y - my, s.w, s.h).intersect(img.bounds());
        let source = img.crop(from);
        for by in s.y..s.y + s.h {
            for bx in s.x..s.x + s.w {
                if custom_color(b, bx, by, self.mode, self.color, ctx.right).is_some() {
                    let (px, py) = (ox + bx, oy + by);
                    let c = source.get(px - mx - from.x, py - my - from.y);
                    paint::smear_pixel(img, px, py, c, ctx);
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
/// Stamps whose brush cannot touch the image are skipped (they would change nothing), and
/// lines are clipped to the image grown by the brush extent first, so painting far off the
/// page costs nothing.
pub struct Stamper<'a> {
    pen: Pen<'a>,
    last: Option<(i32, i32)>,
    /// The pen's extent; `None` for an empty brush, which paints nothing.
    extent: Option<Rect>,
}

impl<'a> Stamper<'a> {
    pub fn new(pen: Pen<'a>) -> Self {
        Self {
            pen,
            last: None,
            extent: pen.extent(),
        }
    }

    /// Hotspot positions whose stamp can touch `img`.
    fn reach(&self, img: &IndexedImage) -> Rect {
        self.extent
            .map_or_else(Rect::default, |e| reach(e, img.width(), img.height()))
    }

    /// One stamp at `p`.
    pub fn at(&mut self, img: &mut IndexedImage, p: (i32, i32)) {
        let motion = self.last.map_or((0, 0), |l| (p.0 - l.0, p.1 - l.1));
        self.last = Some(p);
        if self.reach(img).contains(p.0, p.1) {
            self.pen.stamp(img, p.0, p.1, motion);
        }
    }

    /// A stamp on every pixel of the Bresenham line from `a` to `b`, both ends included. Only
    /// the part of the line near the image is walked; the stamps keep the motion they have on
    /// the whole line.
    pub fn line(&mut self, img: &mut IndexedImage, a: (i32, i32), b: (i32, i32)) {
        if let Some((i0, i1)) = clip_steps(a, b, self.reach(img)) {
            if i0 > 0 {
                self.last = Some(line_point(a, b, i0 - 1));
            }
            for i in i0..=i1 {
                self.at(img, line_point(a, b, i));
            }
        }
        self.last = Some(b);
    }
}

/// Pixels one stamp of a brush (built-in `offsets`, or `custom`) may plot on a
/// `width`×`height` image at most (work estimate).
pub fn stamp_cost(
    offsets: &[(i32, i32)],
    custom: Option<&BrushImage>,
    width: u32,
    height: u32,
) -> u64 {
    match custom {
        Some(b) => u64::from(b.width.min(width)) * u64::from(b.height.min(height)),
        None => offsets.len() as u64,
    }
}

/// Stamps a [`Stamper`] makes in `reach` (see [`reach`]) for the line from `a` to `b`, at
/// most (work estimate).
pub fn line_stamps(a: (i32, i32), b: (i32, i32), reach: Rect) -> u64 {
    clip_steps(a, b, reach).map_or(0, |(i0, i1)| u64::from(i1 - i0) + 1)
}

/// Stamps a [`Stamper`] makes in `reach` for a path through `points`, at most: one per point
/// (`lines` false) or along the lines between consecutive points, and back to the first
/// point when `closed` (work estimate).
pub fn path_stamps(points: &[(i32, i32)], lines: bool, closed: bool, reach: Rect) -> u64 {
    if !lines || points.len() < 2 {
        return points.iter().filter(|p| reach.contains(p.0, p.1)).count() as u64;
    }
    let n = points.len();
    let close = (closed && n > 2).then(|| (points[n - 1], points[0]));
    points
        .windows(2)
        .map(|w| (w[0], w[1]))
        .chain(close)
        .map(|(a, b)| line_stamps(a, b, reach))
        .sum()
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
    // The edges that cross rows of the clip, by their first such row. Rows are half-open in
    // y, so a vertex shared by two edges is counted once. Only the active edges' crossings
    // of one row are held at a time (memory stays proportional to the number of edges).
    let mut edges = polygon_edges(points, y0, y1);
    edges.sort_by_key(|e| e.rows.0);
    let mut active: Vec<Edge> = Vec::new();
    let mut xs: Vec<f64> = Vec::new();
    let mut next = 0;
    for y in y0..=y1 {
        while next < edges.len() && edges[next].rows.0 <= y {
            active.push(edges[next]);
            next += 1;
        }
        active.retain(|e| e.rows.1 > y);
        xs.clear();
        xs.extend(active.iter().map(|e| e.x_at(y)));
        xs.sort_by(f64::total_cmp);
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
        if let Some((i0, i1)) = clip_steps(a, b, clip) {
            for i in i0..=i1 {
                let (x, y) = line_point(a, b, i);
                mark(x, y);
            }
        }
    }
    let w = clip.w as usize;
    for (i, _) in mask.iter().enumerate().filter(|(_, on)| **on) {
        f(clip.x + (i % w) as i32, clip.y + (i / w) as i32);
    }
}

/// A polygon edge that crosses the rows `rows.0..rows.1` of the fill.
#[derive(Clone, Copy, Debug)]
struct Edge {
    rows: (i32, i32),
    top: (i32, i32),
    dx: f64,
    dy: f64,
}

impl Edge {
    /// Where the edge crosses the centre line of row `y`.
    fn x_at(&self, y: i32) -> f64 {
        f64::from(self.top.0) + f64::from(y - self.top.1) * self.dx / self.dy
    }
}

/// The edges of the closed polygon through `points` that cross a row of `y0..=y1`.
fn polygon_edges(points: &[(i32, i32)], y0: i32, y1: i32) -> Vec<Edge> {
    let mut edges = Vec::new();
    for (i, &a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        if a.1 == b.1 {
            continue;
        }
        let (top, bot) = if a.1 < b.1 { (a, b) } else { (b, a) };
        let rows = (top.1.max(y0), bot.1.min(y1 + 1));
        if rows.0 < rows.1 {
            edges.push(Edge {
                rows,
                top,
                dx: f64::from(bot.0 - top.0),
                dy: f64::from(bot.1 - top.1),
            });
        }
    }
    edges
}

/// What [`fill_polygon_pen`] costs on a `width`×`height` image, at most (work estimate): the
/// edge crossings of every row (each `crossing_cost`, for the sort), the edge pixels, the
/// clipped bounding box once for the mask and once more for plotting (each pixel
/// `plot_cost`).
pub fn polygon_work(
    points: &[(i32, i32)],
    (width, height): (u32, u32),
    crossing_cost: u64,
    plot_cost: u64,
) -> u64 {
    let Some(bounds) = Rect::from_points(points) else {
        return 0;
    };
    let clip = bounds.intersect(Rect::new(0, 0, width as i32, height as i32));
    if clip.is_empty() {
        return 0;
    }
    let crossings: u64 = polygon_edges(points, clip.y, clip.y + clip.h - 1)
        .iter()
        .map(|e| (e.rows.1 - e.rows.0) as u64)
        .sum();
    let n = points.len();
    let outline: u64 = (0..n)
        .map(|i| line_stamps(points[i], points[(i + 1) % n], clip))
        .sum();
    let area = clip.w as u64 * clip.h as u64;
    crossings * crossing_cost + outline + area * (1 + plot_cost) + n as u64
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

    #[test]
    fn line_point_is_line_points_for_every_slope() {
        for dx in -40..=40 {
            for dy in -40..=40 {
                let (a, b) = ((3, -7), (3 + dx, -7 + dy));
                let pts = line_points(a, b);
                assert_eq!(pts.len() as u32, line_steps(a, b) + 1);
                for (i, &p) in pts.iter().enumerate() {
                    assert_eq!(line_point(a, b, i as u32), p, "{a:?}->{b:?} step {i}");
                }
            }
        }
        // Long lines with awkward slopes, and the coordinate limits.
        for (a, b) in [
            ((-1_000_000, -1_000_000), (1_000_000, 999_999)),
            ((5, 9), (-7_771, 2_903)),
            ((0, 0), (12_345, -54_321)),
        ] {
            let pts = line_points(a, b);
            for i in (0..pts.len()).step_by(997).chain([pts.len() - 1]) {
                assert_eq!(line_point(a, b, i as u32), pts[i], "{a:?}->{b:?} step {i}");
            }
        }
    }

    #[test]
    fn clip_steps_keeps_every_point_inside() {
        let r = Rect::new(-3, 2, 9, 5);
        let ends = [-20, -4, -3, 0, 2, 5, 6, 7, 30];
        for &ax in &ends {
            for &ay in &ends {
                for &bx in &ends {
                    for &by in &ends {
                        let (a, b) = ((ax, ay), (bx, by));
                        let inside: Vec<u32> = (0..=line_steps(a, b))
                            .filter(|&i| {
                                let p = line_point(a, b, i);
                                r.contains(p.0, p.1)
                            })
                            .collect();
                        match clip_steps(a, b, r) {
                            None => assert!(inside.is_empty(), "{a:?}->{b:?}"),
                            Some((i0, i1)) => {
                                assert!(inside.iter().all(|i| (i0..=i1).contains(i)));
                                // Only the margin's extra steps at either end: one pixel
                                // on the minor axis is n / minor steps of the line.
                                let n = line_steps(a, b);
                                let minor = (bx - ax).abs().min((by - ay).abs()) as u32;
                                let slack = if minor == 0 {
                                    2
                                } else {
                                    2 + (2 * n).div_ceil(minor)
                                };
                                if let (Some(&f), Some(&l)) = (inside.first(), inside.last()) {
                                    assert!(f - i0 <= slack && i1 - l <= slack, "{a:?}->{b:?}");
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(clip_steps((-1_000_000, 0), (-500, 0), r), None);
        assert_eq!(clip_steps((0, 0), (5, 5), Rect::default()), None);
    }

    /// `stroke_pen` as it was before clipping: every line pixel stamped, motion from the
    /// previous stamp.
    fn naive_stroke(img: &mut IndexedImage, pen: &Pen, points: &[(i32, i32)]) {
        let mut last: Option<(i32, i32)> = None;
        for pair in points.windows(2) {
            for p in line_points(pair[0], pair[1]) {
                let motion = last.map_or((0, 0), |l| (p.0 - l.0, p.1 - l.1));
                last = Some(p);
                pen.stamp(img, p.0, p.1, motion);
            }
        }
    }

    #[test]
    fn clipped_strokes_paint_exactly_what_unclipped_ones_did() {
        use crate::palette::Palette;
        let pal = Palette::dpaint_default_32();
        let mut page = IndexedImage::new(24, 16, 0);
        for y in 0..16 {
            for x in 0..24 {
                page.set(x, y, ((x * 3 + y * 5) % 31) as u8 + 1);
            }
        }
        let custom = BrushImage {
            width: 4,
            height: 3,
            pixels: vec![1, 0, 2, 3, 4, 5, 0, 6, 7, 8, 9, 0],
            handle: (5, -2),
            transparent: Some(0),
        };
        let paths: [&[(i32, i32)]; 4] = [
            &[(-500, -300), (600, 400), (-40, 20), (12, 8)],
            &[(-30, 7), (60, 9), (60, -50), (2, 30), (-9, -9)],
            &[(5, 5), (5, 5), (300, -2), (-300, 15)],
            &[(-100, -100), (100, 100), (-100, 100), (100, -100)],
        ];
        for mode in [DrawMode::Matte, DrawMode::Smear] {
            for custom in [None, Some(&custom)] {
                for points in paths {
                    let pen = Pen {
                        offsets: BIG,
                        custom,
                        color: 9,
                        mode,
                        ctx: PaintContext {
                            palette: &pal,
                            range: None,
                            stencil: None,
                            area: None,
                            before: &page,
                            motion: (0, 0),
                            right: false,
                        },
                        fx: Default::default(),
                    };
                    let mut clipped = page.clone();
                    stroke_pen(&mut clipped, &pen, points, true);
                    let mut naive = page.clone();
                    naive_stroke(&mut naive, &pen, points);
                    assert_eq!(
                        clipped,
                        naive,
                        "{mode:?} custom={} {points:?}",
                        custom.is_some()
                    );
                }
            }
        }
    }

    /// `fill_polygon` as it was before the active edge list (one crossing list per row).
    fn naive_fill(img: &mut IndexedImage, points: &[(i32, i32)], color: u8) {
        let bounds = Rect::from_points(points).unwrap();
        let clip = bounds.intersect(img.bounds());
        if clip.is_empty() {
            return;
        }
        let (y0, y1) = (clip.y, clip.y + clip.h - 1);
        let mut rows: Vec<Vec<f64>> = vec![Vec::new(); (y1 - y0 + 1) as usize];
        for (i, &a) in points.iter().enumerate() {
            let b = points[(i + 1) % points.len()];
            if a.1 == b.1 {
                continue;
            }
            let (top, bot) = if a.1 < b.1 { (a, b) } else { (b, a) };
            let (dx, dy) = (f64::from(bot.0 - top.0), f64::from(bot.1 - top.1));
            for y in top.1.max(y0)..bot.1.min(y1 + 1) {
                rows[(y - y0) as usize].push(f64::from(top.0) + f64::from(y - top.1) * dx / dy);
            }
        }
        let mut out = img.clone();
        for (r, xs) in rows.iter_mut().enumerate() {
            xs.sort_by(f64::total_cmp);
            for &[a, b] in xs.as_chunks::<2>().0 {
                let xa = (a.ceil() as i32).max(clip.x);
                let xb = (b.floor() as i32).min(clip.x + clip.w - 1);
                for x in xa..=xb {
                    out.set(x, y0 + r as i32, color);
                }
            }
        }
        for (i, &a) in points.iter().enumerate() {
            for (x, y) in line_points(a, points[(i + 1) % points.len()]) {
                if clip.contains(x, y) {
                    out.set(x, y, color);
                }
            }
        }
        *img = out;
    }

    #[test]
    fn polygon_fill_is_unchanged_by_the_active_edge_list() {
        let polys: [&[(i32, i32)]; 5] = [
            &[(0, 0), (4, 0), (0, 4)],
            &[(-50, -40), (70, 3), (10, 90), (-5, 8), (30, -100)],
            &[(3, 3)],
            &[
                (0, 0),
                (20, 0),
                (20, 15),
                (0, 15),
                (0, 0),
                (5, 5),
                (5, 10),
                (15, 10),
                (15, 5),
                (5, 5),
            ],
            &[(-1000, 7), (1000, 8), (2, -1000), (3, 1000), (-7, -7)],
        ];
        for points in polys {
            let mut a = IndexedImage::new(24, 18, 0);
            fill_polygon(&mut a, points, 1);
            let mut b = IndexedImage::new(24, 18, 0);
            naive_fill(&mut b, points, 1);
            assert_eq!(a, b, "{points:?}");
        }
    }

    #[test]
    fn work_estimates_count_only_what_reaches_the_page() {
        let reach_16x8 = reach(Rect::from_points(BIG).unwrap(), 16, 8);
        // Far off the page: no stamps, however long the lines are.
        let far = [(-1_000_000, -1_000_000), (1_000_000, -999_000)];
        assert_eq!(path_stamps(&far, true, false, reach_16x8), 0);
        // A line across the page costs about its length on the page.
        let across = [(-1_000_000, 4), (1_000_000, 4)];
        let n = path_stamps(&across, true, false, reach_16x8);
        assert!((16..30).contains(&n), "{n}");
        // Dots count where they are.
        assert_eq!(
            path_stamps(&[(0, 0), (500, 0), (3, 3)], false, false, reach_16x8),
            2
        );
        // A polygon far above the page costs nothing; one over it costs its clipped area.
        assert_eq!(
            polygon_work(&[(0, -900), (9, -800), (3, -700)], (16, 8), 16, 1),
            0
        );
        let w = polygon_work(
            &[(-100, -100), (100, -100), (100, 100), (-100, 100)],
            (16, 8),
            16,
            1,
        );
        assert!(w < 16 * 8 * 2 + 2 * 8 * 16 + 100, "{w}");
    }
}
