//! Colour reduction of true-colour pictures (ADR-9).
//!
//! - A picture with no more colours than the target allows keeps them exactly; on OCS (12-bit
//!   palette) each colour is rounded to the nearest 12-bit value, without dithering.
//! - Otherwise a new palette is built by variance-based median cut on a 6-bit-per-channel
//!   histogram, refined by a few k-means passes, and the picture is remapped (Floyd–Steinberg
//!   error diffusion by default, serpentine).
//! - Transparent pixels (alpha below [`ALPHA_THRESHOLD`]) become colour 0, which is reserved for
//!   them and gets the caller's transparent colour (DeluxePainter uses pink #FF00FF).
//! - Palette order: (reserved colour 0,) the most used colour, then the rest from dark to light.
//!   The palette is padded with black to a power of two (whole bitplanes); padding entries are
//!   never used by pixels.
//!
//! Distances are weighted RGB (2·ΔR² + 4·ΔG² + 3·ΔB²), the same for clustering and mapping.

use std::collections::HashMap;

use crate::image::IndexedImage;
use crate::palette::{Palette, Rgb8};
use crate::rgba::{Rgba8, RgbaImage};

/// Pixels with less alpha than this count as transparent.
pub const ALPHA_THRESHOLD: u8 = 128;

const WEIGHTS: [f64; 3] = [2.0, 4.0, 3.0];
const KMEANS_PASSES: usize = 3;

/// Precision of palette entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteDepth {
    /// OCS: 4 bits per channel.
    Rgb4,
    /// AGA: 8 bits per channel.
    Rgb8,
}

