//! Custom brushes and drawing modes (ADR-18).
//!
//! A [`BrushImage`] is a picked-up piece of the page; pixels in its transparent colour (the
//! background colour at pick-up time) are not painted. [`plot`] paints one brush pixel onto
//! the page in a [`DrawMode`]; every stamping path of the drawing tools goes through it so
//! the stencil and the modes apply everywhere.

use serde::{Deserialize, Serialize};

use crate::image::IndexedImage;
use crate::palette::Palette;

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
/// the brush's own colour). The other modes are not implemented yet and also set the pixel.
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
    let _ = mode;
    page.set(x, y, color);
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
}
