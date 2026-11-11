//! Picture ▸ Load of PNG and JPEG pictures in the GUI (ADR-42): the picture is fitted into the
//! current document's page (keeping its proportions on the screen's pixel shape, enlarged or
//! reduced), placed in the centre or the top-left corner on the BG colour, and reduced to
//! indexed colours with the Import requester's options: a new palette of 2–256 colours
//! (ADR-9's median cut, k-means and depth rounding) or the current palette as it is, with no
//! dither, Floyd–Steinberg or a 4×4 ordered dither.
//!
//! The fitting is the slow part for big photos, so it is separate ([`fit`]); the requester
//! keeps the fitted picture ([`Fitted`]) and only reduces it again when an option changes,
//! reusing the palettes made so far (a dither change only maps again) and the indexed
//! picture (a position change only places it again).
//! [`thumbnail`] makes the requester's preview from the very page Ok will give.

use crate::image::IndexedImage;
use crate::palette::{Palette, Rgb8};
use crate::quantize::{self, Dither, PaletteDepth, ReduceOptions, Reduction};
use crate::resample;
use crate::rgba::{Rgba8, RgbaImage};

/// Where the fitted picture lies on the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    /// Centred (the default); odd margins put the extra pixel right and below.
    #[default]
    Centre,
    /// In the top-left corner.
    TopLeft,
}

/// Where the palette comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaletteChoice {
    /// A new palette built from the picture (ADR-9).
    #[default]
    New,
    /// The current document's palette, kept exactly.
    Current,
}

/// The colour counts the requester offers (whole bitplanes).
pub const COLOR_COUNTS: [u16; 8] = [2, 4, 8, 16, 32, 64, 128, 256];

/// `n` as one of [`COLOR_COUNTS`] no larger than `max`: the largest power of two not above
/// either, at least 2.
pub fn color_count(n: u16, max: u16) -> u16 {
    let limit = n.min(max);
    COLOR_COUNTS
        .iter()
        .copied()
        .rev()
        .find(|&c| c <= limit)
        .unwrap_or(2)
}

/// The Import requester's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportOptions {
    pub position: Position,
    pub palette: PaletteChoice,
    /// Palette entries with [`PaletteChoice::New`] (2–256, whole bitplanes).
    pub colors: u16,
    pub dither: Dither,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            position: Position::Centre,
            palette: PaletteChoice::New,
            colors: 32,
            dither: Dither::FloydSteinberg,
        }
    }
}

/// The document the picture goes into.
#[derive(Clone, Copy, Debug)]
pub struct ImportTarget<'a> {
    /// The current page size (the screen, or larger after Picture ▸ Page Size).
    pub page: (u32, u32),
    /// The screen's pixel shape ([`crate::DisplayMode::pixel_shape`]).
    pub pixel_shape: (u32, u32),
    /// Precision of a new palette (OCS 12-bit, AGA 24-bit).
    pub depth: PaletteDepth,
    /// The current palette (kept with [`PaletteChoice::Current`]).
    pub palette: &'a Palette,
    /// The BG colour (an index into `palette`): the page around the picture.
    pub bg: u8,
    /// Colour 0 of a new palette when the picture has transparent pixels (ADR-9: pink).
    pub transparent: Rgb8,
}

/// The page an import gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    pub page: IndexedImage,
    pub palette: Palette,
    /// Colour 0 when the picture had transparent pixels.
    pub transparent_color: Option<u8>,
    /// What the colour reduction had to do.
    pub reduction: Reduction,
    /// The picture's top-left corner on the page and its size there.
    pub at: (u32, u32),
    pub size: (u32, u32),
}

/// Size of a `w`×`h` picture (square pixels) fitted into `page` with the screen's
/// `pixel_shape`: as large as fits, enlarged or reduced, proportions kept.
pub fn fitted_size(w: u32, h: u32, page: (u32, u32), pixel_shape: (u32, u32)) -> (u32, u32) {
    resample::fit_size(w, h, page.0, page.1, pixel_shape)
}

