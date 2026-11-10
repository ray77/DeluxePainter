//! Opening and encoding indexed pictures (IFF ILBM, indexed PNG) for the file tools.
//!
//! This mirrors the indexed paths of the app's `files.rs` (`open_picture`, `encode_document`)
//! because the app crate depends on this one. True-colour PNG and JPEG need the app's colour
//! reduction (ADR-9) and are not supported here (ADR-24).

use dpx_core::{Chipset, DisplayMode, Document, IndexedImage, Rgb8, Target};
use dpx_iff::{Ilbm, Masking, ilbm};

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

/// Output format by extension: `.iff`, `.ilbm`, `.lbm` (IFF ILBM) or `.png`. Unlike the GUI,
/// unknown extensions are refused instead of defaulting to ILBM.
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
            "JPEG is import-only: save as .iff or .png",
        )),
        _ => Err(ToolError::new(
            "UNSUPPORTED_FILE_TYPE",
            "save_picture writes .iff/.ilbm/.lbm (IFF ILBM), .png (indexed PNG) and .anim (IFF ANIM)",
        )),
    }
}

fn unsupported(msg: impl Into<String>) -> ToolError {
    ToolError::new("UNSUPPORTED_FILE_TYPE", msg)
}

fn bad_file(path: &str, e: impl std::fmt::Display) -> ToolError {
    ToolError::new("INVALID_FILE", format!("{path}: {e}"))
}

/// Decodes an indexed ILBM or PNG into a document at revision 0 plus notes about whatever
/// the document model does not keep.
pub fn decode_picture(name: &str, bytes: &[u8]) -> Result<(Document, Vec<String>), ToolError> {
    if bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"ANIM") {
        return decode_anim(name, bytes);
    }
    let mut notes = Vec::new();
    let (image, palette, camg, page_fields) = if bytes.starts_with(b"FORM") {
        let (i, n) = ilbm::read_with_notes(bytes).map_err(|e| bad_file(name, e))?;
        notes.extend(n);
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
                    "{name}: true-colour PNG cannot be opened over MCP (colour reduction is app-level); open an indexed PNG or an IFF ILBM"
                )));
            }
        }
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(unsupported(format!(
            "{name}: JPEG is true colour and cannot be opened over MCP; open an indexed PNG or an IFF ILBM"
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
/// is reduced silently: a violation is an error.
pub fn validate_hardware(doc: &Document) -> Result<(), ToolError> {
    let t = doc.target();
    let (w, h) = (doc.page().width(), doc.page().height());
    let colors = doc.palette().len();
    let Some(max) = t.mode.max_colors(t.chipset) else {
        return Err(hardware(format!(
            "{:?} cannot show {}",
            t.chipset,
            t.mode.name()
        )));
    };
    if colors > usize::from(max) {
        return Err(hardware(format!(
            "{colors} colours exceed the {max} of {} on {:?}",
            t.mode.name(),
            t.chipset
        )));
    }
    if !t.page_fits(w, h) {
        let ((min_w, min_h), (max_w, max_h)) = t.page_limits();
        return Err(hardware(format!(
            "page {w}x{h} is outside the {min_w}x{min_h}..{max_w}x{max_h} limits of {}",
            t.mode.name()
        )));
    }
    if t.chipset == Chipset::Ocs
        && let Some(i) = doc
            .palette()
            .colors()
            .iter()
            .position(|c| Rgb8::from_rgb4(c.to_rgb4()) != *c)
    {
        return Err(hardware(format!(
            "palette entry {i} is not a 12-bit OCS colour"
        )));
    }
    if let Some(&max) = doc.page().pixels().iter().max()
        && usize::from(max) >= colors
    {
        return Err(hardware(format!(
            "pixel colour {max} is outside the {colors}-colour palette"
        )));
    }
    Ok(())
}

/// Encodes the document after validating its hardware limits. ILBM carries the display
/// mode's CAMG, pixel aspect and screen size; PNG cannot store the display mode (noted).
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
    match format {
        Format::Ilbm => {
            let mut out = Ilbm::from_indexed(doc.page().clone(), doc.palette().clone());
            out.camg = Some(doc.target().camg());
            (out.x_aspect, out.y_aspect) = doc.target().mode.aspect();
            let (sw, sh) = doc.target().screen;
            out.page_width = sw as i16;
            out.page_height = sh as i16;
            let bytes = ilbm::write(&out, ilbm::Compression::ByteRun1)
                .map_err(|e| hardware(e.to_string()))?;
            Ok((bytes, notes))
        }
        Format::Anim => {
            let mode = doc.target().mode;
            let (x_aspect, y_aspect) = mode.aspect();
            let images: Vec<IndexedImage> = doc.frames().cloned().collect();
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
            Ok((bytes, notes))
        }
        Format::Png => {
            let bytes = dpx_formats::write_png(doc.page(), doc.palette())
                .map_err(|e| ToolError::new("INVALID_FILE", e.to_string()))?;
            notes.push(format!(
                "PNG cannot store: display mode {}",
                doc.target().mode.name()
            ));
            Ok((bytes, notes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

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

    #[test]
    fn formats_by_extension() {
        assert_eq!(format_for_save("a.IFF").unwrap(), Format::Ilbm);
        assert_eq!(format_for_save("a.png").unwrap(), Format::Png);
        assert!(format_for_save("a.jpg").is_err());
        assert!(format_for_save("a").is_err());
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
