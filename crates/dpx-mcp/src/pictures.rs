//! Opening and encoding pictures for the file tools: indexed IFF ILBM / ANIM / PNG and
//! `.dpxproj` projects for `open_picture`, PNG and JPEG as RGBA for `import_image` (ADR-43),
//! and the export encoders with their hardware validation.
//!
//! This mirrors the indexed paths of the app's `files.rs` (`open_picture`, `encode_document`)
//! because the app crate depends on this one. `open_picture` keeps refusing true-colour PNG
//! and JPEG (ADR-24): they go through `import_image`, which converts them with
//! `dpx_core::import` into the current document's format (ADR-42).

use dpx_core::quantize::PaletteDepth;
use dpx_core::{DisplayMode, Document, IndexedImage, Palette, Rgb8, RgbaImage, Target};
use dpx_formats::project::{self, Project, ProjectExtras};
use dpx_iff::{Ilbm, Masking, SpecialMode, ilbm};

use crate::error::ToolError;

/// Largest picture file `open_picture` reads.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Ilbm,
    Png,
    /// IFF ANIM: all frames (ADR-32).
    Anim,
}

/// Export format by extension: `.iff`, `.ilbm`, `.lbm` (IFF ILBM), `.png` or `.anim`. Unlike
/// the GUI, unknown extensions are refused instead of defaulting to ILBM.
pub fn format_for_save(path: &str) -> Result<Format, ToolError> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("iff" | "ilbm" | "lbm") => Ok(Format::Ilbm),
        Some("png") => Ok(Format::Png),
        Some("anim") => Ok(Format::Anim),
        Some("jpg" | "jpeg") => Err(ToolError::new(
            "UNSUPPORTED_FILE_TYPE",
            "JPEG is import-only: export as .iff or .png",
        )),
        Some("dpxproj") => Err(ToolError::new(
            "UNSUPPORTED_FILE_TYPE",
            ".dpxproj is the project format: use save_project",
        )),
        _ => Err(ToolError::new(
            "UNSUPPORTED_FILE_TYPE",
            "export_image writes .iff/.ilbm/.lbm (IFF ILBM), .png (indexed PNG) and .anim (IFF ANIM)",
        )),
    }
}

fn unsupported(msg: impl Into<String>) -> ToolError {
    ToolError::new("UNSUPPORTED_FILE_TYPE", msg)
}

fn bad_file(path: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError::new("INVALID_FILE", format!("{path}: {e}"))
}

/// EHB as 64 free colours, like the app's Load: the half-bright copies become real palette
/// entries and the EHB bit is cleared so the file reads as a plain 64-colour picture.
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
    pic.camg = Some(pic.camg.unwrap_or(0) & !ilbm::CAMG_EHB);
    notes.push(
        "Extra-Half-Brite picture: opened as 64 free colours (32-63 are the half-bright copies); saving writes a 64-colour 6-plane ILBM, not EHB"
            .into(),
    );
}

