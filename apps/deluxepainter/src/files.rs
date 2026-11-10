//! Loading and saving pictures by file type, plus the headless `info` and `convert` commands.
//!
//! Formats: IFF ILBM and PNG in and out, IFF ANIM in and out (ADR-32), JPEG in only. True-colour PNG and JPEG pictures are
//! reduced to an indexed page when they are opened (ADR-9). EHB pictures open as 64 free
//! colours and HAM pictures are decoded and reduced like true colour (ADR-26). Whatever a target format cannot
//! store is reported, never dropped silently.

use std::collections::HashSet;
use std::path::Path;

use dpx_core::quantize::{self, Dither, PaletteDepth, ReduceOptions, Reduction};
use dpx_core::resample;
use dpx_core::{Chipset, DisplayMode, Document, IndexedImage, Palette, Rgb8, RgbaImage, Target};
use dpx_formats::ham::{HamKind, decode_ham};
use dpx_formats::project::{self, Project, ProjectExtras, ProjectRange};
use dpx_iff::anim_doc::{self, AnimContent};
use dpx_iff::ilbm::CAMG_EHB;
use dpx_iff::{ColorRange, Compression, Ilbm, Masking, SpecialMode, ilbm};

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
    /// A `.dpxproj` project: document, extras and spare page, all lossless (ADR-23).
    Project(Box<Project>),
    /// An IFF ANIM; becomes a multi-frame document when it is opened (ADR-32).
    Anim(Box<AnimContent>),
}

impl Picture {
    /// Pixels and palette of an indexed picture; `None` for true colour.
    pub fn indexed(&self) -> Option<(&IndexedImage, &Palette)> {
        match self {
            Picture::Ilbm(i) => Some((&i.image, &i.palette)),
            Picture::Png { image, palette } => Some((image, palette)),
            Picture::Project(p) => Some((p.document.page(), p.document.palette())),
            Picture::Anim(a) => Some((&a.frames[0], &a.palette)),
            Picture::TrueColor { .. } => None,
        }
    }

