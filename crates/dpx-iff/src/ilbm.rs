//! ILBM pictures: BMHD, CMAP (with the 4-bit fix), CAMG, CRNG/CCRT, GRAB and the BODY in
//! interleaved bitplanes, uncompressed or ByteRun1 (`docs/IFF_SUPPORT_MATRIX.md` §2–3).
//!
//! EHB, HAM, DEEP and PBM files are detected and refused with a clear message; they are
//! planned for later phases.

use std::fmt;

use dpx_core::{IndexedImage, Palette, Rgb8};

use crate::byterun1;
use crate::iff::{self, FourCc, IffError, be_i16, be_u16, be_u32};

/// CAMG flag: Extra-Half-Brite.
pub const CAMG_EHB: u32 = 0x0080;
/// CAMG flag: Hold-And-Modify.
pub const CAMG_HAM: u32 = 0x0800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Masking {
    /// Opaque rectangle.
    None,
    /// A mask plane follows the colour planes in every row.
    HasMask,
    /// Pixels of `transparent_color` are transparent.
    TransparentColor,
    /// Transparent colour, flood-filled inwards from the border.
    Lasso,
}

impl Masking {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Masking::None,
            1 => Masking::HasMask,
            2 => Masking::TransparentColor,
            3 => Masking::Lasso,
            _ => return None,
        })
    }

    fn to_u8(self) -> u8 {
        match self {
            Masking::None => 0,
            Masking::HasMask => 1,
            Masking::TransparentColor => 2,
            Masking::Lasso => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    ByteRun1,
}

/// One CRNG colour-cycling range, kept exactly as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorRange {
    /// 16384 = 60 steps per second.
    pub rate: u16,
    /// Bit 0 active, bit 1 reverse; other bits are kept as they are.
    pub flags: u16,
    pub low: u8,
    pub high: u8,
}

impl ColorRange {
    pub fn active(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn reverse(&self) -> bool {
        self.flags & 2 != 0
    }

    pub fn steps_per_second(&self) -> f64 {
        f64::from(self.rate) * 60.0 / 16384.0
    }
}

/// A chunk this module does not interpret (e.g. DPPS, DPPV, TINY), kept for re-export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawChunk {
    pub id: FourCc,
    pub data: Vec<u8>,
}

/// A decoded ILBM picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ilbm {
    pub image: IndexedImage,
    /// `2^planes` entries (missing CMAP entries are black, surplus entries are dropped).
    pub palette: Palette,
    pub planes: u8,
    pub masking: Masking,
    pub transparent_color: u16,
    /// Mask plane for [`Masking::HasMask`]: one entry per pixel, `true` = opaque.
    pub mask: Option<Vec<bool>>,
    /// How the BODY was stored (informational; the writer takes its own choice).
    pub compression: Compression,
    pub x_aspect: u8,
    pub y_aspect: u8,
    pub page_width: i16,
    pub page_height: i16,
    /// Raw CAMG value, if present.
    pub camg: Option<u32>,
    /// CRNG ranges in file order (CCRT converted when there is no CRNG).
    pub ranges: Vec<ColorRange>,
    /// GRAB hotspot (brushes).
    pub grab: Option<(i16, i16)>,
    /// Uninterpreted chunks found before BODY, in file order.
    pub other_chunks: Vec<RawChunk>,
    /// The CMAP held 4-bit values in the high nibble (0xN0) and was expanded to 0xNN.
    pub cmap_4bit_fixed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IlbmError {
    Iff(IffError),
    /// A FORM of another type (e.g. PBM, ANIM).
    NotIlbm(String),
    MissingBmhd,
    MissingBody,
    /// A chunk is shorter than its fixed layout.
    ShortChunk(String),
    /// Valid IFF that DeluxePainter cannot handle yet.
    Unsupported(String),
    /// Corrupt or truncated pixel data.
    BadBody(String),
    /// The picture cannot be written as described (export validation).
    Invalid(String),
}