/// Decodes an indexed ILBM or PNG into a document at revision 0 plus notes about whatever
/// the document model does not keep.
pub fn decode_picture(name: &str, bytes: &[u8]) -> Result<(Document, Vec<String>), ToolError> {
    if bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"ANIM") {
        return decode_anim(name, bytes);
    }
    let mut notes = Vec::new();
    let (image, palette, camg, page_fields) = if bytes.starts_with(b"FORM") {
        let (mut i, n) = ilbm::read_with_notes(bytes).map_err(|e| bad_file(name, e))?;
        notes.extend(n);
        match i.special {
            Some(SpecialMode::Ham6 | SpecialMode::Ham8) => {
                return Err(unsupported(format!(
                    "{name}: HAM pictures cannot be opened over MCP (HAM is decoded to true colour and reduced by the app); open it in the app, or open an indexed ILBM or PNG"
                )));
            }
            Some(SpecialMode::Ehb) => ehb_to_indexed(&mut i, &mut notes),
            None => {}
        }
        if !i.ranges.is_empty() {
            notes.push(format!(
                "{} colour-cycling ranges not kept (not in the document model yet)",
                i.ranges.len()
            ));
        }
        match i.masking {
            Masking::TransparentColor => notes.push(format!(
                "transparent colour {} not kept (pages are opaque)",
                i.transparent_color
            )),
            Masking::HasMask => notes.push("mask plane not kept (pages are opaque)".into()),
            Masking::None | Masking::Lasso => {}
        }
        let fields = (i.page_width.max(0) as u32, i.page_height.max(0) as u32);
        (i.image, i.palette, i.camg, Some(fields))
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        match dpx_formats::read_png(bytes).map_err(|e| bad_file(name, e))? {
            dpx_formats::PngImage::Indexed(p) => {
                if !p.transparent_entries.is_empty() {
                    notes.push(format!(
                        "PNG transparency of {} palette entries is ignored (pages are opaque)",
                        p.transparent_entries.len()
                    ));
                }
                (p.image, p.palette, None, None)
            }
            dpx_formats::PngImage::TrueColor(_) => {
                return Err(unsupported(format!(
                    "{name}: true-colour PNG: open_picture opens indexed pictures; use import_image to convert it into the document"
                )));
            }
        }
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(unsupported(format!(
            "{name}: JPEG is true colour: use import_image to convert it into the document"
        )));
    } else {
        return Err(unsupported(format!(
            "{name}: not an IFF ILBM or PNG picture"
        )));
    };
    let (w, h) = (image.width(), image.height());
    let mode = DisplayMode::guess(camg, w, h);
    let (sw, sh) = page_fields
        .filter(|&(pw, ph)| mode.screen_fits(pw, ph))
        .unwrap_or_else(|| mode.screen_for_picture(w, h));
    let target = Target::new(Target::default().chipset, mode).with_screen(sw, sh);
    // Pages are never smaller than the screen (ADR-10): a smaller picture sits top-left on a
    // screen-sized page of colour 0.
    let image = if w >= sw && h >= sh {
        image
    } else {
        let mut page = IndexedImage::new(w.max(sw), h.max(sh), 0);
        page.blit(&image, 0, 0);
        notes.push(format!(
            "page enlarged from {w}x{h} to {}x{} (screen size), picture top-left",
            page.width(),
            page.height()
        ));
        page
    };
    Ok((Document::from_picture(image, palette, target), notes))
}

/// A `.dpxproj` project (ADR-23): the document, its extras and the spare page.
pub fn decode_project(
    name: &str,
    bytes: &[u8],
) -> Result<(Document, ProjectExtras, Option<IndexedImage>), ToolError> {
    let p = project::read_project(bytes).map_err(|e| bad_file(name, e.details()))?;
    Ok((p.document, p.extras, p.spare))
}

/// The project bytes of `doc` with its extras and spare page.
pub fn encode_project(
    doc: &Document,
    extras: ProjectExtras,
    spare: Option<IndexedImage>,
) -> Result<Vec<u8>, ToolError> {
    let mut p = Project::new(doc.clone(), extras);
    p.spare = spare;
    project::write_project(&p).map_err(|e| ToolError::new("IO_ERROR", e.details()))
}

/// Largest picture `import_image` decodes, in pixels (as for PNG and JPEG import).
pub const MAX_IMPORT_PIXELS: u64 = 64 * 1024 * 1024;

/// A PNG (indexed or true colour) or JPEG as RGBA for `import_image`, with its number of
/// colours (`None` above 256) and notes. Other files are refused.
pub fn decode_rgba(
    name: &str,
    bytes: &[u8],
) -> Result<(RgbaImage, Option<usize>, Vec<String>), ToolError> {
    let mut notes = Vec::new();
    let image = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        match dpx_formats::read_png(bytes).map_err(|e| bad_file(name, e.details()))? {
            dpx_formats::PngImage::Indexed(p) => {
                if !p.transparent_entries.is_empty() {
                    notes.push(format!(
                        "PNG transparency of {} palette entries is ignored",
                        p.transparent_entries.len()
                    ));
                }
                dpx_core::import::rgba_of(&p.image, &p.palette)
            }
            dpx_formats::PngImage::TrueColor(img) => img,
        }
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        dpx_formats::read_jpeg(bytes).map_err(|e| bad_file(name, e.details()))?
    } else if bytes.starts_with(b"FORM") {
        return Err(unsupported(format!(
            "{name}: IFF pictures are opened with open_picture, not imported"
        )));
    } else {
        return Err(unsupported(format!(
            "{name}: import_image reads PNG and JPEG pictures"
        )));
    };
    if u64::from(image.width()) * u64::from(image.height()) > MAX_IMPORT_PIXELS {
        return Err(ToolError::invalid(format!(
            "{name}: {}x{} pixels exceed the {MAX_IMPORT_PIXELS} pixel limit",
            image.width(),
            image.height()
        )));
    }
    let colors = dpx_core::import::count_colors(&image, 256);
    Ok((image, colors, notes))
}

