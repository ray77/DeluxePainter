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
//!
//! [`convert`] (ADR-45) is the one conversion for generated pictures (GUI and MCP): it frames
//! a picture of any size on the page (crop-to-fill or fit with margins, never distorted),
//! scales it with a chosen scaler (smooth, nearest or a pixel-art majority filter) to exactly
//! the page size, checks the target's colour limits and reduces it with the same reduction.

use serde::{Deserialize, Serialize};

use crate::command::{Command, CommandError, ErrorCode};
use crate::image::IndexedImage;
use crate::mode::{Chipset, Target};
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// [`convert`] with the current palette for an OCS target: how many of its entries are
    /// not 12-bit and were rounded for the result (whose palette holds them rounded; ADR-46:
    /// a project saved before it can hold such colours, and so can an AGA picture). A
    /// front-end that applies the result says so (never silent). 0 otherwise.
    pub rounded_colors: usize,
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
            rounded_colors: 0,
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

// ---- ADR-45: converting generated pictures ([`convert`]).

/// How a generated picture covers the page (ADR-45). Neither distorts it: both keep its
/// proportions on the screen's pixel shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framing {
    /// The page is filled; what sticks out of the page's proportions is cut off evenly on
    /// both sides ([`resample::crop_to_aspect`]).
    #[default]
    Crop,
    /// The whole picture is shown, as large as fits, centred; the rest of the page is a
    /// margin ([`Margins`]).
    Fit,
}

/// How the picture is scaled to the page's pixels (ADR-45).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scaler {
    /// Area averaging ([`resample::scale_area`]): smooth, may mix colours at edges.
    #[default]
    Smooth,
    /// The source pixel under each page pixel's centre ([`resample::scale_nearest`]).
    Nearest,
    /// The most common colour of each page pixel's cell ([`resample::scale_majority`]):
    /// crisp edges for pictures drawn as pixel art.
    PixelArt,
}

/// What lies around a [`Framing::Fit`] picture (ADR-45).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Margins {
    /// The BG colour, as the Import requester's margins (ADR-42).
    #[default]
    Bg,
    /// Transparent: colour 0, reserved for transparency like transparent pixels.
    Transparent,
}

/// The choices for [`convert`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertOptions {
    pub framing: Framing,
    pub scaler: Scaler,
    /// With [`Framing::Fit`] only.
    pub margins: Margins,
    pub palette: PaletteChoice,
    /// Palette entries with [`PaletteChoice::New`]: a power of two from 2 up to the mode's
    /// maximum on the chipset.
    pub colors: u16,
    pub dither: Dither,
    /// Keep transparent pixels (alpha below [`quantize::ALPHA_THRESHOLD`]) as colour 0;
    /// otherwise the picture is laid on the BG colour first and comes out opaque.
    pub transparent_background: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            framing: Framing::Crop,
            scaler: Scaler::Smooth,
            margins: Margins::Bg,
            palette: PaletteChoice::New,
            colors: 32,
            dither: Dither::FloydSteinberg,
            transparent_background: false,
        }
    }
}

/// The document a converted picture is made for.
#[derive(Clone, Copy, Debug)]
pub struct ConvertTarget<'a> {
    /// Chipset, display mode and screen: the colour limits and the pixel shape.
    pub target: Target,
    /// The page size of the result (exactly); it must be a valid page of `target`.
    pub page: (u32, u32),
    /// The palette kept with [`PaletteChoice::Current`] (on OCS rounded to 12-bit values).
    pub palette: &'a Palette,
    /// The BG colour (an index into `palette`): margins, and what transparent pixels are
    /// laid on without `transparent_background`.
    pub bg: u8,
    /// Colour 0 of a new palette when there are transparent pixels (ADR-9: pink).
    pub transparent: Rgb8,
}