impl fmt::Display for IlbmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IlbmError::Iff(e) => write!(f, "{e}"),
            IlbmError::NotIlbm(t) => write!(f, "IFF FORM type {t} is not ILBM"),
            IlbmError::MissingBmhd => write!(f, "ILBM without BMHD chunk"),
            IlbmError::MissingBody => write!(f, "ILBM without BODY chunk"),
            IlbmError::ShortChunk(id) => write!(f, "{id} chunk is too short"),
            IlbmError::Unsupported(what) => write!(f, "not supported yet: {what}"),
            IlbmError::BadBody(what) => write!(f, "corrupt BODY: {what}"),
            IlbmError::Invalid(what) => write!(f, "cannot write ILBM: {what}"),
        }
    }
}

impl std::error::Error for IlbmError {}

impl From<IffError> for IlbmError {
    fn from(e: IffError) -> Self {
        IlbmError::Iff(e)
    }
}

struct Bmhd {
    w: u16,
    h: u16,
    planes: u8,
    masking: Masking,
    compression: Compression,
    transparent_color: u16,
    x_aspect: u8,
    y_aspect: u8,
    page_width: i16,
    page_height: i16,
}

fn parse_bmhd(d: &[u8]) -> Result<Bmhd, IlbmError> {
    if d.len() < 20 {
        return Err(IlbmError::ShortChunk("BMHD".into()));
    }
    let masking = Masking::from_u8(d[9])
        .ok_or_else(|| IlbmError::Unsupported(format!("masking type {}", d[9])))?;
    let compression = match d[10] {
        0 => Compression::None,
        1 => Compression::ByteRun1,
        c => return Err(IlbmError::Unsupported(format!("compression type {c}"))),
    };
    Ok(Bmhd {
        w: be_u16(&d[0..2]),
        h: be_u16(&d[2..4]),
        planes: d[8],
        masking,
        compression,
        transparent_color: be_u16(&d[12..14]),
        x_aspect: d[14],
        y_aspect: d[15],
        page_width: be_i16(&d[16..18]),
        page_height: be_i16(&d[18..20]),
    })
}

/// Expands a CMAP to exactly `count` entries; returns the palette and whether the 4-bit fix
/// was applied.
fn parse_cmap(d: Option<&[u8]>, count: usize) -> (Palette, bool) {
    let bytes = d.unwrap_or(&[]);
    let n = bytes.len() / 3;
    let fix = n > 0 && bytes[..n * 3].iter().all(|b| b & 0x0F == 0);
    let fixed = |v: u8| if fix { v | (v >> 4) } else { v };
    let mut colors: Vec<Rgb8> = bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| Rgb8::new(fixed(c[0]), fixed(c[1]), fixed(c[2])))
        .collect();
    if d.is_none() {
        // No CMAP at all: a grey ramp, so the picture is at least visible.
        let max = (count - 1).max(1);
        colors = (0..count)
            .map(|i| {
                let v = (i * 255 / max) as u8;
                Rgb8::new(v, v, v)
            })
            .collect();
    }
    colors.resize(count, Rgb8::BLACK);
    (Palette::new(colors).expect("1..=256 entries"), fix)
}

