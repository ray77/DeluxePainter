//! Loading and saving pictures by file type, plus the headless `info` and `convert` commands.
//!
//! Formats: IFF ILBM and PNG in and out, JPEG in only. True-colour PNG and JPEG pictures are
//! reduced to an indexed page when they are opened (ADR-9). Whatever a target format cannot
//! store is reported, never dropped silently.

use std::collections::HashSet;
use std::path::Path;

use dpx_core::quantize::{self, Dither, PaletteDepth, ReduceOptions, Reduction};
use dpx_core::resample;
use dpx_core::{Chipset, DisplayMode, Document, IndexedImage, Palette, Rgb8, RgbaImage, Target};
use dpx_iff::{ColorRange, Compression, Ilbm, Masking, ilbm};

/// A loaded picture plus notes about anything that did not survive the import.
pub struct Loaded {
    pub picture: Picture,
    pub notes: Vec<String>,
}

pub enum Picture {
    /// Keeps all ILBM metadata so ILBM → ILBM conversions lose nothing.
    Ilbm(Box<Ilbm>),
    Png {
        image: IndexedImage,
        palette: Palette,
    },
    /// True-colour PNG or JPEG; becomes indexed only when it is opened ([`reduce_true_color`]).
    TrueColor {
        image: RgbaImage,
        format: &'static str,
    },
}

impl Picture {
    /// Pixels and palette of an indexed picture; `None` for true colour.
    pub fn indexed(&self) -> Option<(&IndexedImage, &Palette)> {
        match self {
            Picture::Ilbm(i) => Some((&i.image, &i.palette)),
            Picture::Png { image, palette } => Some((image, palette)),
            Picture::TrueColor { .. } => None,
        }
    }

    /// The ILBM display mode id, if the file has one.
    pub fn camg(&self) -> Option<u32> {
        match self {
            Picture::Ilbm(i) => i.camg,
            Picture::Png { .. } | Picture::TrueColor { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ilbm,
    Png,
    Jpeg,
}

fn kind_of_bytes(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(b"FORM") {
        Some(Kind::Ilbm)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Kind::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Kind::Jpeg)
    } else {
        None
    }
}

/// Output type for `convert`, by extension.
fn kind_of_path(path: &Path) -> Result<Kind, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("iff" | "ilbm" | "lbm") => Ok(Kind::Ilbm),
        Some("png") => Ok(Kind::Png),
        Some("jpg" | "jpeg") => Err(format!(
            "{}: JPEG is import-only (DeluxePainter writes .iff/.ilbm/.lbm and .png)",
            path.display()
        )),
        _ => Err(format!(
            "{}: unsupported output type (DeluxePainter writes .iff/.ilbm/.lbm and .png)",
            path.display()
        )),
    }
}

pub fn load(path: &Path) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    match kind_of_bytes(&bytes) {
        Some(Kind::Ilbm) => {
            let pic = ilbm::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Loaded {
                picture: Picture::Ilbm(Box::new(pic)),
                notes: Vec::new(),
            })
        }
        Some(Kind::Png) => {
            let pic = match dpx_formats::read_png(&bytes) {
                Ok(dpx_formats::PngImage::Indexed(pic)) => pic,
                Ok(dpx_formats::PngImage::TrueColor(image)) => {
                    return Ok(Loaded {
                        picture: Picture::TrueColor {
                            image,
                            format: "PNG",
                        },
                        notes: Vec::new(),
                    });
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            };
            let mut notes = Vec::new();
            if !pic.transparent_entries.is_empty() {
                notes.push(format!(
                    "PNG transparency of {} palette entries is ignored (pages are opaque)",
                    pic.transparent_entries.len()
                ));
            }
            Ok(Loaded {
                picture: Picture::Png {
                    image: pic.image,
                    palette: pic.palette,
                },
                notes,
            })
        }
        Some(Kind::Jpeg) => {
            let image =
                dpx_formats::read_jpeg(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Loaded {
                picture: Picture::TrueColor {
                    image,
                    format: "JPEG",
                },
                notes: Vec::new(),
            })
        }
        None => Err(format!(
            "{}: unknown file type (DeluxePainter reads IFF ILBM, PNG and JPEG)",
            path.display()
        )),
    }
}