/// Converts a true-colour picture of any size (a generated picture, ADR-45) into a page of
/// exactly `t.page` pixels with the target's colour limits: framed ([`Framing`]), scaled
/// ([`Scaler`]), reduced as the Import requester does (ADR-42: a new palette of exactly
/// `colors` entries, rounded to 12-bit on OCS, or the current palette; the dither; margins
/// in the BG colour taking no part in dithering) and transparent pixels as colour 0.
///
/// Refuses, without converting, a mode the chipset cannot show (`UNSUPPORTED_MODE`), a colour
/// count that is no power of two in 2..=the mode's maximum or a current palette larger than
/// that (`PALETTE_RANGE_INVALID`), a page outside the target's page limits and an empty
/// picture (`INVALID_ARGUMENT`). With the current palette on OCS its entries are rounded to
/// 12-bit values first (a 12-bit palette stays identical), so the result always fits the
/// chipset; [`Imported::rounded_colors`] counts the entries that changed.
pub fn convert(
    img: &RgbaImage,
    opts: &ConvertOptions,
    t: &ConvertTarget,
) -> Result<Imported, CommandError> {
    let target = t.target;
    let mode = target.mode;
    let max = mode.max_colors(target.chipset).ok_or_else(|| {
        CommandError::new(
            ErrorCode::UnsupportedMode,
            format!("{} is not available on {:?}", mode.name(), target.chipset),
        )
    })?;
    match opts.palette {
        PaletteChoice::New => {
            let n = opts.colors;
            if !n.is_power_of_two() || n < 2 || n > max {
                return Err(CommandError::new(
                    ErrorCode::PaletteRangeInvalid,
                    format!(
                        "{n} colours: {} on {:?} allows 2..={max} (powers of two)",
                        mode.name(),
                        target.chipset
                    ),
                ));
            }
        }
        PaletteChoice::Current => {
            if t.palette.len() > usize::from(max) {
                return Err(CommandError::new(
                    ErrorCode::PaletteRangeInvalid,
                    format!(
                        "the current palette has {} colours; {} on {:?} shows at most {max}",
                        t.palette.len(),
                        mode.name(),
                        target.chipset
                    ),
                ));
            }
        }
    }
    let (pw, ph) = t.page;
    if !target.page_fits(pw, ph) {
        let ((a, b), (c, d)) = target.page_limits();
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            format!("page {pw}x{ph}: this target allows {a}x{b} to {c}x{d}"),
        ));
    }
    if img.width() == 0 || img.height() == 0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "the picture is empty",
        ));
    }
    let depth = match target.chipset {
        Chipset::Ocs => PaletteDepth::Rgb4,
        Chipset::Aga => PaletteDepth::Rgb8,
    };
    let mut colors = t.palette.colors().to_vec();
    let rounded = crate::hardware::snap_colors(target.chipset, &mut colors);
    let current = Palette::new(colors).expect("same number of entries");
    let bg = t.bg.min((current.len() - 1) as u8);
    let bg_rgb = current.get(bg).unwrap_or(Rgb8::BLACK);
    let source = if opts.transparent_background {
        img.clone()
    } else {
        flatten(img, bg_rgb)
    };
    let pixel_shape = mode.pixel_shape();
    let scale = |img: &RgbaImage, w: u32, h: u32| match opts.scaler {
        Scaler::Smooth => resample::scale_area(img, w, h),
        Scaler::Nearest => resample::scale_nearest(img, w, h),
        Scaler::PixelArt => resample::scale_majority(img, w, h),
    };
    let import_opts = ImportOptions {
        position: Position::Centre,
        palette: opts.palette,
        colors: opts.colors,
        dither: opts.dither,
    };
    let it = ImportTarget {
        page: t.page,
        pixel_shape,
        depth,
        palette: &current,
        bg,
        transparent: t.transparent,
    };
    let mut result = match opts.framing {
        Framing::Crop => {
            let aspect = resample::display_aspect(t.page, pixel_shape);
            let (x, y, w, h) = resample::crop_to_aspect(source.width(), source.height(), aspect);
            let scaled = scale(&source.crop(x, y, w, h), pw, ph);
            reduce_fitted(&scaled, &import_opts, &it)
        }
        Framing::Fit => {
            let (w, h) = fitted_size(source.width(), source.height(), t.page, pixel_shape);
            let scaled = scale(&source, w, h);
            match opts.margins {
                Margins::Bg => reduce_fitted(&scaled, &import_opts, &it),
                Margins::Transparent => {
                    // The margins are transparent pixels of a page-sized picture.
                    let at = place((w, h), t.page, Position::Centre);
                    let mut canvas = RgbaImage::new(pw, ph, Rgba8::new(0, 0, 0, 0));
                    for (row, y) in (at.1..at.1 + h).enumerate() {
                        for (col, x) in (at.0..at.0 + w).enumerate() {
                            if let Some(p) = scaled.get(col as u32, row as u32) {
                                canvas.set(x, y, p);
                            }
                        }
                    }
                    let mut r = reduce_fitted(&canvas, &import_opts, &it);
                    r.at = at;
                    r.size = (w, h);
                    r
                }
            }
        }
    };
    debug_assert_eq!((result.page.width(), result.page.height()), t.page);
    if opts.palette == PaletteChoice::Current {
        result.rounded_colors = rounded;
    }
    Ok(result)
}