/// Reads an ILBM file.
pub fn read(bytes: &[u8]) -> Result<Ilbm, IlbmError> {
    let (form_type, chunks) = iff::parse_form(bytes)?;
    if &form_type != b"ILBM" {
        return Err(IlbmError::NotIlbm(iff::id_str(form_type)));
    }
    let mut bmhd = None;
    let mut cmap = None;
    let mut camg = None;
    let mut body = None;
    let mut ranges = Vec::new();
    let mut ccrt = Vec::new();
    let mut grab = None;
    let mut other_chunks = Vec::new();
    for c in &chunks {
        let short = || IlbmError::ShortChunk(iff::id_str(c.id));
        match &c.id {
            b"BMHD" => bmhd = Some(parse_bmhd(c.data)?),
            b"CMAP" => cmap = Some(c.data),
            b"CAMG" if c.data.len() >= 4 => camg = Some(be_u32(c.data)),
            b"CRNG" => {
                let d = c.data.get(..8).ok_or_else(short)?;
                ranges.push(ColorRange {
                    rate: be_u16(&d[2..4]),
                    flags: be_u16(&d[4..6]),
                    low: d[6],
                    high: d[7],
                });
            }
            b"CCRT" => {
                let d = c.data.get(..14).ok_or_else(short)?;
                ccrt.push(ccrt_to_range(d));
            }
            b"GRAB" => {
                let d = c.data.get(..4).ok_or_else(short)?;
                grab = Some((be_i16(&d[0..2]), be_i16(&d[2..4])));
            }
            b"BODY" => {
                body = Some(c.data);
                break;
            }
            b"DEEP" => return Err(IlbmError::Unsupported("true-colour DEEP ILBM".into())),
            _ => other_chunks.push(RawChunk {
                id: c.id,
                data: c.data.to_vec(),
            }),
        }
    }
    let bmhd = bmhd.ok_or(IlbmError::MissingBmhd)?;
    if ranges.is_empty() {
        ranges = ccrt;
    }
    if bmhd.w == 0 || bmhd.h == 0 {
        return Err(IlbmError::Unsupported("zero-sized picture".into()));
    }
    if bmhd.planes > 8 {
        return Err(IlbmError::Unsupported(format!(
            "{}-plane (true-colour) ILBM",
            bmhd.planes
        )));
    }
    let mode = camg.unwrap_or(0);
    if mode & CAMG_HAM != 0 {
        return Err(IlbmError::Unsupported(
            "HAM pictures (planned for Phase 5)".into(),
        ));
    }
    if mode & CAMG_EHB != 0 {
        return Err(IlbmError::Unsupported(
            "Extra-Half-Brite pictures (planned for Phase 5)".into(),
        ));
    }
    if bmhd.planes == 6 && camg.is_none() {
        return Err(IlbmError::Unsupported(
            "6-plane picture without CAMG (HAM or EHB, planned for Phase 5)".into(),
        ));
    }
    if bmhd.masking == Masking::Lasso {
        return Err(IlbmError::Unsupported("lasso masking".into()));
    }

    let colors = if bmhd.planes == 0 {
        cmap.map_or(1, |d| (d.len() / 3).clamp(1, 256))
    } else {
        1usize << bmhd.planes
    };
    let (palette, cmap_4bit_fixed) = parse_cmap(cmap, colors);
    let (w, h) = (usize::from(bmhd.w), usize::from(bmhd.h));
    let (pixels, mask) = if bmhd.planes == 0 {
        (vec![0; w * h], None)
    } else {
        decode_body(&bmhd, body.ok_or(IlbmError::MissingBody)?)?
    };
    Ok(Ilbm {
        image: IndexedImage::from_pixels(u32::from(bmhd.w), u32::from(bmhd.h), pixels)
            .expect("w*h pixels"),
        palette,
        planes: bmhd.planes,
        masking: bmhd.masking,
        transparent_color: bmhd.transparent_color,
        mask,
        compression: bmhd.compression,
        x_aspect: bmhd.x_aspect,
        y_aspect: bmhd.y_aspect,
        page_width: bmhd.page_width,
        page_height: bmhd.page_height,
        camg,
        ranges,
        grab,
        other_chunks,
        cmap_4bit_fixed,
    })
}

/// CCRT (Graphicraft) → CRNG semantics: direction 0 off, 1 forward, -1 reverse; the delay
/// between steps becomes a CRNG rate (16384 = 60 steps/s).
fn ccrt_to_range(d: &[u8]) -> ColorRange {
    let direction = be_i16(&d[0..2]);
    let delay = f64::from(be_u32(&d[4..8])) + f64::from(be_u32(&d[8..12])) / 1e6;
    let rate = if delay > 0.0 {
        (16384.0 / 60.0 / delay).round().clamp(0.0, 65535.0) as u16
    } else {
        0
    };
    let flags = match direction {
        0 => 0,
        d if d < 0 => 3,
        _ => 1,
    };
    ColorRange {
        rate,
        flags,
        low: d[2],
        high: d[3],
    }
}

