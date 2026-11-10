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
    /// Extra-HalfBrite: the colour under the brush becomes its half-bright twin (index + 32).
    HBrite,
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
    /// The right mouse button is painting (Shade goes down, Color uses BG).
    pub right: bool,
}

/// Paints one brush pixel of colour `color` at (`x`, `y`) in `mode`. Pixels outside the page
/// and pixels whose current colour the stencil protects are left alone.
///
/// Matte, Color and Replace set the pixel to `color` (the caller has already chosen FG, BG or
/// the brush's own colour). The other modes (ADR-19) ignore `color` unless noted and read the
/// colour under the brush from `ctx.before`, so a gesture changes a pixel at most one step
/// however often the brush passes over it. A mode that has nothing to do leaves the pixel.
pub fn plot(
    page: &mut IndexedImage,
    x: i32,
    y: i32,
    color: u8,
    mode: DrawMode,
    ctx: &PaintContext,
) {
    let Some(under) = page.get(x, y) else {
        return;
    };
    if ctx
        .stencil
        .is_some_and(|s| s.get(usize::from(under)).copied().unwrap_or(false))
    {
        return;
    }
    let before = ctx.before.get(x, y).unwrap_or(under);
    let new = match mode {
        DrawMode::Matte | DrawMode::Color | DrawMode::Replace => Some(color),
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
        DrawMode::HBrite => (before < 32 && ctx.palette.len() == 64).then_some(before + 32),
    };
    if let Some(c) = new {
        page.set(x, y, c);
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
}