/// An IFF ANIM as a multi-frame document, like the app's Load (ADR-32).
fn decode_anim(name: &str, bytes: &[u8]) -> Result<(Document, Vec<String>), ToolError> {
    let (anim, mut notes) = dpx_iff::anim::read_with_notes(bytes).map_err(|e| bad_file(name, e))?;
    let content = dpx_iff::anim_doc::content_of(&anim);
    notes.extend(content.notes);
    if !content.ranges.is_empty() {
        notes.push(format!(
            "{} colour-cycling ranges not kept (not in the document model yet)",
            content.ranges.len()
        ));
    }
    let (w, h) = (content.frames[0].width(), content.frames[0].height());
    let mode = DisplayMode::guess(content.camg, w, h);
    let (sw, sh) = mode.screen_for_picture(w, h);
    let target = Target::new(Target::default().chipset, mode).with_screen(sw, sh);
    let doc = Document::from_frames(content.frames, 0, content.fps, content.palette, target)
        .map_err(|e| bad_file(name, e.message))?;
    Ok((doc, notes))
}

fn hardware(msg: String) -> ToolError {
    ToolError::new("HARDWARE_LIMIT", msg)
}

/// The hardware rules of the document's target, checked before anything is written. Nothing
/// is reduced silently: a violation is an error. The rules live in `dpx_core::hardware`
/// (ADR-46), shared with the GUI's and the CLI's exports.
pub fn validate_hardware(doc: &Document) -> Result<(), ToolError> {
    dpx_core::hardware::validate(doc).map_err(|e| ToolError::new(e.code(), e.message))
}

