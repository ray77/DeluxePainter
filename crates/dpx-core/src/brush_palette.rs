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
    let AdoptedCels {
        colors,
        mut cels,
        reduced,
    } = use_cels_palette(std::slice::from_ref(brush), own, current, depth);
    Adopted {
        colors,
        brush: cels.pop().expect("one cel"),
        reduced,
    }
}

/// The result of [`use_cels_palette`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptedCels {
    /// The picture's new palette, with the picture's number of colours.
    pub colors: Vec<Rgb8>,
    /// The cels in the new palette's colour numbers.
    pub cels: Vec<BrushImage>,
    /// See [`Adopted::reduced`].
    pub reduced: bool,
}

/// [`use_brush_palette`] for the cels of an AnimBrush (ADR-36): one palette for all of them.
/// When every cel fits the picture's colour count the palette is taken as it is and the
/// cels keep their numbers; otherwise it is recomputed from the pixels of every cel together
/// and every cel is repainted in it (a cel's transparent pixels stay transparent).
pub fn use_cels_palette(
    cels: &[BrushImage],
    own: &[Rgb8],
    current: &Palette,
    depth: PaletteDepth,
) -> AdoptedCels {
    let n = current.len();
    let fits = |c: u8| usize::from(c) < n;
    // The palette precision: an OCS picture takes the brush colours as 12-bit values
    // (ADR-46), also when the palette is taken as it is.
    let snap = |c: Rgb8| match depth {
        PaletteDepth::Rgb4 => Rgb8::from_rgb4(c.to_rgb4()),
        PaletteDepth::Rgb8 => c,
    };
    if cels
        .iter()
        .all(|b| b.pixels.iter().all(|&p| fits(p)) && b.transparent.is_none_or(fits))
    {
        let colors = current
            .colors()
            .iter()
            .enumerate()
            .map(|(i, &c)| own.get(i).copied().map_or(c, snap))
            .collect();
        return AdoptedCels {
            colors,
            cels: cels.to_vec(),
            reduced: false,
        };
    }
    // The cels' pixels (transparent ones with alpha 0, which the reduction keeps in colour
    // 0) decide the colours; the brush palette's other colours fill the entries left. The
    // cels lie one below the other on a canvas as wide as the widest; the space beside a
    // narrower one is transparent.
    let color = |c: u8| own.get(usize::from(c)).copied().unwrap_or(Rgb8::BLACK);
    let width = cels.iter().map(|b| b.width).max().unwrap_or(0).max(1);
    let height = cels.iter().map(|b| b.height).sum::<u32>().max(1);
    let mut pixels = vec![Rgba8::new(0, 0, 0, 0); width as usize * height as usize];
    let mut top = 0usize;
    for b in cels {
        for (y, row) in b.pixels.chunks(b.width.max(1) as usize).enumerate() {
            for (x, &p) in row.iter().enumerate() {
                let c = color(p);
                let alpha = if Some(p) == b.transparent { 0 } else { 255 };
                pixels[(top + y) * width as usize + x] = Rgba8::new(c.r, c.g, c.b, alpha);
            }
        }
        top += b.height as usize;
    }
    let image = RgbaImage::from_pixels(width, height, pixels).expect("canvas size");
    let transparent = cels
        .iter()
        .find_map(|b| b.transparent)
        .map_or(Rgb8::BLACK, color);
    let reduced = quantize::reduce(
        &image,
        &ReduceOptions {
            max_colors: n as u16,
            depth,
            dither: Dither::None,
            transparent,
        },
    );
    let used = reduced.colors + usize::from(reduced.transparent_index.is_some());
    let mut colors = reduced.palette.colors()[..used].to_vec();
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
    let out = reduced.image.pixels();
    let mut top = 0usize;
    let cels = cels
        .iter()
        .map(|b| {
            let w = b.width as usize;
            let pixels = (0..b.height as usize)
                .flat_map(|y| {
                    let start = (top + y) * width as usize;
                    out[start..start + w].iter().copied()
                })
                .collect();
            top += b.height as usize;
            BrushImage {
                pixels,
                transparent: b.transparent.and(reduced.transparent_index),
                ..b.clone()
            }
        })
        .collect();
    AdoptedCels {
        colors,
        cels,
        reduced: true,
    }
}

/// `brush`'s colours (numbers into `from`) as the nearest colours of `to`: what a changed
/// brush's pixels are in its own palette `to`. Opaque pixels never land on the transparent
/// colour; numbers `from` lacks stay as they are.
pub fn fit_brush(brush: &BrushImage, from: &Palette, to: &[Rgb8]) -> BrushImage {
    fit_brush_into(brush, from, to, brush.transparent)
}