impl PaletteDepth {
    fn snap(self, c: Rgb8) -> Rgb8 {
        match self {
            PaletteDepth::Rgb4 => Rgb8::from_rgb4(c.to_rgb4()),
            PaletteDepth::Rgb8 => c,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dither {
    /// Nearest palette colour only.
    None,
    /// Floyd–Steinberg error diffusion.
    FloydSteinberg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReduceOptions {
    /// Most palette entries, including the reserved transparent one (clamped to 2..=256).
    pub max_colors: u16,
    pub depth: PaletteDepth,
    pub dither: Dither,
    /// Colour of the reserved entry 0 when the picture has transparent pixels.
    pub transparent: Rgb8,
}

/// What the reduction had to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reduction {
    /// Every colour kept exactly.
    Exact,
    /// Few enough colours, but rounded to the palette depth (OCS 12-bit); no dithering.
    Rounded,
    /// New palette built and the picture remapped.
    Quantized,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reduced {
    pub image: IndexedImage,
    pub palette: Palette,
    /// Colour 0 when the picture had transparent pixels.
    pub transparent_index: Option<u8>,
    pub reduction: Reduction,
    /// Palette entries that pixels use (without the reserved and padding entries).
    pub colors: usize,
}

fn rgb(p: Rgba8) -> Rgb8 {
    Rgb8::new(p.r, p.g, p.b)
}

fn is_transparent(p: Rgba8) -> bool {
    p.a < ALPHA_THRESHOLD
}

fn to_f(c: Rgb8) -> [f64; 3] {
    [f64::from(c.r), f64::from(c.g), f64::from(c.b)]
}

fn from_f(c: [f64; 3]) -> Rgb8 {
    let q = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    Rgb8::new(q(c[0]), q(c[1]), q(c[2]))
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| WEIGHTS[i] * (a[i] - b[i]).powi(2)).sum()
}

fn luma(c: Rgb8) -> u32 {
    299 * u32::from(c.r) + 587 * u32::from(c.g) + 114 * u32::from(c.b)
}

/// Index into `pal` of the colour nearest to `c` (ties: lowest index).
fn nearest(pal: &[[f64; 3]], c: [f64; 3]) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (i, &p) in pal.iter().enumerate() {
        let d = dist(p, c);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// Reduces `img` to an indexed picture as described in the module documentation.
pub fn reduce(img: &RgbaImage, opts: &ReduceOptions) -> Reduced {
    let max = usize::from(opts.max_colors.clamp(2, 256));
    let transparent = img.pixels().iter().any(|&p| is_transparent(p));
    let offset = usize::from(transparent);
    let avail = max - offset;

    let (entries, reduction) = match unique_colors(img, avail) {
        Some(counts) => {
            let mut merged: HashMap<Rgb8, u64> = HashMap::new();
            let mut rounded = false;
            for (c, n) in counts {
                let s = opts.depth.snap(c);
                rounded |= s != c;
                *merged.entry(s).or_default() += n;
            }
            let r = if rounded {
                Reduction::Rounded
            } else {
                Reduction::Exact
            };
            (merged.into_iter().collect::<Vec<_>>(), r)
        }
        None => (palette_for(img, avail, opts.depth), Reduction::Quantized),
    };
    let entries = order(entries);
    let colors = entries.len();

    let mut pal: Vec<Rgb8> = Vec::with_capacity(max);
    if transparent {
        pal.push(opts.depth.snap(opts.transparent));
    }
    pal.extend(entries.iter().map(|&(c, _)| c));
    let padded = pal.len().max(2).next_power_of_two();
    pal.resize(padded, Rgb8::BLACK);

    let real: Vec<[f64; 3]> = entries.iter().map(|&(c, _)| to_f(c)).collect();
    let pixels = match reduction {
        Reduction::Exact | Reduction::Rounded => {
            let index: HashMap<Rgb8, u8> = entries
                .iter()
                .enumerate()
                .map(|(i, &(c, _))| (c, (i + offset) as u8))
                .collect();
            img.pixels()
                .iter()
                .map(|&p| {
                    if is_transparent(p) {
                        0
                    } else {
                        index[&opts.depth.snap(rgb(p))]
                    }
                })
                .collect()
        }
        Reduction::Quantized => match opts.dither {
            Dither::None => map_nearest(img, &real, offset),
            Dither::FloydSteinberg => map_floyd_steinberg(img, &real, offset),
        },
    };
    Reduced {
        image: IndexedImage::from_pixels(img.width(), img.height(), pixels)
            .expect("one index per pixel"),
        palette: Palette::new(pal).expect("2..=256 entries"),
        transparent_index: transparent.then_some(0),
        reduction,
        colors,
    }
}

/// The opaque colours with their pixel counts, or `None` if there are more than `limit`.
fn unique_colors(img: &RgbaImage, limit: usize) -> Option<Vec<(Rgb8, u64)>> {
    let mut counts: HashMap<Rgb8, u64> = HashMap::new();
    for &p in img.pixels() {
        if is_transparent(p) {
            continue;
        }
        *counts.entry(rgb(p)).or_default() += 1;
        if counts.len() > limit {
            return None;
        }
    }
    Some(counts.into_iter().collect())
}

/// Most used colour first, then dark to light (ties broken by RGB, so the order is stable).
fn order(mut entries: Vec<(Rgb8, u64)>) -> Vec<(Rgb8, u64)> {
    let key = |c: Rgb8| (luma(c), c.r, c.g, c.b);
    entries.sort_by_key(|&(c, _)| key(c));
    if let Some(top) = (0..entries.len()).max_by_key(|&i| (entries[i].1, std::cmp::Reverse(i))) {
        let e = entries.remove(top);
        entries.insert(0, e);
    }
    entries
}

#[derive(Clone, Copy)]
struct Item {
    c: [f64; 3],
    w: f64,
}

/// A new palette of at most `n` colours (with pixel weights) for the opaque pixels.
fn palette_for(img: &RgbaImage, n: usize, depth: PaletteDepth) -> Vec<(Rgb8, u64)> {
    // 6-bit-per-channel histogram; each bin keeps the exact mean of its pixels.
    let mut bins: HashMap<u32, (u64, [u64; 3])> = HashMap::new();
    for &p in img.pixels() {
        if is_transparent(p) {
            continue;
        }
        let key = (u32::from(p.r >> 2) << 12) | (u32::from(p.g >> 2) << 6) | u32::from(p.b >> 2);
        let bin = bins.entry(key).or_default();
        bin.0 += 1;
        bin.1[0] += u64::from(p.r);
        bin.1[1] += u64::from(p.g);
        bin.1[2] += u64::from(p.b);
    }
    let mut keys: Vec<u32> = bins.keys().copied().collect();
    keys.sort_unstable();
    let mut items: Vec<Item> = keys
        .iter()
        .map(|k| {
            let (count, sum) = bins[k];
            let w = count as f64;
            Item {
                c: [sum[0] as f64 / w, sum[1] as f64 / w, sum[2] as f64 / w],
                w,
            }
        })
        .collect();

    let centres = median_cut(&mut items, n);
    let (centres, weights) = kmeans(&items, centres);

    let mut merged: HashMap<Rgb8, u64> = HashMap::new();
    for (c, w) in centres.into_iter().zip(weights) {
        if w > 0.0 {
            *merged.entry(depth.snap(from_f(c))).or_default() += w as u64;
        }
    }
    merged.into_iter().collect()
}

/// Weighted sums over a run of items, for the sum of squared errors around their mean.
#[derive(Clone, Copy, Default)]
struct Sums {
    w: f64,
    c: [f64; 3],
    /// Σ w · Σ_ch weight_ch · c_ch².
    sq: f64,
}

impl Sums {
    fn add(&mut self, it: &Item) {
        self.w += it.w;
        for ((sum, &v), wt) in self.c.iter_mut().zip(&it.c).zip(WEIGHTS) {
            *sum += it.w * v;
            self.sq += it.w * wt * v * v;
        }
    }

    fn minus(self, o: Sums) -> Sums {
        Sums {
            w: self.w - o.w,
            c: [self.c[0] - o.c[0], self.c[1] - o.c[1], self.c[2] - o.c[2]],
            sq: self.sq - o.sq,
        }
    }

    fn sse(&self) -> f64 {
        if self.w <= 0.0 {
            return 0.0;
        }
        let mean: f64 = (0..3)
            .map(|ch| WEIGHTS[ch] * self.c[ch] * self.c[ch])
            .sum::<f64>()
            / self.w;
        (self.sq - mean).max(0.0)
    }

    fn mean(&self) -> [f64; 3] {
        [self.c[0] / self.w, self.c[1] / self.w, self.c[2] / self.w]
    }
}

fn sums(items: &[Item]) -> Sums {
    let mut s = Sums::default();
    items.iter().for_each(|it| s.add(it));
    s
}

/// Splits the colour space into at most `n` boxes; always splits the box with the largest
/// error, along its widest (weighted) axis, at the point that minimises the error of both
/// halves. Returns the box means.
fn median_cut(items: &mut [Item], n: usize) -> Vec<[f64; 3]> {
    // (start, end, sse)
    let mut boxes: Vec<(usize, usize, f64)> = vec![(0, items.len(), sums(items).sse())];
    while boxes.len() < n {
        let Some(bi) = (0..boxes.len())
            .filter(|&i| boxes[i].1 - boxes[i].0 > 1 && boxes[i].2 > 0.0)
            .max_by(|&a, &b| boxes[a].2.total_cmp(&boxes[b].2))
        else {
            break;
        };
        let (start, end, _) = boxes[bi];
        let run = &mut items[start..end];
        let axis = widest_axis(run);
        run.sort_by(|a, b| a.c[axis].total_cmp(&b.c[axis]));
        let total = sums(run);
        let mut left = Sums::default();
        let mut best = (f64::INFINITY, 1);
        for k in 1..run.len() {
            left.add(&run[k - 1]);
            // Only split between different values, so equal colours stay together.
            if run[k - 1].c[axis] == run[k].c[axis] {
                continue;
            }
            let e = left.sse() + total.minus(left).sse();
            if e < best.0 {
                best = (e, k);
            }
        }
        if !best.0.is_finite() {
            // All items share the axis value; nothing to split here.
            boxes[bi].2 = 0.0;
            continue;
        }
        let mid = start + best.1;
        let (a, b) = (sums(&items[start..mid]).sse(), sums(&items[mid..end]).sse());
        boxes[bi] = (start, mid, a);
        boxes.push((mid, end, b));
    }
    boxes
        .iter()
        .map(|&(s, e, _)| sums(&items[s..e]).mean())
        .collect()
}

fn widest_axis(items: &[Item]) -> usize {
    let s = sums(items);
    let var = |ch: usize| {
        let sq: f64 = items.iter().map(|it| it.w * it.c[ch] * it.c[ch]).sum();
        WEIGHTS[ch] * (sq - s.c[ch] * s.c[ch] / s.w)
    };
    (0..3)
        .max_by(|&a, &b| var(a).total_cmp(&var(b)))
        .unwrap_or(0)
}

/// A few Lloyd passes; returns the centres and the pixel weight of each.
fn kmeans(items: &[Item], mut centres: Vec<[f64; 3]>) -> (Vec<[f64; 3]>, Vec<f64>) {
    let mut weights = vec![0.0; centres.len()];
    for pass in 0..=KMEANS_PASSES {
        let mut acc = vec![Sums::default(); centres.len()];
        for it in items {
            acc[nearest(&centres, it.c)].add(it);
        }
        weights = acc.iter().map(|s| s.w).collect();
        if pass == KMEANS_PASSES {
            break;
        }
        for (c, s) in centres.iter_mut().zip(&acc) {
            if s.w > 0.0 {
                *c = s.mean();
            }
        }
    }
    (centres, weights)
}

fn map_nearest(img: &RgbaImage, pal: &[[f64; 3]], offset: usize) -> Vec<u8> {
    let mut cache: HashMap<Rgb8, u8> = HashMap::new();
    img.pixels()
        .iter()
        .map(|&p| {
            if is_transparent(p) {
                return 0;
            }
            *cache
                .entry(rgb(p))
                .or_insert_with(|| (nearest(pal, to_f(rgb(p))) + offset) as u8)
        })
        .collect()
}

/// Serpentine Floyd–Steinberg. Transparent pixels neither take nor pass on error.
fn map_floyd_steinberg(img: &RgbaImage, pal: &[[f64; 3]], offset: usize) -> Vec<u8> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut out = vec![0u8; w * h];
    // One spare column on each side, so neighbours never need bounds checks.
    let mut cur = vec![[0.0f64; 3]; w + 2];
    let mut next = vec![[0.0f64; 3]; w + 2];
    for y in 0..h {
        let forward = y % 2 == 0;
        for i in 0..w {
            let x = if forward { i } else { w - 1 - i };
            let p = img.pixels()[y * w + x];
            if is_transparent(p) {
                continue;
            }
            let e = cur[x + 1];
            let src = to_f(rgb(p));
            let want = [
                (src[0] + e[0]).clamp(0.0, 255.0),
                (src[1] + e[1]).clamp(0.0, 255.0),
                (src[2] + e[2]).clamp(0.0, 255.0),
            ];
            let k = nearest(pal, want);
            out[y * w + x] = (k + offset) as u8;
            let err = [
                want[0] - pal[k][0],
                want[1] - pal[k][1],
                want[2] - pal[k][2],
            ];
            // Padded positions: ahead = x+1 ± 1, behind = the other side.
            let (ahead, behind) = if forward { (x + 2, x) } else { (x, x + 2) };
            for ch in 0..3 {
                cur[ahead][ch] += err[ch] * 7.0 / 16.0;
                next[behind][ch] += err[ch] * 3.0 / 16.0;
                next[x + 1][ch] += err[ch] * 5.0 / 16.0;
                next[ahead][ch] += err[ch] / 16.0;
            }
        }
        std::mem::swap(&mut cur, &mut next);
        next.iter_mut().for_each(|e| *e = [0.0; 3]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PINK: Rgb8 = Rgb8::new(255, 0, 255);

    fn opts(max_colors: u16, depth: PaletteDepth) -> ReduceOptions {
        ReduceOptions {
            max_colors,
            depth,
            dither: Dither::FloydSteinberg,
            transparent: PINK,
        }
    }

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> Rgba8) -> RgbaImage {
        let mut img = RgbaImage::new(w, h, Rgba8::default());
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, f(x, y));
            }
        }
        img
    }

    fn shown(r: &Reduced, x: u32, y: u32) -> Rgb8 {
        r.palette
            .get(r.image.get(x as i32, y as i32).unwrap())
            .unwrap()
    }

    /// A smooth photo-like picture with thousands of colours.
    fn photo(w: u32, h: u32) -> RgbaImage {
        image(w, h, |x, y| {
            Rgba8::opaque(
                (x * 255 / (w - 1)) as u8,
                (y * 255 / (h - 1)) as u8,
                ((x + y) * 127 / (w + h)) as u8 + 60,
            )
        })
    }

    #[test]
    fn few_colours_are_kept_exactly() {
        let cols = [
            Rgba8::opaque(10, 20, 30),
            Rgba8::opaque(200, 0, 0),
            Rgba8::opaque(0, 200, 0),
        ];
        // Colour 1 is the most used.
        let img = image(6, 4, |x, y| cols[if y < 3 { 1 } else { x as usize % 3 }]);
        let r = reduce(&img, &opts(32, PaletteDepth::Rgb8));
        assert_eq!(r.reduction, Reduction::Exact);
        assert_eq!(r.colors, 3);
        assert_eq!(r.palette.len(), 4, "padded to whole bitplanes");
        assert_eq!(
            r.palette.get(0),
            Some(Rgb8::new(200, 0, 0)),
            "most used first"
        );
        assert_eq!(r.transparent_index, None);
        for y in 0..4 {
            for x in 0..6 {
                assert_eq!(shown(&r, x, y), rgb(img.get(x, y).unwrap()));
            }
        }
    }

    #[test]
    fn transparent_pixels_get_reserved_pink_colour_zero() {
        let img = image(4, 4, |x, _| {
            if x == 0 {
                Rgba8::new(255, 255, 255, 0)
            } else {
                Rgba8::opaque(255, 0, 255)
            }
        });
        let r = reduce(&img, &opts(32, PaletteDepth::Rgb8));
        assert_eq!(r.transparent_index, Some(0));
        assert_eq!(r.palette.get(0), Some(PINK));
        for y in 0..4 {
            assert_eq!(r.image.get(0, y), Some(0));
            // Opaque pink is a colour of its own, never the transparent entry.
            assert_eq!(r.image.get(1, y), Some(1));
        }
        assert_eq!(r.palette.get(1), Some(PINK));
    }

    #[test]
    fn ocs_rounds_few_colours_to_12_bit() {
        let img = image(2, 1, |x, _| {
            if x == 0 {
                Rgba8::opaque(0x12, 0x34, 0x56)
            } else {
                Rgba8::opaque(0x11, 0x33, 0x55)
            }
        });
        let r = reduce(&img, &opts(32, PaletteDepth::Rgb4));
        assert_eq!(r.reduction, Reduction::Rounded);
        // Both round to 0x135 and merge into one entry.
        assert_eq!(r.colors, 1);
        assert_eq!(shown(&r, 0, 0), Rgb8::from_rgb4(0x135));
        assert_eq!(shown(&r, 1, 0), Rgb8::from_rgb4(0x135));
    }

    #[test]
    fn exactly_max_colours_stay_exact_one_more_is_quantized() {
        let img = image(16, 16, |x, y| {
            Rgba8::opaque((x * 16) as u8, (y * 16) as u8, 7)
        });
        let r = reduce(&img, &opts(256, PaletteDepth::Rgb8));
        assert_eq!(r.reduction, Reduction::Exact);
        assert_eq!(r.colors, 256);
        let more = image(17, 16, |x, y| {
            Rgba8::opaque((x * 15) as u8, (y * 16) as u8, 7)
        });
        let r = reduce(&more, &opts(256, PaletteDepth::Rgb8));
        assert_eq!(r.reduction, Reduction::Quantized);
        assert!(r.colors <= 256 && r.palette.len() == 256);
    }

    #[test]
    fn photo_reduction_keeps_the_picture_close() {
        let img = photo(64, 48);
        // (colours, depth, mean block error, worst block error). The worst blocks are the
        // corners: no palette made of cluster means reaches the very ends of a gradient.
        for (max, depth, mean_limit, worst_limit) in [
            (256, PaletteDepth::Rgb8, 1.5, 6.0),
            (32, PaletteDepth::Rgb8, 4.5, 30.0),
            (16, PaletteDepth::Rgb4, 8.0, 40.0),
        ] {
            let r = reduce(&img, &opts(max, depth));
            assert_eq!(r.reduction, Reduction::Quantized);
            assert_eq!(r.palette.len(), usize::from(max));
            // Dithering keeps local averages: compare 4×4 block means.
            let (mut worst, mut total): (f64, f64) = (0.0, 0.0);
            for by in 0..12 {
                for bx in 0..16 {
                    let (mut a, mut b) = ([0.0; 3], [0.0; 3]);
                    for y in by * 4..by * 4 + 4 {
                        for x in bx * 4..bx * 4 + 4 {
                            let s = to_f(rgb(img.get(x, y).unwrap()));
                            let d = to_f(shown(&r, x, y));
                            for ch in 0..3 {
                                a[ch] += s[ch] / 16.0;
                                b[ch] += d[ch] / 16.0;
                            }
                        }
                    }
                    for ch in 0..3 {
                        let e = (a[ch] - b[ch]).abs();
                        worst = worst.max(e);
                        total += e;
                    }
                }
            }
            let mean = total / (12.0 * 16.0 * 3.0);
            assert!(mean < mean_limit, "{max} colours: mean block error {mean}");
            assert!(
                worst < worst_limit,
                "{max} colours: worst block error {worst}"
            );
            if depth == PaletteDepth::Rgb4 {
                for &c in r.palette.colors() {
                    assert_eq!(Rgb8::from_rgb4(c.to_rgb4()), c, "12-bit entry");
                }
            }
        }
    }

    #[test]
    fn quantized_transparency_keeps_colour_zero_for_transparent_pixels_only() {
        let base = photo(40, 30);
        let img = image(40, 30, |x, y| {
            let p = base.get(x, y).unwrap();
            if x < 5 { Rgba8 { a: 0, ..p } } else { p }
        });
        let r = reduce(&img, &opts(16, PaletteDepth::Rgb8));
        assert_eq!(r.reduction, Reduction::Quantized);
        assert_eq!(r.transparent_index, Some(0));
        assert_eq!(r.palette.get(0), Some(PINK));
        for y in 0..30 {
            for x in 0..40 {
                let i = r.image.get(x, y).unwrap();
                assert_eq!(i == 0, x < 5, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn two_clusters_give_two_matching_colours() {
        // Hundreds of slightly different reds and blues, reduced to two colours.
        let img = image(32, 32, |x, y| {
            let n = ((x * 7 + y * 13) % 40) as u8;
            if x < 16 {
                Rgba8::opaque(200 + n / 2, n, n / 2)
            } else {
                Rgba8::opaque(n / 2, n, 200 + n / 2)
            }
        });
        let r = reduce(
            &img,
            &ReduceOptions {
                dither: Dither::None,
                ..opts(2, PaletteDepth::Rgb8)
            },
        );
        assert_eq!(r.reduction, Reduction::Quantized);
        let left = shown(&r, 3, 3);
        let right = shown(&r, 28, 3);
        assert!(left.r > 180 && left.b < 40, "{left}");
        assert!(right.b > 180 && right.r < 40, "{right}");
        // Without dithering each half is one flat colour.
        for y in 0..32 {
            for x in 0..32 {
                assert_eq!(shown(&r, x, y), if x < 16 { left } else { right });
            }
        }
    }

    #[test]
    fn reduction_is_deterministic() {
        let img = photo(50, 20);
        let a = reduce(&img, &opts(32, PaletteDepth::Rgb8));
        let b = reduce(&img, &opts(32, PaletteDepth::Rgb8));
        assert_eq!(a, b);
    }

    #[test]
    fn single_colour_and_fully_transparent_pictures() {
        let one = RgbaImage::new(3, 3, Rgba8::opaque(1, 2, 3));
        let r = reduce(&one, &opts(256, PaletteDepth::Rgb8));
        assert_eq!(r.palette.len(), 2);
        assert_eq!(r.colors, 1);
        assert!(r.image.pixels().iter().all(|&p| p == 0));
        let none = RgbaImage::new(3, 3, Rgba8::default());
        let r = reduce(&none, &opts(256, PaletteDepth::Rgb8));
        assert_eq!(r.transparent_index, Some(0));
        assert_eq!(r.colors, 0);
        assert_eq!(r.palette.len(), 2);
    }
}
