//! Custom brushes and drawing modes (ADR-18).
//!
//! A [`BrushImage`] is a picked-up piece of the page; pixels in its transparent colour (the
//! background colour at pick-up time) are not painted. [`plot`] paints one brush pixel onto
//! the page in a [`DrawMode`]; every stamping path of the drawing tools goes through it so
//! the stencil and the modes apply everywhere.

use serde::{Deserialize, Serialize};

use crate::image::IndexedImage;
use crate::palette::{Palette, Rgb8};

/// A custom brush: indexed pixels with a handle (the pixel under the pointer) and an
/// optional transparent colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrushImage {
    pub width: u32,
    pub height: u32,
    /// Row-major colour indices, `width * height` of them.
    pub pixels: Vec<u8>,
    /// The handle in brush coordinates; stamping at (x, y) puts this pixel there.
    pub handle: (i32, i32),
    /// Pixels of this colour are transparent (not painted in Matte and Color mode).
    pub transparent: Option<u8>,
}

impl BrushImage {
    /// The brush from a cut-out image, with the handle in the centre.
    pub fn from_image(img: &IndexedImage, transparent: Option<u8>) -> Self {
        let (width, height) = (img.width(), img.height());
        Self {
            width,
            height,
            pixels: img.pixels().to_vec(),
            handle: (width as i32 / 2, height as i32 / 2),
            transparent,
        }
    }

    /// The brush as an image (all pixels, transparent ones included).
    pub fn to_image(&self) -> IndexedImage {
        IndexedImage::from_pixels(self.width, self.height, self.pixels.clone())
            .expect("brush pixels match its size")
    }

    /// Colour at brush coordinates, `None` outside.
    pub fn get(&self, x: i32, y: i32) -> Option<u8> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some(self.pixels[y as usize * self.width as usize + x as usize])
    }

    /// Whether the pixel at brush coordinates is painted in Matte mode.
    pub fn is_opaque(&self, x: i32, y: i32) -> bool {
        self.get(x, y).is_some_and(|c| Some(c) != self.transparent)
    }

    /// Offsets (relative to the handle) of the opaque pixels, for outline previews and the
    /// one-colour stamping paths.
    pub fn opaque_offsets(&self) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                if self.is_opaque(x, y) {
                    out.push((x - self.handle.0, y - self.handle.1));
                }
            }
        }
        out
    }
}

/// Drawing modes of the Mode menu (DeluxePaint IV AGA). `Matte` paints a brush's own colours
/// (built-in brushes: the FG colour) without its transparent pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawMode {
    #[default]
    Matte,
    /// The brush's shape in one colour (FG; BG with the right button).
    Color,
    /// Like Matte, but transparent pixels are painted in the BG colour too.
    Replace,
    /// Drags the colours under the brush along the stroke.
    Smear,
    /// Moves the colours under the brush one step up (left button) or down (right button)
    /// within the colour range.
    Shade,
    /// Mixes the brush colour with the colour under it and uses the nearest palette colour.
    Blend,
    /// Replaces a colour under the brush with the next colour of the range.
    Cycle,
    /// Averages each pixel with its neighbours.
    Smooth,
    /// Like Blend, AGA: the RGB mix of brush and page colour.
    Mix,
    /// Extra-HalfBrite: the colour under the brush becomes its half-bright twin (index + 32);
    /// the right button restores it (index - 32).
    HBrite,
}

/// Effect ▸ AntiAlias (ADR-31): how the edge pixels of a custom brush blend with the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntiAlias {
    #[default]
    None,
    /// Brush pixels that touch a transparent (or outside) pixel are mixed 50 % with the page.
    Low,
    /// Every brush pixel is weighted by the share of opaque pixels in its 3x3 neighbourhood.
    High,
}

/// Effect ▸ Process (ADR-31): which parts of the page colour the brush colour replaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessMode {
    /// The page colour takes the brush colour's hue and saturation and keeps its own value.
    #[default]
    Tint,
    /// Only the hue comes from the brush colour.
    Hue,
    /// Only the value (brightness) comes from the brush colour.
    Value,
}