/// Writes `picture` in the format given by the file extension; returns notes about metadata
/// the target format cannot hold.
/// True-colour pictures must be opened (reduced) first; see [`convert`].
pub fn save(picture: &Picture, path: &Path) -> Result<Vec<String>, String> {
    let kind = kind_of_path(path)?;
    let (image, palette) = picture
        .indexed()
        .ok_or_else(|| "true-colour picture must be reduced before saving".to_string())?;
    let mut notes = Vec::new();
    let bytes = match kind {
        Kind::Ilbm | Kind::Jpeg => {
            let ilbm = match picture {
                Picture::Ilbm(i) => (**i).clone(),
                _ => Ilbm::from_indexed(image.clone(), palette.clone()),
            };
            ilbm::write(&ilbm, Compression::ByteRun1).map_err(|e| e.to_string())?
        }
        Kind::Png => {
            if let Picture::Ilbm(i) = picture {
                notes.extend(png_drops(i));
            }
            dpx_formats::write_png(image, palette).map_err(|e| e.to_string())?
        }
    };
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(notes)
}

/// Picture-file details the document model does not hold yet (colour cycling arrives with
/// Phase 3). They are kept from Load to Save so a DeluxePaint picture round-trips.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PictureExtras {
    pub ranges: Vec<ColorRange>,
    pub grab: Option<(i16, i16)>,
    /// Palette entry DeluxePaint marks transparent (ILBM masking "transparent colour").
    pub transparent_color: Option<u16>,
}

/// Colour 0 of a reduced true-colour picture with transparent pixels (owner decision: the
/// old "magic pink", so transparent areas stay visible). Saved as the ILBM transparent colour.
pub const TRANSPARENT_PINK: Rgb8 = Rgb8::new(255, 0, 255);

/// A true-colour picture turned into an indexed page (ADR-9).
pub struct ReducedPicture {
    pub image: IndexedImage,
    pub palette: Palette,
    pub target: Target,
    pub transparent_color: Option<u16>,
    pub notes: Vec<String>,
}

/// Owner decisions (ADR-9): a picture that fits a screen format keeps its size and gets the
/// mode guessed from it, like an ILBM without CAMG; a bigger one is scaled down to the
/// `current` screen, keeping its proportions on that screen's pixel shape. The palette is
/// new, with as many colours as the mode allows (exact when the picture has that few), and
/// the reduction uses Floyd–Steinberg dithering. Transparent pixels become colour 0 (pink).
pub fn reduce_true_color(img: &RgbaImage, format: &str, current: Target) -> ReducedPicture {
    let (w, h) = (img.width(), img.height());
    let default = Target::default();
    let fits = DisplayMode::all().iter().any(|m| {
        let (sw, sh) = m.size();
        m.max_colors(default.chipset).is_some() && w <= sw && h <= sh
    });
    let mut notes = Vec::new();
    let (target, scaled) = if fits {
        let mode = DisplayMode::guess(None, w, h);
        let (sw, sh) = mode.screen_for_picture(w, h);
        (Target::new(default.chipset, mode).with_screen(sw, sh), None)
    } else {
        let mode = current.mode;
        let (sw, sh) = current.screen;
        let (nw, nh) = resample::fit_size(w, h, sw, sh, mode.pixel_shape());
        notes.push(format!(
            "{format} picture {w}x{h} is larger than any screen: scaled to {nw}x{nh} for {}",
            mode.name()
        ));
        (current, Some(resample::scale_area(img, nw, nh)))
    };
    let source = scaled.as_ref().unwrap_or(img);
    let opts = ReduceOptions {
        max_colors: target.mode.max_colors(target.chipset).unwrap_or(16),
        depth: match target.chipset {
            Chipset::Ocs => PaletteDepth::Rgb4,
            Chipset::Aga => PaletteDepth::Rgb8,
        },
        dither: Dither::FloydSteinberg,
        transparent: TRANSPARENT_PINK,
    };
    let r = quantize::reduce(source, &opts);
    match r.reduction {
        Reduction::Exact => {}
        Reduction::Rounded => {
            notes.push(format!("{} colours rounded to 12-bit OCS values", r.colors))
        }
        Reduction::Quantized => notes.push(format!(
            "{format} true colour reduced to {} colours (Floyd-Steinberg dithering)",
            r.colors
        )),
    }
    if r.transparent_index.is_some() {
        notes.push("transparent pixels became colour 0 (pink), the transparent colour".into());
    }
    ReducedPicture {
        image: r.image,
        palette: r.palette,
        target,
        transparent_color: r.transparent_index.map(u16::from),
        notes,
    }
}

