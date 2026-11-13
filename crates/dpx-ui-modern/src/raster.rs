//! Turns a display list into pixels on the CPU (ADR-47): shapes with `tiny-skia`, text from
//! `ab_glyph` outlines through a small glyph cache, pictures by nearest-neighbour sampling.
//! The window and the headless snapshots run this same code, so a screenshot is what the
//! program shows.
//!
//! Text is unhinted and blended without gamma correction. Glyphs sit on whole pixel rows
//! (the baseline is rounded) and on quarter pixels horizontally.

use std::collections::HashMap;

use ab_glyph::{Font, point};
use tiny_skia::{
    FillRule, LineCap, LineJoin, Mask, Paint, PathBuilder, Pixmap, Stroke as SkStroke, Transform,
};

use crate::font;
use crate::paint::{Color, List, Op, Path, Rect, Seg, Stroke, Weight};

/// A picture a list refers to: straight RGBA8, row by row.
#[derive(Clone, Copy, Debug)]
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    medium: bool,
    id: u16,
    /// The pixel size, by its bits.
    size: u32,
    /// Quarter-pixel position of the pen.
    bin: u8,
}

/// A rasterised glyph: coverage and where it sits relative to the pen's pixel on the
/// baseline.
struct Glyph {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    coverage: Vec<u8>,
}

/// Renders display lists; keeps the glyphs it has rasterised.
#[derive(Default)]
pub struct Renderer {
    glyphs: HashMap<GlyphKey, Option<Glyph>>,
}

/// More cached glyphs than this and the cache starts over (sizes change with the device
/// scale; the interface uses a handful of sizes).
const GLYPH_CACHE_LIMIT: usize = 4096;

fn sk_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn sk_paint(c: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(sk_color(c));
    paint.anti_alias = true;
    paint
}

/// Where a list lands in the image: `scale` physical pixels per point, then moved so that
/// the physical pixel `origin` of the window becomes the image's (0, 0). The origin is in
/// whole pixels, so a region is rasterised as the same part of the whole window (flat
/// surfaces, text and pictures exactly; a few anti-aliased pixels of curves may differ slightly).
#[derive(Clone, Copy, Debug)]
struct Place {
    scale: f32,
    ox: f32,
    oy: f32,
}

impl Place {
    fn x(self, x: f32) -> f32 {
        x * self.scale - self.ox
    }

    fn y(self, y: f32) -> f32 {
        y * self.scale - self.oy
    }
}

/// The outline in physical pixels of the image.
fn sk_path(path: &Path, at: Place) -> Option<tiny_skia::Path> {
    let mut b = PathBuilder::new();
    for seg in &path.0 {
        match *seg {
            Seg::Move(x, y) => b.move_to(at.x(x), at.y(y)),
            Seg::Line(x, y) => b.line_to(at.x(x), at.y(y)),
            Seg::Quad(cx, cy, x, y) => b.quad_to(at.x(cx), at.y(cy), at.x(x), at.y(y)),
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                b.cubic_to(at.x(ax), at.y(ay), at.x(bx), at.y(by), at.x(x), at.y(y));
            }
            Seg::Close => b.close(),
        }
    }
    b.finish()
}

fn sk_stroke(width: f32) -> SkStroke {
    SkStroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..SkStroke::default()
    }
}