    /// The ILBM display mode id, if the file has one.
    pub fn camg(&self) -> Option<u32> {
        match self {
            Picture::Ilbm(i) => i.camg,
            Picture::Anim(a) => a.camg,
            Picture::Png { .. } | Picture::TrueColor { .. } | Picture::Project(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ilbm,
    Png,
    Jpeg,
    Project,
    Anim,
}

impl From<&PictureExtras> for ProjectExtras {
    fn from(e: &PictureExtras) -> Self {
        Self {
            ranges: e
                .ranges
                .iter()
                .map(|r| ProjectRange {
                    rate: r.rate,
                    flags: r.flags,
                    low: r.low,
                    high: r.high,
                })
                .collect(),
            grab: e.grab,
            transparent_color: e.transparent_color,
        }
    }
}

impl From<&ProjectExtras> for PictureExtras {
    fn from(e: &ProjectExtras) -> Self {
        Self {
            ranges: e
                .ranges
                .iter()
                .map(|r| ColorRange {
                    rate: r.rate,
                    flags: r.flags,
                    low: r.low,
                    high: r.high,
                })
                .collect(),
            grab: e.grab,
            transparent_color: e.transparent_color,
        }
    }
}

fn kind_of_bytes(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(b"FORM") {
        if bytes.get(8..12) == Some(b"ANIM") {
            Some(Kind::Anim)
        } else {
            Some(Kind::Ilbm)
        }
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Kind::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Kind::Jpeg)
    } else if project::is_zip(bytes) {
        Some(Kind::Project)
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
        Some("dpxproj") => Ok(Kind::Project),
        Some("anim") => Ok(Kind::Anim),
        Some("jpg" | "jpeg") => Err(format!(
            "{}: JPEG is import-only (DeluxePainter writes .iff/.ilbm/.lbm, .png, .anim and .dpxproj)",
            path.display()
        )),
        _ => Err(format!(
            "{}: unsupported output type (DeluxePainter writes .iff/.ilbm/.lbm, .png, .anim and .dpxproj)",
            path.display()
        )),
    }
}

/// Loads a picture file. Errors read `<path>: <short line>` followed by indented detail lines
/// (ADR-17): the short line (at most 34 characters) fits the file requester's message line,
/// the whole text is meant for stderr and the CLI.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|e| with_path(path, read_error(&e)))?;
    match kind_of_bytes(&bytes) {
        Some(Kind::Anim) => {
            let (anim, mut notes) =
                dpx_iff::anim::read_with_notes(&bytes).map_err(|e| with_path(path, e))?;
            let content = anim_doc::content_of(&anim);
            notes.extend(content.notes.iter().cloned());
            Ok(Loaded {
                picture: Picture::Anim(Box::new(AnimContent {
                    notes: Vec::new(),
                    ..content
                })),
                notes,
            })
        }
        Some(Kind::Ilbm) => {
            let (mut pic, mut notes) =
                ilbm::read_with_notes(&bytes).map_err(|e| with_path(path, e))?;
            match pic.special {
                None => {}
                Some(SpecialMode::Ehb) => ehb_to_indexed(&mut pic, &mut notes),
                Some(mode) => {
                    let kind = if mode == SpecialMode::Ham6 {
                        HamKind::Ham6
                    } else {
                        HamKind::Ham8
                    };
                    let image = decode_ham(kind, &pic.image, pic.palette.colors());
                    if !pic.ranges.is_empty() {
                        notes.push(format!(
                            "{} colour-cycling ranges (CRNG) not kept",
                            pic.ranges.len()
                        ));
                    }
                    if pic.grab.is_some() {
                        notes.push("GRAB point not kept".to_string());
                    }
                    match pic.masking {
                        Masking::TransparentColor => notes.push(format!(
                            "transparent colour {} not kept",
                            pic.transparent_color
                        )),
                        Masking::HasMask => {
                            notes.push("mask plane not kept (pages are opaque)".to_string());
                        }
                        Masking::None | Masking::Lasso => {}
                    }
                    for c in &pic.other_chunks {
                        notes.push(format!(
                            "{} chunk ({} bytes) not kept",
                            dpx_iff::iff::id_str(c.id),
                            c.data.len()
                        ));
                    }
                    return Ok(Loaded {
                        picture: Picture::TrueColor {
                            image,
                            format: mode.name(),
                        },
                        notes,
                    });
                }
            }
            Ok(Loaded {
                picture: Picture::Ilbm(Box::new(pic)),
                notes,
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
                Err(e) => return Err(with_path(path, e)),
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
            let image = dpx_formats::read_jpeg(&bytes).map_err(|e| with_path(path, e))?;
            Ok(Loaded {
                picture: Picture::TrueColor {
                    image,
                    format: "JPEG",
                },
                notes: Vec::new(),
            })
        }
        Some(Kind::Project) => {
            let project = project::read_project(&bytes).map_err(|e| with_path(path, e))?;
            Ok(Loaded {
                picture: Picture::Project(Box::new(project)),
                notes: Vec::new(),
            })
        }
        None => Err(with_path(path, unknown_type(&bytes))),
    }
}

/// Opens an Extra-Half-Brite ILBM as a plain 64-colour indexed picture: the palette becomes the
/// 32 base colours plus their half-bright copies, the pixels stay exactly as they are, and the
/// EHB flag leaves CAMG, so saving writes a 64-colour 6-plane ILBM (ADR-26). Colours 32-63 are
/// derived with OCS precision (4 bit) when every base colour is a 12-bit value, else 8 bit.
fn ehb_to_indexed(pic: &mut Ilbm, notes: &mut Vec<String>) {
    let base = pic.palette.colors().to_vec();
    let is_12_bit = |c: &Rgb8| [c.r, c.g, c.b].iter().all(|&v| v & 0x0F == v >> 4);
    let depth = if pic.cmap_4bit_fixed || base.iter().all(is_12_bit) {
        PaletteDepth::Rgb4
    } else {
        PaletteDepth::Rgb8
    };
    pic.palette =
        Palette::new(dpx_formats::ehb::ehb_palette(&base, depth).to_vec()).expect("64 entries");
    pic.special = None;
    // A plain mode id marks 64 colours; without any CAMG the writer's output would read as HAM.
    pic.camg = Some(pic.camg.unwrap_or(0) & !CAMG_EHB);
    notes.push(
        "Extra-Half-Brite picture: edited as 64 free colours (32-63 are the half-bright copies) \
         until EHB editing exists; saving writes a 64-colour 6-plane ILBM, not EHB"
            .into(),
    );
}

/// `<path>: <first line>` with every further line indented by two spaces.
fn with_path(path: &Path, e: impl std::fmt::Display) -> String {
    let text = e.to_string();
    let mut lines = text.lines();
    let mut out = format!("{}: {}", path.display(), lines.next().unwrap_or_default());
    for l in lines {
        out += "\n  ";
        out += l;
    }
    out
}

/// A file that cannot be read at all: short line plus the system's reason.
fn read_error(e: &std::io::Error) -> String {
    let summary = match e.kind() {
        std::io::ErrorKind::NotFound => "File not found",
        std::io::ErrorKind::PermissionDenied => "No permission to read the file",
        std::io::ErrorKind::IsADirectory => "That is a drawer",
        _ => "Cannot read the file",
    };
    format!("{summary}\n{e}")
}

/// Names what a file is when it is none of the picture formats DeluxePainter reads.
fn unknown_type(bytes: &[u8]) -> String {
    let start = |magic: &[u8]| bytes.starts_with(magic);
    let (summary, what) = if bytes.is_empty() {
        return "File is empty\nThe file has 0 bytes.".into();
    } else if start(b"GIF8") {
        ("GIF pictures cannot be loaded", "a GIF picture")
    } else if start(b"BM") && bytes.len() >= 14 {
        ("BMP pictures cannot be loaded", "a BMP picture")
    } else if start(b"II*\0") || start(b"MM\0*") {
        ("TIFF pictures cannot be loaded", "a TIFF picture")
    } else if start(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        ("WebP pictures cannot be loaded", "a WebP picture")
    } else if start(b"PP20") {
        (
            "PowerPacker file: unpack it first",
            "packed with PowerPacker; decrunch it and load the result",
        )
    } else if start(b"XPKF") {
        (
            "XPK-packed file: unpack it first",
            "packed with XPK; unpack it and load the result",
        )
    } else if bytes.get(2..5) == Some(b"-lh") || start(b"LZX") || start(b"PK\x03\x04") {
        (
            "Archive: unpack it first",
            "an archive (LhA, LZX or ZIP); unpack it and load the picture inside",
        )
    } else if start(&[0x00, 0x00, 0x03, 0xF3]) {
        ("Amiga program, not a picture", "an Amiga program")
    } else if start(b"LIST") || start(b"CAT ") {
        // IFF containers of several FORMs: dpx-iff names them.
        return dpx_iff::iff::IffError::NotIff(bytes[..4].try_into().expect("4 bytes")).to_string();
    } else {
        ("Unknown file type", "")
    };
    let head: Vec<String> = bytes.iter().take(4).map(|b| format!("{b:02X}")).collect();
    let is = if what.is_empty() {
        format!("this file starts with {}", head.join(" "))
    } else {
        format!("this file is {what}")
    };
    format!("{summary}\nDeluxePainter reads IFF ILBM, PNG and JPEG pictures; {is}.")
}

/// Writes `picture` in the format given by the file extension; returns notes about metadata
/// the target format cannot hold.
/// True-colour pictures must be opened (reduced) first; see [`convert`].
pub fn save(picture: &Picture, path: &Path) -> Result<Vec<String>, String> {
    let kind = kind_of_path(path)?;
    let (image, palette) = picture
        .indexed()
        .ok_or_else(|| "true-colour picture must be reduced before saving".to_string())?;
    if matches!(picture, Picture::Project(_) | Picture::Anim(_)) && kind != Kind::Project {
        return Err("a project is exported through the document: use convert".into());
    }
    let mut notes = Vec::new();
    let bytes = match kind {
        Kind::Project => match picture {
            Picture::Project(p) => project::write_project(p).map_err(|e| e.to_string())?,
            _ => return Err("only a project is saved as is: use convert".into()),
        },
        Kind::Anim => {
            return Err("an animation is written through the document: use convert".into());
        }
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
    write_atomic(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
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

impl PictureExtras {
    /// Extras of a new picture: DeluxePaint-like default ranges for its palette.
    pub fn for_new_picture(palette: &Palette) -> Self {
        Self {
            ranges: default_ranges(palette),
            ..Self::default()
        }
    }
}

/// Default colour ranges of a new picture (ADR-22). The manual only says that Palette ▸
/// Default Palette restores "the startup palette and default ranges" without listing them,
/// so these are our choice for the 32-colour default palette: range 1 the grey ramp
/// (entries 20–27), range 2 the colour sweep (entries 8–15), both inactive at 15 steps per
/// second, so Tab changes nothing until a range is switched on. Palettes of fewer than
/// 32 colours get none.
pub fn default_ranges(palette: &Palette) -> Vec<ColorRange> {
    if palette.len() < 32 {
        return Vec::new();
    }
    let range = |low, high| ColorRange {
        rate: 4096,
        flags: 0,
        low,
        high,
    };
    vec![range(20, 27), range(8, 15)]
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
    let ham = format.starts_with("HAM");
    match r.reduction {
        Reduction::Exact if ham => notes.push(format!(
            "{format} picture opened as an indexed picture with its {} colours; HAM export comes later",
            r.colors
        )),
        Reduction::Exact => {}
        Reduction::Rounded => {
            notes.push(format!("{} colours rounded to 12-bit OCS values", r.colors));
            if ham {
                notes.push(format!("{format} picture is indexed now; HAM export comes later"));
            }
        }
        Reduction::Quantized if ham => notes.push(format!(
            "{format} picture reduced to {} colours (Floyd-Steinberg dithering); HAM export comes later",
            r.colors
        )),
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
    if let Picture::Project(p) = &picture {
        // Lossless: the document and extras come back as saved (the spare page is not
        // part of the app's state yet).
        let mut notes = Vec::new();
        if p.spare.is_some() {
            notes.push("spare page not kept (not supported yet)".to_string());
        }
        return (p.document.clone(), PictureExtras::from(&p.extras), notes);
    }
    if let Picture::Anim(a) = picture {
        return open_anim(*a);
    }
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
        Picture::TrueColor { .. } | Picture::Project(_) | Picture::Anim(_) => {
            unreachable!("handled above")
        }
    };
    (
        Document::from_picture(image, palette, target),
        extras,
        notes,
    )
}

/// An ANIM as a multi-frame document: display mode from CAMG and size like an ILBM picture,
/// the screen from the picture size (the file has no page fields), current frame 1 (ADR-32).
fn open_anim(a: AnimContent) -> (Document, PictureExtras, Vec<String>) {
    let (w, h) = (a.frames[0].width(), a.frames[0].height());
    let mode = DisplayMode::guess(a.camg, w, h);
    let (sw, sh) = mode.screen_for_picture(w, h);
    let target = Target::new(Target::default().chipset, mode).with_screen(sw, sh);
    let extras = PictureExtras {
        ranges: a.ranges,
        ..PictureExtras::default()
    };
    let first = a.frames[0].clone();
    match Document::from_frames(a.frames, 0, a.fps, a.palette.clone(), target) {
        Ok(doc) => (doc, extras, Vec::new()),
        // More frames than a document holds (ADR-28 limits): open the first frame only.
        Err(e) => (
            Document::from_picture(first, a.palette, target),
            extras,
            vec![format!(
                "only the first frame is opened: {}",
                e.message.trim_end_matches('.')
            )],
        ),
    }
}

/// DeluxePaint pages are never smaller than the screen (ADR-10): a smaller picture is placed
/// top-left on a screen-sized page of colour 0 (the background, transparent in brushes).
/// Returns a note when the page grew. Used when the GUI opens a picture; `convert` keeps the
/// picture's own size.
pub fn page_at_least_screen(doc: Document) -> (Document, Option<String>) {
    let (sw, sh) = doc.target().screen;
    if !doc.target().mode.screen_fits(sw, sh) {
        let note = format!("page not enlarged: {sw}x{sh} is no screen of this mode");
        return (doc, Some(note));
    }
    let (pw, ph) = (doc.page().width(), doc.page().height());
    if pw >= sw && ph >= sh {
        return (doc, None);
    }
    let (w, h) = (pw.max(sw), ph.max(sh));
    let note = format!("page enlarged from {pw}x{ph} to the {w}x{h} screen, picture top-left");
    // Every animation frame grows the same way (ADR-28).
    let frames: Vec<IndexedImage> = doc
        .frames()
        .map(|f| {
            let mut page = IndexedImage::new(w, h, 0);
            page.blit(f, 0, 0);
            page
        })
        .collect();
    let grown = Document::from_frames(
        frames,
        doc.current_frame(),
        doc.fps(),
        doc.palette().clone(),
        doc.target(),
    );
    match grown {
        Ok(grown) => (grown, Some(note)),
        // Too many frames for the bigger page: keep the picture as it is.
        Err(e) => (
            doc,
            Some(format!("page not enlarged to the screen: {}", e.message)),
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFormat {
    Ilbm,
    Png,
    /// The lossless project (`.dpxproj`, ADR-23).
    Project,
    /// IFF ANIM (`.anim`, ADR-32).
    Anim,
}

/// Picture ▸ Save picks the format by name: `.png` writes PNG, `.dpxproj` the lossless
/// project, `.anim` an IFF ANIM; everything else writes IFF ILBM, because Amiga names often have no extension or tags like `.LoRes`.
pub fn save_format(path: &Path) -> Result<SaveFormat, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("png") => Ok(SaveFormat::Png),
        Some("dpxproj") => Ok(SaveFormat::Project),
        Some("anim") => Ok(SaveFormat::Anim),
        Some("jpg" | "jpeg") => Err("JPEG is import-only: use .iff or .png".into()),
        Some("gif" | "bmp" | "tif" | "tiff" | "webp") => {
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
    let frames = doc.frame_count();
    if frames > 1 && !matches!(save_format(path)?, SaveFormat::Project | SaveFormat::Anim) {
        // ADR-28: a picture format holds one frame; .anim writes all of them (ADR-32).
        notes.push(format!(
            "frame {} of {frames} exported as a picture (save as .anim for all frames)",
            doc.current_frame() + 1
        ));
    }
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
        SaveFormat::Anim => {
            let mode = doc.target().mode;
            let (xa, ya) = mode.aspect();
            let (w, h) = (doc.page().width(), doc.page().height());
            let images: Vec<IndexedImage> = doc.frames().cloned().collect();
            let fps = doc.fps();
            let opts = dpx_iff::anim::EncodeOptions {
                hold: dpx_iff::anim::Hold::PerFrame(anim_doc::holds_for_fps(fps, images.len())),
                ranges,
                camg: Some(doc.target().camg()),
                x_aspect: xa,
                y_aspect: ya,
                ..Default::default()
            };
            if doc.target().screen != mode.screen_for_picture(w, h) {
                notes.push("ANIM cannot store the screen size, it opens as the picture's".into());
            }
            if let Some(c) = transparent {
                notes.push(format!("ANIM output cannot store: transparent colour {c}"));
            }
            if extras.grab.is_some() {
                notes.push("ANIM output cannot store: grab point".into());
            }
            let bytes =
                dpx_iff::anim::encode(&images, doc.palette(), &opts).map_err(|e| e.to_string())?;
            Ok((bytes, notes))
        }
        SaveFormat::Project => {
            // Lossless: nothing is dropped, even ranges outside the palette stay.
            let project = Project::new(doc.clone(), ProjectExtras::from(extras));
            let bytes = project::write_project(&project).map_err(|e| e.to_string())?;
            Ok((bytes, Vec::new()))
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
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    path.file_name().ok_or_else(|| "no file name".to_string())?;
    // A short name, so a target name at the file system's limit still saves.
    let tmp = path.with_file_name(format!(
        ".dpx-{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&tmp, path));
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
        let mut text = true_color_info(&name.to_string(), image, format);
        for n in &loaded.notes {
            text += &format!("  note: {n}\n");
        }
        return Ok(text);
    }
    let (img, _) = loaded.picture.indexed().expect("indexed picture");
    match &loaded.picture {
        Picture::Anim(a) => {
            let (_, _, opened) = open_picture(Picture::Anim(a.clone()), Target::default());
            let n = a.frames.len();
            out += &format!(
                "{name}: IFF ANIM {}x{}, {} colours, {n} frame{}, {} per second\n",
                img.width(),
                img.height(),
                a.palette.len(),
                if n == 1 { "" } else { "s" },
                a.fps
            );
            for (i, r) in a.ranges.iter().enumerate() {
                out += &format!(
                    "  CRNG {i}: colours {}-{}, {:.1} steps/s{}{}\n",
                    r.low,
                    r.high,
                    r.steps_per_second(),
                    if r.active() { ", active" } else { "" },
                    if r.reverse() { ", reverse" } else { "" }
                );
            }
            for o in opened {
                out += &format!("  note: {o}\n");
            }
        }
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
        Picture::Project(p) => {
            let doc = &p.document;
            let t = doc.target();
            out += &format!(
                "{name}: DeluxePainter project (dpxproj format {}) {}x{}, {} colours\n",
                project::format_version_for(doc),
                img.width(),
                img.height(),
                doc.palette().len()
            );
            out += &format!(
                "  target: {:?} {}, screen {}x{}\n  {}, 1 layer{}\n",
                t.chipset,
                t.mode.name(),
                t.screen.0,
                t.screen.1,
                match doc.frame_count() {
                    1 => "1 frame".to_string(),
                    n => format!(
                        "{n} frames (frame {} current, {} per second)",
                        doc.current_frame() + 1,
                        doc.fps()
                    ),
                },
                match &p.spare {
                    Some(s) => format!(", spare page {}x{}", s.width(), s.height()),
                    None => String::new(),
                }
            );
            for (n, r) in p.extras.ranges.iter().enumerate() {
                let r = ColorRange {
                    rate: r.rate,
                    flags: r.flags,
                    low: r.low,
                    high: r.high,
                };
                out += &format!(
                    "  range {n}: colours {}-{}, {:.1} steps/s{}{}\n",
                    r.low,
                    r.high,
                    r.steps_per_second(),
                    if r.active() { ", active" } else { "" },
                    if r.reverse() { ", reverse" } else { "" }
                );
            }
            if let Some((x, y)) = p.extras.grab {
                out += &format!("  grab: {x},{y}\n");
            }
            if let Some(c) = p.extras.transparent_color {
                out += &format!("  transparent colour: {c}\n");
            }
        }
        Picture::TrueColor { .. } => unreachable!("handled above"),
    }
    let used = {
        let mut seen = [false; 256];
        let frames: Vec<&IndexedImage> = match &loaded.picture {
            Picture::Anim(a) => a.frames.iter().collect(),
            Picture::Project(p) => p.document.frames().collect(),
            _ => vec![img],
        };
        for &p in frames.iter().flat_map(|f| f.pixels()) {
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
    let kind = if format.starts_with("HAM") {
        format!("IFF ILBM {format} (decoded to RGB)")
    } else {
        format!("{format} true colour")
    };
    let mut out = format!(
        "{name}: {kind} {}x{}, {} colours",
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
    let out_kind = kind_of_path(output)?;
    let loaded = load(input)?;
    let mut notes = loaded.notes;
    // True colour is reduced, and projects are written from or read into a document; ILBM and
    // PNG pictures otherwise convert directly so ILBM → ILBM keeps every chunk.
    let via_document = matches!(
        (&loaded.picture, out_kind),
        (Picture::TrueColor { .. }, _)
            | (Picture::Project(_) | Picture::Anim(_), _)
            | (_, Kind::Project | Kind::Anim)
    );
    if via_document {
        let (doc, extras, opened) = open_picture(loaded.picture, Target::default());
        notes.extend(opened);
        let (bytes, saved) = encode_document(&doc, &extras, output)?;
        write_atomic(output, &bytes).map_err(|e| format!("{}: {e}", output.display()))?;
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

    /// Writes `bytes` to a file of its own drawer and loads it; returns the error text with
    /// the path replaced by `<file>`.
    fn load_error(name: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("dpx-files-diag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let e = match load(&path) {
            Ok(_) => panic!("{name}: expected an error"),
            Err(e) => e,
        };
        let _ = std::fs::remove_file(&path);
        e.replace(&path.display().to_string(), "<file>")
    }

    /// The part of a load error the file requester shows: the first line without the path.
    fn requester_line(e: &str) -> &str {
        let first = e.lines().next().unwrap();
        first.strip_prefix("<file>: ").unwrap_or(first)
    }

    #[test]
    fn broken_and_foreign_files_get_short_first_lines_and_details() {
        let fixture = |name: &str| {
            std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../crates/dpx-iff/tests/data")
                    .join(name),
            )
            .unwrap()
        };
        let png = {
            let pal = Palette::new(vec![Rgb8::BLACK, Rgb8::WHITE]).unwrap();
            let good = dpx_formats::write_png(&IndexedImage::new(8, 8, 1), &pal).unwrap();
            good[..good.len() - 20].to_vec()
        };
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "empty.iff",
                vec![],
                "<file>: File is empty\n  The file has 0 bytes.",
            ),
            (
                "pic.gif",
                b"GIF89a\x01\x00\x01\x00".to_vec(),
                "<file>: GIF pictures cannot be loaded\n  DeluxePainter reads IFF ILBM, PNG \
                 and JPEG pictures; this file is a GIF picture.",
            ),
            (
                "crunched",
                b"PP20\x09\x0a\x0a\x0a".to_vec(),
                "<file>: PowerPacker file: unpack it first\n  DeluxePainter reads IFF ILBM, \
                 PNG and JPEG pictures; this file is packed with PowerPacker; decrunch it and \
                 load the result.",
            ),
            (
                "notes.txt",
                b"hello".to_vec(),
                "<file>: Unknown file type\n  DeluxePainter reads IFF ILBM, PNG and JPEG \
                 pictures; this file starts with 68 65 6C 6C.",
            ),
            (
                "group.iff",
                b"LIST\0\0\0\x04ILBM".to_vec(),
                "<file>: IFF LIST files: not supported yet\n  The file is an IFF LIST (several \
                 FORMs in one file). DeluxePainter reads a single FORM ILBM.",
            ),
            (
                "run.iff",
                fixture("broken-bad-run.iff"),
                "<file>: BODY: bad ByteRun1 run, row 1\n  At offset 65 ($41) a ByteRun1 \
                 repeat of 4 bytes does not fit: plane 0 of row 1 has 2 of its 2 bytes left. \
                 The BODY is corrupt (or packed across rows, which ILBM does not allow).",
            ),
            (
                "cut.png",
                png,
                "<file>: PNG file is cut off\n  The PNG decoder reports: unexpected end of file",
            ),
            (
                "cut.jpg",
                include_bytes!("../../../crates/dpx-formats/tests/data/red-blue-16x8.jpg")[..40]
                    .to_vec(),
                "<file>: JPEG file is cut off\n  The JPEG decoder reports: I/O errors Not \
                 enough bytes, expected 64 but found 15",
            ),
        ];
        // The file requester's message line is 264 - 12 = 252 pixels of the UI font wide.
        let font = dpx_ui::font::Font::builtin();
        for (name, bytes, want) in cases {
            let e = load_error(name, &bytes);
            assert_eq!(e, want, "{name}");
            let line = requester_line(&e);
            assert!(line.chars().count() <= 34, "{name}: {line}");
            assert!(font.text_width(line) <= 252, "{name}: {line}");
        }
        let Err(missing) = load(Path::new("/nonexistent-dpx-drawer/pic.iff")) else {
            panic!("expected an error");
        };
        assert!(
            missing.starts_with("/nonexistent-dpx-drawer/pic.iff: File not found\n  "),
            "{missing}"
        );
    }

    #[test]
    fn tolerated_ilbm_quirks_reach_the_notes_and_info() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/dpx-iff/tests/data/broken-body-cut.iff");
        let loaded = load(&path).unwrap();
        let note = "BODY ends in row 2 of 4, plane 0 (the file ends inside the BODY, 9 of 16 \
                    bytes): the rest of the picture is colour 0";
        assert_eq!(loaded.notes, [note]);
        let (img, _) = loaded.picture.indexed().unwrap();
        assert!(img.pixels()[32..].iter().all(|&c| c == 0));
        assert!(info(&path).unwrap().contains(&format!("  note: {note}\n")));
    }

    fn iff_fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/dpx-iff/tests/data")
            .join(name)
    }

    #[test]
    fn ehb_opens_as_64_free_colours_with_exact_indices_and_saves_as_64_colour_ilbm() {
        let path = iff_fixture("ehb-16x2.iff");
        let raw = ilbm::read(&std::fs::read(&path).unwrap()).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.notes.len(), 1, "{:?}", loaded.notes);
        assert!(
            loaded.notes[0].starts_with("Extra-Half-Brite picture: edited as 64 free colours")
                && loaded.notes[0].contains("saving writes a 64-colour 6-plane ILBM, not EHB"),
            "{:?}",
            loaded.notes
        );
        let (img, pal) = loaded.picture.indexed().unwrap();
        assert_eq!(img, &raw.image, "indices are exact");
        assert_eq!(pal.len(), 64);
        assert!(img.pixels().iter().any(|&p| p >= 32));
        for i in 0..32 {
            let c = pal.colors()[i];
            assert_eq!(pal.colors()[i], raw.palette.colors()[i]);
            assert_eq!(
                pal.colors()[i + 32],
                Rgb8::new(c.r >> 1, c.g >> 1, c.b >> 1),
                "entry {}",
                i + 32
            );
        }
        // Opening gives an AGA document with 64 colours; saving validates and writes 6 planes
        // without the EHB flag, and reading that gives the same picture again.
        let (doc, extras, _) = open_picture(loaded.picture, Target::default());
        assert_eq!(doc.palette().len(), 64);
        assert_eq!(doc.page().pixels(), raw.image.pixels());
        let out = tmp("ehb-saved.iff");
        let (bytes, notes) = encode_document(&doc, &extras, &out).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let (back, back_notes) = ilbm::read_with_notes(&bytes).unwrap();
        assert!(back_notes.is_empty(), "{back_notes:?}");
        assert_eq!(
            (back.planes, back.special, back.palette.len()),
            (6, None, 64)
        );
        assert_eq!(back.camg.unwrap() & 0x80, 0);
        assert_eq!(&back.image, doc.page());
        assert_eq!(&back.palette, doc.palette());
        // ILBM -> ILBM conversion keeps it a plain 64-colour picture too.
        let notes = convert(&path, &out).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        let again = ilbm::read(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!((again.special, again.planes), (None, 6));
        assert_eq!(again.image, raw.image);
        assert_eq!(again.palette.len(), 64);
        let _ = std::fs::remove_file(out);
    }

    #[test]
    fn ham6_and_ham8_open_decoded_with_notes() {
        let px = |p: &Picture| match p {
            Picture::TrueColor { image, format } => (image.clone(), *format),
            _ => panic!("expected a decoded picture"),
        };
        let c = |r, g, b| dpx_core::Rgba8::opaque(r, g, b);
        // Palette entries of the fixtures: colour i = (37 i, 91 i + 5, 255 - i) as bytes.
        let loaded = load(&iff_fixture("ham6-16x2.iff")).unwrap();
        assert!(loaded.notes.is_empty(), "{:?}", loaded.notes);
        let (img, format) = px(&loaded.picture);
        assert_eq!(format, "HAM6");
        assert_eq!((img.width(), img.height()), (16, 2));
        let p = img.pixels();
        // set 1; blue = F; red = A (0xAA); green = 0; row 1: red = F from colour 0.
        assert_eq!(
            [p[0], p[1], p[2], p[3]],
            [
                c(37, 96, 254),
                c(37, 96, 255),
                c(170, 96, 255),
                c(170, 0, 255)
            ]
        );
        assert_eq!(p[16], c(255, 5, 255));
        let (doc, _, notes) = open_picture(loaded.picture, Target::default());
        assert_eq!(doc.page().width(), 16);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].starts_with("HAM6 picture opened as an indexed picture with its ")
                && notes[0].ends_with(" colours; HAM export comes later"),
            "{notes:?}"
        );

        let loaded = load(&iff_fixture("ham8-16x2.iff")).unwrap();
        let (img, format) = px(&loaded.picture);
        assert_eq!(format, "HAM8");
        let p = img.pixels();
        // set 2; blue = 0x10 over the kept low bits; red = 0x20; green = 0; row 1 red = 0x3F.
        assert_eq!(
            [p[0], p[1], p[2], p[3]],
            [
                c(74, 187, 253),
                c(74, 187, 65),
                c(130, 187, 65),
                c(130, 3, 65)
            ]
        );
        assert_eq!(p[16], c(252, 5, 255));
        let (_, _, notes) = open_picture(loaded.picture, Target::default());
        assert!(notes[0].starts_with("HAM8 picture"), "{notes:?}");
    }

    #[test]
    fn ham_with_many_colours_is_reduced_with_a_note_and_info_names_the_mode() {
        // 64x64 HAM6 file made by patching the CAMG of a plain 6-plane file (the writer does
        // not write HAM): thousands of colours, far more than the 256 of the default screen.
        let mut img = IndexedImage::new(64, 64, 0);
        for y in 0..64 {
            for x in 0..64 {
                img.set(x, y, ((x * 5 + y * 11 + x * y) % 64) as u8);
            }
        }
        let pal = Palette::new((0..64).map(|i| Rgb8::new(i, 255 - i, i * 2)).collect()).unwrap();
        let mut src = Ilbm::from_indexed(img, pal);
        src.camg = Some(0);
        let mut bytes = ilbm::write(&src, Compression::None).unwrap();
        let at = bytes.windows(4).position(|w| w == b"CAMG").unwrap() + 8;
        bytes[at..at + 4].copy_from_slice(&0x800u32.to_be_bytes());
        let path = tmp("many-ham6.iff");
        std::fs::write(&path, &bytes).unwrap();
        let loaded = load(&path).unwrap();
        assert!(matches!(
            loaded.picture,
            Picture::TrueColor { format: "HAM6", .. }
        ));
        let (_, _, notes) = open_picture(loaded.picture, Target::default());
        assert_eq!(
            notes,
            [
                "HAM6 picture reduced to 256 colours (Floyd-Steinberg dithering); HAM export comes later"
            ]
        );
        let text = info(&path).unwrap();
        assert!(
            text.contains("IFF ILBM HAM6 (decoded to RGB) 64x64"),
            "{text}"
        );
        assert!(
            text.contains("note: HAM6 picture reduced to 256 colours"),
            "{text}"
        );
        let _ = std::fs::remove_file(path);
    }

    /// A 3-frame 320x256 document at `fps` with different pixels in every frame.
    fn three_frames(fps: u8) -> Document {
        let frames: Vec<IndexedImage> = (0..3)
            .map(|i| {
                let mut f = IndexedImage::new(320, 256, 2 + i);
                f.set(10 + i32::from(i), 20, 31);
                f.set(319, 255, 7);
                f
            })
            .collect();
        Document::from_frames(
            frames,
            0,
            fps,
            Palette::dpaint_default_32(),
            Target::default(),
        )
        .unwrap()
    }

    #[test]
    fn anim_files_convert_both_ways_and_describe_themselves() {
        let doc = three_frames(20);
        let extras = PictureExtras {
            ranges: vec![ColorRange {
                rate: 8192,
                flags: 1,
                low: 4,
                high: 9,
            }],
            ..PictureExtras::default()
        };
        let a = tmp("walk-a.anim");
        let (bytes, notes) = encode_document(&doc, &extras, &a).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        std::fs::write(&a, &bytes).unwrap();
        assert_eq!(&bytes[8..12], b"ANIM");

        // info describes frames, size, colours, rate and the range.
        let info = info(&a).unwrap();
        assert!(
            info.contains("IFF ANIM 320x256, 32 colours, 3 frames, 20 per second"),
            "{info}"
        );
        assert!(info.contains("CRNG 0: colours 4-9"), "{info}");
        assert!(info.contains("colours used: 5"), "{info}");
        assert!(!info.contains("note:"), "{info}");

        // anim -> dpxproj -> anim: frames, palette, rate and ranges survive; bytes are equal.
        let proj = tmp("walk-b.dpxproj");
        let notes = convert(&a, &proj).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let (back, back_extras, _) = open_picture(load(&proj).unwrap().picture, Target::default());
        assert!(back.frames().eq(doc.frames()));
        assert_eq!(back.palette(), doc.palette());
        assert_eq!((back.fps(), back.frame_count()), (20, 3));
        assert_eq!(back_extras.ranges, extras.ranges);
        let c = tmp("walk-c.anim");
        assert!(convert(&proj, &c).unwrap().is_empty());
        assert_eq!(std::fs::read(&c).unwrap(), bytes, "deterministic");

        // anim -> iff keeps the first frame and says so; anim -> png too.
        let iff = tmp("walk-d.iff");
        let notes = convert(&a, &iff).unwrap();
        assert!(
            notes.iter().any(|n| n.contains("frame 1 of 3 exported")),
            "{notes:?}"
        );
        let first = load(&iff).unwrap().picture;
        assert_eq!(first.indexed().unwrap().0, doc.page());
        // iff -> anim gives a one-frame animation.
        let one = tmp("walk-e.anim");
        convert(&iff, &one).unwrap();
        let text = info_of(&one);
        assert!(text.contains("1 frame, "), "{text}");
        for f in [&a, &proj, &c, &iff, &one] {
            let _ = std::fs::remove_file(f);
        }
    }

    fn info_of(path: &Path) -> String {
        info(path).unwrap()
    }

    #[test]
    fn anim_timing_and_palette_changes_are_noted() {
        use dpx_iff::anim::{self, EncodeOptions, Hold};
        let doc = three_frames(30);
        let frames: Vec<IndexedImage> = doc.frames().cloned().collect();
        let encode = |hold: Hold| {
            let opts = EncodeOptions {
                hold,
                ..EncodeOptions::default()
            };
            let path = tmp(&format!("timing-{}.anim", std::process::id()));
            std::fs::write(&path, anim::encode(&frames, doc.palette(), &opts).unwrap()).unwrap();
            let loaded = load(&path).unwrap();
            let _ = std::fs::remove_file(&path);
            let (d, _, _) = open_picture(loaded.picture, Target::default());
            (d, loaded.notes)
        };
        // 30 per second is 2 jiffies per frame: no note.
        let (d, notes) = encode(Hold::Uniform(2));
        assert_eq!((d.fps(), notes.len()), (30, 0), "{notes:?}");
        // Whole-jiffy rounding by one is no difference worth a note.
        let (d, notes) = encode(Hold::PerFrame(vec![2, 3, 2]));
        assert_eq!(d.fps(), 26, "{notes:?}");
        assert!(notes.iter().all(|n| !n.contains("differs")), "{notes:?}");
        // Different holds: the average rate and a note.
        let (d, notes) = encode(Hold::PerFrame(vec![6, 6, 30]));
        assert_eq!(d.fps(), 4);
        assert!(
            notes.iter().any(|n| n.contains("frame timing differs")),
            "{notes:?}"
        );
        // Slower than 1 per second is clamped to the slowest Set Rate value.
        let (d, _) = encode(Hold::Uniform(240));
        assert_eq!(d.fps(), 1);
    }

    #[test]
    fn a_broken_or_cut_anim_is_refused_with_a_short_line() {
        let doc = three_frames(30);
        let frames: Vec<IndexedImage> = doc.frames().cloned().collect();
        let good = dpx_iff::anim::encode(
            &frames,
            doc.palette(),
            &dpx_iff::anim::EncodeOptions::default(),
        )
        .unwrap();
        // Nothing after the FORM header.
        let e = load_error("empty.anim", b"FORM\0\0\0\x04ANIM");
        assert!(e.starts_with("<file>: "), "{e}");
        assert!(requester_line(&e).len() <= 34 + 8, "{e}");
        // Cut inside the first picture: no frame to show.
        let e = load_error("cut.anim", &good[..300]);
        assert!(e.starts_with("<file>: "), "{e}");
        assert!(requester_line(&e).chars().count() <= 34, "{e}");
        // A damaged delta: the error names the frame.
        let mut bad = good.clone();
        let at = bad.windows(4).position(|w| w == b"DLTA").unwrap();
        for b in &mut bad[at + 8..at + 8 + 64] {
            *b = 0xFF;
        }
        let e = load_error("bad.anim", &bad);
        assert!(!e.is_empty());
        assert!(requester_line(&e).chars().count() <= 34, "{e}");
        // The ILBM reader says what an ANIM is instead of "Phase 6".
        let e = ilbm::read(&good).unwrap_err();
        assert!(e.summary.contains("animation"), "{e}");
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
        // Its own drawer: removing it at the end must not race other tests using tmp().
        let own = own_dir("convert-all");
        let a = own.join("a.iff");
        std::fs::write(&a, ilbm::write(&src, Compression::ByteRun1).unwrap()).unwrap();

        let b = own.join("b.png");
        let notes = convert(&a, &b).unwrap();
        assert!(notes.iter().any(|n| n.contains("CRNG")), "{notes:?}");
        let c = own.join("c.lbm");
        assert!(convert(&b, &c).unwrap().is_empty());
        let back = load(&c).unwrap().picture;
        assert_eq!(back.indexed(), Some((&img, &pal)));

        // ILBM → ILBM keeps the colour cycling.
        let d = own.join("d.ilbm");
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
        assert!(save(&back, &own.join("x.jpg")).is_err(), "no JPEG export");
        let _ = std::fs::remove_dir_all(&own);
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
        assert_eq!(f("Venus.DPXPROJ"), Ok(SaveFormat::Project));
    }

    /// An ILBM written the way the app writes it, with extras, in its own drawer.
    fn sample_ilbm(dir: &Path) -> std::path::PathBuf {
        use dpx_core::{Resolution, Video};
        let mut img = IndexedImage::new(640, 512, 0);
        img.set(3, 4, 7);
        img.set(639, 511, 31);
        let target = Target::new(
            Chipset::Aga,
            DisplayMode::new(Video::Pal, Resolution::Hi, true),
        );
        let doc = Document::from_picture(img, Palette::dpaint_default_32(), target);
        let extras = PictureExtras {
            ranges: vec![ColorRange {
                rate: 2560,
                flags: 1,
                low: 20,
                high: 27,
            }],
            grab: Some((5, -6)),
            transparent_color: Some(3),
        };
        let path = dir.join("sample.iff");
        let (bytes, _) = encode_document(&doc, &extras, &path).unwrap();
        write_atomic(&path, &bytes).unwrap();
        path
    }

    fn own_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-files-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn convert_ilbm_to_project_and_back_gives_identical_ilbm_bytes() {
        let dir = own_dir("proj-rt");
        let iff = sample_ilbm(&dir);
        let proj = dir.join("sample.dpxproj");
        let back = dir.join("back.iff");
        assert!(convert(&iff, &proj).unwrap().is_empty());
        assert!(convert(&proj, &back).unwrap().is_empty());
        assert_eq!(
            std::fs::read(&iff).unwrap(),
            std::fs::read(&back).unwrap(),
            "ILBM -> dpxproj -> ILBM"
        );
        // The project is a ZIP whatever its name, and loads as a project.
        let renamed = dir.join("noext");
        std::fs::copy(&proj, &renamed).unwrap();
        assert!(matches!(
            load(&renamed).unwrap().picture,
            Picture::Project(_)
        ));
        // dpxproj -> dpxproj and -> png work; the project is deterministic.
        let again = dir.join("again.dpxproj");
        convert(&proj, &again).unwrap();
        assert_eq!(
            std::fs::read(&proj).unwrap(),
            std::fs::read(&again).unwrap()
        );
        let png = dir.join("sample.png");
        let notes = convert(&proj, &png).unwrap();
        assert!(notes.iter().any(|n| n.contains("CRNG")), "{notes:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn project_opens_with_document_and_extras_unchanged() {
        let dir = own_dir("proj-open");
        let iff = sample_ilbm(&dir);
        let (doc, extras, _) = open_picture(load(&iff).unwrap().picture, Target::default());
        let proj = dir.join("p.dpxproj");
        let (bytes, notes) = encode_document(&doc, &extras, &proj).unwrap();
        assert!(notes.is_empty());
        write_atomic(&proj, &bytes).unwrap();
        let (doc2, extras2, notes) = open_picture(load(&proj).unwrap().picture, Target::default());
        assert!(notes.is_empty());
        assert_eq!(doc2, doc);
        assert_eq!(extras2, extras);
        // Ranges outside the palette stay in a project (an ILBM would drop them).
        let mut wild = extras.clone();
        wild.ranges.push(ColorRange {
            rate: 1,
            flags: 0,
            low: 40,
            high: 50,
        });
        let (bytes, notes) = encode_document(&doc, &wild, &proj).unwrap();
        assert!(notes.is_empty());
        write_atomic(&proj, &bytes).unwrap();
        let (_, back, _) = open_picture(load(&proj).unwrap().picture, Target::default());
        assert_eq!(back, wild);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn info_describes_a_project_and_bad_projects_say_why() {
        let dir = own_dir("proj-info");
        let iff = sample_ilbm(&dir);
        let proj = dir.join("i.dpxproj");
        convert(&iff, &proj).unwrap();
        let text = info(&proj).unwrap();
        assert!(text.contains("DeluxePainter project (dpxproj format 1) 640x512, 32 colours"));
        assert!(text.contains("PAL:High Res Laced"), "{text}");
        assert!(text.contains("range 0: colours 20-27"), "{text}");
        assert!(text.contains("grab: 5,-6"), "{text}");
        assert!(text.contains("transparent colour: 3"), "{text}");
        assert!(text.contains("colours used: 3"), "{text}");
        // A cut-off project: short first line after the path.
        let bytes = std::fs::read(&proj).unwrap();
        let cut = dir.join("cut.dpxproj");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let e = info(&cut).unwrap_err();
        assert!(e.contains("Not a DeluxePainter project"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
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

    #[test]
    fn forty_colour_png_converts_to_iff_and_loads_the_same() {
        let own = own_dir("png40");
        let mut img = IndexedImage::new(12, 5, 0);
        for x in 0..12 {
            for y in 0..5 {
                img.set(x, y, ((x * 5 + y) % 40) as u8);
            }
        }
        let pal = Palette::new((0..40).map(|i| Rgb8::new(i * 6, i, 255 - i)).collect()).unwrap();
        let png = own.join("a.png");
        std::fs::write(&png, dpx_formats::write_png(&img, &pal).unwrap()).unwrap();
        let iff = own.join("a.iff");
        convert(&png, &iff).unwrap();
        let loaded = load(&iff).unwrap();
        match loaded.picture {
            Picture::Ilbm(i) => {
                assert_eq!(i.image, img);
                assert_eq!(&i.palette.colors()[..40], pal.colors());
            }
            _ => panic!("expected an indexed picture"),
        }
        let _ = std::fs::remove_dir_all(own);
    }

    #[test]
    fn ham_load_notes_dropped_extras() {
        let mut src = Ilbm::from_indexed(IndexedImage::new(16, 2, 0), {
            Palette::new((0..16).map(|i| Rgb8::new(i, i, i)).collect()).unwrap()
        });
        src.planes = 6;
        src.camg = Some(ilbm::CAMG_HAM);
        src.grab = Some((1, 2));
        src.ranges.push(ColorRange {
            rate: 1,
            flags: 1,
            low: 0,
            high: 3,
        });
        src.other_chunks.push(dpx_iff::ilbm::RawChunk {
            id: *b"DPPS",
            data: vec![0; 4],
        });
        let own = own_dir("ham-extras");
        let path = own.join("h.iff");
        std::fs::write(&path, write_ham_like(&src)).unwrap();
        let loaded = load(&path).unwrap();
        let all = loaded.notes.join("|");
        assert!(all.contains("CRNG"), "{all}");
        assert!(all.contains("GRAB"), "{all}");
        assert!(all.contains("DPPS"), "{all}");
        let _ = std::fs::remove_dir_all(own);
    }

    /// The writer refuses HAM, so the CAMG is written as plain and patched afterwards.
    fn write_ham_like(src: &Ilbm) -> Vec<u8> {
        let mut plain = src.clone();
        plain.camg = Some(0);
        let mut bytes = ilbm::write(&plain, Compression::ByteRun1).unwrap();
        let at = bytes.windows(4).position(|w| w == b"CAMG").unwrap() + 8;
        bytes[at..at + 4].copy_from_slice(&ilbm::CAMG_HAM.to_be_bytes());
        bytes
    }

    #[test]
    fn write_atomic_handles_long_names_and_leaves_no_temp_file() {
        let own = own_dir("atomic-long");
        let path = own.join("n".repeat(250));
        write_atomic(&path, b"data").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"data");
        assert_eq!(std::fs::read_dir(&own).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(own);
    }

    #[test]
    fn page_at_least_screen_refuses_a_hostile_screen() {
        let doc = Document::new(8, 8, Palette::dpaint_default_32());
        let mut t = doc.target();
        t.screen = (u32::MAX, u32::MAX);
        let hostile = Document::from_picture(doc.page().clone(), doc.palette().clone(), t);
        let (out, note) = page_at_least_screen(hostile);
        assert_eq!(out.page().width(), 8);
        assert!(note.is_some());
    }
}