/// Turns a loaded picture into a document. Indexed pictures get the display mode guessed from
/// CAMG and size; true-colour ones are reduced ([`reduce_true_color`], which scales pictures
/// too big for any screen to `current`). Returns notes about everything that is not kept.
pub fn open_picture(picture: Picture, current: Target) -> (Document, PictureExtras, Vec<String>) {
    if let Picture::TrueColor { image, format } = &picture {
        let r = reduce_true_color(image, format, current);
        let extras = PictureExtras {
            transparent_color: r.transparent_color,
            ..PictureExtras::default()
        };
        return (
            Document::from_picture(r.image, r.palette, r.target),
            extras,
            r.notes,
        );
    }
    let (w, h) = picture
        .indexed()
        .map_or((0, 0), |(i, _)| (i.width(), i.height()));
    let mode = DisplayMode::guess(picture.camg(), w, h);
    // The screen: the ILBM's page fields (DeluxePaint writes its screen size there) if they
    // are a valid screen for the mode, else from the picture size (ADR-11).
    let page_fields = match &picture {
        Picture::Ilbm(i) => Some((i.page_width.max(0) as u32, i.page_height.max(0) as u32)),
        _ => None,
    };
    let (sw, sh) = page_fields
        .filter(|&(pw, ph)| mode.screen_fits(pw, ph))
        .unwrap_or_else(|| mode.screen_for_picture(w, h));
    let target = Target::new(Target::default().chipset, mode).with_screen(sw, sh);
    let mut extras = PictureExtras::default();
    let mut notes = Vec::new();
    let (image, palette) = match picture {
        Picture::Ilbm(i) => {
            let i = *i;
            extras.ranges = i.ranges;
            extras.grab = i.grab;
            match i.masking {
                Masking::TransparentColor => extras.transparent_color = Some(i.transparent_color),
                Masking::HasMask => {
                    notes.push("mask plane not kept (pages are opaque)".to_string());
                }
                Masking::None | Masking::Lasso => {}
            }
            for c in &i.other_chunks {
                notes.push(format!(
                    "{} chunk ({} bytes) not kept",
                    dpx_iff::iff::id_str(c.id),
                    c.data.len()
                ));
            }
            (i.image, i.palette)
        }
        Picture::Png { image, palette } => (image, palette),
        Picture::TrueColor { .. } => unreachable!("handled above"),
    };
    (
        Document::from_picture(image, palette, target),
        extras,
        notes,
    )
}

/// DeluxePaint pages are never smaller than the screen (ADR-10): a smaller picture is placed
/// top-left on a screen-sized page of colour 0 (the background, transparent in brushes).
/// Returns a note when the page grew. Used when the GUI opens a picture; `convert` keeps the
/// picture's own size.
pub fn page_at_least_screen(doc: Document) -> (Document, Option<String>) {
    let (sw, sh) = doc.target().screen;
    let (pw, ph) = (doc.page().width(), doc.page().height());
    if pw >= sw && ph >= sh {
        return (doc, None);
    }
    let (w, h) = (pw.max(sw), ph.max(sh));
    let mut page = IndexedImage::new(w, h, 0);
    page.blit(doc.page(), 0, 0);
    let note = format!("page enlarged from {pw}x{ph} to the {w}x{h} screen, picture top-left");
    let doc = Document::from_picture(page, doc.palette().clone(), doc.target());
    (doc, Some(note))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFormat {
    Ilbm,
    Png,
}

/// Picture ▸ Save picks the format by name: `.png` writes PNG; everything else writes IFF
/// ILBM, because Amiga names often have no extension or tags like `.LoRes`.
pub fn save_format(path: &Path) -> Result<SaveFormat, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("png") => Ok(SaveFormat::Png),
        Some("jpg" | "jpeg") => Err("JPEG is import-only: use .iff or .png".into()),
        Some("gif" | "bmp" | "tif" | "tiff" | "webp" | "dpxproj") => {
            Err("DeluxePainter saves pictures as IFF or PNG".into())
        }
        _ => Ok(SaveFormat::Ilbm),
    }
}

