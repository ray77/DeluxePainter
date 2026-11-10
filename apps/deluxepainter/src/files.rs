//! Loading and saving pictures by file type, plus the headless `info` and `convert` commands.
//!
//! Formats: IFF ILBM and PNG in and out (JPEG import follows). Whatever a target format cannot
//! store is reported, never dropped silently.

use std::path::Path;

use dpx_core::{DisplayMode, Document, IndexedImage, Palette, Target};
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
}

impl Picture {
    pub fn image(&self) -> &IndexedImage {
        match self {
            Picture::Ilbm(i) => &i.image,
            Picture::Png { image, .. } => image,
        }
    }

    pub fn palette(&self) -> &Palette {
        match self {
            Picture::Ilbm(i) => &i.palette,
            Picture::Png { palette, .. } => palette,
        }
    }

    /// The ILBM display mode id, if the file has one.
    pub fn camg(&self) -> Option<u32> {
        match self {
            Picture::Ilbm(i) => i.camg,
            Picture::Png { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ilbm,
    Png,
}

fn kind_of_bytes(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(b"FORM") {
        Some(Kind::Ilbm)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Kind::Png)
    } else {
        None
    }
}

fn kind_of_path(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "iff" | "ilbm" | "lbm" => Some(Kind::Ilbm),
        "png" => Some(Kind::Png),
        _ => None,
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
            let pic =
                dpx_formats::read_png(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
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
        None => Err(format!(
            "{}: unknown file type (DeluxePainter reads IFF ILBM and PNG)",
            path.display()
        )),
    }
}

/// Writes `picture` in the format given by the file extension; returns notes about metadata
/// the target format cannot hold.
pub fn save(picture: &Picture, path: &Path) -> Result<Vec<String>, String> {
    let kind = kind_of_path(path).ok_or_else(|| {
        format!(
            "{}: unsupported output type (DeluxePainter writes .iff/.ilbm/.lbm and .png)",
            path.display()
        )
    })?;
    let mut notes = Vec::new();
    let bytes = match kind {
        Kind::Ilbm => {
            let ilbm = match picture {
                Picture::Ilbm(i) => (**i).clone(),
                Picture::Png { image, palette } => {
                    Ilbm::from_indexed(image.clone(), palette.clone())
                }
            };
            ilbm::write(&ilbm, Compression::ByteRun1).map_err(|e| e.to_string())?
        }
        Kind::Png => {
            if let Picture::Ilbm(i) = picture {
                notes.extend(png_drops(i));
            }
            dpx_formats::write_png(picture.image(), picture.palette()).map_err(|e| e.to_string())?
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

/// Turns a loaded picture into a document; the display mode is guessed from CAMG and size.
/// Returns notes about everything that is not kept.
pub fn open_picture(picture: Picture) -> (Document, PictureExtras, Vec<String>) {
    let mode = DisplayMode::guess(
        picture.camg(),
        picture.image().width(),
        picture.image().height(),
    );
    let target = Target {
        mode,
        ..Target::default()
    };
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
    };
    (
        Document::from_picture(image, palette, target),
        extras,
        notes,
    )
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
            let (sw, sh) = mode.size();
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
    let img = loaded.picture.image();
    let name = path.display();
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

/// `deluxepainter convert <in> <out>`.
pub fn convert(input: &Path, output: &Path) -> Result<Vec<String>, String> {
    let loaded = load(input)?;
    let mut notes = loaded.notes;
    notes.extend(save(&loaded.picture, output)?);
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
        assert_eq!(back.image(), &img);
        assert_eq!(back.palette(), &pal);

        // ILBM → ILBM keeps the colour cycling.
        let d = tmp("d.ilbm");
        convert(&a, &d).unwrap();
        match load(&d).unwrap().picture {
            Picture::Ilbm(i) => assert_eq!(i.ranges, src.ranges),
            Picture::Png { .. } => panic!("expected ILBM"),
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
        let (doc, extras, notes) = open_picture(Picture::Ilbm(Box::new(src)));
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
}