fn row_bytes(w: usize) -> usize {
    w.div_ceil(16) * 2
}

/// Decodes interleaved bitplanes into indices (+ mask plane).
fn decode_body(bmhd: &Bmhd, body: &[u8]) -> Result<(Vec<u8>, Option<Vec<bool>>), IlbmError> {
    let (w, h) = (usize::from(bmhd.w), usize::from(bmhd.h));
    let rb = row_bytes(w);
    let planes = usize::from(bmhd.planes);
    let has_mask = bmhd.masking == Masking::HasMask;
    let rows_per_line = planes + usize::from(has_mask);
    let mut pixels = vec![0u8; w * h];
    let mut mask = has_mask.then(|| vec![false; w * h]);
    let mut pos = 0;
    for y in 0..h {
        for p in 0..rows_per_line {
            let row: std::borrow::Cow<[u8]> = match bmhd.compression {
                Compression::None => {
                    let r = body.get(pos..pos + rb).ok_or_else(|| {
                        IlbmError::BadBody(format!("data ends in row {y}, plane {p}"))
                    })?;
                    pos += rb;
                    r.into()
                }
                Compression::ByteRun1 => {
                    let (r, used) = byterun1::unpack_row(&body[pos.min(body.len())..], rb)
                        .map_err(|e| IlbmError::BadBody(format!("row {y}, plane {p}: {e}")))?;
                    pos += used;
                    r.into()
                }
            };
            let line = y * w;
            if p < planes {
                let bit = 1u8 << p;
                for x in 0..w {
                    if row[x >> 3] & (0x80 >> (x & 7)) != 0 {
                        pixels[line + x] |= bit;
                    }
                }
            } else if let Some(m) = &mut mask {
                for x in 0..w {
                    m[line + x] = row[x >> 3] & (0x80 >> (x & 7)) != 0;
                }
            }
        }
    }
    Ok((pixels, mask))
}

impl Ilbm {
    /// A new opaque picture with the smallest plane count for the palette and default header
    /// values (1:1 aspect, page = picture size) until screen modes provide better ones.
    pub fn from_indexed(image: IndexedImage, palette: Palette) -> Self {
        let planes = planes_for(palette.len());
        Self {
            page_width: image.width().min(i16::MAX as u32) as i16,
            page_height: image.height().min(i16::MAX as u32) as i16,
            image,
            palette,
            planes,
            masking: Masking::None,
            transparent_color: 0,
            mask: None,
            compression: Compression::ByteRun1,
            x_aspect: 1,
            y_aspect: 1,
            camg: None,
            ranges: Vec::new(),
            grab: None,
            other_chunks: Vec::new(),
            cmap_4bit_fixed: false,
        }
    }
}

/// Smallest plane count (1..=8) that can address `colors` palette entries.
pub fn planes_for(colors: usize) -> u8 {
    (1..=8u8).find(|&p| colors <= 1usize << p).unwrap_or(8)
}

