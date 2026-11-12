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
//!
//! The GUI's Import requester (ADR-42) adds an ordered (4×4 Bayer) dither next to
//! Floyd–Steinberg and none, extra colours that count for the palette without being pixels
//! ([`reduce_with`]: the page around an imported picture), [`reduce_to_palette`], which
//! maps a picture onto a fixed palette (the current document's) instead of building one, and
//! the two halves of [`reduce_with`] ([`build_palette`], [`map_built`]), so a dither change
//! does not build the palette again. Nearest colours are found through a grid of candidate
//! entries per RGB cell (`Finder`), which gives exactly the plain search's answers.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dither {
    /// Nearest palette colour only.
    None,
    /// Floyd–Steinberg error diffusion.
    FloydSteinberg,
    /// Ordered dither with the 4×4 Bayer matrix (the pattern of the gradient fills' dither,
    /// ADR-20) between each pixel's two nearest colours, by where it lies between them; a
    /// pixel that is a palette colour keeps it (ADR-42).
    Ordered,
}

/// 4×4 Bayer thresholds, 0..16 (the same matrix as the gradient fills' dither).
const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

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

/// Index into `pal` of the colour nearest to `c` (ties: lowest index): the plain search
/// that [`Finder`] must agree with.
#[cfg(test)]
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

/// Cells per channel of [`Finder`]'s grid, and their width.
const CELLS: usize = 16;
const CELL_W: f64 = 16.0;

/// Below this many searches a [`Finder`] looks at every entry (the grid costs more to make).
const GRID_MIN_SEARCHES: usize = 16_384;

/// The same answers as a plain search over every entry ([`nearest`]: the weighted distance,
/// ties to the lowest index), found faster for many searches: the RGB cube is cut into
/// 16×16×16 cells, and each cell keeps the entries that can be the nearest one (or the
/// nearest one of another colour) for some point in it. For a cell, `m` is the smallest
/// distance within which some entry lies from every point of the cell (its farthest corner);
/// an entry whose distance from the cell's nearest point is above `m` can never be the
/// nearest, so it is left out. Entries that can tie are kept, so the lowest index still wins.
struct Finder<'a> {
    pal: &'a [[f64; 3]],
    grid: Option<Grid>,
}

/// Candidate entries per cell, for [`Finder::nearest`] and [`Finder::nearest_two`].
struct Grid {
    near: Vec<u16>,
    near_at: Vec<u32>,
    two: Vec<u16>,
    two_at: Vec<u32>,
}

/// The cell of a channel value (0..=255; values outside are clamped).
fn cell(v: f64) -> usize {
    ((v / CELL_W) as usize).min(CELLS - 1)
}

impl<'a> Finder<'a> {
    /// A finder for about `searches` searches.
    fn new(pal: &'a [[f64; 3]], searches: usize) -> Self {
        let grid = (searches >= GRID_MIN_SEARCHES && pal.len() > 4).then(|| Grid::new(pal));
        Self { pal, grid }
    }

    /// The entries to look at for `c`, in index order.
    fn candidates(&self, c: [f64; 3], two: bool) -> Candidates<'_> {
        match &self.grid {
            None => Candidates::All(0..self.pal.len()),
            Some(g) => {
                let k = (cell(c[0]) * CELLS + cell(c[1])) * CELLS + cell(c[2]);
                let (list, at) = if two {
                    (&g.two, &g.two_at)
                } else {
                    (&g.near, &g.near_at)
                };
                Candidates::Some(list[at[k] as usize..at[k + 1] as usize].iter())
            }
        }
    }

    /// Index of the entry nearest to `c` (ties: the lowest index); 0 for no entries.
    fn nearest(&self, c: [f64; 3]) -> usize {
        let mut best = (f64::INFINITY, 0usize);
        for i in self.candidates(c, false) {
            let d = dist(self.pal[i], c);
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }

    /// The nearest entry and the nearest entry of another colour (ties: the lowest index),
    /// if the palette has another colour.
    fn nearest_two(&self, c: [f64; 3]) -> (usize, Option<usize>) {
        let pal = self.pal;
        let mut best = (f64::INFINITY, 0usize);
        let mut second: (f64, Option<usize>) = (f64::INFINITY, None);
        // Candidates come in index order, so `<` keeps the lowest index on ties.
        for i in self.candidates(c, true) {
            let d = dist(pal[i], c);
            if d < best.0 {
                if best.0.is_finite() && pal[best.1] != pal[i] {
                    second = (best.0, Some(best.1));
                }
                best = (d, i);
            } else if pal[i] != pal[best.1] && d < second.0 {
                second = (d, Some(i));
            }
        }
        (best.1, second.1)
    }
}