/// The picture scaled to [`fitted_size`]: enlarged axes take the nearest pixel (pixel art
/// stays crisp, no new colours), reduced ones average areas ([`resample::scale_fit`]).
pub fn fit(img: &RgbaImage, page: (u32, u32), pixel_shape: (u32, u32)) -> RgbaImage {
    let (w, h) = fitted_size(img.width(), img.height(), page, pixel_shape);
    resample::scale_fit(img, w, h)
}

/// The top-left corner of a picture of `size` on `page`.
pub fn place(size: (u32, u32), page: (u32, u32), position: Position) -> (u32, u32) {
    match position {
        Position::TopLeft => (0, 0),
        Position::Centre => (
            page.0.saturating_sub(size.0) / 2,
            page.1.saturating_sub(size.1) / 2,
        ),
    }
}

/// Reduces a picture already fitted to the page ([`fit`]) and places it on the page.
///
/// - The page around the picture is the BG colour. With a new palette its colour counts for
///   the palette like that many picture pixels; it gets its own entry while there is room,
///   else the nearest one: a picture whose own colours fit is always kept exactly
///   ([`quantize::reduce_with`]). With the current palette it is the BG colour's index
///   exactly. Either way it takes no part in dithering, so the edge of the page stays clean.
/// - New palette: ADR-9's reduction with `colors` entries (exact when the picture has no more
///   colours, rounded to the chipset's depth), padded with black to exactly `colors` entries;
///   transparent pixels become colour 0, reserved and pink.
/// - Current palette: kept exactly ([`quantize::reduce_to_palette`]); transparent pixels
///   become colour 0, which opaque pixels then avoid.
///
/// [`Fitted::reduce`] gives the same and reuses work between calls.
pub fn reduce_fitted(fitted: &RgbaImage, opts: &ImportOptions, t: &ImportTarget) -> Imported {
    let mapped = match opts.palette {
        PaletteChoice::New => {
            let ro = new_options(opts, t);
            let extra = margin_extra(fitted, t);
            map_new(
                fitted,
                &quantize::build_palette(fitted, &ro, &extra),
                opts,
                &extra,
            )
        }
        PaletteChoice::Current => map_current(fitted, opts, t),
    };
    mapped.place(t.page, opts.position)
}

/// The reduction's options for a new palette.
fn new_options(opts: &ImportOptions, t: &ImportTarget) -> ReduceOptions {
    ReduceOptions {
        max_colors: color_count(opts.colors, 256),
        depth: t.depth,
        dither: opts.dither,
        transparent: t.transparent,
    }
}

/// The BG colour and the number of page pixels the fitted picture leaves to it (the same for
/// both positions).
fn margin_extra(fitted: &RgbaImage, t: &ImportTarget) -> [(Rgb8, u64); 1] {
    let (pw, ph) = t.page;
    let size = (fitted.width().min(pw), fitted.height().min(ph));
    let margin = u64::from(pw) * u64::from(ph) - u64::from(size.0) * u64::from(size.1);
    let bg_rgb = t.palette.get(bg_index(t)).unwrap_or(Rgb8::BLACK);
    [(bg_rgb, margin)]
}

fn bg_index(t: &ImportTarget) -> u8 {
    t.bg.min((t.palette.len() - 1) as u8)
}

/// The fitted picture in indexed colours, before it is placed on the page.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mapped {
    pixels: IndexedImage,
    palette: Palette,
    /// The page around the picture.
    fill: u8,
    transparent: Option<u8>,
    reduction: Reduction,
}

impl Mapped {
    fn place(&self, page: (u32, u32), position: Position) -> Imported {
        let (pw, ph) = page;
        let size = (self.pixels.width().min(pw), self.pixels.height().min(ph));
        let at = place(size, page, position);
        let mut out = IndexedImage::new(pw, ph, self.fill);
        out.blit(&self.pixels, at.0 as i32, at.1 as i32);
        Imported {
            page: out,
            palette: self.palette.clone(),
            transparent_color: self.transparent,
            reduction: self.reduction,
            at,
            size,
        }
    }
}

fn map_new(
    fitted: &RgbaImage,
    built: &quantize::Built,
    opts: &ImportOptions,
    extra: &[(Rgb8, u64)],
) -> Mapped {
    let colors = color_count(opts.colors, 256);
    let (r, ids) = quantize::map_built(fitted, built, opts.dither, extra);
    let mut entries = r.palette.colors().to_vec();
    entries.resize(usize::from(colors).max(entries.len()), Rgb8::BLACK);
    Mapped {
        pixels: r.image,
        palette: Palette::new(entries).expect("2..=256 entries"),
        fill: ids[0],
        transparent: r.transparent_index,
        reduction: r.reduction,
    }
}