/// [`fit_brush`] into a palette whose transparent colour is number `transparent` (the
/// brush Brush ▸ Restore gives back after Use Brush Palette recomputed the palette): `brush`'s
/// transparent pixels take that number, the others the nearest other colours of `to`. With
/// `None`, a brush without transparent pixels becomes opaque, and one with them keeps its
/// transparent number.
pub fn fit_brush_into(
    brush: &BrushImage,
    from: &Palette,
    to: &[Rgb8],
    transparent: Option<u8>,
) -> BrushImage {
    let Ok(to) = Palette::new(to.to_vec()) else {
        return brush.clone();
    };
    let shows_through = brush.transparent.is_some_and(|t| brush.pixels.contains(&t));
    let target = transparent.or(brush.transparent.filter(|_| shows_through));
    let exclude: Vec<u8> = target.into_iter().collect();
    let mut map: [Option<u8>; 256] = [None; 256];
    let mut out = brush.clone();
    out.transparent = target;
    for p in &mut out.pixels {
        if Some(*p) == brush.transparent {
            *p = target.unwrap_or(*p);
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
        // Also when the palette is taken as it is (ADR-46): the brush fits, the brush
        // palette's colours come in rounded to 12 bits, the brush keeps its numbers.
        let fits = brush(vec![1, 2, 3], None);
        let taken = use_brush_palette(&fits, &own, &current, PaletteDepth::Rgb4);
        assert!(!taken.reduced);
        assert_eq!(taken.brush, fits);
        assert_ne!(taken.colors[1], own[1], "#1000EF is not 12-bit");
        for (i, c) in taken.colors.iter().enumerate() {
            assert_eq!(*c, Rgb8::from_rgb4(own[i].to_rgb4()), "entry {i}");
        }
        let aga = use_brush_palette(&fits, &own, &current, PaletteDepth::Rgb8);
        assert_eq!(aga.colors, own[..4].to_vec(), "AGA takes them exactly");
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

    #[test]
    fn animbrush_cels_share_one_palette() {
        let current = Palette::new((0..4).map(rgb).collect()).unwrap();
        let own: Vec<Rgb8> = (0..16)
            .map(|i| Rgb8::new(i * 16, 0, 255 - i * 16))
            .collect();
        // Every cel fits: the palette as it is, the numbers kept.
        let small = [brush(vec![1, 2], Some(0)), brush(vec![3, 0, 3], Some(0))];
        let a = use_cels_palette(&small, &own, &current, PaletteDepth::Rgb8);
        assert!(!a.reduced);
        assert_eq!(a.cels, small.to_vec());
        assert_eq!(a.colors, own[..4].to_vec());
        // One cel does not fit: one palette from the colours of both, both repainted in it.
        let cels = [
            brush(vec![9, 15, 9], Some(15)),
            brush(vec![12, 12], Some(15)),
        ];
        let a = use_cels_palette(&cels, &own, &current, PaletteDepth::Rgb8);
        assert!(a.reduced);
        assert_eq!(a.colors.len(), 4);
        assert_eq!(a.cels.len(), 2);
        let shown = |b: &BrushImage| -> Vec<Option<Rgb8>> {
            b.pixels
                .iter()
                .map(|&p| (Some(p) != b.transparent).then(|| a.colors[usize::from(p)]))
                .collect()
        };
        assert_eq!(shown(&a.cels[0]), vec![Some(own[9]), None, Some(own[9])]);
        assert_eq!(shown(&a.cels[1]), vec![Some(own[12]), Some(own[12])]);
        assert_eq!((a.cels[1].width, a.cels[1].height), (2, 1));
        // A single brush gets what Use Brush Palette gives it.
        let one = use_brush_palette(&cels[0], &own, &current, PaletteDepth::Rgb8);
        let alone = use_cels_palette(&cels[..1], &own, &current, PaletteDepth::Rgb8);
        assert_eq!((one.colors, vec![one.brush]), (alone.colors, alone.cels));
    }

    #[test]
    fn fit_brush_into_takes_the_new_palettes_transparent_colour() {
        let from = Palette::new(vec![rgb(0), rgb(100), rgb(250)]).unwrap();
        let to = [rgb(250), rgb(0), rgb(100)];
        // Transparent pixels (colour 0) take the new transparent number 1; 0 (black) is no
        // longer free for opaque pixels then.
        let b = brush(vec![0, 1, 2], Some(0));
        let fitted = fit_brush_into(&b, &from, &to, Some(1));
        assert_eq!(
            (fitted.pixels.as_slice(), fitted.transparent),
            (&[1, 2, 0][..], Some(1))
        );
        // No transparent number in the new palette: a brush that nowhere shows through becomes
        // opaque, so every colour is free again ...
        let b = brush(vec![1, 2, 1], Some(0));
        let fitted = fit_brush_into(&b, &from, &[rgb(100), rgb(250)], None);
        assert_eq!(
            (fitted.pixels.as_slice(), fitted.transparent),
            (&[0, 1, 0][..], None)
        );
        // ... one that shows through keeps its transparent number.
        let b = brush(vec![0, 2], Some(0));
        let fitted = fit_brush_into(&b, &from, &[rgb(250), rgb(240)], None);
        assert_eq!(
            (fitted.pixels.as_slice(), fitted.transparent),
            (&[0, 1][..], Some(0))
        );
    }
}