enum Candidates<'g> {
    All(std::ops::Range<usize>),
    Some(std::slice::Iter<'g, u16>),
}

impl Iterator for Candidates<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        match self {
            Candidates::All(r) => r.next(),
            Candidates::Some(it) => it.next().map(|&i| usize::from(i)),
        }
    }
}

impl Grid {
    fn new(pal: &[[f64; 3]]) -> Self {
        let mut grid = Grid {
            near: Vec::new(),
            near_at: vec![0],
            two: Vec::new(),
            two_at: vec![0],
        };
        let mut lows = vec![0.0f64; pal.len()];
        let mut highs = vec![0.0f64; pal.len()];
        // A little room for rounding: never leave out an entry that could tie.
        let within = |d: f64, m: f64| d <= m * (1.0 + 1e-9) + 1e-6;
        for r in 0..CELLS {
            for g in 0..CELLS {
                for b in 0..CELLS {
                    let lo = [r as f64 * CELL_W, g as f64 * CELL_W, b as f64 * CELL_W];
                    let hi = lo.map(|v| (v + CELL_W).min(255.0));
                    for (i, p) in pal.iter().enumerate() {
                        let (mut near, mut far) = (0.0, 0.0);
                        for ch in 0..3 {
                            let below = (lo[ch] - p[ch]).max(0.0);
                            let above = (p[ch] - hi[ch]).max(0.0);
                            let gap = below.max(above);
                            near += WEIGHTS[ch] * gap * gap;
                            let span = (p[ch] - lo[ch]).abs().max((hi[ch] - p[ch]).abs());
                            far += WEIGHTS[ch] * span * span;
                        }
                        lows[i] = near;
                        highs[i] = far;
                    }
                    // Every point of the cell has an entry within `m1`, and one of another
                    // colour than its nearest within `m2`.
                    let a = (0..pal.len())
                        .min_by(|&x, &y| highs[x].total_cmp(&highs[y]))
                        .unwrap_or(0);
                    let m1 = highs[a];
                    let m2 = (0..pal.len())
                        .filter(|&i| pal[i] != pal[a])
                        .map(|i| highs[i])
                        .fold(f64::INFINITY, f64::min);
                    for (i, &low) in lows.iter().enumerate() {
                        if within(low, m1) {
                            grid.near.push(i as u16);
                        }
                        if within(low, m2) {
                            grid.two.push(i as u16);
                        }
                    }
                    grid.near_at.push(grid.near.len() as u32);
                    grid.two_at.push(grid.two.len() as u32);
                }
            }
        }
        grid
    }
}

/// Reduces `img` to an indexed picture as described in the module documentation.
pub fn reduce(img: &RgbaImage, opts: &ReduceOptions) -> Reduced {
    reduce_with(img, opts, &[]).0
}

/// Like [`reduce`], with `extra` colours and pixel counts that are not pixels of `img` but
/// belong to the picture all the same (the page around an imported picture, ADR-42): they
/// count for the histogram and the palette order. When the picture's own colours fit, they
/// are kept exactly and an extra colour gets an entry of its own only while there is room
/// (else it takes the nearest picture colour), so the extras never push a picture with few
/// colours into quantizing. Also returns the palette index of each extra colour (its own
/// entry, else the nearest).
///
/// [`build_palette`] then [`map_built`]: the palette does not depend on the dither.
pub fn reduce_with(
    img: &RgbaImage,
    opts: &ReduceOptions,
    extra: &[(Rgb8, u64)],
) -> (Reduced, Vec<u8>) {
    let built = build_palette(img, opts, extra);
    map_built(img, &built, opts.dither, extra)
}