fn map_current(fitted: &RgbaImage, opts: &ImportOptions, t: &ImportTarget) -> Mapped {
    let m = quantize::reduce_to_palette(fitted, t.palette, opts.dither);
    Mapped {
        pixels: m.image,
        palette: t.palette.clone(),
        fill: bg_index(t),
        transparent: m.transparent.then_some(0),
        reduction: if m.exact {
            Reduction::Exact
        } else {
            Reduction::Quantized
        },
    }
}

/// What a result depends on apart from the position (which only moves the picture).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    palette: PaletteChoice,
    /// With a new palette only.
    colors: u16,
    dither: Dither,
    page: (u32, u32),
    depth: PaletteDepth,
    current: Palette,
    bg: u8,
    transparent: Rgb8,
}

impl Key {
    fn new(opts: &ImportOptions, t: &ImportTarget) -> Self {
        Self {
            palette: opts.palette,
            colors: match opts.palette {
                PaletteChoice::New => color_count(opts.colors, 256),
                PaletteChoice::Current => 0,
            },
            dither: opts.dither,
            page: t.page,
            depth: t.depth,
            current: t.palette.clone(),
            bg: t.bg,
            transparent: t.transparent,
        }
    }
}

/// A picture fitted into the page ([`fit`], the slow part for big photos, done once) and
/// what the Import requester's option changes can reuse: every new palette made so far (it
/// depends on the colour count, not on the dither or the position) and the last indexed
/// picture (a position change only places it again). [`Fitted::reduce`] gives exactly
/// [`reduce_fitted`]'s result.
#[derive(Clone, Debug)]
pub struct Fitted {
    image: RgbaImage,
    palettes: Vec<(Key, quantize::Built)>,
    last: Option<(Key, Mapped)>,
    /// How often a picture was mapped onto a palette (for tests and timings).
    mappings: usize,
}

impl Fitted {
    /// `img` fitted into `page` ([`fit`]).
    pub fn new(img: &RgbaImage, page: (u32, u32), pixel_shape: (u32, u32)) -> Self {
        Self::of(fit(img, page, pixel_shape))
    }

    /// A picture that is fitted already.
    pub fn of(image: RgbaImage) -> Self {
        Self {
            image,
            palettes: Vec::new(),
            last: None,
            mappings: 0,
        }
    }

    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    /// How many new palettes were built so far.
    pub fn palettes_made(&self) -> usize {
        self.palettes.len()
    }

    /// How many times the picture was mapped onto a palette so far (a position change
    /// does not map it again).
    pub fn mappings_made(&self) -> usize {
        self.mappings
    }

    /// [`reduce_fitted`] of the fitted picture, reusing earlier work.
    pub fn reduce(&mut self, opts: &ImportOptions, t: &ImportTarget) -> Imported {
        let key = Key::new(opts, t);
        if let Some((k, m)) = &self.last
            && *k == key
        {
            return m.place(t.page, opts.position);
        }
        let mapped = match opts.palette {
            PaletteChoice::New => {
                let palette_key = Key {
                    dither: Dither::None,
                    ..key.clone()
                };
                let extra = margin_extra(&self.image, t);
                let i = match self.palettes.iter().position(|(k, _)| *k == palette_key) {
                    Some(i) => i,
                    None => {
                        let built =
                            quantize::build_palette(&self.image, &new_options(opts, t), &extra);
                        self.palettes.push((palette_key, built));
                        self.palettes.len() - 1
                    }
                };
                map_new(&self.image, &self.palettes[i].1, opts, &extra)
            }
            PaletteChoice::Current => map_current(&self.image, opts, t),
        };
        let result = mapped.place(t.page, opts.position);
        self.last = Some((key, mapped));
        self.mappings += 1;
        result
    }
}

/// [`fit`] then [`reduce_fitted`]: the whole import in one call.
pub fn import(img: &RgbaImage, opts: &ImportOptions, t: &ImportTarget) -> Imported {
    reduce_fitted(&fit(img, t.page, t.pixel_shape), opts, t)
}