/// Encodes the document for `path`: ILBM with the display mode's CAMG, pixel aspect and page
/// size plus the kept extras, or indexed PNG. Notes name what the chosen format cannot hold.
pub fn encode_document(
    doc: &Document,
    extras: &PictureExtras,
    path: &Path,
) -> Result<(Vec<u8>, Vec<String>), String> {
    let colors = doc.palette().len();
    let (ranges, dropped): (Vec<ColorRange>, Vec<ColorRange>) = extras
        .ranges
        .iter()
        .partition(|r| usize::from(r.low) < colors && usize::from(r.high) < colors);
    let mut notes: Vec<String> = dropped
        .iter()
        .map(|r| {
            format!(
                "colour range {}-{} lies outside the palette, not saved",
                r.low, r.high
            )
        })
        .collect();
    let transparent = extras
        .transparent_color
        .filter(|&c| usize::from(c) < colors);
    match save_format(path)? {
        SaveFormat::Ilbm => {
            let mode = doc.target().mode;
            let mut out = Ilbm::from_indexed(doc.page().clone(), doc.palette().clone());
            out.camg = Some(doc.target().camg());
            (out.x_aspect, out.y_aspect) = mode.aspect();
            let (sw, sh) = doc.target().screen;
            out.page_width = sw as i16;
            out.page_height = sh as i16;
            out.ranges = ranges;
            out.grab = extras.grab;
            if let Some(c) = transparent {
                out.masking = Masking::TransparentColor;
                out.transparent_color = c;
            }
            let bytes = ilbm::write(&out, Compression::ByteRun1).map_err(|e| e.to_string())?;
            Ok((bytes, notes))
        }
        SaveFormat::Png => {
            let mut drops = vec![format!("display mode {}", doc.target().mode.name())];
            if !ranges.is_empty() {
                drops.push(format!("{} colour-cycling ranges (CRNG)", ranges.len()));
            }
            if let Some(c) = transparent {
                drops.push(format!("transparent colour {c}"));
            }
            notes.extend(drops.into_iter().map(|d| format!("PNG cannot store: {d}")));
            let bytes =
                dpx_formats::write_png(doc.page(), doc.palette()).map_err(|e| e.to_string())?;
            Ok((bytes, notes))
        }
    }
}

/// Writes through a temporary file in the same drawer and renames it, so a failed save never
/// leaves a half-written picture behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path
        .file_name()
        .ok_or_else(|| "no file name".to_string())?
        .to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.dpx-tmp"));
    let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

/// ILBM details that PNG has no place for.
fn png_drops(i: &Ilbm) -> Vec<String> {
    let mut v = Vec::new();
    if !i.ranges.is_empty() {
        v.push(format!("{} colour-cycling ranges (CRNG)", i.ranges.len()));
    }
    if let Some(camg) = i.camg {
        v.push(format!("display mode CAMG {camg:#010x}"));
    }
    if i.masking != Masking::None {
        v.push(format!("masking ({:?})", i.masking));
    }
    if i.x_aspect != i.y_aspect {
        v.push(format!("pixel aspect {}:{}", i.x_aspect, i.y_aspect));
    }
    for c in &i.other_chunks {
        v.push(format!(
            "{} chunk ({} bytes)",
            dpx_iff::iff::id_str(c.id),
            c.data.len()
        ));
    }
    v.into_iter()
        .map(|what| format!("PNG cannot store: {what}"))
        .collect()
}