/// `img` laid on `bg`: every pixel opaque, mixed with `bg` by its alpha.
fn flatten(img: &RgbaImage, bg: Rgb8) -> RgbaImage {
    let mix = |c: u8, b: u8, a: u8| {
        let (c, b, a) = (u32::from(c), u32::from(b), u32::from(a));
        ((c * a + b * (255 - a) + 127) / 255) as u8
    };
    let pixels = img
        .pixels()
        .iter()
        .map(|&p| match p.a {
            255 => p,
            a => Rgba8::opaque(mix(p.r, bg.r, a), mix(p.g, bg.g, a), mix(p.b, bg.b, a)),
        })
        .collect();
    RgbaImage::from_pixels(img.width(), img.height(), pixels).expect("same size")
}

impl Imported {
    /// The undoable command that makes this page the whole picture of a document for
    /// `target` (ADR-45: one undo step; [`Command::ReplacePicture`]).
    pub fn replace_command(&self, target: Target) -> Command {
        Command::ReplacePicture {
            target,
            palette: self.palette.colors().to_vec(),
            width: self.page.width(),
            height: self.page.height(),
            pixels: self.page.pixels().to_vec(),
            stored: false,
        }
    }
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

    // ---- ADR-45: convert.

    use crate::mode::{DisplayMode, Resolution, Video};

    fn ocs_320x240() -> Target {
        Target::new(Chipset::Ocs, DisplayMode::PAL_LORES).with_screen(320, 240)
    }