/// Writes an ILBM file. Nothing is reduced silently: pixels outside the palette, a palette
/// larger than the plane count allows, or an inconsistent mask are errors.
pub fn write(ilbm: &Ilbm, compression: Compression) -> Result<Vec<u8>, IlbmError> {
    let invalid = |m: String| Err(IlbmError::Invalid(m));
    let (w, h) = (ilbm.image.width(), ilbm.image.height());
    if w == 0 || h == 0 || w > u32::from(u16::MAX) || h > u32::from(u16::MAX) {
        return invalid(format!("picture size {w}x{h} is outside 1..=65535"));
    }
    if !(1..=8).contains(&ilbm.planes) {
        return invalid(format!("{} bitplanes (1..=8 supported)", ilbm.planes));
    }
    if ilbm.palette.len() > 1 << ilbm.planes {
        return invalid(format!(
            "{} colours do not fit into {} bitplanes",
            ilbm.palette.len(),
            ilbm.planes
        ));
    }
    if let Some(max) = ilbm.image.pixels().iter().max()
        && usize::from(*max) >= ilbm.palette.len()
    {
        return invalid(format!(
            "pixel colour {max} is outside the {}-colour palette",
            ilbm.palette.len()
        ));
    }
    let n = w as usize * h as usize;
    match (&ilbm.mask, ilbm.masking) {
        (Some(m), Masking::HasMask) if m.len() == n => {}
        (None, Masking::None | Masking::TransparentColor) => {}
        _ => return invalid("mask plane does not match the masking type".into()),
    }

    let mut out = iff::FormWriter::new(*b"ILBM");
    let mut bmhd = Vec::with_capacity(20);
    bmhd.extend_from_slice(&(w as u16).to_be_bytes());
    bmhd.extend_from_slice(&(h as u16).to_be_bytes());
    bmhd.extend_from_slice(&[0, 0, 0, 0]); // x, y
    bmhd.push(ilbm.planes);
    bmhd.push(ilbm.masking.to_u8());
    bmhd.push(match compression {
        Compression::None => 0,
        Compression::ByteRun1 => 1,
    });
    bmhd.push(0);
    bmhd.extend_from_slice(&ilbm.transparent_color.to_be_bytes());
    bmhd.push(ilbm.x_aspect);
    bmhd.push(ilbm.y_aspect);
    bmhd.extend_from_slice(&ilbm.page_width.to_be_bytes());
    bmhd.extend_from_slice(&ilbm.page_height.to_be_bytes());
    out.chunk(*b"BMHD", &bmhd);

    let cmap: Vec<u8> = ilbm
        .palette
        .colors()
        .iter()
        .flat_map(|c| [c.r, c.g, c.b])
        .collect();
    out.chunk(*b"CMAP", &cmap);
    if let Some((x, y)) = ilbm.grab {
        let mut d = x.to_be_bytes().to_vec();
        d.extend_from_slice(&y.to_be_bytes());
        out.chunk(*b"GRAB", &d);
    }
    if let Some(camg) = ilbm.camg {
        out.chunk(*b"CAMG", &camg.to_be_bytes());
    }
    for r in &ilbm.ranges {
        let mut d = vec![0, 0];
        d.extend_from_slice(&r.rate.to_be_bytes());
        d.extend_from_slice(&r.flags.to_be_bytes());
        d.push(r.low);
        d.push(r.high);
        out.chunk(*b"CRNG", &d);
    }
    for c in &ilbm.other_chunks {
        out.chunk(c.id, &c.data);
    }
    out.chunk(*b"BODY", &encode_body(ilbm, compression));
    Ok(out.finish())
}

