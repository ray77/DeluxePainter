//! Loading and saving pictures by file type, plus the headless `info` and `convert` commands.
//!
//! Formats: IFF ILBM and PNG in and out (JPEG import follows). Whatever a target format cannot
//! store is reported, never dropped silently.

use std::path::Path;

use dpx_core::{IndexedImage, Palette};
use dpx_iff::{Compression, Ilbm, Masking, ilbm};

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
}