    fn ctarget(target: Target, page: (u32, u32), palette: &Palette) -> ConvertTarget<'_> {
        ConvertTarget {
            target,
            page,
            palette,
            bg: 0,
            transparent: PINK,
        }
    }

    fn copts(scaler: Scaler, colors: u16, dither: Dither) -> ConvertOptions {
        ConvertOptions {
            scaler,
            colors,
            dither,
            ..ConvertOptions::default()
        }
    }

    fn is_12_bit(c: Rgb8) -> bool {
        Rgb8::from_rgb4(c.to_rgb4()) == c
    }

    #[test]
    fn convert_gives_the_exact_page_and_at_most_the_colour_count() {
        let pal = Palette::dpaint_default_32();
        let img = photo(1024, 768);
        let hires = Target::new(
            Chipset::Ocs,
            DisplayMode::new(Video::Pal, Resolution::Hi, false),
        );
        let aga = Target::new(Chipset::Aga, DisplayMode::PAL_LORES);
        for (target, page, colors) in [
            (ocs_320x240(), (320, 240), 32),
            (hires, (640, 256), 16),
            (aga, (320, 256), 256),
            (aga, (400, 300), 64),
        ] {
            for scaler in [Scaler::Smooth, Scaler::Nearest, Scaler::PixelArt] {
                for dither in [Dither::None, Dither::FloydSteinberg, Dither::Ordered] {
                    let r = convert(
                        &img,
                        &copts(scaler, colors, dither),
                        &ctarget(target, page, &pal),
                    )
                    .unwrap();
                    let what = format!("{target:?} {page:?} {scaler:?} {dither:?}");
                    assert_eq!((r.page.width(), r.page.height()), page, "{what}");
                    assert_eq!(r.palette.len(), usize::from(colors), "{what}");
                    let max = r.page.pixels().iter().copied().max().unwrap();
                    assert!(u16::from(max) < colors, "{what}");
                    if target.chipset == Chipset::Ocs {
                        assert!(r.palette.colors().iter().all(|&c| is_12_bit(c)), "{what}");
                    }
                    // The command takes the result as it is.
                    let doc = crate::Document::new(320, 256, Palette::dpaint_default_32());
                    r.replace_command(target).validate(&doc).unwrap();
                }
            }
        }
    }

    #[test]
    fn convert_refuses_what_the_target_cannot_show() {
        let pal = Palette::dpaint_default_32();
        let img = photo(64, 48);
        let code = |target: Target, page, opts: ConvertOptions, pal: &Palette| {
            convert(&img, &opts, &ctarget(target, page, pal))
                .unwrap_err()
                .code
        };
        let hires = Target::new(
            Chipset::Ocs,
            DisplayMode::new(Video::Pal, Resolution::Hi, false),
        );
        let superhi = Target::new(
            Chipset::Ocs,
            DisplayMode::new(Video::Pal, Resolution::SuperHi, false),
        );
        let d = ConvertOptions::default();
        assert_eq!(
            code(hires, (640, 256), d, &pal),
            ErrorCode::PaletteRangeInvalid
        );
        assert_eq!(
            code(superhi, (1280, 256), d, &pal),
            ErrorCode::UnsupportedMode
        );
        let twelve = ConvertOptions { colors: 12, ..d };
        assert_eq!(
            code(ocs_320x240(), (320, 240), twelve, &pal),
            ErrorCode::PaletteRangeInvalid
        );
        // A page smaller than the screen.
        assert_eq!(
            code(ocs_320x240(), (320, 200), d, &pal),
            ErrorCode::InvalidArgument
        );
        // A current palette larger than the mode shows.
        let big = Palette::new(vec![Rgb8::BLACK; 64]).unwrap();
        let current = ConvertOptions {
            palette: PaletteChoice::Current,
            ..d
        };
        assert_eq!(
            code(ocs_320x240(), (320, 240), current, &big),
            ErrorCode::PaletteRangeInvalid
        );
        let empty = RgbaImage::new(0, 0, Rgba8::default());
        assert_eq!(
            convert(&empty, &d, &ctarget(ocs_320x240(), (320, 240), &pal))
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    /// ADR-46, wave-8 integration: the current palette's entries an OCS result cannot show
    /// (an older project's, or an AGA picture's) are rounded for it and counted, so a
    /// front-end can say so; nothing else counts.
    #[test]
    fn the_current_palettes_entries_rounded_for_ocs_are_counted() {
        use crate::hardware::snap;
        let mut colors = Palette::dpaint_default_32().colors().to_vec();
        colors[9] = Rgb8::new(11, 11, 11);
        colors[20] = Rgb8::new(0x10, 0x20, 0x31);
        let pal = Palette::new(colors.clone()).unwrap();
        let img = photo(64, 48);
        let current = ConvertOptions {
            palette: PaletteChoice::Current,
            ..copts(Scaler::Smooth, 32, Dither::None)
        };
        let ocs = ctarget(ocs_320x240(), (320, 240), &pal);
        let r = convert(&img, &current, &ocs).unwrap();
        assert_eq!(r.rounded_colors, 2);
        assert_eq!(r.palette.get(9), Some(snap(Chipset::Ocs, colors[9])));
        assert_eq!(r.palette.get(20), Some(snap(Chipset::Ocs, colors[20])));
        assert_ne!(r.palette.get(9), Some(colors[9]));
        assert!(r.palette.colors().iter().all(|&c| is_12_bit(c)));
        // AGA keeps them; a new palette is made for OCS; a 12-bit palette has none to round.
        let aga = Target::new(Chipset::Aga, DisplayMode::PAL_LORES);
        let r = convert(&img, &current, &ctarget(aga, (320, 256), &pal)).unwrap();
        assert_eq!((r.rounded_colors, r.palette.get(9)), (0, Some(colors[9])));
        let new = ConvertOptions {
            palette: PaletteChoice::New,
            ..current
        };
        assert_eq!(convert(&img, &new, &ocs).unwrap().rounded_colors, 0);
        let plain = Palette::dpaint_default_32();
        let r = convert(&img, &current, &ctarget(ocs_320x240(), (320, 240), &plain)).unwrap();
        assert_eq!(r.rounded_colors, 0);
    }

    const GREEN: Rgba8 = Rgba8::opaque(0, 255, 0);
    const RED: Rgba8 = Rgba8::opaque(255, 0, 0);
    const YELLOW: Rgba8 = Rgba8::opaque(255, 255, 0);
    const BLUE: Rgba8 = Rgba8::opaque(0, 0, 255);

    /// 1024x1024: green above row 102, blue from row 921, red left and yellow right between.
    fn bands() -> RgbaImage {
        image(1024, 1024, |x, y| match (x, y) {
            (_, 0..102) => GREEN,
            (_, 921..) => BLUE,
            (0..512, _) => RED,
            _ => YELLOW,
        })
    }

    fn shown_rgba(r: &Imported, x: u32, y: u32) -> Rgba8 {
        let c = shown(r, x, y);
        Rgba8::opaque(c.r, c.g, c.b)
    }

    #[test]
    fn crop_fills_the_page_and_cuts_evenly() {
        let pal = Palette::dpaint_default_32();
        // 5:4 on LoRes and on HiRes (pixels twice as tall).
        let hires = Target::new(
            Chipset::Aga,
            DisplayMode::new(Video::Pal, Resolution::Hi, false),
        );
        let lores = Target::new(Chipset::Aga, DisplayMode::PAL_LORES);
        for (target, page) in [(lores, (320, 256)), (hires, (640, 256))] {
            let r = convert(
                &bands(),
                &copts(Scaler::Nearest, 32, Dither::None),
                &ctarget(target, page, &pal),
            )
            .unwrap();
            assert_eq!(r.at, (0, 0));
            assert_eq!(r.size, page);
            let (w, h) = page;
            for y in 0..h {
                for x in 0..w {
                    let want = if x < w / 2 { RED } else { YELLOW };
                    assert_eq!(shown_rgba(&r, x, y), want, "{page:?} {x},{y}");
                }
            }
        }
    }

    #[test]
    fn fit_shows_everything_with_bg_or_transparent_margins() {
        let pal = Palette::dpaint_default_32();
        let quads = image(1024, 1024, |x, y| match (x < 512, y < 512) {
            (true, true) => RED,
            (false, true) => YELLOW,
            (true, false) => GREEN,
            (false, false) => BLUE,
        });
        let target = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES);
        let mut t = ctarget(target, (320, 256), &pal);
        // BG = colour 3, white in the default palette.
        t.bg = 3;
        let fit = ConvertOptions {
            framing: Framing::Fit,
            ..copts(Scaler::Nearest, 32, Dither::FloydSteinberg)
        };
        let r = convert(&quads, &fit, &t).unwrap();
        assert_eq!((r.at, r.size), ((32, 0), (256, 256)));
        assert_eq!(r.transparent_color, None);
        let white = Rgba8::opaque(255, 255, 255);
        for y in [0, 100, 255] {
            assert_eq!(shown_rgba(&r, 0, y), white);
            assert_eq!(shown_rgba(&r, 31, y), white);
            assert_eq!(shown_rgba(&r, 288, y), white);
            assert_eq!(shown_rgba(&r, 319, y), white);
        }
        assert_eq!(shown_rgba(&r, 32, 0), RED);
        assert_eq!(shown_rgba(&r, 287, 0), YELLOW);
        assert_eq!(shown_rgba(&r, 32, 255), GREEN);
        assert_eq!(shown_rgba(&r, 287, 255), BLUE);
        // Transparent margins are colour 0, reserved (pink); the picture avoids it.
        let clear = ConvertOptions {
            margins: Margins::Transparent,
            ..fit
        };
        let r = convert(&quads, &clear, &t).unwrap();
        assert_eq!((r.at, r.size), ((32, 0), (256, 256)));
        assert_eq!(r.transparent_color, Some(0));
        assert_eq!(r.palette.get(0), Some(Rgb8::from_rgb4(PINK.to_rgb4())));
        for y in 0..256 {
            for x in 0..320 {
                let margin = !(32..288).contains(&x);
                assert_eq!(r.page.get(x, y) == Some(0), margin, "{x},{y}");
            }
        }
    }

    #[test]
    fn transparent_background_is_colour_0_or_laid_on_the_bg() {
        let pal = Palette::new(vec![
            Rgb8::BLACK,
            Rgb8::WHITE,
            Rgb8::new(255, 0, 0),
            BLUE_RGB,
        ])
        .unwrap();
        // Left half transparent, right half red.
        let img = image(
            64,
            48,
            |x, _| {
                if x < 32 { Rgba8::new(0, 0, 0, 0) } else { RED }
            },
        );
        let target = Target::new(Chipset::Aga, DisplayMode::PAL_LORES);
        let mut t = ctarget(target, (320, 256), &pal);
        t.bg = 1;
        let keep = ConvertOptions {
            transparent_background: true,
            ..copts(Scaler::Nearest, 4, Dither::None)
        };
        let r = convert(&img, &keep, &t).unwrap();
        assert_eq!(r.transparent_color, Some(0));
        assert_eq!(r.palette.get(0), Some(PINK));
        assert_eq!(r.page.get(10, 100), Some(0));
        assert_eq!(shown_rgba(&r, 300, 100), RED);
        // Without it the transparent half is laid on the BG colour (white) and opaque.
        let flat = ConvertOptions {
            transparent_background: false,
            ..keep
        };
        let r = convert(&img, &flat, &t).unwrap();
        assert_eq!(r.transparent_color, None);
        assert_eq!(shown_rgba(&r, 10, 100), Rgba8::opaque(255, 255, 255));
        assert_eq!(shown_rgba(&r, 300, 100), RED);
        // The current palette: transparent pixels are its colour 0 too.
        let current = ConvertOptions {
            palette: PaletteChoice::Current,
            ..keep
        };
        let r = convert(&img, &current, &t).unwrap();
        assert_eq!(r.palette, pal);
        assert_eq!(r.transparent_color, Some(0));
        assert_eq!(r.page.get(10, 100), Some(0));
        assert_eq!(r.page.get(300, 100), Some(2));
    }

    const BLUE_RGB: Rgb8 = Rgb8::new(0, 0, 255);

    #[test]
    fn ocs_rounds_the_current_palette_to_12_bit_and_aga_keeps_it() {
        let pal = Palette::new(vec![
            Rgb8::new(1, 2, 3),
            Rgb8::new(250, 120, 7),
            Rgb8::new(17, 34, 51),
            Rgb8::new(200, 200, 201),
        ])
        .unwrap();
        let img = photo(100, 80);
        let opts = ConvertOptions {
            palette: PaletteChoice::Current,
            ..ConvertOptions::default()
        };
        let r = convert(&img, &opts, &ctarget(ocs_320x240(), (320, 240), &pal)).unwrap();
        assert_eq!(r.palette.len(), 4);
        assert!(r.palette.colors().iter().all(|&c| is_12_bit(c)));
        // Entry 2 is 12-bit already and stays.
        assert_eq!(r.palette.get(2), Some(Rgb8::new(17, 34, 51)));
        let aga = Target::new(Chipset::Aga, DisplayMode::PAL_LORES).with_screen(320, 240);
        let r = convert(&img, &opts, &ctarget(aga, (320, 240), &pal)).unwrap();
        assert_eq!(r.palette, pal);
    }

    /// 320x240 pixel art in 8 colours whose channels sit in the middle of the majority
    /// filter's buckets, blown up to 1024x768 (3.2x) by nearest pixel, with or without +-3
    /// noise.
    fn pixel_art(noisy: bool) -> (IndexedImage, RgbaImage) {
        let colours: [Rgb8; 8] = std::array::from_fn(|i| {
            let v = |bit: usize, hi: u8| if i >> bit & 1 == 1 { hi } else { 28 };
            Rgb8::new(v(0, 228), v(1, 196), v(2, 164))
        });
        let art = IndexedImage::from_pixels(
            320,
            240,
            (0..320 * 240)
                .map(|i| ((i % 320 / 7 + i / 320 / 5 * 3) % 8) as u8)
                .collect(),
        )
        .unwrap();
        let pal = Palette::new(colours.to_vec()).unwrap();
        let big = resample::scale_nearest(&rgba_of(&art, &pal), 1024, 768);
        // Deterministic noise in -3..=3 from the position and the channel.
        let noise = |x: u32, y: u32, c: u32| {
            let h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663))
                .wrapping_add(c.wrapping_mul(83_492_791));
            (h.wrapping_mul(2_654_435_761) >> 16) % 7
        };
        let noisy = image(1024, 768, |x, y| {
            let p = big.get(x, y).unwrap();
            let d = |c: u32| if noisy { noise(x, y, c) as i32 - 3 } else { 0 };
            let q = |v: u8, c: u32| (i32::from(v) + d(c)) as u8;
            Rgba8::opaque(q(p.r, 0), q(p.g, 1), q(p.b, 2))
        });
        (art, noisy)
    }

    /// Page pixels whose colour does not follow the art's colour one to one.
    fn mismatches(art: &IndexedImage, page: &IndexedImage) -> usize {
        let mut map: std::collections::HashMap<u8, std::collections::HashMap<u8, usize>> =
            Default::default();
        for (&a, &p) in art.pixels().iter().zip(page.pixels()) {
            *map.entry(a).or_default().entry(p).or_default() += 1;
        }
        // Each art colour's most common page colour counts as right; the rest are wrong.
        let wrong: usize = map
            .values()
            .map(|m| m.values().sum::<usize>() - m.values().max().unwrap())
            .sum();
        let mut firsts: Vec<u8> = map
            .values()
            .map(|m| *m.iter().max_by_key(|(_, n)| **n).unwrap().0)
            .collect();
        firsts.sort_unstable();
        firsts.dedup();
        assert_eq!(firsts.len(), map.len(), "two art colours became one");
        wrong
    }

    #[test]
    fn the_pixel_art_scaler_keeps_every_cell_crisp() {
        let pal = Palette::dpaint_default_32();
        let t = ctarget(ocs_320x240(), (320, 240), &pal);
        let (art, noisy) = pixel_art(true);
        let r = convert(&noisy, &copts(Scaler::PixelArt, 8, Dither::None), &t).unwrap();
        assert_eq!(mismatches(&art, &r.page), 0);
        // Without noise the cells keep the art's very colours: none is mixed, so the 8
        // colours fit exactly (rounded to 12-bit on OCS).
        let (art, clean) = pixel_art(false);
        let r = convert(&clean, &copts(Scaler::PixelArt, 8, Dither::None), &t).unwrap();
        assert_eq!(mismatches(&art, &r.page), 0);
        assert_eq!(r.reduction, Reduction::Rounded);
        // Area averaging mixes neighbouring cells along the edges: new colours, which push
        // the picture into quantizing.
        let smooth = convert(&clean, &copts(Scaler::Smooth, 8, Dither::None), &t).unwrap();
        assert_eq!(smooth.reduction, Reduction::Quantized);
    }
}