/// `deluxepainter info <file>`: a readable summary of a picture file.
pub fn info(path: &Path) -> Result<String, String> {
    let loaded = load(path)?;
    let mut out = String::new();
    let name = path.display();
    if let Picture::TrueColor { image, format } = &loaded.picture {
        return Ok(true_color_info(&name.to_string(), image, format));
    }
    let (img, _) = loaded.picture.indexed().expect("indexed picture");
    match &loaded.picture {
        Picture::Ilbm(i) => {
            out += &format!(
                "{name}: IFF ILBM {}x{}, {} planes ({} colours), {:?}, masking {:?}",
                img.width(),
                img.height(),
                i.planes,
                i.palette.len(),
                i.compression,
                i.masking
            );
            if i.masking == Masking::TransparentColor {
                out += &format!(" (colour {})", i.transparent_color);
            }
            out += &format!(
                "\n  aspect {}:{}, page {}x{}\n",
                i.x_aspect, i.y_aspect, i.page_width, i.page_height
            );
            out += &format!(
                "  CMAP: {} entries{}\n",
                i.palette.len(),
                if i.cmap_4bit_fixed {
                    ", 4-bit values expanded (0xN0 -> 0xNN)"
                } else {
                    ""
                }
            );
            out += &match i.camg {
                Some(c) => format!("  CAMG: {c:#010x}\n"),
                None => "  CAMG: none\n".into(),
            };
            for (n, r) in i.ranges.iter().enumerate() {
                out += &format!(
                    "  CRNG {n}: colours {}-{}, {:.1} steps/s{}{}\n",
                    r.low,
                    r.high,
                    r.steps_per_second(),
                    if r.active() { ", active" } else { "" },
                    if r.reverse() { ", reverse" } else { "" }
                );
            }
            if let Some((x, y)) = i.grab {
                out += &format!("  GRAB: {x},{y}\n");
            }
            for c in &i.other_chunks {
                out += &format!(
                    "  {}: {} bytes (kept, not interpreted)\n",
                    dpx_iff::iff::id_str(c.id),
                    c.data.len()
                );
            }
        }
        Picture::Png { palette, .. } => {
            out += &format!(
                "{name}: PNG indexed {}x{}, {} colours\n",
                img.width(),
                img.height(),
                palette.len()
            );
        }
        Picture::TrueColor { .. } => unreachable!("handled above"),
    }
    let used = {
        let mut seen = [false; 256];
        for &p in img.pixels() {
            seen[usize::from(p)] = true;
        }
        seen.iter().filter(|&&s| s).count()
    };
    out += &format!("  colours used: {used}\n");
    for n in &loaded.notes {
        out += &format!("  note: {n}\n");
    }
    Ok(out)
}

/// `info` for a true-colour picture: what it is and how it would open (on the default screen).
fn true_color_info(name: &str, img: &RgbaImage, format: &str) -> String {
    let opaque: HashSet<(u8, u8, u8)> = img
        .pixels()
        .iter()
        .filter(|p| p.a >= quantize::ALPHA_THRESHOLD)
        .map(|p| (p.r, p.g, p.b))
        .collect();
    let transparent = img
        .pixels()
        .iter()
        .filter(|p| p.a < quantize::ALPHA_THRESHOLD)
        .count();
    let mut out = format!(
        "{name}: {format} true colour {}x{}, {} colours",
        img.width(),
        img.height(),
        opaque.len()
    );
    if transparent > 0 {
        out += &format!(", {transparent} transparent pixels");
    }
    let r = reduce_true_color(img, format, Target::default());
    out += &format!(
        "\n  opens as: {}x{} {} with {} palette entries\n",
        r.image.width(),
        r.image.height(),
        r.target.mode.name(),
        r.palette.len()
    );
    for n in &r.notes {
        out += &format!("  note: {n}\n");
    }
    out
}