/// Encodes the document after validating its hardware limits. ILBM carries the display
/// mode's CAMG, pixel aspect and screen size; PNG cannot store the display mode (noted).
/// Every format names a custom or overscan screen, and IFF what the file cannot carry of the
/// target (`dpx_core::hardware::export_notes`, as the app's exports, ADR-46).
/// Every format holds the picture as it shows: the visible layers flattened (ADR-44, noted).
pub fn encode_document(
    doc: &Document,
    format: Format,
) -> Result<(Vec<u8>, Vec<String>), ToolError> {
    validate_hardware(doc)?;
    // ADR-28: a picture format holds one frame; .anim writes all of them (ADR-32).
    let mut notes = Vec::new();
    if doc.frame_count() > 1 && format != Format::Anim {
        notes.push(format!(
            "frame {} of {} saved as a picture (save as .anim for all frames)",
            doc.current_frame() + 1,
            doc.frame_count()
        ));
    }
    notes.extend(dpx_core::layer::flatten_note(doc.layers()));
    match format {
        Format::Ilbm => {
            let picture = doc.composite().into_owned();
            let mut out = Ilbm::from_indexed(picture, doc.palette().clone());
            out.camg = Some(doc.target().camg());
            (out.x_aspect, out.y_aspect) = doc.target().mode.aspect();
            let (sw, sh) = doc.target().screen;
            out.page_width = sw as i16;
            out.page_height = sh as i16;
            let bytes = ilbm::write(&out, ilbm::Compression::ByteRun1)
                .map_err(|e| hardware(e.to_string()))?;
            notes.extend(dpx_core::hardware::export_notes(doc, true));
            Ok((bytes, notes))
        }
        Format::Anim => {
            let mode = doc.target().mode;
            let (x_aspect, y_aspect) = mode.aspect();
            let images: Vec<IndexedImage> =
                doc.composite_frames().map(|f| f.into_owned()).collect();
            let opts = dpx_iff::anim::EncodeOptions {
                hold: dpx_iff::anim::Hold::PerFrame(dpx_iff::anim_doc::holds_for_fps(
                    doc.fps(),
                    images.len(),
                )),
                camg: Some(doc.target().camg()),
                x_aspect,
                y_aspect,
                ..Default::default()
            };
            let bytes = dpx_iff::anim::encode(&images, doc.palette(), &opts)
                .map_err(|e| hardware(e.to_string()))?;
            notes.extend(dpx_core::hardware::export_notes(doc, true));
            Ok((bytes, notes))
        }
        Format::Png => {
            let bytes = dpx_formats::write_png(&doc.composite(), doc.palette())
                .map_err(|e| ToolError::new("INVALID_FILE", e.to_string()))?;
            notes.push(format!(
                "PNG cannot store: display mode {}",
                doc.target().mode.name()
            ));
            notes.extend(dpx_core::hardware::export_notes(doc, false));
            Ok((bytes, notes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Chipset;

    fn data(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/../dpx-iff/tests/data/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[test]
    fn ehb_opens_as_64_colours_and_saves() {
        let (d, notes) = decode_picture("e", &data("ehb-16x2.iff")).unwrap();
        assert_eq!(d.palette().len(), 64);
        let max = d.page().pixels().iter().copied().max().unwrap();
        assert!(usize::from(max) < 64);
        assert!(notes.iter().any(|n| n.contains("Extra-Half-Brite")));
        encode_document(&d, Format::Ilbm).unwrap();
        encode_document(&d, Format::Png).unwrap();
    }

    #[test]
    fn ham_is_refused() {
        for f in ["ham6-16x2.iff", "ham8-16x2.iff"] {
            let e = decode_picture("h", &data(f)).unwrap_err();
            assert_eq!(e.code, "UNSUPPORTED_FILE_TYPE", "{f}");
            assert!(e.message.contains("HAM"));
        }
    }

    fn doc() -> Document {
        let mut page = IndexedImage::new(320, 256, 0);
        page.set(3, 4, 7);
        Document::from_picture(page, Palette::dpaint_default_32(), Target::default())
    }

    #[test]
    fn ilbm_and_png_round_trip() {
        let d = doc();
        for f in [Format::Ilbm, Format::Png] {
            let (bytes, _) = encode_document(&d, f).unwrap();
            let (back, _) = decode_picture("x", &bytes).unwrap();
            assert_eq!(back.page(), d.page(), "{f:?}");
            assert_eq!(back.palette(), d.palette(), "{f:?}");
            assert_eq!(back.revision(), 0);
        }
    }

    #[test]
    fn anim_round_trip_keeps_every_frame_and_the_rate() {
        let frames: Vec<IndexedImage> = (0..3)
            .map(|i| {
                let mut f = IndexedImage::new(320, 256, i);
                f.set(5, 6 + i32::from(i), 9);
                f
            })
            .collect();
        let d = Document::from_frames(
            frames,
            1,
            10,
            Palette::dpaint_default_32(),
            Target::default(),
        )
        .unwrap();
        assert_eq!(format_for_save("a.ANIM").unwrap(), Format::Anim);
        let (bytes, notes) = encode_document(&d, Format::Anim).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let (back, notes) = decode_picture("x", &bytes).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert!(back.frames().eq(d.frames()));
        assert_eq!((back.fps(), back.current_frame()), (10, 0));
        assert_eq!(back.palette(), d.palette());
        // Pictures still say which frame they hold.
        let (_, notes) = encode_document(&d, Format::Ilbm).unwrap();
        assert!(notes[0].contains("frame 2 of 3"), "{notes:?}");
    }

    #[test]
    fn hardware_limits_are_errors() {
        // OCS cannot show SuperHiRes.
        let d = Document::from_picture(
            IndexedImage::new(1280, 256, 0),
            Palette::dpaint_default_32(),
            Target::new(
                Chipset::Ocs,
                DisplayMode::new(dpx_core::Video::Pal, dpx_core::Resolution::SuperHi, false),
            ),
        );
        let e = encode_document(&d, Format::Ilbm).unwrap_err();
        assert_eq!(e.code, "HARDWARE_LIMIT", "{e}");
        // 32 colours exceed OCS HiRes (16).
        let d = Document::from_picture(
            IndexedImage::new(640, 256, 0),
            Palette::dpaint_default_32(),
            Target::new(
                Chipset::Ocs,
                DisplayMode::new(dpx_core::Video::Pal, dpx_core::Resolution::Hi, false),
            ),
        );
        assert_eq!(
            encode_document(&d, Format::Png).unwrap_err().code,
            "HARDWARE_LIMIT"
        );
        // A page smaller than the screen.
        let d = Document::from_picture(
            IndexedImage::new(100, 100, 0),
            Palette::dpaint_default_32(),
            Target::default(),
        );
        assert!(
            encode_document(&d, Format::Ilbm)
                .unwrap_err()
                .message
                .contains("limits")
        );
    }

    /// ADR-46: export_image names a custom screen in every format and, for IFF, the chipset
    /// the file cannot carry, as the app's exports do.
    #[test]
    fn saved_pictures_name_custom_screens_and_the_lost_chipset() {
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES).with_screen(320, 240);
        let d = Document::from_picture(
            IndexedImage::new(320, 240, 0),
            Palette::dpaint_default_32(),
            ocs,
        );
        let custom = "320x240 is a custom screen size on PAL:Low Res (standard 320x256); exact \
                      hardware compatibility is not guaranteed";
        let chipset = "IFF files store no chipset: this OCS picture reopens as AGA";
        for f in [Format::Ilbm, Format::Anim] {
            let (_, notes) = encode_document(&d, f).unwrap();
            assert_eq!(notes, [custom, chipset], "{f:?}");
        }
        let (_, notes) = encode_document(&d, Format::Png).unwrap();
        assert_eq!(
            notes,
            ["PNG cannot store: display mode PAL:Low Res", custom]
        );
        // A standard AGA screen says nothing more.
        let (_, notes) = encode_document(&doc(), Format::Ilbm).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn formats_by_extension() {
        assert_eq!(format_for_save("a.IFF").unwrap(), Format::Ilbm);
        assert_eq!(format_for_save("a.png").unwrap(), Format::Png);
        assert!(format_for_save("a.jpg").is_err());
        assert!(format_for_save("a").is_err());
    }

    /// ADR-44: a picture holds what the frames show (all visible layers), with a note.
    #[test]
    fn saved_pictures_hold_the_visible_layers() {
        use dpx_core::{Command, Engine};
        let mut e = Engine::new(doc());
        e.execute(Command::AddLayer { name: None }).unwrap();
        e.execute(Command::Clear { color: 2 }).unwrap();
        e.execute(Command::AddFrames { count: 1 }).unwrap();
        // Frame 2's upper layer is cleared: the bottom layer's colour 7 at (3, 4) shows.
        e.execute(Command::Clear { color: 0 }).unwrap();
        let d = e.document();
        for format in [Format::Ilbm, Format::Png, Format::Anim] {
            let (bytes, notes) = encode_document(d, format).unwrap();
            assert!(
                notes
                    .iter()
                    .any(|n| n.contains("2 layers saved as one picture")),
                "{format:?}: {notes:?}"
            );
            let (back, _) = decode_picture("x", &bytes).unwrap();
            let last = back.frame_count() - 1;
            assert_eq!(
                back.frame(last).unwrap(),
                d.composite_frame(1).unwrap().as_ref(),
                "{format:?}"
            );
            assert_eq!(back.frame(last).unwrap().get(3, 4), Some(7));
        }
        // Frame 1 shows the upper layer's colour 2 everywhere.
        let (bytes, _) = encode_document(d, Format::Anim).unwrap();
        let (back, _) = decode_picture("x", &bytes).unwrap();
        assert!(back.frame(0).unwrap().pixels().iter().all(|&p| p == 2));
    }

    #[test]
    fn unsupported_inputs() {
        assert_eq!(
            decode_picture("x", b"hello").unwrap_err().code,
            "UNSUPPORTED_FILE_TYPE"
        );
        assert_eq!(
            decode_picture("x", &[0xFF, 0xD8, 0xFF, 0])
                .unwrap_err()
                .code,
            "UNSUPPORTED_FILE_TYPE"
        );
        assert_eq!(
            decode_picture("x", b"FORMxxxxILBM").unwrap_err().code,
            "INVALID_FILE"
        );
    }
}
