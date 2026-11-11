//! Color ▸ Palette ▸ Use Brush Palette (ADR-39): the picture takes the custom brush's own
//! palette (AGA p.267-268).
//!
//! The brush's own palette is the palette its colour numbers belong to: a brush file's CMAP,
//! or the picture palette when it was picked up. If the brush fits the picture's colour count,
//! the palette is taken as it is (entries the brush palette lacks keep the picture's colours)
//! and the brush keeps its colour numbers. Otherwise the palette is recomputed with the
//! picture's number of colours ([`crate::quantize::reduce`] over the brush's pixels: its own
//! colours when they fit, else median cut; nearest colour, no dither), the brush palette's
//! other colours fill the entries left, and the brush is repainted in it.

use crate::paint::BrushImage;
use crate::palette::{Palette, Rgb8};
use crate::quantize::{self, Dither, PaletteDepth, ReduceOptions};
use crate::rgba::{Rgba8, RgbaImage};

/// The result of [`use_brush_palette`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adopted {
    /// The picture's new palette, with the picture's number of colours.
    pub colors: Vec<Rgb8>,
    /// The brush in the new palette's colour numbers.
    pub brush: BrushImage,
    /// The brush palette had more colours than the picture and was recomputed; colour numbers
    /// (and with them colour ranges) do not carry over then.
    pub reduced: bool,
}

/// Use Brush Palette: `brush`'s colour numbers belong to `own`; `current` is the picture
/// palette (its length is kept). `depth` is the palette precision for a recomputed palette
/// (OCS: 12-bit).
pub fn use_brush_palette(
    brush: &BrushImage,
    own: &[Rgb8],
    current: &Palette,
    depth: PaletteDepth,
) -> Adopted {
    let n = current.len();
    let fits = |c: u8| usize::from(c) < n;
    if brush.pixels.iter().all(|&p| fits(p)) && brush.transparent.is_none_or(fits) {
        let colors = current
            .colors()
            .iter()
            .enumerate()
            .map(|(i, &c)| own.get(i).copied().unwrap_or(c))
            .collect();
        return Adopted {
            colors,
            brush: brush.clone(),
            reduced: false,
        };
    }
    // The brush's pixels (transparent ones with alpha 0, which the reduction keeps in colour
    // 0) decide the colours; the brush palette's other colours fill the entries left.
    let color = |c: u8| own.get(usize::from(c)).copied().unwrap_or(Rgb8::BLACK);
    let pixels: Vec<Rgba8> = brush
        .pixels
        .iter()
        .map(|&p| {
            let c = color(p);
            let alpha = if Some(p) == brush.transparent { 0 } else { 255 };
            Rgba8::new(c.r, c.g, c.b, alpha)
        })
        .collect();
    let image = RgbaImage::from_pixels(brush.width, brush.height, pixels).expect("brush size");
    let reduced = quantize::reduce(
        &image,
        &ReduceOptions {
            max_colors: n as u16,
            depth,
            dither: Dither::None,
            transparent: brush.transparent.map_or(Rgb8::BLACK, color),
        },
    );
    let used = reduced.colors + usize::from(reduced.transparent_index.is_some());
    let mut colors = reduced.palette.colors()[..used].to_vec();
    let snap = |c: Rgb8| match depth {
        PaletteDepth::Rgb4 => Rgb8::from_rgb4(c.to_rgb4()),
        PaletteDepth::Rgb8 => c,
    };
    for &c in own {
        if colors.len() >= n {
            break;
        }
        let c = snap(c);
        if !colors.contains(&c) {
            colors.push(c);
        }
    }
    colors.resize(n, Rgb8::BLACK);
    Adopted {
        colors,
        brush: BrushImage {
            pixels: reduced.image.pixels().to_vec(),
            transparent: reduced.transparent_index,
            ..brush.clone()
        },
        reduced: true,
    }
}