/// A palette made for a picture by [`build_palette`] (the first half of [`reduce_with`]); it
/// does not depend on the dither, so [`map_built`] can map the same picture with any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// (Reserved colour 0,) the entries, black padding to whole bitplanes.
    pub palette: Palette,
    /// What the reduction had to do.
    pub reduction: Reduction,
    /// The entries pixels may use, in palette order, from index `offset`.
    entries: Vec<Rgb8>,
    offset: usize,
    transparent: bool,
    depth: PaletteDepth,
}

/// The palette [`reduce_with`] makes for `img` and `extra` (`opts.dither` is not used).
pub fn build_palette(img: &RgbaImage, opts: &ReduceOptions, extra: &[(Rgb8, u64)]) -> Built {
    let max = usize::from(opts.max_colors.clamp(2, 256));
    let transparent = img.pixels().iter().any(|&p| is_transparent(p));
    let offset = usize::from(transparent);
    let avail = max - offset;
    let counted: Vec<(Rgb8, u64)> = extra.iter().copied().filter(|&(_, n)| n > 0).collect();

    let (entries, reduction) = match unique_colors(img, avail) {
        Some(counts) => {
            let mut merged: HashMap<Rgb8, u64> = HashMap::new();
            let mut rounded = false;
            for (c, n) in counts {
                let s = opts.depth.snap(c);
                rounded |= s != c;
                *merged.entry(s).or_default() += n;
            }
            // Extra colours join while there is room; the picture's colours come first.
            for &(c, n) in &counted {
                let s = opts.depth.snap(c);
                if let Some(e) = merged.get_mut(&s) {
                    *e += n;
                } else if merged.len() < avail {
                    rounded |= s != c;
                    merged.insert(s, n);
                }
            }
            let r = if rounded {
                Reduction::Rounded
            } else {
                Reduction::Exact
            };
            (merged.into_iter().collect::<Vec<_>>(), r)
        }
        None => (
            palette_for(img, avail, opts.depth, &counted),
            Reduction::Quantized,
        ),
    };
    let entries = order(entries);

    let mut pal: Vec<Rgb8> = Vec::with_capacity(max);
    if transparent {
        pal.push(opts.depth.snap(opts.transparent));
    }
    pal.extend(entries.iter().map(|&(c, _)| c));
    let padded = pal.len().max(2).next_power_of_two();
    pal.resize(padded, Rgb8::BLACK);
    Built {
        palette: Palette::new(pal).expect("2..=256 entries"),
        reduction,
        entries: entries.into_iter().map(|(c, _)| c).collect(),
        offset,
        transparent,
        depth: opts.depth,
    }
}

/// Maps `img` (the picture `built` was made for) onto its palette with `dither` (the second
/// half of [`reduce_with`]); also returns the index of each `extra` colour.
pub fn map_built(
    img: &RgbaImage,
    built: &Built,
    dither: Dither,
    extra: &[(Rgb8, u64)],
) -> (Reduced, Vec<u8>) {
    let offset = built.offset;
    let real: Vec<[f64; 3]> = built.entries.iter().map(|&c| to_f(c)).collect();
    let ids: Vec<u8> = (0..real.len()).map(|i| (i + offset) as u8).collect();
    // For the extra colours only.
    let finder = Finder::new(&real, extra.len());
    let (pixels, extra_ids) = match built.reduction {
        Reduction::Exact | Reduction::Rounded => {
            let depth = built.depth;
            let index: HashMap<Rgb8, u8> = built
                .entries
                .iter()
                .enumerate()
                .map(|(i, &c)| (c, (i + offset) as u8))
                .collect();
            // Runs of one colour (enlarged pixel art) look it up once.
            let mut last: Option<(Rgba8, u8)> = None;
            let pixels = img
                .pixels()
                .iter()
                .map(|&p| {
                    if is_transparent(p) {
                        return 0;
                    }
                    match last {
                        Some((q, i)) if q == p => i,
                        _ => {
                            let i = index[&depth.snap(rgb(p))];
                            last = Some((p, i));
                            i
                        }
                    }
                })
                .collect();
            let lookup = |c: Rgb8| {
                index
                    .get(&depth.snap(c))
                    .copied()
                    .unwrap_or_else(|| ids.get(finder.nearest(to_f(c))).copied().unwrap_or(0))
            };
            (pixels, extra.iter().map(|&(c, _)| lookup(c)).collect())
        }
        Reduction::Quantized => {
            let pixels = map_pixels(img, &real, &ids, dither);
            let extra_ids = extra
                .iter()
                .map(|&(c, _)| ids[finder.nearest(to_f(c))])
                .collect();
            (pixels, extra_ids)
        }
    };
    let reduced = Reduced {
        image: IndexedImage::from_pixels(img.width(), img.height(), pixels)
            .expect("one index per pixel"),
        palette: built.palette.clone(),
        transparent_index: built.transparent.then_some(0),
        reduction: built.reduction,
        colors: built.entries.len(),
    };
    (reduced, extra_ids)
}