/// An indexed picture in its palette's colours (opaque).
pub fn rgba_of(image: &IndexedImage, palette: &Palette) -> RgbaImage {
    let pixels = image
        .pixels()
        .iter()
        .map(|&i| {
            let c = palette.get(i).unwrap_or(Rgb8::BLACK);
            Rgba8::opaque(c.r, c.g, c.b)
        })
        .collect();
    RgbaImage::from_pixels(image.width(), image.height(), pixels).expect("one pixel each")
}

/// The requester's preview: `page` in `palette`'s colours, reduced to `width`×`height` by
/// area averaging, so it shows exactly what Ok gives, in true colour.
pub fn thumbnail(page: &IndexedImage, palette: &Palette, width: u32, height: u32) -> RgbaImage {
    resample::scale_area(&rgba_of(page, palette), width.max(1), height.max(1))
}

/// The number of different opaque colours in `img`, or `None` above `limit`.
pub fn count_colors(img: &RgbaImage, limit: usize) -> Option<usize> {
    let mut seen = std::collections::HashSet::new();
    for &p in img.pixels() {
        if p.a >= quantize::ALPHA_THRESHOLD && seen.insert((p.r, p.g, p.b)) && seen.len() > limit {
            return None;
        }
    }
    Some(seen.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PINK: Rgb8 = Rgb8::new(255, 0, 255);

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> Rgba8) -> RgbaImage {
        let mut img = RgbaImage::new(w, h, Rgba8::default());
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, f(x, y));
            }
        }
        img
    }

    fn photo(w: u32, h: u32) -> RgbaImage {
        image(w, h, |x, y| {
            Rgba8::opaque(
                (x * 255 / (w - 1)) as u8,
                (y * 255 / (h - 1)) as u8,
                ((x * 3 + y * 5) % 256) as u8,
            )
        })
    }

    fn target(page: (u32, u32), palette: &Palette) -> ImportTarget<'_> {
        ImportTarget {
            page,
            pixel_shape: (1, 1),
            depth: PaletteDepth::Rgb8,
            palette,
            bg: 0,
            transparent: PINK,
        }
    }

    fn opts(palette: PaletteChoice, colors: u16, dither: Dither) -> ImportOptions {
        ImportOptions {
            position: Position::Centre,
            palette,
            colors,
            dither,
        }
    }

    fn shown(r: &Imported, x: u32, y: u32) -> Rgb8 {
        r.palette
            .get(r.page.get(x as i32, y as i32).unwrap())
            .unwrap()
    }

    #[test]
    fn colour_counts_are_whole_bitplanes_up_to_the_maximum() {
        assert_eq!(color_count(32, 256), 32);
        assert_eq!(color_count(33, 256), 32);
        assert_eq!(color_count(256, 32), 32);
        assert_eq!(color_count(64, 16), 16);
        assert_eq!(color_count(1, 256), 2);
        assert_eq!(color_count(0, 0), 2);
    }

    #[test]
    fn fitted_sizes_for_both_pixel_shapes_enlarging_and_reducing() {
        // Reduce on square pixels (LoRes) and on HiRes pixels (1×2: half as wide as high).
        assert_eq!(fitted_size(4032, 3024, (320, 256), (1, 1)), (320, 240));
        assert_eq!(fitted_size(4032, 3024, (640, 256), (1, 2)), (640, 240));
        // Enlarge: 40×30 pixel art fills 320×240 of a LoRes page, 640×240 of a HiRes one.
        assert_eq!(fitted_size(40, 30, (320, 256), (1, 1)), (320, 240));
        assert_eq!(fitted_size(40, 30, (640, 256), (1, 2)), (640, 240));
        // Tall pictures fit the height; LoRes laced pixels are twice as wide as high.
        assert_eq!(fitted_size(100, 400, (320, 256), (1, 1)), (64, 256));
        assert_eq!(fitted_size(100, 400, (320, 512), (2, 1)), (64, 512));
        // A page larger than the screen (Page Size) is filled as a whole.
        assert_eq!(fitted_size(1000, 1000, (320, 640), (1, 1)), (320, 320));
    }

    #[test]
    fn both_positions_and_the_margins_are_the_bg_colour() {
        let pal = Palette::new(vec![Rgb8::new(10, 20, 30), Rgb8::new(250, 250, 250)]).unwrap();
        let red = image(30, 10, |_, _| Rgba8::opaque(200, 0, 0));
        for (position, at) in [(Position::Centre, (5, 0)), (Position::TopLeft, (0, 0))] {
            for choice in [PaletteChoice::New, PaletteChoice::Current] {
                let t = ImportTarget {
                    bg: 1,
                    ..target((40, 10), &pal)
                };
                let o = ImportOptions {
                    position,
                    ..opts(choice, 4, Dither::FloydSteinberg)
                };
                let r = import(&red, &o, &t);
                assert_eq!((r.at, r.size), (at, (30, 10)), "{position:?} {choice:?}");
                for y in 0..10 {
                    for x in 0..40 {
                        let inside = (at.0..at.0 + 30).contains(&x);
                        let want = if inside {
                            match choice {
                                PaletteChoice::New => Rgb8::new(200, 0, 0),
                                // Red is nearer to the dark entry by the weighted distance.
                                PaletteChoice::Current => Rgb8::new(10, 20, 30),
                            }
                        } else {
                            Rgb8::new(250, 250, 250)
                        };
                        assert_eq!(shown(&r, x, y), want, "{position:?} {choice:?} {x},{y}");
                    }
                }
                if choice == PaletteChoice::Current {
                    assert_eq!(r.palette, pal, "the current palette is kept exactly");
                    assert!(
                        (0..40).all(|x| (at.0..at.0 + 30).contains(&x)
                            || r.page.get(x as i32, 0) == Some(1)),
                        "margins are the BG index"
                    );
                } else {
                    assert_eq!(r.palette.len(), 4, "exactly the chosen count");
                    assert_eq!(r.reduction, Reduction::Exact);
                }
            }
        }
    }

    #[test]
    fn centre_puts_the_odd_pixel_right_and_below() {
        assert_eq!(place((3, 3), (8, 8), Position::Centre), (2, 2));
        assert_eq!(place((8, 8), (8, 8), Position::Centre), (0, 0));
        assert_eq!(place((3, 3), (8, 8), Position::TopLeft), (0, 0));
    }

    #[test]
    fn a_small_indexed_picture_enlarged_stays_exact_and_crisp() {
        // 4×3 pixel art with three colours, enlarged 8× into a 32×24 page.
        let cols = [
            Rgba8::opaque(255, 0, 0),
            Rgba8::opaque(0, 128, 0),
            Rgba8::opaque(0, 0, 255),
        ];
        let art = image(4, 3, |x, y| cols[((x + y) % 3) as usize]);
        let pal = Palette::dpaint_default_32();
        let t = target((32, 24), &pal);
        let r = import(
            &art,
            &opts(PaletteChoice::New, 8, Dither::FloydSteinberg),
            &t,
        );
        assert_eq!(r.size, (32, 24));
        assert_eq!(r.reduction, Reduction::Exact);
        for y in 0..24 {
            for x in 0..32 {
                let src = art.get(x / 8, y / 8).unwrap();
                assert_eq!(shown(&r, x, y), Rgb8::new(src.r, src.g, src.b), "{x},{y}");
            }
        }
        // Not a whole factor (4 → 10): still only the picture's own colours.
        let r = import(
            &art,
            &opts(PaletteChoice::New, 8, Dither::FloydSteinberg),
            &target((10, 10), &pal),
        );
        let used: std::collections::HashSet<Rgb8> = (0..10)
            .flat_map(|y| (0..10).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                (r.at.1..r.at.1 + r.size.1).contains(&y) && (r.at.0..r.at.0 + r.size.0).contains(&x)
            })
            .map(|(x, y)| shown(&r, x, y))
            .collect();
        assert!(
            used.iter()
                .all(|c| cols.iter().any(|p| (p.r, p.g, p.b) == (c.r, c.g, c.b)))
        );
    }

    /// `n` different 12-bit colours (exact on OCS too), none of them black.
    fn many_colours(n: u32) -> Vec<Rgba8> {
        (0..n)
            .map(|i| {
                let c = |v: u32| (v % 16 * 17) as u8;
                Rgba8::opaque(c(i), c(15 + i * 3), c(i / 16 * 5 + 2))
            })
            .collect()
    }

    #[test]
    fn a_picture_with_as_many_colours_as_the_count_stays_exact_with_margins() {
        // 32 colours (the count), none the BG colour, on pages with margins: the picture
        // keeps every colour; the margin takes the nearest of them (no room for its own).
        let cols = many_colours(32);
        assert_eq!(
            cols.iter()
                .map(|p| (p.r, p.g, p.b))
                .collect::<std::collections::HashSet<_>>()
                .len(),
            32
        );
        let pal = Palette::dpaint_default_32();
        let bg = pal.get(0).unwrap();
        assert!(!cols.iter().any(|p| Rgb8::new(p.r, p.g, p.b) == bg));
        let art = image(8, 4, |x, y| cols[(y * 8 + x) as usize]);
        for depth in [PaletteDepth::Rgb8, PaletteDepth::Rgb4] {
            // 1:1 with margin rows, and enlarged 4× (32×16 on 32×24).
            for (page, zoom) in [((8, 6), 1), ((32, 24), 4)] {
                for dither in [Dither::None, Dither::FloydSteinberg, Dither::Ordered] {
                    for position in [Position::Centre, Position::TopLeft] {
                        let t = ImportTarget {
                            depth,
                            ..target(page, &pal)
                        };
                        let o = ImportOptions {
                            position,
                            ..opts(PaletteChoice::New, 32, dither)
                        };
                        let r = import(&art, &o, &t);
                        let what = format!("{depth:?} {page:?} {dither:?} {position:?}");
                        assert_eq!(r.reduction, Reduction::Exact, "{what}");
                        assert_eq!(r.size, (8 * zoom, 4 * zoom), "{what}");
                        let mut used = std::collections::HashSet::new();
                        for y in 0..r.size.1 {
                            for x in 0..r.size.0 {
                                let src = art.get(x / zoom, y / zoom).unwrap();
                                let got = shown(&r, r.at.0 + x, r.at.1 + y);
                                assert_eq!(got, Rgb8::new(src.r, src.g, src.b), "{what} {x},{y}");
                                used.insert(got);
                            }
                        }
                        assert_eq!(used.len(), 32, "{what}");
                        // The margin: the picture colour nearest to the BG colour.
                        let margin = shown(&r, 0, page.1 - 1);
                        let nearest = cols
                            .iter()
                            .map(|p| Rgb8::new(p.r, p.g, p.b))
                            .min_by_key(|c| {
                                let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
                                2 * d(c.r, bg.r) + 4 * d(c.g, bg.g) + 3 * d(c.b, bg.b)
                            })
                            .unwrap();
                        assert_eq!(margin, nearest, "{what}");
                    }
                }
            }
        }
        // 31 colours: the BG colour gets the free entry.
        let art = image(31, 1, |x, _| cols[x as usize]);
        let r = import(
            &art,
            &opts(PaletteChoice::New, 32, Dither::FloydSteinberg),
            &target((31, 3), &pal),
        );
        assert_eq!(r.reduction, Reduction::Exact);
        assert_eq!(shown(&r, 0, 0), bg);
        assert_eq!(shown(&r, 5, 1), Rgb8::new(cols[5].r, cols[5].g, cols[5].b));
    }

    #[test]
    fn the_cached_reduction_gives_the_plain_result() {
        let img = photo(80, 50);
        let pal = Palette::dpaint_default_32();
        let t = ImportTarget {
            bg: 3,
            ..target((96, 64), &pal)
        };
        let mut fitted = Fitted::new(&img, t.page, t.pixel_shape);
        assert_eq!(fitted.image(), &fit(&img, t.page, t.pixel_shape));
        let mut o = opts(PaletteChoice::New, 16, Dither::FloydSteinberg);
        let mut steps = Vec::new();
        for change in 0..14 {
            match change % 7 {
                0 => o.position = Position::TopLeft,
                1 => o.dither = Dither::Ordered,
                2 => o.colors = 64,
                3 => o.position = Position::Centre,
                4 => o.palette = PaletteChoice::Current,
                5 => o.dither = Dither::None,
                _ => {
                    o.palette = PaletteChoice::New;
                    o.colors = 16;
                }
            }
            steps.push(o);
        }
        for o in steps {
            let want = reduce_fitted(fitted.image(), &o, &t);
            assert_eq!(fitted.reduce(&o, &t), want, "{o:?}");
        }
        // Another target (BG colour) is not answered from the cache.
        let other = ImportTarget { bg: 1, ..t };
        assert_eq!(
            fitted.reduce(&o, &other),
            reduce_fitted(fitted.image(), &o, &other)
        );
    }

    #[test]
    fn new_palette_has_the_chosen_number_of_entries() {
        let img = photo(64, 48);
        let pal = Palette::dpaint_default_32();
        for n in COLOR_COUNTS {
            let r = import(
                &img,
                &opts(PaletteChoice::New, n, Dither::FloydSteinberg),
                &target((64, 48), &pal),
            );
            assert_eq!(r.palette.len(), usize::from(n));
            assert!(
                r.page
                    .pixels()
                    .iter()
                    .all(|&p| usize::from(p) < usize::from(n))
            );
        }
        // OCS: 12-bit entries.
        let t = ImportTarget {
            depth: PaletteDepth::Rgb4,
            ..target((64, 48), &pal)
        };
        let r = import(&img, &opts(PaletteChoice::New, 16, Dither::Ordered), &t);
        for &c in r.palette.colors() {
            assert_eq!(Rgb8::from_rgb4(c.to_rgb4()), c);
        }
    }

    #[test]
    fn current_palette_maps_exact_colours_to_their_own_index_lowest_of_duplicates() {
        // Entries 1 and 3 are the same red; 2 is green.
        let pal = Palette::new(vec![
            Rgb8::BLACK,
            Rgb8::new(200, 0, 0),
            Rgb8::new(0, 200, 0),
            Rgb8::new(200, 0, 0),
        ])
        .unwrap();
        let img = image(6, 2, |x, _| match x % 3 {
            0 => Rgba8::opaque(200, 0, 0),
            1 => Rgba8::opaque(0, 200, 0),
            _ => Rgba8::opaque(0, 0, 0),
        });
        for dither in [Dither::None, Dither::FloydSteinberg, Dither::Ordered] {
            let m = quantize::reduce_to_palette(&img, &pal, dither);
            assert!(m.exact && !m.transparent);
            for x in 0..6 {
                let want = [1, 2, 0][x as usize % 3];
                assert_eq!(m.image.get(x, 0), Some(want), "{dither:?} {x}");
            }
        }
        // Colours near an entry go to it; the red duplicate is never used.
        let near = image(4, 4, |x, _| {
            if x < 2 {
                Rgba8::opaque(190, 10, 5)
            } else {
                Rgba8::opaque(5, 190, 10)
            }
        });
        let m = quantize::reduce_to_palette(&near, &pal, Dither::None);
        assert!(!m.exact);
        for x in 0..4 {
            assert_eq!(m.image.get(x, 1), Some(if x < 2 { 1 } else { 2 }));
        }
    }

    #[test]
    fn no_dither_takes_the_nearest_colour() {
        let pal = Palette::new(vec![Rgb8::BLACK, Rgb8::new(100, 100, 100), Rgb8::WHITE]).unwrap();
        let img = image(8, 1, |x, _| {
            let v = (x * 32) as u8;
            Rgba8::opaque(v, v, v)
        });
        let m = quantize::reduce_to_palette(&img, &pal, Dither::None);
        let want: Vec<u8> = (0..8i32)
            .map(|x| {
                let v = x * 32;
                [0, 100, 255]
                    .iter()
                    .enumerate()
                    .min_by_key(|&(_, &p)| (p - v).abs())
                    .map(|(i, _)| i as u8)
                    .unwrap()
            })
            .collect();
        assert_eq!(m.image.pixels(), &want[..]);
    }

    #[test]
    fn ordered_dither_is_a_fixed_pattern_of_palette_colours() {
        // A flat grey halfway between black and white: half of every 4×4 cell each.
        let pal = Palette::new(vec![Rgb8::BLACK, Rgb8::WHITE]).unwrap();
        let grey = RgbaImage::new(16, 8, Rgba8::opaque(128, 128, 128));
        let a = quantize::reduce_to_palette(&grey, &pal, Dither::Ordered);
        let b = quantize::reduce_to_palette(&grey, &pal, Dither::Ordered);
        assert_eq!(a, b, "deterministic");
        assert!(a.image.pixels().iter().all(|&p| p < 2));
        for (cx, cy) in [(0, 0), (4, 0), (12, 4)] {
            let white = (0..4)
                .flat_map(|y| (0..4).map(move |x| (x, y)))
                .filter(|&(x, y)| a.image.get(cx + x, cy + y) == Some(1))
                .count();
            assert_eq!(white, 8, "cell {cx},{cy}");
        }
        // The pattern repeats every 4 pixels.
        for y in 0..8 {
            for x in 0..12 {
                assert_eq!(a.image.get(x, y), a.image.get(x + 4, y));
            }
        }
        // With a new palette it uses only the palette's entries, and differs from FS.
        let img = photo(48, 32);
        let p32 = Palette::dpaint_default_32();
        let t = target((48, 32), &p32);
        let o = import(&img, &opts(PaletteChoice::New, 8, Dither::Ordered), &t);
        let f = import(
            &img,
            &opts(PaletteChoice::New, 8, Dither::FloydSteinberg),
            &t,
        );
        assert!(o.page.pixels().iter().all(|&p| p < 8));
        assert_ne!(o.page, f.page);
        assert_eq!(
            o.palette, f.palette,
            "the palette does not depend on the dither"
        );
    }

    #[test]
    fn transparency_new_reserves_pink_zero_current_uses_zero() {
        let img = image(8, 8, |x, _| {
            if x < 2 {
                Rgba8::new(0, 0, 0, 0)
            } else {
                Rgba8::opaque(0, 0, 0)
            }
        });
        let pal = Palette::dpaint_default_32();
        // New: colour 0 pink, only transparent pixels use it; opaque black is its own entry.
        let r = import(
            &img,
            &opts(PaletteChoice::New, 4, Dither::FloydSteinberg),
            &target((8, 8), &pal),
        );
        assert_eq!(r.transparent_color, Some(0));
        assert_eq!(r.palette.get(0), Some(PINK));
        for x in 0..8 {
            assert_eq!(r.page.get(x, 3) == Some(0), x < 2, "{x}");
        }
        // Current: transparent pixels are colour 0 of the current palette, opaque black
        // (colour 0 there too) goes to another entry.
        let r = import(
            &img,
            &opts(PaletteChoice::Current, 4, Dither::None),
            &target((8, 8), &pal),
        );
        assert_eq!(r.transparent_color, Some(0));
        assert_eq!(r.palette, pal);
        for x in 0..8 {
            assert_eq!(r.page.get(x, 3) == Some(0), x < 2, "{x}");
        }
        // No transparent pixels: no transparent colour; Current may use colour 0.
        let opaque = RgbaImage::new(4, 4, Rgba8::opaque(0, 0, 0));
        let r = import(
            &opaque,
            &opts(PaletteChoice::Current, 4, Dither::None),
            &target((4, 4), &pal),
        );
        assert_eq!(r.transparent_color, None);
        assert!(r.page.pixels().iter().all(|&p| p == 0));
    }

    #[test]
    fn thumbnail_is_the_page_reduced_in_its_colours() {
        let pal = Palette::new(vec![Rgb8::BLACK, Rgb8::WHITE]).unwrap();
        let mut page = IndexedImage::new(4, 2, 0);
        page.set(2, 0, 1);
        page.set(3, 0, 1);
        page.set(2, 1, 1);
        page.set(3, 1, 1);
        let t = thumbnail(&page, &pal, 2, 1);
        assert_eq!(t.get(0, 0), Some(Rgba8::opaque(0, 0, 0)));
        assert_eq!(t.get(1, 0), Some(Rgba8::opaque(255, 255, 255)));
    }

    #[test]
    fn count_colors_stops_above_the_limit() {
        let img = photo(40, 40);
        assert_eq!(count_colors(&img, 256), None);
        let few = image(5, 5, |x, _| Rgba8::opaque(x as u8, 0, 0));
        assert_eq!(count_colors(&few, 256), Some(5));
    }
}