/// `brush`'s colours (numbers into `from`) as the nearest colours of `to`: what a changed
/// brush's pixels are in its own palette `to`. Opaque pixels never land on the transparent
/// colour; numbers `from` lacks stay as they are.
pub fn fit_brush(brush: &BrushImage, from: &Palette, to: &[Rgb8]) -> BrushImage {
    let Ok(to) = Palette::new(to.to_vec()) else {
        return brush.clone();
    };
    let exclude: Vec<u8> = brush.transparent.into_iter().collect();
    let mut map: [Option<u8>; 256] = [None; 256];
    let mut out = brush.clone();
    for p in &mut out.pixels {
        if Some(*p) == brush.transparent {
            continue;
        }
        let slot = &mut map[usize::from(*p)];
        *p = *slot.get_or_insert_with(|| match from.get(*p) {
            Some(c) => to.nearest_excluding(c, &exclude),
            None => *p,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(v: u8) -> Rgb8 {
        Rgb8::new(v, v, v)
    }

    fn brush(pixels: Vec<u8>, transparent: Option<u8>) -> BrushImage {
        BrushImage {
            width: pixels.len() as u32,
            height: 1,
            pixels,
            handle: (0, 0),
            transparent,
        }
    }

    #[test]
    fn a_brush_that_fits_keeps_its_colour_numbers() {
        let current = Palette::new((0..8).map(|i| rgb(i * 10)).collect()).unwrap();
        // Fewer entries: the rest of the picture palette stays.
        let own: Vec<Rgb8> = (0..4).map(|i| rgb(200 + i)).collect();
        let b = brush(vec![1, 3, 0], Some(0));
        let a = use_brush_palette(&b, &own, &current, PaletteDepth::Rgb8);
        assert!(!a.reduced);
        assert_eq!(a.brush, b);
        assert_eq!(
            a.colors,
            [200, 201, 202, 203, 40, 50, 60, 70].map(rgb).to_vec()
        );
        // More entries, but the brush uses only the first ones: cut to the picture's count.
        let own: Vec<Rgb8> = (0..32).map(|i| rgb(100 + i)).collect();
        let a = use_brush_palette(&b, &own, &current, PaletteDepth::Rgb8);
        assert!(!a.reduced);
        assert_eq!(a.colors, (0..8).map(|i| rgb(100 + i)).collect::<Vec<_>>());
    }

    #[test]
    fn a_brush_with_more_colours_gets_a_recomputed_palette() {
        let current = Palette::new((0..4).map(rgb).collect()).unwrap();
        // 16 brush palette entries; the brush uses colours 9 and 12 and is transparent in 15.
        let own: Vec<Rgb8> = (0..16)
            .map(|i| Rgb8::new(i * 16, 0, 255 - i * 16))
            .collect();
        let b = brush(vec![9, 12, 15, 9, 9], Some(15));
        let a = use_brush_palette(&b, &own, &current, PaletteDepth::Rgb8);
        assert!(a.reduced);
        assert_eq!(a.brush.transparent, Some(0), "transparent kept in colour 0");
        assert_eq!(a.brush.pixels, [1, 2, 0, 1, 1], "most used first");
        // Few enough colours: exactly the brush's, then the brush palette's first.
        assert_eq!(a.colors, vec![own[15], own[9], own[12], own[0]]);
        // More colours than entries: the nearest of a recomputed palette, never transparent.
        let b = brush(vec![1, 3, 5, 7, 9, 11, 13, 15], Some(15));
        let a = use_brush_palette(&b, &own, &current, PaletteDepth::Rgb8);
        assert_eq!(a.colors.len(), 4, "the picture's colour count");
        assert_eq!(a.brush.pixels[7], 0);
        for (i, &p) in a.brush.pixels[..7].iter().enumerate() {
            assert_ne!(p, 0, "opaque pixel {i} stays opaque");
            let want = own[usize::from(b.pixels[i])];
            assert!(a.colors[usize::from(p)].distance_sq(want) < 3 * 64 * 64);
        }
        // OCS pictures get 12-bit colours.
        let ocs = use_brush_palette(&b, &own, &current, PaletteDepth::Rgb4);
        assert!(
            ocs.colors
                .iter()
                .all(|c| Rgb8::from_rgb4(c.to_rgb4()) == *c)
        );
    }

    #[test]
    fn fit_brush_finds_the_nearest_own_colours_but_not_the_transparent_one() {
        let from = Palette::new(vec![rgb(0), rgb(100), rgb(250)]).unwrap();
        let own = [rgb(240), rgb(110), rgb(5)];
        // Colour 0 is transparent: 100 goes to own 1 (110); 250 would be nearest to own 0
        // (240), the transparent colour, so it takes the nearest other one (110).
        let b = brush(vec![0, 1, 2], Some(0));
        let fitted = fit_brush(&b, &from, &own);
        assert_eq!(fitted.pixels, vec![0, 1, 1]);
        let b = brush(vec![0, 1, 2], None);
        assert_eq!(fit_brush(&b, &from, &own).pixels, vec![2, 1, 0]);
    }
}