/// A picture mapped onto a fixed palette ([`reduce_to_palette`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapped {
    pub image: IndexedImage,
    /// The picture had transparent pixels: they are colour 0, which opaque pixels avoid.
    pub transparent: bool,
    /// Every opaque pixel was a palette colour already and keeps it exactly (no dithering).
    pub exact: bool,
}

/// Maps `img` onto `palette` as it is (the Import requester's "Current" palette, ADR-42):
/// every opaque pixel becomes the nearest palette colour by the weighted distance above
/// (ties: the lowest index, so of duplicate entries the first is used), with `dither`. A
/// picture made only of palette colours is mapped exactly, without dithering (as [`reduce`]
/// keeps few colours exactly). Transparent pixels become colour 0; while the picture has
/// any, opaque pixels do not use colour 0 (it marks transparency, as in [`reduce`]), unless
/// the palette has no other colour.
pub fn reduce_to_palette(img: &RgbaImage, palette: &Palette, dither: Dither) -> Mapped {
    let transparent = img.pixels().iter().any(|&p| is_transparent(p));
    let first = usize::from(transparent && palette.len() > 1);
    let ids: Vec<u8> = (first..palette.len()).map(|i| i as u8).collect();
    let colours: Vec<Rgb8> = ids
        .iter()
        .map(|&i| palette.get(i).unwrap_or(Rgb8::BLACK))
        .collect();
    // Exact colours: the lowest index of each.
    let mut exact_index: HashMap<Rgb8, u8> = HashMap::new();
    for (&i, &c) in ids.iter().zip(&colours) {
        exact_index.entry(c).or_insert(i);
    }
    let exact = img
        .pixels()
        .iter()
        .all(|&p| is_transparent(p) || exact_index.contains_key(&rgb(p)));
    let pixels = if exact {
        img.pixels()
            .iter()
            .map(|&p| {
                if is_transparent(p) {
                    0
                } else {
                    exact_index[&rgb(p)]
                }
            })
            .collect()
    } else {
        let pal: Vec<[f64; 3]> = colours.iter().map(|&c| to_f(c)).collect();
        map_pixels(img, &pal, &ids, dither)
    };
    Mapped {
        image: IndexedImage::from_pixels(img.width(), img.height(), pixels)
            .expect("one index per pixel"),
        transparent,
        exact,
    }
}

/// Maps the opaque pixels to `ids[nearest entry of pal]` with `dither`; transparent pixels
/// become 0.
fn map_pixels(img: &RgbaImage, pal: &[[f64; 3]], ids: &[u8], dither: Dither) -> Vec<u8> {
    match dither {
        Dither::None => map_nearest(img, pal, ids),
        Dither::FloydSteinberg => map_floyd_steinberg(img, pal, ids),
        Dither::Ordered => map_ordered(img, pal, ids),
    }
}