fn encode_body(ilbm: &Ilbm, compression: Compression) -> Vec<u8> {
    let (w, h) = (ilbm.image.width() as usize, ilbm.image.height() as usize);
    let rb = row_bytes(w);
    let px = ilbm.image.pixels();
    let mut body = Vec::new();
    let mut row = vec![0u8; rb];
    let emit = |row: &[u8], body: &mut Vec<u8>| match compression {
        Compression::None => body.extend_from_slice(row),
        Compression::ByteRun1 => body.extend_from_slice(&byterun1::pack_row(row)),
    };
    for y in 0..h {
        let line = &px[y * w..(y + 1) * w];
        for p in 0..ilbm.planes {
            row.fill(0);
            for (x, &c) in line.iter().enumerate() {
                if c & (1 << p) != 0 {
                    row[x >> 3] |= 0x80 >> (x & 7);
                }
            }
            emit(&row, &mut body);
        }
        if let Some(m) = &ilbm.mask {
            row.fill(0);
            for (x, &opaque) in m[y * w..(y + 1) * w].iter().enumerate() {
                if opaque {
                    row[x >> 3] |= 0x80 >> (x & 7);
                }
            }
            emit(&row, &mut body);
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(w: u32, h: u32, colors: usize) -> Ilbm {
        let mut img = IndexedImage::new(w, h, 0);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                img.set(
                    x,
                    y,
                    ((x * 7 + y * 3 + (x * y) % 5) as usize % colors) as u8,
                );
            }
        }
        let pal = Palette::new(
            (0..colors)
                .map(|i| Rgb8::new(i as u8, 255 - i as u8, (i * 3) as u8))
                .collect(),
        )
        .unwrap();
        Ilbm::from_indexed(img, pal)
    }

    /// A file assembled by hand from the spec, independent of our writer: 16×1 pixels,
    /// 2 planes, 4-bit style CMAP, BODY once uncompressed and once hand-packed ByteRun1.
    #[test]
    fn decodes_hand_built_fixture() {
        // Plane 0 = AA 0F, plane 1 = CC F0 → indices 3,2,1,0, 3,2,1,0, 2,2,2,2, 1,1,1,1.
        let expected = [3, 2, 1, 0, 3, 2, 1, 0, 2, 2, 2, 2, 1, 1, 1, 1];
        for (compression, body) in [
            (0u8, vec![0xAA, 0x0F, 0xCC, 0xF0]),
            (1u8, vec![0x01, 0xAA, 0x0F, 0x01, 0xCC, 0xF0]),
        ] {
            let mut bmhd = vec![0, 16, 0, 1, 0, 0, 0, 0, 2, 0, compression, 0, 0, 0, 10, 11];
            bmhd.extend_from_slice(&[1, 64, 0, 200]); // page 320×200
            let cmap = [
                0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00, 0xF0,
            ];
            let mut f = iff::FormWriter::new(*b"ILBM");
            f.chunk(*b"BMHD", &bmhd);
            f.chunk(*b"CMAP", &cmap);
            f.chunk(*b"BODY", &body);
            let pic = read(&f.finish()).unwrap();
            assert_eq!(pic.image.pixels(), &expected);
            assert_eq!((pic.page_width, pic.page_height), (320, 200));
            assert_eq!((pic.x_aspect, pic.y_aspect), (10, 11));
            assert!(pic.cmap_4bit_fixed);
            assert_eq!(pic.palette.get(1), Some(Rgb8::new(0xFF, 0, 0)));
            assert_eq!(pic.palette.get(3), Some(Rgb8::new(0, 0, 0xFF)));
        }
    }

    #[test]
    fn round_trip_all_depths_and_odd_widths() {
        for (w, h, colors) in [
            (1, 1, 2),
            (15, 3, 4),
            (17, 5, 8),
            (320, 200, 32),
            (33, 9, 64),
            (100, 7, 128),
            (321, 4, 256),
        ] {
            let mut src = sample(w, h, colors);
            assert_eq!(src.planes, planes_for(colors));
            if src.planes == 6 {
                // Without CAMG a 6-plane file is HAM or EHB; a plain mode id marks 64 colours.
                src.camg = Some(0);
            }
            for comp in [Compression::None, Compression::ByteRun1] {
                let bytes = write(&src, comp).unwrap();
                let back = read(&bytes).unwrap();
                assert_eq!(back.image, src.image, "{w}x{h} {colors} {comp:?}");
                assert_eq!(back.palette, src.palette);
                assert_eq!(back.compression, comp);
            }
        }
    }

    #[test]
    fn metadata_round_trip() {
        let mut src = sample(20, 10, 32);
        src.masking = Masking::HasMask;
        src.mask = Some((0..200).map(|i| i % 3 != 0).collect());
        src.camg = Some(0x0002_9000);
        src.ranges = vec![
            ColorRange {
                rate: 8192,
                flags: 1,
                low: 20,
                high: 31,
            },
            ColorRange {
                rate: 0,
                flags: 0,
                low: 0,
                high: 0,
            },
        ];
        src.grab = Some((-3, 4));
        src.x_aspect = 10;
        src.y_aspect = 11;
        src.other_chunks = vec![RawChunk {
            id: *b"DPPS",
            data: vec![1, 2, 3],
        }];
        let back = read(&write(&src, Compression::ByteRun1).unwrap()).unwrap();
        assert_eq!(back, src);
    }

    #[test]
    fn cmap_4bit_fix_and_padding() {
        let mut src = sample(8, 2, 4);
        src.palette = Palette::new(vec![
            Rgb8::new(0xF0, 0x00, 0x80),
            Rgb8::new(0x10, 0x20, 0x30),
            Rgb8::BLACK,
            Rgb8::WHITE,
        ])
        .unwrap();
        // White (0xFF) has low nibbles set, so no fix for this palette.
        assert!(
            !read(&write(&src, Compression::None).unwrap())
                .unwrap()
                .cmap_4bit_fixed
        );
        src.palette = Palette::new(vec![
            Rgb8::new(0xF0, 0x00, 0x80),
            Rgb8::new(0x10, 0x20, 0x30),
            Rgb8::BLACK,
            Rgb8::new(0xA0, 0xA0, 0xA0),
        ])
        .unwrap();
        let back = read(&write(&src, Compression::None).unwrap()).unwrap();
        assert!(back.cmap_4bit_fixed);
        assert_eq!(back.palette.get(0), Some(Rgb8::new(0xFF, 0x00, 0x88)));
        assert_eq!(back.palette.get(3), Some(Rgb8::new(0xAA, 0xAA, 0xAA)));
    }

    #[test]
    fn short_cmap_is_padded_to_the_plane_count() {
        let src = sample(16, 1, 4);
        let mut bytes = write(&src, Compression::None).unwrap();
        // Patch BMHD planes from 2 to 3: 8 colours expected, CMAP has 4.
        let planes_at = 12 + 8 + 8;
        bytes[planes_at] = 3;
        // The BODY now lacks a plane per row, so it must fail cleanly rather than panic.
        assert!(matches!(read(&bytes), Err(IlbmError::BadBody(_))));
    }

    #[test]
    fn writer_refuses_silent_loss() {
        let mut src = sample(4, 4, 4);
        src.palette = Palette::new(vec![Rgb8::BLACK; 2]).unwrap();
        assert!(matches!(
            write(&src, Compression::None),
            Err(IlbmError::Invalid(_))
        ));
        let mut src = sample(4, 4, 4);
        src.planes = 1;
        assert!(matches!(
            write(&src, Compression::None),
            Err(IlbmError::Invalid(_))
        ));
        let mut src = sample(4, 4, 4);
        src.masking = Masking::HasMask;
        assert!(matches!(
            write(&src, Compression::None),
            Err(IlbmError::Invalid(_))
        ));
    }

    #[test]
    fn refuses_unsupported_variants() {
        let mut src = sample(4, 2, 64);
        src.camg = Some(CAMG_EHB);
        let bytes = write(&src, Compression::None).unwrap();
        assert!(matches!(read(&bytes), Err(IlbmError::Unsupported(_))));
        src.camg = None;
        let bytes = write(&src, Compression::None).unwrap();
        assert!(matches!(read(&bytes), Err(IlbmError::Unsupported(_))));
        src.camg = Some(CAMG_HAM);
        let bytes = write(&src, Compression::None).unwrap();
        assert!(matches!(read(&bytes), Err(IlbmError::Unsupported(_))));

        let mut pbm = iff::FormWriter::new(*b"PBM ");
        pbm.chunk(*b"BMHD", &[0; 20]);
        assert_eq!(read(&pbm.finish()), Err(IlbmError::NotIlbm("PBM ".into())));
    }

    #[test]
    fn ccrt_becomes_a_range() {
        let mut d = Vec::new();
        d.extend_from_slice(&(-1i16).to_be_bytes());
        d.extend_from_slice(&[4, 9]);
        d.extend_from_slice(&0u32.to_be_bytes());
        d.extend_from_slice(&(1_000_000u32 / 30).to_be_bytes());
        d.extend_from_slice(&[0, 0]);
        let r = ccrt_to_range(&d);
        assert_eq!((r.low, r.high, r.active(), r.reverse()), (4, 9, true, true));
        assert!((r.steps_per_second() - 30.0).abs() < 0.1);
    }
}