/// The Effect menu's settings that change how [`plot_fx`] sets a pixel (ADR-31). They travel
/// with every stroke and shape command, so the GUI, the CLI and MCP paint alike; left out of
/// the JSON while at their defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaintFx {
    /// Translucency: the percentage of the page colour that remains (DeluxePaint's slider,
    /// 50 by default); `None` = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translucency: Option<u8>,
    /// Process painting; `None` = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessMode>,
    /// Edge blending of custom brushes.
    #[serde(default, skip_serializing_if = "is_none_aa")]
    pub antialias: AntiAlias,
}

fn is_none_aa(a: &AntiAlias) -> bool {
    *a == AntiAlias::None
}

impl PaintFx {
    /// Whether no effect is on.
    pub fn is_off(&self) -> bool {
        *self == PaintFx::default()
    }
}

/// What [`plot`] needs besides the pixel: palette, range, stencil, the page before the
/// gesture and how the brush moved since the last stamp.
#[derive(Clone, Copy, Debug)]
pub struct PaintContext<'a> {
    pub palette: &'a Palette,
    /// Colour range for Shade and Cycle, `low..=high` (the range that holds the FG colour).
    pub range: Option<(u8, u8)>,
    /// Stencil: `stencil[c]` is true when colour `c` on the page is protected.
    pub stencil: Option<&'a [bool]>,
    /// The page before the gesture; Smear and Smooth read from it.
    pub before: &'a IndexedImage,
    /// Brush movement since the previous stamp (Smear).
    pub motion: (i32, i32),
    /// The right mouse button is painting (Shade goes down, Color uses BG, HBrite restores).
    pub right: bool,
}

/// Paints one brush pixel of colour `color` at (`x`, `y`) in `mode`. Pixels outside the page
/// and pixels whose current colour the stencil protects are left alone.
///
/// Matte, Color and Replace set the pixel to `color` (the caller has already chosen FG, BG or
/// the brush's own colour). The other modes (ADR-19) ignore `color` unless noted and read the
/// colour under the brush from `ctx.before`, so a gesture changes a pixel at most one step
/// however often the brush passes over it. A mode that has nothing to do leaves the pixel.
///
/// Smear reads `page` itself, `ctx.motion` behind the pixel. A whole stamp must read the page
/// as it was before that stamp, or the result depends on the order of the brush pixels:
/// [`crate::raster::Pen::stamp`] takes that snapshot and paints with [`smear_pixel`].
pub fn plot(
    page: &mut IndexedImage,
    x: i32,
    y: i32,
    color: u8,
    mode: DrawMode,
    ctx: &PaintContext,
) {
    plot_fx(page, x, y, color, mode, ctx, &PaintFx::default(), 100);
}

/// [`plot`] with the Effect menu (ADR-31). In the modes that set a colour (Matte, Color,
/// Replace) the colour first goes through Process (against the page colour before the
/// gesture), then is mixed with that page colour: `cover` percent of it (100 = the whole
/// brush pixel, less at antialiased brush edges) times the share Translucency leaves, and the
/// nearest palette colour is used. The other modes ignore the effects.
#[allow(clippy::too_many_arguments)]
pub fn plot_fx(
    page: &mut IndexedImage,
    x: i32,
    y: i32,
    color: u8,
    mode: DrawMode,
    ctx: &PaintContext,
    fx: &PaintFx,
    cover: u8,
) {
    let Some(under) = unprotected(page, x, y, ctx) else {
        return;
    };
    let before = ctx.before.get(x, y).unwrap_or(under);
    let new = match mode {
        DrawMode::Matte | DrawMode::Color | DrawMode::Replace => {
            Some(apply_fx(ctx, before, color, fx, cover))
        }
        DrawMode::Smear => page.get(x - ctx.motion.0, y - ctx.motion.1),
        DrawMode::Shade => step_in_range(before, ctx, false),
        DrawMode::Cycle => match ctx.range {
            // FG in no range: Cycle paints plain FG (manual notes).
            None => Some(color),
            Some(_) => step_in_range(before, ctx, true),
        },
        DrawMode::Blend => mix(ctx, before, color, 1, 2),
        DrawMode::Mix => mix(ctx, before, color, 1, 4),
        DrawMode::Smooth => smooth(ctx, x, y),
        DrawMode::HBrite => half_brite(before, ctx),
    };
    if let Some(c) = new {
        page.set(x, y, c);
    }
}