/// The opaque colours with their pixel counts, or `None` if there are more than `limit`.
fn unique_colors(img: &RgbaImage, limit: usize) -> Option<Vec<(Rgb8, u64)>> {
    let mut counts: HashMap<Rgb8, u64> = HashMap::new();
    // Runs of one colour (enlarged pixel art, flat areas) are counted together.
    let mut run: Option<(Rgb8, u64)> = None;
    for &p in img.pixels() {
        if is_transparent(p) {
            continue;
        }
        let c = rgb(p);
        match &mut run {
            Some((r, n)) if *r == c => *n += 1,
            _ => {
                if let Some((r, n)) = run.replace((c, 1)) {
                    *counts.entry(r).or_default() += n;
                }
                if !counts.contains_key(&c) && counts.len() >= limit {
                    return None;
                }
            }
        }
    }
    if let Some((r, n)) = run {
        *counts.entry(r).or_default() += n;
    }
    (counts.len() <= limit).then(|| counts.into_iter().collect())
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

/// A new palette of at most `n` colours (with pixel weights) for the opaque pixels and the
/// `extra` colours.
fn palette_for(
    img: &RgbaImage,
    n: usize,
    depth: PaletteDepth,
    extra: &[(Rgb8, u64)],
) -> Vec<(Rgb8, u64)> {
    // 6-bit-per-channel histogram; each bin keeps the exact mean of its pixels. A table of
    // all 2^18 bins, read in key order (the order the items need).
    let mut bins: Vec<(u64, [u64; 3])> = vec![(0, [0; 3]); 1 << 18];
    let mut add = |c: Rgb8, count: u64| {
        let key =
            (usize::from(c.r >> 2) << 12) | (usize::from(c.g >> 2) << 6) | usize::from(c.b >> 2);
        let bin = &mut bins[key];
        bin.0 += count;
        bin.1[0] += count * u64::from(c.r);
        bin.1[1] += count * u64::from(c.g);
        bin.1[2] += count * u64::from(c.b);
    };
    for &(c, count) in extra {
        add(c, count);
    }
    for &p in img.pixels() {
        if !is_transparent(p) {
            add(rgb(p), 1);
        }
    }
    let mut items: Vec<Item> = bins
        .iter()
        .filter(|(count, _)| *count > 0)
        .map(|&(count, sum)| {
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
        let finder = Finder::new(&centres, items.len());
        for it in items {
            acc[finder.nearest(it.c)].add(it);
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

fn map_nearest(img: &RgbaImage, pal: &[[f64; 3]], ids: &[u8]) -> Vec<u8> {
    let finder = Finder::new(pal, img.pixels().len());
    // Runs of one colour (flat areas, enlarged pixel art) are looked up once.
    let mut last: Option<(Rgba8, u8)> = None;
    img.pixels()
        .iter()
        .map(|&p| {
            if is_transparent(p) {
                return 0;
            }
            if let Some((q, i)) = last
                && q == p
            {
                return i;
            }
            let i = ids[finder.nearest(to_f(rgb(p)))];
            last = Some((p, i));
            i
        })
        .collect()
}

/// The two colours the ordered dither chooses from for `c`, and where `c` lies between them:
/// `(first, second, f)` with the nearest colour and the nearest colour different from it in
/// index order (so the pattern does not change where the nearest colour changes), and `f` the
/// position of `c` projected onto the line from the first to the second (weighted, 0 at the
/// first, 1 at the second, clamped). A palette colour itself is always `f` = 0 or 1.
fn ordered_pair(finder: &Finder, ids: &[u8], c: [f64; 3]) -> (u8, u8, f64) {
    let pal = finder.pal;
    let (k, other) = finder.nearest_two(c);
    let Some(j) = other else {
        let id = ids.get(k).copied().unwrap_or(0);
        return (id, id, 0.0);
    };
    let (a, b) = if k < j { (k, j) } else { (j, k) };
    let (pa, pb) = (pal[a], pal[b]);
    let along: f64 = (0..3)
        .map(|ch| WEIGHTS[ch] * (pb[ch] - pa[ch]) * (c[ch] - pa[ch]))
        .sum();
    let length: f64 = (0..3)
        .map(|ch| WEIGHTS[ch] * (pb[ch] - pa[ch]) * (pb[ch] - pa[ch]))
        .sum();
    let f = if length > 0.0 {
        (along / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (ids[a], ids[b], f)
}

/// Ordered dither with the 4×4 Bayer matrix between two palette colours ([`ordered_pair`]):
/// a pixel at `f` between its two colours takes the second where `f` is above the cell's
/// threshold `(t + ½) / 16`, so `f` of the 16 cells in every 4×4 block show the second
/// colour. A pixel that is a palette colour never changes (`f` is 0 or 1), the mix follows any
/// direction between two colours (hue as well as brightness), and the pattern depends on the
/// pixel position only (deterministic; a flat area between two colours gets a regular
/// texture). Transparent pixels become 0.
fn map_ordered(img: &RgbaImage, pal: &[[f64; 3]], ids: &[u8]) -> Vec<u8> {
    let finder = Finder::new(pal, img.pixels().len());
    let w = (img.width() as usize).max(1);
    let mut last: Option<(Rgba8, (u8, u8, f64))> = None;
    img.pixels()
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            if is_transparent(p) {
                return 0;
            }
            let pair = match last {
                Some((q, pair)) if q == p => pair,
                _ => {
                    let pair = ordered_pair(&finder, ids, to_f(rgb(p)));
                    last = Some((p, pair));
                    pair
                }
            };
            let (x, y) = (i % w, i / w);
            let threshold = (f64::from(BAYER4[y % 4][x % 4]) + 0.5) / 16.0;
            if pair.2 > threshold { pair.1 } else { pair.0 }
        })
        .collect()
}

/// Serpentine Floyd–Steinberg. Transparent pixels neither take nor pass on error.
fn map_floyd_steinberg(img: &RgbaImage, pal: &[[f64; 3]], ids: &[u8]) -> Vec<u8> {
    let finder = Finder::new(pal, img.pixels().len());
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
            let k = finder.nearest(want);
            out[y * w + x] = ids[k];
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

    /// A small deterministic pseudo-random sequence for the search tests.
    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *seed >> 33
    }

    #[test]
    fn the_grid_search_gives_the_plain_searchs_answers() {
        let mut seed = 7;
        for size in [1usize, 2, 3, 16, 64, 256] {
            let mut pal: Vec<[f64; 3]> = (0..size)
                .map(|_| {
                    let mut v = || (lcg(&mut seed) % 256) as f64;
                    [v(), v(), v()]
                })
                .collect();
            // Duplicates and colours on one grey line (ties along the axis).
            if size >= 16 {
                pal[5] = pal[2];
                pal[9] = pal[2];
                pal[10] = [10.0, 10.0, 10.0];
                pal[11] = [30.0, 30.0, 30.0];
            }
            let linear = Finder::new(&pal, 0);
            let grid = Finder::new(&pal, usize::MAX);
            assert_eq!(grid.grid.is_some(), size > 4);
            for q in 0..4000 {
                let c = if q % 3 == 0 {
                    pal[q % size]
                } else if q % 3 == 1 && q % 2 == 0 {
                    // On cell edges and the cube's faces.
                    let mut v = || ((lcg(&mut seed) % 17) as f64 * 16.0).min(255.0);
                    [v(), v(), v()]
                } else {
                    let mut v = || (lcg(&mut seed) % 2560) as f64 / 10.0;
                    [v(), v(), v()]
                };
                for finder in [&linear, &grid] {
                    assert_eq!(finder.nearest(c), nearest(&pal, c), "{size} {c:?}");
                }
                // Brute force: the nearest entry of another colour, lowest index on ties.
                let k = nearest(&pal, c);
                let mut want: Option<(f64, usize)> = None;
                for (i, &p) in pal.iter().enumerate() {
                    let d = dist(p, c);
                    if p != pal[k] && want.is_none_or(|(b, _)| d < b) {
                        want = Some((d, i));
                    }
                }
                for finder in [&linear, &grid] {
                    assert_eq!(
                        finder.nearest_two(c),
                        (k, want.map(|w| w.1)),
                        "{size} {c:?}"
                    );
                }
            }
        }
        assert_eq!(Finder::new(&[], 0).nearest([1.0, 2.0, 3.0]), 0);
        assert_eq!(Finder::new(&[], usize::MAX).nearest([1.0, 2.0, 3.0]), 0);
    }

    #[test]
    fn extra_colours_never_push_a_picture_into_quantizing() {
        // Exactly 8 colours and an extra colour that is not one of them.
        let cols: Vec<Rgba8> = (0..8u8)
            .map(|i| Rgba8::opaque(30 * i + 10, 200 - 20 * i, 17 * i))
            .collect();
        let img = image(8, 2, |x, _| cols[x as usize]);
        let grey = Rgb8::new(128, 128, 128);
        let extra = [(grey, 1000)];
        let (r, ids) = reduce_with(&img, &opts(8, PaletteDepth::Rgb8), &extra);
        assert_eq!(r.reduction, Reduction::Exact);
        assert_eq!(r.palette.len(), 8);
        for x in 0..8 {
            assert_eq!(shown(&r, x, 1), rgb(cols[x as usize]), "{x}");
        }
        // The extra takes the nearest picture colour.
        let real: Vec<[f64; 3]> = r.palette.colors().iter().map(|&c| to_f(c)).collect();
        assert_eq!(usize::from(ids[0]), nearest(&real, to_f(grey)));
        // With room the extra gets an entry of its own.
        let (r, ids) = reduce_with(&img, &opts(16, PaletteDepth::Rgb8), &extra);
        assert_eq!(r.reduction, Reduction::Exact);
        assert_eq!(r.palette.get(ids[0]), Some(grey));
        assert_eq!(r.palette.get(0), Some(grey), "the most used colour first");
        // One colour too many for the picture alone: quantized, the extra in the histogram.
        let (r, _) = reduce_with(&img, &opts(4, PaletteDepth::Rgb8), &extra);
        assert_eq!(r.reduction, Reduction::Quantized);
        // Splitting into palette and mapping gives the same as one call.
        let o = ReduceOptions {
            dither: Dither::Ordered,
            ..opts(4, PaletteDepth::Rgb8)
        };
        let built = build_palette(&img, &o, &extra);
        assert_eq!(
            map_built(&img, &built, o.dither, &extra),
            reduce_with(&img, &o, &extra)
        );
    }

    #[test]
    fn ordered_dither_keeps_flat_palette_colours_and_mixes_any_two() {
        // Pure black left, pure white right, a colour ramp between them.
        let img = image(48, 8, |x, _| match x {
            0..16 => Rgba8::opaque(0, 0, 0),
            32.. => Rgba8::opaque(255, 255, 255),
            _ => {
                let v = ((x - 16) * 16) as u8;
                Rgba8::opaque(v, (f64::from(v) * 0.6) as u8, 255 - v)
            }
        });
        let pal = Palette::dpaint_default_32();
        let m = reduce_to_palette(&img, &pal, Dither::Ordered);
        assert!(!m.exact);
        let (black, white) = (pal.nearest(Rgb8::BLACK), pal.nearest(Rgb8::WHITE));
        assert_eq!(pal.get(black), Some(Rgb8::BLACK));
        assert_eq!(pal.get(white), Some(Rgb8::WHITE));
        for y in 0..8 {
            for x in 0..16 {
                assert_eq!(m.image.get(x, y), Some(black), "black {x},{y}");
                assert_eq!(m.image.get(x + 32, y), Some(white), "white {x},{y}");
            }
        }
        // A new palette: pure white beside a grey ramp keeps its white entry exactly.
        let img = image(64, 8, |x, _| {
            if x < 32 {
                Rgba8::opaque(255, 255, 255)
            } else {
                let v = ((x - 32) * 7) as u8;
                Rgba8::opaque(v, v, v)
            }
        });
        for n in [4, 16, 32] {
            let r = reduce(
                &img,
                &ReduceOptions {
                    dither: Dither::Ordered,
                    ..opts(n, PaletteDepth::Rgb8)
                },
            );
            assert_eq!(r.reduction, Reduction::Quantized, "{n}");
            assert!(r.palette.colors().contains(&Rgb8::WHITE), "{n}");
            for y in 0..8 {
                for x in 0..32 {
                    assert_eq!(shown(&r, x, y), Rgb8::WHITE, "{n} colours, {x},{y}");
                }
            }
        }
        // Halfway between blue and orange (a hue change, not only brightness): half of every
        // 4×4 cell each.
        let two = Palette::new(vec![Rgb8::new(0, 0, 255), Rgb8::new(255, 128, 0)]).unwrap();
        let mid = RgbaImage::new(8, 8, Rgba8::opaque(128, 64, 128));
        let m = reduce_to_palette(&mid, &two, Dither::Ordered);
        for (cx, cy) in [(0, 0), (4, 4)] {
            let orange = (0..4)
                .flat_map(|y| (0..4).map(move |x| (x, y)))
                .filter(|&(x, y)| m.image.get(cx + x, cy + y) == Some(1))
                .count();
            assert_eq!(orange, 8, "cell {cx},{cy}");
        }
        // A quarter of the way: 4 of 16.
        let quarter = RgbaImage::new(4, 4, Rgba8::opaque(64, 32, 191));
        let m = reduce_to_palette(&quarter, &two, Dither::Ordered);
        assert_eq!(m.image.pixels().iter().filter(|&&p| p == 1).count(), 4);
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