/// The rectangle in whole physical pixels, when all four of its edges lie on pixel edges.
fn pixel_aligned(rect: &Rect, at: Place) -> Option<tiny_skia::Rect> {
    let edges = [
        at.x(rect.x),
        at.y(rect.y),
        at.x(rect.right()),
        at.y(rect.bottom()),
    ];
    edges
        .iter()
        .all(|v| (v - v.round()).abs() < 1e-3)
        .then(|| {
            tiny_skia::Rect::from_ltrb(
                edges[0].round(),
                edges[1].round(),
                edges[2].round(),
                edges[3].round(),
            )
        })
        .flatten()
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Renders `list` into an image of `width` × `height` physical pixels at `scale` pixels
    /// per point, over an opaque `background`. Returns opaque RGBA8, row by row. `images` are
    /// the pictures the list's `Image` steps refer to by index; a step whose picture is
    /// missing or whose data is too short draws nothing.
    pub fn render(
        &mut self,
        list: &List,
        images: &[Image<'_>],
        width: u32,
        height: u32,
        scale: f32,
        background: Color,
    ) -> Vec<u8> {
        self.render_region(list, images, (0, 0), width, height, scale, background)
    }

    /// Renders the part of `list` that starts at the physical pixel `origin` of the window
    /// and is `width` × `height` physical pixels large; otherwise as [`Renderer::render`].
    /// The result is that part of the whole window's image, so the live window can keep one
    /// image per region (bar, dock, viewport) and redo only the one that changed.
    #[allow(clippy::too_many_arguments)]
    pub fn render_region(
        &mut self,
        list: &List,
        images: &[Image<'_>],
        origin: (i32, i32),
        width: u32,
        height: u32,
        scale: f32,
        background: Color,
    ) -> Vec<u8> {
        let at = Place {
            scale,
            ox: origin.0 as f32,
            oy: origin.1 as f32,
        };
        let Some(mut pixmap) = Pixmap::new(width, height) else {
            return Vec::new();
        };
        pixmap.fill(sk_color(Color {
            a: 255,
            ..background
        }));
        let identity = Transform::identity();
        let mut clip: Option<Mask> = None;
        for op in list.ops() {
            match op {
                Op::RoundRect {
                    rect,
                    radius,
                    fill,
                    stroke,
                } => {
                    if let Some(fill) = fill {
                        match pixel_aligned(rect, at).filter(|_| *radius <= 0.0) {
                            // A flat surface on whole pixels: no anti-aliasing, exact colour.
                            Some(r) => {
                                let mut paint = sk_paint(*fill);
                                paint.anti_alias = false;
                                pixmap.fill_rect(r, &paint, identity, clip.as_ref());
                            }
                            None => {
                                let path = Path::new().round_rect(*rect, *radius);
                                fill_path(&mut pixmap, &path, *fill, at, clip.as_ref());
                            }
                        }
                    }
                    if let Some(stroke) = stroke {
                        // Inside the rectangle: the centre line lies half a width in.
                        let half = stroke.width / 2.0;
                        let path =
                            Path::new().round_rect(rect.inset(half), (radius - half).max(0.0));
                        stroke_path(&mut pixmap, &path, *stroke, at, clip.as_ref());
                    }
                }
                Op::Line { from, to, stroke } => {
                    let path = Path::new().segment(from.0, from.1, to.0, to.1);
                    stroke_path(&mut pixmap, &path, *stroke, at, clip.as_ref());
                }
                Op::Path { path, fill, stroke } => {
                    if let Some(fill) = fill {
                        fill_path(&mut pixmap, path, *fill, at, clip.as_ref());
                    }
                    if let Some(stroke) = stroke {
                        stroke_path(&mut pixmap, path, *stroke, at, clip.as_ref());
                    }
                }
                Op::Text {
                    x,
                    baseline,
                    text,
                    size,
                    weight,
                    color,
                } => self.text(
                    &mut pixmap,
                    clip.as_ref(),
                    (at.x(*x), at.y(*baseline)),
                    text,
                    *size * scale,
                    *weight,
                    *color,
                ),
                Op::Image { rect, image } => {
                    if let Some(image) = images.get(*image) {
                        blit_nearest(&mut pixmap, clip.as_ref(), rect, at, image);
                    }
                }
                Op::Clip(None) => clip = None,
                Op::Clip(Some(path)) => {
                    clip = Mask::new(width, height).map(|mut mask| {
                        if let Some(p) = sk_path(path, at) {
                            mask.fill_path(&p, FillRule::Winding, true, identity);
                        }
                        mask
                    });
                }
            }
        }
        // Everything is drawn over an opaque background, so premultiplied equals straight.
        pixmap.take()
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        pixmap: &mut Pixmap,
        clip: Option<&Mask>,
        (x, baseline): (f32, f32),
        text: &str,
        size_px: f32,
        weight: Weight,
        color: Color,
    ) {
        if self.glyphs.len() > GLYPH_CACHE_LIMIT {
            self.glyphs.clear();
        }
        let face = font::face(weight);
        let px_scale = font::px_scale(face, size_px);
        let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
        let row = baseline.round() as i32;
        let mask = clip.map(Mask::data);
        let data = pixmap.data_mut();
        let mut pen = x;
        let mut previous = None;
        for c in text.chars() {
            let id = face.glyph_id(c);
            if let Some(p) = previous {
                pen += ab_glyph::ScaleFont::kern(&face.as_scaled(px_scale), p, id);
            }
            previous = Some(id);
            let origin = pen.floor();
            let bin = (((pen - origin) * 4.0).floor() as u8).min(3);
            let key = GlyphKey {
                medium: weight == Weight::Medium,
                id: id.0,
                size: size_px.to_bits(),
                bin,
            };
            let glyph = self.glyphs.entry(key).or_insert_with(|| {
                let positioned =
                    id.with_scale_and_position(px_scale, point(f32::from(bin) / 4.0, 0.0));
                face.outline_glyph(positioned).map(|outline| {
                    let b = outline.px_bounds();
                    let (gw, gh) = (b.width().ceil() as usize, b.height().ceil() as usize);
                    let mut coverage = vec![0u8; gw * gh];
                    outline.draw(|gx, gy, c| {
                        if let Some(v) = coverage.get_mut(gy as usize * gw + gx as usize) {
                            *v = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    });
                    Glyph {
                        left: b.min.x.floor() as i32,
                        top: b.min.y.floor() as i32,
                        width: gw,
                        height: gh,
                        coverage,
                    }
                })
            });
            if let Some(g) = glyph {
                let (gx0, gy0) = (origin as i32 + g.left, row + g.top);
                for gy in 0..g.height {
                    let py = gy0 + gy as i32;
                    if py < 0 || py >= h {
                        continue;
                    }
                    for gx in 0..g.width {
                        let px = gx0 + gx as i32;
                        let cover = u32::from(g.coverage[gy * g.width + gx]);
                        if cover == 0 || px < 0 || px >= w {
                            continue;
                        }
                        let at = (py * w + px) as usize;
                        let mut a = cover * u32::from(color.a) / 255;
                        if let Some(mask) = mask {
                            a = a * u32::from(mask[at]) / 255;
                        }
                        blend(&mut data[at * 4..at * 4 + 4], color, a);
                    }
                }
            }
            pen += ab_glyph::ScaleFont::h_advance(&face.as_scaled(px_scale), id);
        }
    }
}

/// Blends `color` at opacity `a` (0 to 255) over one premultiplied pixel.
fn blend(pixel: &mut [u8], color: Color, a: u32) {
    let over =
        |src: u8, dst: u8| ((u32::from(src) * a + u32::from(dst) * (255 - a) + 127) / 255) as u8;
    pixel[0] = over(color.r, pixel[0]);
    pixel[1] = over(color.g, pixel[1]);
    pixel[2] = over(color.b, pixel[2]);
    pixel[3] = over(255, pixel[3]);
}

fn fill_path(pixmap: &mut Pixmap, path: &Path, color: Color, at: Place, clip: Option<&Mask>) {
    if let Some(p) = sk_path(path, at) {
        pixmap.fill_path(
            &p,
            &sk_paint(color),
            FillRule::Winding,
            Transform::identity(),
            clip,
        );
    }
}

fn stroke_path(pixmap: &mut Pixmap, path: &Path, stroke: Stroke, at: Place, clip: Option<&Mask>) {
    if let Some(p) = sk_path(path, at) {
        pixmap.stroke_path(
            &p,
            &sk_paint(stroke.color),
            &sk_stroke(stroke.width * at.scale),
            Transform::identity(),
            clip,
        );
    }
}

/// Draws `image` into `rect` with nearest-neighbour sampling: when the rectangle is a whole
/// multiple of the picture, every picture pixel becomes an exact block.
fn blit_nearest(
    pixmap: &mut Pixmap,
    clip: Option<&Mask>,
    rect: &Rect,
    at: Place,
    image: &Image<'_>,
) {
    let (sw, sh) = (i64::from(image.width), i64::from(image.height));
    if sw == 0 || sh == 0 || (image.rgba.len() as i64) < sw * sh * 4 {
        return;
    }
    let x0 = at.x(rect.x).round() as i64;
    let y0 = at.y(rect.y).round() as i64;
    let dw = at.x(rect.right()).round() as i64 - x0;
    let dh = at.y(rect.bottom()).round() as i64 - y0;
    if dw <= 0 || dh <= 0 {
        return;
    }
    let (w, h) = (i64::from(pixmap.width()), i64::from(pixmap.height()));
    let mask = clip.map(Mask::data);
    let data = pixmap.data_mut();
    for py in y0.max(0)..(y0 + dh).min(h) {
        let sy = (py - y0) * sh / dh;
        for px in x0.max(0)..(x0 + dw).min(w) {
            let at = (py * w + px) as usize;
            if mask.is_some_and(|m| m[at] < 128) {
                continue;
            }
            let sx = (px - x0) * sw / dw;
            let from = ((sy * sw + sx) * 4) as usize;
            data[at * 4..at * 4 + 3].copy_from_slice(&image.rgba[from..from + 3]);
            data[at * 4 + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Color = Color::hex(0x202227);

    fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> Color {
        let at = ((y * width + x) * 4) as usize;
        Color {
            r: rgba[at],
            g: rgba[at + 1],
            b: rgba[at + 2],
            a: rgba[at + 3],
        }
    }

    #[test]
    fn the_background_is_opaque_and_exact() {
        let out = Renderer::new().render(&List::new(), &[], 8, 4, 1.0, BG.with_opacity(10));
        assert_eq!(out.len(), 8 * 4 * 4);
        assert!(
            out.as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0x20, 0x22, 0x27, 255])
        );
    }

    #[test]
    fn a_flat_rectangle_on_pixel_edges_has_sharp_exact_edges() {
        let red = Color::hex(0xCC2200);
        for scale in [1.0, 2.0] {
            let mut list = List::new();
            list.rect(Rect::new(2.0, 1.0, 4.0, 3.0), red);
            let (w, h) = ((10.0 * scale) as u32, (6.0 * scale) as u32);
            let out = Renderer::new().render(&list, &[], w, h, scale, BG);
            let s = scale as u32;
            for y in 0..h {
                for x in 0..w {
                    let inside = (2 * s..6 * s).contains(&x) && (s..4 * s).contains(&y);
                    assert_eq!(
                        pixel(&out, w, x, y),
                        if inside { red } else { BG },
                        "({x}, {y}) at {scale}x"
                    );
                }
            }
        }
    }

    #[test]
    fn a_round_rect_fills_its_middle_and_spares_its_corners() {
        let accent = Color::hex(0xFF8A1F);
        let mut list = List::new();
        list.round_rect(Rect::new(4.0, 4.0, 36.0, 36.0), 7.0, accent);
        let out = Renderer::new().render(&list, &[], 44, 44, 1.0, BG);
        assert_eq!(pixel(&out, 44, 22, 22), accent);
        assert_eq!(
            pixel(&out, 44, 5, 22),
            accent,
            "the straight part of the edge"
        );
        assert_eq!(
            pixel(&out, 44, 4, 4),
            BG,
            "the corner pixel is outside the arc"
        );
        assert_eq!(pixel(&out, 44, 39, 39), BG);
        assert_eq!(pixel(&out, 44, 2, 22), BG);
    }

    #[test]
    fn a_ring_lies_inside_its_rectangle_and_leaves_the_middle() {
        let accent = Color::hex(0xFF8A1F);
        let mut list = List::new();
        list.ring(Rect::new(4.0, 4.0, 20.0, 20.0), 0.0, accent, 2.0);
        let out = Renderer::new().render(&list, &[], 28, 28, 1.0, BG);
        assert_eq!(pixel(&out, 28, 14, 4), accent);
        assert_eq!(pixel(&out, 28, 14, 5), accent);
        assert_eq!(pixel(&out, 28, 14, 6), BG, "two pixels wide");
        assert_eq!(pixel(&out, 28, 14, 3), BG, "nothing outside");
        assert_eq!(pixel(&out, 28, 14, 14), BG);
    }

    #[test]
    fn a_clip_keeps_drawing_inside_its_outline_until_it_is_lifted() {
        let (a, b) = (Color::hex(0xFF8A1F), Color::hex(0x1E90FF));
        let all = Rect::new(0.0, 0.0, 20.0, 20.0);
        let mut list = List::new();
        // The upper-left triangle of the square.
        list.clip(Path::new().polygon(&[(0.0, 0.0), (20.0, 0.0), (0.0, 20.0)]));
        list.rect(all, a);
        list.text(12.0, 18.0, "W", 14.0, Weight::Medium, b);
        list.unclip();
        list.rect(Rect::new(18.0, 18.0, 2.0, 2.0), b);
        let out = Renderer::new().render(&list, &[], 20, 20, 1.0, BG);
        assert_eq!(pixel(&out, 20, 3, 3), a);
        assert_eq!(
            pixel(&out, 20, 16, 16),
            BG,
            "outside the clip, and no text there either"
        );
        for y in 9..19 {
            for x in 11..18 {
                assert_eq!(pixel(&out, 20, x, y), BG, "({x}, {y})");
            }
        }
        assert_eq!(
            pixel(&out, 20, 19, 19),
            b,
            "drawn after the clip was lifted"
        );
    }

    #[test]
    fn text_puts_ink_between_its_start_and_its_measured_end_and_nowhere_else() {
        let ink = Color::hex(0xE4E6EB);
        for scale in [1.0, 2.0] {
            let (size, x, base) = (13.5, 10.0, 20.0);
            let mut list = List::new();
            list.text(x, base, "Picture", size, Weight::Regular, ink);
            let (w, h) = ((90.0 * scale) as u32, (30.0 * scale) as u32);
            let out = Renderer::new().render(&list, &[], w, h, scale, BG);
            let end = x + font::measure("Picture", size, Weight::Regular);
            let (mut inked, mut full) = (0, 0);
            for py in 0..h {
                for px in 0..w {
                    let c = pixel(&out, w, px, py);
                    if c == BG {
                        continue;
                    }
                    inked += 1;
                    full += usize::from(c == ink);
                    let (tx, ty) = (px as f32 / scale, py as f32 / scale);
                    assert!(
                        tx >= x - 1.0 && tx <= end + 1.0,
                        "ink at x {tx} outside {x}..{end} at {scale}x"
                    );
                    // Between the ascender line and the descender ("P", "t" above; "c" below).
                    assert!(
                        ty >= base - size && ty <= base + size * 0.3,
                        "ink at y {ty}"
                    );
                }
            }
            assert!(inked > 100, "only {inked} pixels of ink at {scale}x");
            assert!(full > 10, "stems reach the full ink colour ({full})");
        }
    }

    #[test]
    fn the_glyph_cache_gives_the_same_pixels_the_second_time() {
        let mut list = List::new();
        list.text(
            3.3,
            15.0,
            "DeluxePainter 12/40",
            13.5,
            Weight::Medium,
            Color::hex(0xFFFFFF),
        );
        let mut renderer = Renderer::new();
        let first = renderer.render(&list, &[], 160, 22, 1.0, BG);
        let cached = renderer.glyphs.len();
        assert!(cached > 10, "{cached} glyphs cached");
        assert_eq!(renderer.render(&list, &[], 160, 22, 1.0, BG), first);
        assert_eq!(renderer.glyphs.len(), cached);
        assert_eq!(Renderer::new().render(&list, &[], 160, 22, 1.0, BG), first);
    }

    /// A picture of distinct colours, enlarged by whole factors: every pixel becomes an
    /// exact block, with no blending at the block edges.
    #[test]
    fn a_picture_at_factor_n_is_exact_n_pixel_blocks() {
        let (sw, sh) = (5u32, 3u32);
        let rgba: Vec<u8> = (0..sw * sh)
            .flat_map(|i| {
                [
                    (i * 16) as u8,
                    255 - (i * 9) as u8,
                    (i * 37 % 256) as u8,
                    255,
                ]
            })
            .collect();
        let image = Image {
            width: sw,
            height: sh,
            rgba: &rgba,
        };
        for (scale, factor) in [(1.0, 1u32), (1.0, 3), (2.0, 4), (1.5, 3)] {
            // `factor` physical pixels per picture pixel, placed on a pixel edge.
            let (ox, oy) = (4u32, 2u32);
            let rect = Rect::new(
                ox as f32 / scale,
                oy as f32 / scale,
                (sw * factor) as f32 / scale,
                (sh * factor) as f32 / scale,
            );
            let mut list = List::new();
            list.image(rect, 0);
            let (w, h) = (ox + sw * factor + 3, oy + sh * factor + 3);
            let out = Renderer::new().render(&list, &[image], w, h, scale, BG);
            for y in 0..h {
                for x in 0..w {
                    let inside =
                        (ox..ox + sw * factor).contains(&x) && (oy..oy + sh * factor).contains(&y);
                    let want = if inside {
                        let at = (((y - oy) / factor) * sw + (x - ox) / factor) as usize * 4;
                        Color::rgb(rgba[at], rgba[at + 1], rgba[at + 2])
                    } else {
                        BG
                    };
                    assert_eq!(
                        pixel(&out, w, x, y),
                        want,
                        "({x}, {y}) factor {factor} at {scale}x"
                    );
                }
            }
        }
    }

    #[test]
    fn a_missing_or_short_picture_and_one_off_the_edge_are_harmless() {
        let rgba = [255u8; 4 * 4];
        let mut list = List::new();
        list.image(Rect::new(0.0, 0.0, 4.0, 4.0), 3);
        list.image(Rect::new(0.0, 0.0, 4.0, 4.0), 0);
        list.image(Rect::new(-2.0, -2.0, 4.0, 4.0), 1);
        let short = Image {
            width: 4,
            height: 4,
            rgba: &rgba,
        };
        let whole = Image {
            width: 2,
            height: 2,
            rgba: &rgba,
        };
        let out = Renderer::new().render(&list, &[short, whole], 6, 6, 1.0, BG);
        assert_eq!(
            pixel(&out, 6, 0, 0),
            Color::rgb(255, 255, 255),
            "the part inside is drawn"
        );
        assert_eq!(pixel(&out, 6, 1, 1), Color::rgb(255, 255, 255));
        assert_eq!(pixel(&out, 6, 2, 2), BG);
        assert_eq!(pixel(&out, 6, 3, 3), BG, "the short picture drew nothing");
    }
}