/// The page colour at (`x`, `y`) when the pixel may be painted: on the page and not protected
/// by the stencil.
fn unprotected(page: &IndexedImage, x: i32, y: i32, ctx: &PaintContext) -> Option<u8> {
    let under = page.get(x, y)?;
    let protected = ctx
        .stencil
        .is_some_and(|s| s.get(usize::from(under)).copied().unwrap_or(false));
    (!protected).then_some(under)
}

/// Smear for one brush pixel: (`x`, `y`) takes `source`, the colour `ctx.motion` behind it as
/// the page was before the current stamp (`None` when that lies off the page: the pixel stays).
/// The stencil applies as in [`plot`].
pub(crate) fn smear_pixel(
    page: &mut IndexedImage,
    x: i32,
    y: i32,
    source: Option<u8>,
    ctx: &PaintContext,
) {
    if let (Some(_), Some(c)) = (unprotected(page, x, y, ctx), source) {
        page.set(x, y, c);
    }
}

/// HBrite on a 64-colour (Extra-HalfBrite) palette: the left button darkens one of the first
/// 32 colours to its half-bright twin (index + 32), the right button restores a half-bright
/// pixel (index - 32), as DeluxePaint IV does (manual notes section 5). Other palettes: no
/// change.
fn half_brite(c: u8, ctx: &PaintContext) -> Option<u8> {
    if ctx.palette.len() != 64 {
        return None;
    }
    if ctx.right {
        (32..64).contains(&c).then(|| c - 32)
    } else {
        (c < 32).then_some(c + 32)
    }
}

/// Work units (see [`crate::command::MAX_PAINT_WORK`]) of one pixel [`plot_fx`] paints in
/// `mode` with `fx` on a palette of `colors` entries: 1, or more where every pixel looks up
/// the nearest palette colour (Blend, Mix, Smooth, and the set modes with an effect on), as
/// that search walks the whole palette (measured: Blend on 32 colours costs about 9 times
/// Matte).
pub fn pixel_work(mode: DrawMode, fx: &PaintFx, colors: usize) -> u64 {
    let searches = match mode {
        DrawMode::Blend | DrawMode::Mix | DrawMode::Smooth => true,
        DrawMode::Matte | DrawMode::Color | DrawMode::Replace => !fx.is_off(),
        DrawMode::Smear | DrawMode::Shade | DrawMode::Cycle | DrawMode::HBrite => false,
    };
    if searches { 1 + colors as u64 / 4 } else { 1 }
}

/// The colour a set-mode pixel gets: `color` after Process, mixed with `under`.
fn apply_fx(ctx: &PaintContext, under: u8, color: u8, fx: &PaintFx, cover: u8) -> u8 {
    if fx.is_off() && cover >= 100 {
        return color;
    }
    let (Some(u), Some(mut b)) = (ctx.palette.get(under), ctx.palette.get(color)) else {
        return color;
    };
    if let Some(p) = fx.process {
        b = process(u, b, p);
    }
    // Weight of the brush colour in percent.
    let own = 100 - u32::from(fx.translucency.map_or(0, |t| t.min(100)));
    let weight = own * u32::from(cover.min(100)) / 100;
    if weight == 0 {
        return under;
    }
    if weight >= 100 && fx.process.is_none() {
        return color;
    }
    let ch =
        |p: u8, q: u8| ((u32::from(p) * (100 - weight) + u32::from(q) * weight + 50) / 100) as u8;
    let mixed = Rgb8::new(ch(u.r, b.r), ch(u.g, b.g), ch(u.b, b.b));
    // An unchanged colour keeps its index (the nearest of two equal entries could differ).
    if mixed == u {
        return under;
    }
    ctx.palette.nearest(mixed)
}