/// `deluxepainter convert <in> <out>`. True-colour input is reduced as on opening, for the
/// default screen (PAL LoRes, AGA), and written with its display mode.
pub fn convert(input: &Path, output: &Path) -> Result<Vec<String>, String> {
    kind_of_path(output)?;
    let loaded = load(input)?;
    let mut notes = loaded.notes;
    if let Picture::TrueColor { .. } = loaded.picture {
        let (doc, extras, opened) = open_picture(loaded.picture, Target::default());
        notes.extend(opened);
        let (bytes, saved) = encode_document(&doc, &extras, output)?;
        std::fs::write(output, bytes).map_err(|e| format!("{}: {e}", output.display()))?;
        notes.extend(saved);
    } else {
        notes.extend(save(&loaded.picture, output)?);
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Rgb8;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-files-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn convert_between_ilbm_and_png_keeps_pixels_and_palette() {
        let mut img = IndexedImage::new(40, 10, 0);
        for x in 0..40 {
            img.set(x, x % 10, (x % 16) as u8);
        }
        let pal = Palette::new(
            (0..16)
                .map(|i| Rgb8::new(i * 16, 0, 255 - i * 16))
                .collect(),
        )
        .unwrap();
        let mut src = Ilbm::from_indexed(img.clone(), pal.clone());
        src.ranges.push(dpx_iff::ColorRange {
            rate: 8192,
            flags: 1,
            low: 1,
            high: 4,
        });
        let a = tmp("a.iff");
        std::fs::write(&a, ilbm::write(&src, Compression::ByteRun1).unwrap()).unwrap();

        let b = tmp("b.png");
        let notes = convert(&a, &b).unwrap();
        assert!(notes.iter().any(|n| n.contains("CRNG")), "{notes:?}");
        let c = tmp("c.lbm");
        assert!(convert(&b, &c).unwrap().is_empty());
        let back = load(&c).unwrap().picture;
        assert_eq!(back.indexed(), Some((&img, &pal)));

        // ILBM → ILBM keeps the colour cycling.
        let d = tmp("d.ilbm");
        convert(&a, &d).unwrap();
        match load(&d).unwrap().picture {
            Picture::Ilbm(i) => assert_eq!(i.ranges, src.ranges),
            _ => panic!("expected ILBM"),
        }
        assert!(
            info(&a)
                .unwrap()
                .contains("CRNG 0: colours 1-4, 30.0 steps/s, active")
        );
        assert!(save(&back, &tmp("x.jpg")).is_err(), "no JPEG export");
        let _ = std::fs::remove_dir_all(a.parent().unwrap());
    }

    #[test]
    fn save_format_by_name() {
        let f = |n: &str| save_format(Path::new(n));
        assert_eq!(f("a.PNG"), Ok(SaveFormat::Png));
        assert_eq!(f("a.iff"), Ok(SaveFormat::Ilbm));
        assert_eq!(f("Venus.LoRes"), Ok(SaveFormat::Ilbm));
        assert_eq!(f("noext"), Ok(SaveFormat::Ilbm));
        assert!(f("a.jpg").unwrap_err().contains("import-only"));
        assert!(f("a.gif").is_err());
    }

    #[test]
    fn open_then_save_keeps_mode_ranges_and_transparency() {
        use dpx_core::{Resolution, Video};
        let mut img = IndexedImage::new(640, 512, 0);
        img.set(3, 4, 7);
        let mut src = Ilbm::from_indexed(img.clone(), Palette::dpaint_default_32());
        let pal_hi_lace = DisplayMode::new(Video::Pal, Resolution::Hi, true);
        src.camg = Some(pal_hi_lace.mode_id());
        src.ranges = vec![
            ColorRange {
                rate: 2560,
                flags: 1,
                low: 20,
                high: 31,
            },
            ColorRange {
                rate: 0,
                flags: 0,
                low: 40,
                high: 50,
            },
        ];
        src.grab = Some((5, 6));
        src.masking = Masking::TransparentColor;
        src.transparent_color = 3;
        src.other_chunks.push(ilbm::RawChunk {
            id: *b"DPPS",
            data: vec![0; 10],
        });
        let (doc, extras, notes) = open_picture(Picture::Ilbm(Box::new(src)), Target::default());
        assert_eq!(doc.target().mode, pal_hi_lace);
        assert_eq!(notes, ["DPPS chunk (10 bytes) not kept"]);

        // Own drawer: the other test removes the shared one when it ends.
        let dir = std::env::temp_dir().join(format!("dpx-files-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.LoRes");
        let (bytes, notes) = encode_document(&doc, &extras, &path).unwrap();
        assert_eq!(
            notes.len(),
            1,
            "range 40-50 is outside 32 colours: {notes:?}"
        );
        write_atomic(&path, &bytes).unwrap();
        let Picture::Ilbm(back) = load(&path).unwrap().picture else {
            panic!("expected ILBM");
        };
        assert_eq!(back.image, img);
        assert_eq!(back.camg, Some(0x0002_9004));
        assert_eq!((back.x_aspect, back.y_aspect), pal_hi_lace.aspect());
        assert_eq!((back.page_width, back.page_height), (640, 512));
        assert_eq!(back.ranges, extras.ranges[..1]);
        assert_eq!(back.grab, Some((5, 6)));
        assert_eq!(
            (back.masking, back.transparent_color),
            (Masking::TransparentColor, 3)
        );
        assert!(back.other_chunks.is_empty());

        // PNG says what it cannot hold.
        let (_, notes) = encode_document(&doc, &extras, &dir.join("x.png")).unwrap();
        assert!(
            notes.iter().any(|n| n.contains("PAL:High Res Laced")),
            "{notes:?}"
        );
        assert!(notes.iter().any(|n| n.contains("CRNG")));
        assert!(notes.iter().any(|n| n.contains("transparent colour 3")));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn rgba_image(w: u32, h: u32, f: impl Fn(u32, u32) -> dpx_core::Rgba8) -> RgbaImage {
        let mut img = RgbaImage::new(w, h, dpx_core::Rgba8::default());
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, f(x, y));
            }
        }
        img
    }

    fn photo(w: u32, h: u32) -> RgbaImage {
        rgba_image(w, h, |x, y| {
            dpx_core::Rgba8::opaque(
                (x * 255 / (w - 1)) as u8,
                (y * 255 / (h - 1)) as u8,
                ((x * 7 + y * 3) % 256) as u8,
            )
        })
    }

    fn write_rgba_png(path: &Path, img: &RgbaImage) {
        let data: Vec<u8> = img
            .pixels()
            .iter()
            .flat_map(|p| [p.r, p.g, p.b, p.a])
            .collect();
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, img.width(), img.height());
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&data).unwrap();
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn true_colour_pixel_art_opens_exactly_at_its_size() {
        use dpx_core::{Resolution, Video};
        let cols = [(0, 0, 0), (255, 255, 255), (12, 34, 56)];
        let img = rgba_image(640, 200, |x, y| {
            let (r, g, b) = cols[((x / 8 + y / 8) % 3) as usize];
            dpx_core::Rgba8::opaque(r, g, b)
        });
        let r = reduce_true_color(&img, "PNG", Target::default());
        assert_eq!(
            r.target.mode,
            DisplayMode::new(Video::Ntsc, Resolution::Hi, false)
        );
        assert_eq!((r.image.width(), r.image.height()), (640, 200));
        assert_eq!(r.palette.len(), 4);
        assert!(r.notes.is_empty(), "nothing lost: {:?}", r.notes);
        for (x, y) in [(0, 0), (9, 0), (17, 3), (639, 199)] {
            let p = img.get(x, y).unwrap();
            let shown = r
                .palette
                .get(r.image.get(x as i32, y as i32).unwrap())
                .unwrap();
            assert_eq!(shown, Rgb8::new(p.r, p.g, p.b));
        }
    }

    #[test]
    fn big_photo_is_scaled_to_the_current_screen() {
        use dpx_core::{Resolution, Video};
        let img = photo(2000, 1500);
        // An OCS HiRes screen: 16 colours, 12-bit palette, tall pixels (1:2).
        let current = Target::new(
            Chipset::Ocs,
            DisplayMode::new(Video::Pal, Resolution::Hi, false),
        );
        let r = reduce_true_color(&img, "JPEG", current);
        assert_eq!(r.target, current);
        // 640 wide; 4:3 shown with 1:2 pixels → 480 display units = 240 lines.
        assert_eq!((r.image.width(), r.image.height()), (640, 240));
        assert_eq!(r.palette.len(), 16);
        for &c in r.palette.colors() {
            assert_eq!(Rgb8::from_rgb4(c.to_rgb4()), c, "OCS 12-bit entry");
        }
        assert!(r.notes[0].contains("2000x1500") && r.notes[0].contains("640x240"));
        assert!(
            r.notes[1].contains("reduced to 16 colours"),
            "{:?}",
            r.notes
        );
        assert_eq!(r.transparent_color, None);
    }

    #[test]
    fn transparent_png_opens_with_pink_colour_zero_and_saves_it_as_ilbm_transparency() {
        let dir = std::env::temp_dir().join(format!("dpx-files-tc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = rgba_image(48, 40, |x, y| {
            if x < 8 {
                dpx_core::Rgba8::new(0, 0, 0, 0)
            } else {
                dpx_core::Rgba8::opaque((x / 4 * 20) as u8, (y / 4 * 25) as u8, 99)
            }
        });
        let png_path = dir.join("sprite.png");
        write_rgba_png(&png_path, &img);
        let loaded = load(&png_path).unwrap();
        assert!(matches!(
            loaded.picture,
            Picture::TrueColor { format: "PNG", .. }
        ));
        let (doc, extras, notes) = open_picture(loaded.picture, Target::default());
        assert_eq!(extras.transparent_color, Some(0));
        assert_eq!(doc.palette().get(0), Some(TRANSPARENT_PINK));
        assert_eq!(doc.page().get(3, 3), Some(0));
        assert_ne!(doc.page().get(20, 3), Some(0));
        assert!(notes.iter().any(|n| n.contains("pink")), "{notes:?}");
        // 10 x 10 opaque colours fit into 255: kept exactly, no reduction note.
        assert!(!notes.iter().any(|n| n.contains("reduced")), "{notes:?}");

        let iff = dir.join("sprite.iff");
        let (bytes, _) = encode_document(&doc, &extras, &iff).unwrap();
        let back = ilbm::read(&bytes).unwrap();
        assert_eq!(
            (back.masking, back.transparent_color),
            (Masking::TransparentColor, 0)
        );
        assert!(info(&png_path).unwrap().contains("320 transparent pixels"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn jpeg_converts_to_ilbm_and_info_explains_it() {
        let dir = std::env::temp_dir().join(format!("dpx-files-jpg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jpg = dir.join("red-blue.jpg");
        std::fs::write(
            &jpg,
            include_bytes!("../../../crates/dpx-formats/tests/data/red-blue-16x8.jpg"),
        )
        .unwrap();
        let text = info(&jpg).unwrap();
        assert!(text.contains("JPEG true colour 16x8"), "{text}");
        assert!(text.contains("opens as: 16x8 NTSC:Low Res"), "{text}");
        let out = dir.join("red-blue.iff");
        convert(&jpg, &out).unwrap();
        let Picture::Ilbm(i) = load(&out).unwrap().picture else {
            panic!("expected ILBM");
        };
        assert_eq!((i.image.width(), i.image.height()), (16, 8));
        assert!(i.camg.is_some(), "written with its display mode");
        let left = i.palette.get(i.image.get(2, 2).unwrap()).unwrap();
        let right = i.palette.get(i.image.get(13, 5).unwrap()).unwrap();
        assert!(left.r > 200 && left.b < 60, "{left}");
        assert!(right.b > 200 && right.r < 60, "{right}");
        assert!(
            convert(&jpg, &dir.join("x.jpg"))
                .unwrap_err()
                .contains("import-only")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn overscan_pictures_open_on_overscan_screens() {
        let dir = std::env::temp_dir().join(format!("dpx-files-os-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A PAL LoRes overscan ILBM as DeluxePaint writes it: page fields = screen size.
        let mut img = IndexedImage::new(368, 283, 0);
        img.set(367, 282, 5);
        let mut src = Ilbm::from_indexed(img.clone(), Palette::dpaint_default_32());
        src.camg = Some(DisplayMode::PAL_LORES.mode_id());
        (src.page_width, src.page_height) = (368, 283);
        let (doc, _, _) = open_picture(Picture::Ilbm(Box::new(src.clone())), Target::default());
        assert_eq!(doc.target().mode, DisplayMode::PAL_LORES);
        assert_eq!(doc.target().screen, (368, 283));
        // Saving writes the screen size into the page fields again.
        let path = dir.join("overscan.iff");
        let (bytes, _) = encode_document(&doc, &PictureExtras::default(), &path).unwrap();
        let back = ilbm::read(&bytes).unwrap();
        assert_eq!((back.page_width, back.page_height), (368, 283));
        assert_eq!(back.image, img);

        // Page fields that are no valid screen: the screen comes from the picture size.
        (src.page_width, src.page_height) = (0, 0);
        let (doc, _, _) = open_picture(Picture::Ilbm(Box::new(src.clone())), Target::default());
        assert_eq!(doc.target().screen, (368, 283));
        // Standard pictures keep the standard screen; bigger pages scroll.
        let mut big = Ilbm::from_indexed(IndexedImage::new(640, 512, 0), src.palette.clone());
        big.camg = src.camg;
        (big.page_width, big.page_height) = (320, 256);
        let (doc, _, _) = open_picture(Picture::Ilbm(Box::new(big)), Target::default());
        assert_eq!(doc.target().screen, (320, 256));

        // An indexed PNG of 352x272: PAL LoRes with a custom 352x272 screen.
        let png_path = dir.join("custom.png");
        std::fs::write(
            &png_path,
            dpx_formats::write_png(&IndexedImage::new(352, 272, 1), &src.palette).unwrap(),
        )
        .unwrap();
        let (doc, _, _) = open_picture(load(&png_path).unwrap().picture, Target::default());
        assert_eq!(doc.target().mode, DisplayMode::PAL_LORES);
        assert_eq!(doc.target().screen, (352, 272));
        let _ = std::fs::remove_dir_all(dir);
    }
}