/// Process painting: `under` recoloured with parts of `brush` (see [`ProcessMode`]).
pub fn process(under: Rgb8, brush: Rgb8, mode: ProcessMode) -> Rgb8 {
    let (uh, us, uv) = rgb_to_hsv(under);
    let (bh, bs, bv) = rgb_to_hsv(brush);
    let (h, s, v) = match mode {
        ProcessMode::Tint => (bh, bs, uv),
        ProcessMode::Hue => (bh, us, uv),
        ProcessMode::Value => (uh, us, bv),
    };
    hsv_to_rgb(h, s, v)
}

/// Hue 0..6, saturation and value 0..1.
fn rgb_to_hsv(c: Rgb8) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
    );
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb8 {
    let f = |n: f32| {
        let k = (n + h).rem_euclid(6.0);
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    let q = |x: f32| (x * 255.0 + 0.5) as u8;
    Rgb8::new(q(f(5.0)), q(f(3.0)), q(f(1.0)))
}

/// How much of the brush colour (percent) the brush pixel (`bx`, `by`) of `brush` paints
/// under `aa`: edge pixels less, inner pixels fully.
pub fn edge_cover(brush: &BrushImage, bx: i32, by: i32, aa: AntiAlias) -> u8 {
    let opaque = |dx: i32, dy: i32| brush.is_opaque(bx + dx, by + dy);
    match aa {
        AntiAlias::None => 100,
        AntiAlias::Low => {
            if opaque(-1, 0) && opaque(1, 0) && opaque(0, -1) && opaque(0, 1) {
                100
            } else {
                50
            }
        }
        AntiAlias::High => {
            let n = (-1..=1)
                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                .filter(|&(dx, dy)| opaque(dx, dy))
                .count() as u32;
            (n * 100 / 9) as u8
        }
    }
}

/// Shade / Cycle: the colour one step along the range (up, or down with the right button),
/// clamped (Shade) or wrapped (Cycle); `None` when `c` is outside the range.
fn step_in_range(c: u8, ctx: &PaintContext, wrap: bool) -> Option<u8> {
    let (lo, hi) = ctx.range?;
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    if c < lo || c > hi {
        return None;
    }
    Some(if ctx.right {
        if c > lo {
            c - 1
        } else if wrap {
            hi
        } else {
            lo
        }
    } else if c < hi {
        c + 1
    } else if wrap {
        lo
    } else {
        hi
    })
}

/// Nearest palette colour to `under` mixed with `color`; `num / den` is the weight of `color`.
fn mix(ctx: &PaintContext, under: u8, color: u8, num: u32, den: u32) -> Option<u8> {
    let (a, b) = (ctx.palette.get(under)?, ctx.palette.get(color)?);
    let ch =
        |p: u8, q: u8| ((u32::from(p) * (den - num) + u32::from(q) * num + den / 2) / den) as u8;
    Some(
        ctx.palette
            .nearest(Rgb8::new(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b))),
    )
}

/// Smooth: the palette colour nearest to the average of the 3x3 neighbourhood in `before`.
fn smooth(ctx: &PaintContext, x: i32, y: i32) -> Option<u8> {
    let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
    for dy in -1..=1 {
        for dx in -1..=1 {
            if let Some(c) = ctx
                .before
                .get(x + dx, y + dy)
                .and_then(|i| ctx.palette.get(i))
            {
                r += u32::from(c.r);
                g += u32::from(c.g);
                b += u32::from(c.b);
                n += 1;
            }
        }
    }
    if n == 0 {
        return None;
    }
    let avg = |v: u32| ((v + n / 2) / n) as u8;
    Some(ctx.palette.nearest(Rgb8::new(avg(r), avg(g), avg(b))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_from_image_centres_handle_and_skips_transparent() {
        let img = IndexedImage::from_pixels(3, 2, vec![0, 5, 0, 5, 5, 0]).unwrap();
        let b = BrushImage::from_image(&img, Some(0));
        assert_eq!(b.handle, (1, 1));
        assert_eq!(b.opaque_offsets(), vec![(0, -1), (-1, 0), (0, 0)]);
        assert_eq!(b.to_image(), img);
    }

    #[test]
    fn plot_respects_stencil_and_page_edges() {
        let pal = Palette::dpaint_default_32();
        let before = IndexedImage::new(4, 4, 1);
        let mut page = before.clone();
        let mut stencil = vec![false; 256];
        stencil[1] = true;
        let ctx = PaintContext {
            palette: &pal,
            range: None,
            stencil: Some(&stencil),
            before: &before,
            motion: (0, 0),
            right: false,
        };
        plot(&mut page, 1, 1, 7, DrawMode::Matte, &ctx);
        assert_eq!(page.get(1, 1), Some(1), "protected colour stays");
        let ctx = PaintContext {
            stencil: None,
            ..ctx
        };
        plot(&mut page, 1, 1, 7, DrawMode::Matte, &ctx);
        plot(&mut page, -1, 9, 7, DrawMode::Matte, &ctx);
        assert_eq!(page.get(1, 1), Some(7));
    }

    fn ctx<'a>(pal: &'a Palette, before: &'a IndexedImage) -> PaintContext<'a> {
        PaintContext {
            palette: pal,
            range: None,
            stencil: None,
            before,
            motion: (0, 0),
            right: false,
        }
    }

    fn bw() -> Palette {
        Palette::new(vec![
            Rgb8::new(0, 0, 0),
            Rgb8::new(255, 255, 255),
            Rgb8::new(128, 128, 128),
            Rgb8::new(64, 64, 64),
        ])
        .unwrap()
    }

    fn one(mode: DrawMode, page: &IndexedImage, c: &PaintContext, color: u8) -> Option<u8> {
        let mut p = page.clone();
        plot(&mut p, 1, 1, color, mode, c);
        p.get(1, 1)
    }

    #[test]
    fn shade_steps_within_range_and_clamps() {
        let pal = Palette::dpaint_default_32();
        let img = IndexedImage::new(3, 3, 5);
        let mut c = ctx(&pal, &img);
        c.range = Some((4, 6));
        assert_eq!(one(DrawMode::Shade, &img, &c, 1), Some(6));
        c.right = true;
        assert_eq!(one(DrawMode::Shade, &img, &c, 1), Some(4));
        let top = IndexedImage::new(3, 3, 6);
        c.right = false;
        let c2 = PaintContext { before: &top, ..c };
        assert_eq!(one(DrawMode::Shade, &top, &c2, 1), Some(6), "clamped");
        let out = IndexedImage::new(3, 3, 9);
        let c3 = PaintContext { before: &out, ..c };
        assert_eq!(one(DrawMode::Shade, &out, &c3, 1), Some(9), "outside range");
        let c4 = PaintContext { range: None, ..c };
        assert_eq!(one(DrawMode::Shade, &img, &c4, 1), Some(5), "no range");
    }

    #[test]
    fn shade_reads_before_so_a_gesture_moves_one_step() {
        let pal = Palette::dpaint_default_32();
        let before = IndexedImage::new(3, 3, 5);
        let mut page = before.clone();
        let mut c = ctx(&pal, &before);
        c.range = Some((4, 8));
        for _ in 0..3 {
            plot(&mut page, 1, 1, 1, DrawMode::Shade, &c);
        }
        assert_eq!(page.get(1, 1), Some(6));
    }

    #[test]
    fn cycle_wraps_and_paints_plain_colour_without_range() {
        let pal = Palette::dpaint_default_32();
        let top = IndexedImage::new(3, 3, 8);
        let mut c = ctx(&pal, &top);
        c.range = Some((4, 8));
        assert_eq!(one(DrawMode::Cycle, &top, &c, 1), Some(4));
        c.right = true;
        let bottom = IndexedImage::new(3, 3, 4);
        let c2 = PaintContext {
            before: &bottom,
            ..c
        };
        assert_eq!(one(DrawMode::Cycle, &bottom, &c2, 1), Some(8));
        let c3 = PaintContext { range: None, ..c };
        assert_eq!(one(DrawMode::Cycle, &top, &c3, 7), Some(7));
    }

    #[test]
    fn blend_and_mix_pick_the_nearest_palette_colour() {
        let pal = bw();
        let page = IndexedImage::new(3, 3, 0);
        let c = ctx(&pal, &page);
        // 50 % of white over black is 128 grey = index 2.
        assert_eq!(one(DrawMode::Blend, &page, &c, 1), Some(2));
        // Mix keeps 75 % of the page: 64 grey = index 3.
        assert_eq!(one(DrawMode::Mix, &page, &c, 1), Some(3));
    }

    #[test]
    fn smooth_averages_the_neighbourhood() {
        let pal = bw();
        let mut img = IndexedImage::new(3, 3, 0);
        for x in 0..3 {
            img.set(x, 0, 1);
        }
        img.set(0, 1, 1);
        // Centre pixel: 4 white of 9 = 113 -> nearest of {0,64,128,255} is 128 (index 2).
        let c = ctx(&pal, &img);
        assert_eq!(one(DrawMode::Smooth, &img, &c, 1), Some(2));
        let flat = IndexedImage::new(3, 3, 1);
        let c = ctx(&pal, &flat);
        assert_eq!(one(DrawMode::Smooth, &flat, &c, 0), Some(1));
    }

    #[test]
    fn smear_drags_the_colour_behind_the_brush() {
        let pal = bw();
        let mut img = IndexedImage::new(4, 3, 0);
        img.set(0, 1, 1);
        let mut c = ctx(&pal, &img);
        c.motion = (1, 0);
        let mut page = img.clone();
        plot(&mut page, 1, 1, 3, DrawMode::Smear, &c);
        assert_eq!(page.get(1, 1), Some(1));
        // Behind the page edge: unchanged.
        plot(&mut page, 0, 1, 3, DrawMode::Smear, &c);
        assert_eq!(page.get(0, 1), Some(1));
        c.motion = (0, 0);
        let mut page = img.clone();
        plot(&mut page, 2, 1, 3, DrawMode::Smear, &c);
        assert_eq!(page.get(2, 1), Some(0));
    }

    #[test]
    fn hbrite_needs_64_colours() {
        let pal64 = Palette::new(vec![Rgb8::new(200, 100, 50); 64]).unwrap();
        let img = IndexedImage::new(3, 3, 5);
        let c = ctx(&pal64, &img);
        assert_eq!(one(DrawMode::HBrite, &img, &c, 1), Some(37));
        let high = IndexedImage::new(3, 3, 40);
        let c = ctx(&pal64, &high);
        assert_eq!(one(DrawMode::HBrite, &high, &c, 1), Some(40));
        let pal32 = Palette::dpaint_default_32();
        let c = ctx(&pal32, &img);
        assert_eq!(one(DrawMode::HBrite, &img, &c, 1), Some(5));
    }

    #[test]
    fn hbrite_right_button_restores() {
        // DeluxePaint IV: the left button darkens the first 32 colours to their half-bright
        // twins, the right button restores half-bright pixels (manual-ui-notes.md section 5).
        let pal64 = Palette::new(vec![Rgb8::new(200, 100, 50); 64]).unwrap();
        let before = IndexedImage::from_pixels(2, 1, vec![5, 37]).unwrap();
        let mut c = ctx(&pal64, &before);
        c.right = true;
        let mut page = before.clone();
        plot(&mut page, 0, 0, 1, DrawMode::HBrite, &c);
        plot(&mut page, 1, 0, 1, DrawMode::HBrite, &c);
        assert_eq!(page.pixels(), &[5, 5]);
        c.right = false;
        let mut page = before.clone();
        plot(&mut page, 0, 0, 1, DrawMode::HBrite, &c);
        plot(&mut page, 1, 0, 1, DrawMode::HBrite, &c);
        assert_eq!(page.pixels(), &[37, 37]);
        // Without 64 colours the right button changes nothing either.
        let pal32 = Palette::dpaint_default_32();
        let low = IndexedImage::new(3, 3, 5);
        let mut c = ctx(&pal32, &low);
        c.right = true;
        assert_eq!(one(DrawMode::HBrite, &low, &c, 1), Some(5));
    }

    #[test]
    fn modes_respect_the_stencil() {
        let pal = Palette::dpaint_default_32();
        let img = IndexedImage::new(3, 3, 5);
        let mut stencil = vec![false; 32];
        stencil[5] = true;
        let mut c = ctx(&pal, &img);
        c.range = Some((4, 8));
        c.stencil = Some(&stencil);
        assert_eq!(one(DrawMode::Shade, &img, &c, 1), Some(5));
    }

    fn plot_with(
        pal: &Palette,
        under: u8,
        color: u8,
        fx: PaintFx,
        cover: u8,
        mode: DrawMode,
    ) -> u8 {
        let before = IndexedImage::new(3, 3, under);
        let c = ctx(pal, &before);
        let mut page = before.clone();
        plot_fx(&mut page, 1, 1, color, mode, &c, &fx, cover);
        page.get(1, 1).unwrap()
    }

    #[test]
    fn effects_off_paint_the_plain_colour() {
        let pal = bw();
        for mode in [DrawMode::Matte, DrawMode::Color, DrawMode::Replace] {
            assert_eq!(plot_with(&pal, 0, 1, PaintFx::default(), 100, mode), 1);
        }
        assert!(PaintFx::default().is_off());
    }

    #[test]
    fn translucency_keeps_the_given_share_of_the_page() {
        let pal = bw();
        let fx = |t| PaintFx {
            translucency: Some(t),
            ..PaintFx::default()
        };
        // White over black: 50 % remains = 128 grey (index 2); 75 % remains = 64 (index 3).
        assert_eq!(plot_with(&pal, 0, 1, fx(50), 100, DrawMode::Matte), 2);
        assert_eq!(plot_with(&pal, 0, 1, fx(75), 100, DrawMode::Matte), 3);
        // 100 % remains: nothing changes; 0 % remains: the plain colour.
        assert_eq!(plot_with(&pal, 0, 1, fx(100), 100, DrawMode::Matte), 0);
        assert_eq!(plot_with(&pal, 0, 1, fx(0), 100, DrawMode::Matte), 1);
        // The same colour over itself stays put.
        assert_eq!(plot_with(&pal, 1, 1, fx(50), 100, DrawMode::Color), 1);
        // Modes that are not "set" modes ignore it.
        assert_eq!(plot_with(&pal, 0, 1, fx(50), 100, DrawMode::Blend), 2);
    }

    #[test]
    fn translucency_reads_the_page_before_the_gesture() {
        let pal = bw();
        let before = IndexedImage::new(3, 3, 0);
        let c = ctx(&pal, &before);
        let fx = PaintFx {
            translucency: Some(50),
            ..PaintFx::default()
        };
        let mut page = before.clone();
        for _ in 0..3 {
            plot_fx(&mut page, 1, 1, 1, DrawMode::Matte, &c, &fx, 100);
        }
        assert_eq!(
            page.get(1, 1),
            Some(2),
            "painting over twice does not add up"
        );
    }

    #[test]
    fn cover_blends_antialiased_edges() {
        let pal = bw();
        let fx = PaintFx::default();
        assert_eq!(plot_with(&pal, 0, 1, fx, 50, DrawMode::Matte), 2);
        assert_eq!(plot_with(&pal, 0, 1, fx, 0, DrawMode::Matte), 0);
        // Cover and Translucency multiply: 50 % * 50 % of white = 64 grey.
        let fx = PaintFx {
            translucency: Some(50),
            ..fx
        };
        assert_eq!(plot_with(&pal, 0, 1, fx, 50, DrawMode::Matte), 3);
    }

    #[test]
    fn edge_cover_low_and_high() {
        let img = IndexedImage::new(3, 3, 5);
        let b = BrushImage::from_image(&img, Some(0));
        assert_eq!(edge_cover(&b, 1, 1, AntiAlias::None), 100);
        assert_eq!(edge_cover(&b, 1, 1, AntiAlias::Low), 100, "inner pixel");
        assert_eq!(edge_cover(&b, 0, 1, AntiAlias::Low), 50, "left edge");
        assert_eq!(edge_cover(&b, 0, 0, AntiAlias::Low), 50, "corner");
        assert_eq!(edge_cover(&b, 1, 1, AntiAlias::High), 100);
        assert_eq!(edge_cover(&b, 0, 1, AntiAlias::High), 66, "6 of 9");
        assert_eq!(edge_cover(&b, 0, 0, AntiAlias::High), 44, "4 of 9");
    }

    fn colours() -> Palette {
        Palette::new(vec![
            Rgb8::new(128, 128, 128),
            Rgb8::new(255, 0, 0),
            Rgb8::new(0, 255, 0),
            Rgb8::new(255, 255, 255),
            Rgb8::new(40, 40, 40),
            Rgb8::new(128, 0, 0),
        ])
        .unwrap()
    }

    #[test]
    fn process_modes_take_parts_of_the_brush_colour() {
        let grey = Rgb8::new(128, 128, 128);
        let red = Rgb8::new(255, 0, 0);
        // Tint: red's hue and saturation at the grey's brightness.
        assert_eq!(process(grey, red, ProcessMode::Tint), Rgb8::new(128, 0, 0));
        // Hue: a grey has no saturation of its own, so it stays grey.
        assert_eq!(process(grey, red, ProcessMode::Hue), grey);
        // Value: the brush's brightness on the page's hue.
        assert_eq!(
            process(
                Rgb8::new(0, 100, 0),
                Rgb8::new(255, 255, 255),
                ProcessMode::Value
            ),
            Rgb8::new(0, 255, 0)
        );
        assert_eq!(
            process(Rgb8::new(0, 100, 0), Rgb8::new(255, 0, 0), ProcessMode::Hue),
            Rgb8::new(100, 0, 0)
        );
    }

    #[test]
    fn process_paints_the_processed_page_colour() {
        let pal = colours();
        let fx = PaintFx {
            process: Some(ProcessMode::Tint),
            ..PaintFx::default()
        };
        // Page grey (0), FG red (1): the page turns dark red = entry 5.
        assert_eq!(plot_with(&pal, 0, 1, fx, 100, DrawMode::Matte), 5);
        // Value with white brightens the grey to white.
        let fx = PaintFx {
            process: Some(ProcessMode::Value),
            ..PaintFx::default()
        };
        assert_eq!(plot_with(&pal, 0, 3, fx, 100, DrawMode::Matte), 3);
    }

    #[test]
    fn fx_round_trips_through_json_and_is_left_out_by_default() {
        let fx = PaintFx {
            translucency: Some(30),
            process: Some(ProcessMode::Hue),
            antialias: AntiAlias::High,
        };
        let json = serde_json::to_string(&fx).unwrap();
        assert_eq!(serde_json::from_str::<PaintFx>(&json).unwrap(), fx);
        assert_eq!(serde_json::to_string(&PaintFx::default()).unwrap(), "{}");
    }
}
