//! ILBM pictures: BMHD, CMAP (with the 4-bit fix), CAMG, CRNG/CCRT, GRAB and the BODY in
//! interleaved bitplanes, uncompressed or ByteRun1 (`docs/IFF_SUPPORT_MATRIX.md` §2–3).
//!
//! EHB, HAM, DEEP and PBM files are detected and refused with a clear message; they are
//! planned for later phases.
//!
//! Diagnostics (ADR-17): errors say which chunk, at which file offset, and what is wrong,
//! with a short first line for the file requester. Common writer quirks that leave the
//! picture intact (or recoverable) are tolerated and reported as notes, see
//! [`read_with_notes`].

use std::fmt;

use dpx_core::{IndexedImage, Palette, Rgb8};

use crate::byterun1::{self, UnpackError};
use crate::iff::{self, Chunk, Form, FourCc, IffError, at, be_i16, be_u16, be_u32};

/// CAMG flag: Extra-Half-Brite.
pub const CAMG_EHB: u32 = 0x0080;
/// CAMG flag: Hold-And-Modify.
pub const CAMG_HAM: u32 = 0x0800;

/// Largest picture the reader accepts (64 megapixels, as for PNG and JPEG). Protects against
/// corrupt BMHDs that would need gigabytes of memory.
pub const MAX_PIXELS: usize = 64 * 1024 * 1024;

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

/// What kind of problem an [`IlbmError`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Not an IFF FORM, or the container is broken ([`IffError`]).
    Iff,
    /// A FORM of another type (PBM, ACBM, ANIM, 8SVX …).
    NotIlbm,
    /// BMHD or BODY is missing.
    MissingChunk,
    /// A fixed-layout chunk is shorter than its layout or cut off by the end of the file.
    ShortChunk,
    /// BMHD values that make no sense (zero or absurd size, plane count or masking).
    BadHeader,
    /// Valid IFF that DeluxePainter cannot read yet (HAM, EHB, true colour, compression).
    Unsupported,
    /// Corrupt pixel data.
    BadBody,
    /// The picture cannot be written as described (export validation).
    Invalid,
}

/// Why an ILBM could not be read or written.
///
/// `Display` gives [`IlbmError::summary`] as the first line and the details below it, so
/// callers can show the first line in a one-line message and print the whole text to the
/// terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IlbmError {
    pub kind: ErrorKind,
    /// One short line (at most 34 characters) for the file requester's message line.
    pub summary: String,
    /// Which chunk, which offset, what exactly is wrong; may be empty.
    pub details: String,
}

impl IlbmError {
    fn new(kind: ErrorKind, summary: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
            details: details.into(),
        }
    }

    /// An export validation error: one line, no details.
    fn invalid(what: String) -> Self {
        Self::new(ErrorKind::Invalid, format!("cannot write ILBM: {what}"), "")
    }
}

impl fmt::Display for IlbmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary)?;
        if !self.details.is_empty() {
            write!(f, "\n{}", self.details)?;
        }
        Ok(())
    }
}

impl std::error::Error for IlbmError {}

impl From<IffError> for IlbmError {
    fn from(e: IffError) -> Self {
        IlbmError::new(ErrorKind::Iff, e.summary(), e.details())
    }
}

struct Bmhd {
    /// File offset of the BMHD chunk header.
    offset: usize,
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

/// A chunk that is shorter than its fixed layout: why, in words.
fn too_short(c: &Chunk<'_>, need: usize) -> String {
    let id = iff::name(c.id);
    if c.is_cut() && c.size >= need {
        format!(
            "{id} at {} is cut off: the file ends after {} of its {} bytes",
            at(c.offset),
            c.data.len(),
            c.size
        )
    } else {
        format!(
            "{id} at {} has {}, {need} needed",
            at(c.offset),
            iff::byte_count(c.data.len())
        )
    }
}

fn parse_bmhd(c: &Chunk<'_>) -> Result<Bmhd, IlbmError> {
    let d = c.data;
    if d.len() < 20 {
        let summary = if c.is_cut() {
            "Picture header (BMHD) is cut off"
        } else {
            "Picture header (BMHD) too short"
        };
        return Err(IlbmError::new(
            ErrorKind::ShortChunk,
            summary,
            format!("{}.", too_short(c, 20)),
        ));
    }
    let masking = Masking::from_u8(d[9]).ok_or_else(|| {
        IlbmError::new(
            ErrorKind::BadHeader,
            format!("Unknown masking type {}", d[9]),
            format!(
                "BMHD at {} says masking {}; ILBM knows 0 (none), 1 (mask plane), \
                 2 (transparent colour) and 3 (lasso).",
                at(c.offset),
                d[9]
            ),
        )
    })?;
    let compression = match d[10] {
        0 => Compression::None,
        1 => Compression::ByteRun1,
        n => {
            return Err(IlbmError::new(
                ErrorKind::Unsupported,
                format!("Unknown compression type {n}"),
                format!(
                    "BMHD at {} says compression {n}; ILBM knows 0 (none) and 1 (ByteRun1).",
                    at(c.offset)
                ),
            ));
        }
    };
    Ok(Bmhd {
        offset: c.offset,
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

/// Refuses headers that cannot be decoded: zero or absurd size, plane counts outside 0..=8.
fn check_bmhd(b: &Bmhd) -> Result<(), IlbmError> {
    let (w, h) = (usize::from(b.w), usize::from(b.h));
    if w == 0 || h == 0 {
        return Err(IlbmError::new(
            ErrorKind::BadHeader,
            format!("Bad picture size {w}x{h}"),
            format!(
                "BMHD at {} says {w}x{h} pixels; a picture needs at least 1x1.",
                at(b.offset)
            ),
        ));
    }
    if w * h > MAX_PIXELS {
        return Err(IlbmError::new(
            ErrorKind::BadHeader,
            format!("Picture too big: {w}x{h}"),
            format!(
                "BMHD at {} says {w}x{h} pixels; DeluxePainter reads pictures up to 64 \
                 megapixels. The header may be corrupt.",
                at(b.offset)
            ),
        ));
    }
    match b.planes {
        0..=8 => Ok(()),
        24 | 32 => Err(IlbmError::new(
            ErrorKind::Unsupported,
            format!("{}-bit ILBM: not supported yet", b.planes),
            format!(
                "BMHD at {} says {} bitplanes: a true-colour ILBM. DeluxePainter reads 1 to 8 \
                 bitplanes for now.",
                at(b.offset),
                b.planes
            ),
        )),
        n => Err(IlbmError::new(
            ErrorKind::BadHeader,
            format!("Bad BMHD: {n} bitplanes"),
            format!(
                "BMHD at {} says {n} bitplanes; ILBM pictures have 1 to 8 (or 24 and 32 for \
                 true colour).",
                at(b.offset)
            ),
        )),
    }
}

/// Refuses display modes DeluxePainter cannot show yet (HAM, EHB: Phase 5).
fn check_mode(planes: u8, camg: Option<(u32, usize)>) -> Result<(), IlbmError> {
    let later = "DeluxePainter shows HAM and Extra-Half-Brite pictures from Phase 5 on.";
    let refuse = |summary: &str, what: &str, (mode, offset): (u32, usize)| {
        Err(IlbmError::new(
            ErrorKind::Unsupported,
            summary,
            format!(
                "CAMG at {} is ${mode:08X}: {what} with {planes} bitplanes. {later}",
                at(offset)
            ),
        ))
    };
    match camg {
        Some(c) if c.0 & CAMG_HAM != 0 => {
            refuse("HAM pictures arrive in Phase 5", "Hold-And-Modify", c)
        }
        Some(c) if c.0 & CAMG_EHB != 0 => {
            refuse("EHB pictures arrive in Phase 5", "Extra-Half-Brite", c)
        }
        None if planes == 6 => Err(IlbmError::new(
            ErrorKind::Unsupported,
            "HAM/EHB arrive in Phase 5",
            format!(
                "6 bitplanes and no CAMG chunk: an Amiga shows this as HAM or \
                 Extra-Half-Brite. {later}"
            ),
        )),
        _ => Ok(()),
    }
}

/// A picture without a CMAP: a grey ramp, so it is at least visible.
fn grey_ramp(count: usize) -> Vec<Rgb8> {
    let max = (count - 1).max(1);
    (0..count)
        .map(|i| {
            let v = (i * 255 / max) as u8;
            Rgb8::new(v, v, v)
        })
        .collect()
}

/// Expands a CMAP to exactly `count` entries; returns the palette and whether the 4-bit fix
/// was applied. Anything that does not fit is reported in `notes`.
fn parse_cmap(
    cmap: Option<&Chunk<'_>>,
    count: usize,
    planes: u8,
    notes: &mut Vec<String>,
) -> (Palette, bool) {
    let palette = |colors: Vec<Rgb8>| Palette::new(colors).expect("1..=256 entries");
    let Some(c) = cmap else {
        if planes > 0 {
            notes.push("no CMAP chunk: a grey palette is used".into());
        }
        return (palette(grey_ramp(count)), false);
    };
    let bytes = c.data;
    let n = bytes.len() / 3;
    if bytes.len() % 3 != 0 {
        notes.push(format!(
            "CMAP at {} has {} bytes, not a multiple of 3: the last {} ignored",
            at(c.offset),
            bytes.len(),
            match bytes.len() % 3 {
                1 => "byte is",
                _ => "2 bytes are",
            }
        ));
    }
    if n == 0 {
        notes.push(format!(
            "CMAP at {} holds no complete colour: a grey palette is used",
            at(c.offset)
        ));
        return (palette(grey_ramp(count)), false);
    }
    let fix = bytes[..n * 3].iter().all(|b| b & 0x0F == 0);
    let fixed = |v: u8| if fix { v | (v >> 4) } else { v };
    let mut colors: Vec<Rgb8> = bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| Rgb8::new(fixed(c[0]), fixed(c[1]), fixed(c[2])))
        .collect();
    let colours = |n: usize| if n == 1 { "colour" } else { "colours" };
    if n < count {
        let missing = if count - n == 1 {
            format!("colour {n} is")
        } else {
            format!("colours {n}-{} are", count - 1)
        };
        notes.push(format!(
            "CMAP has {n} {} for {count} ({planes} bitplanes): {missing} black",
            colours(n)
        ));
    } else if n > count {
        let dropped = if n - count == 1 {
            format!("colour {count} is")
        } else {
            format!("colours {count}-{} are", n - 1)
        };
        let uses = if planes == 0 {
            format!("a palette holds at most {count}")
        } else {
            format!("{planes} bitplanes show {count}")
        };
        notes.push(format!(
            "CMAP has {n} colours but {uses}: {dropped} not kept"
        ));
    }
    colors.resize(count, Rgb8::BLACK);
    (palette(colors), fix)
}

/// Reads an ILBM file. Tolerated quirks are dropped silently here; use [`read_with_notes`]
/// to tell the user about them.
pub fn read(bytes: &[u8]) -> Result<Ilbm, IlbmError> {
    read_with_notes(bytes).map(|(ilbm, _)| ilbm)
}

/// Reads an ILBM file and returns notes (lower-case sentences) about everything that was
/// tolerated rather than refused (ADR-17): container quirks from [`iff::read_form`], short
/// or odd CMAPs, a BODY that ends early (the rest of the picture is colour 0), short CAMG,
/// CRNG, CCRT or GRAB chunks (ignored), chunks with invalid ids, chunks after the BODY,
/// lasso masking (read as transparent colour).
pub fn read_with_notes(bytes: &[u8]) -> Result<(Ilbm, Vec<String>), IlbmError> {
    let form = iff::read_form(bytes)?;
    if &form.form_type != b"ILBM" {
        return Err(foreign_form(form.form_type));
    }
    let mut notes = form.notes.clone();
    let mut bmhd = None;
    let mut cmap = None;
    let mut camg = None;
    let mut body = None;
    let mut ranges = Vec::new();
    let mut ccrt = Vec::new();
    let mut grab = None;
    let mut other_chunks = Vec::new();
    for (i, c) in form.chunks.iter().enumerate() {
        let mut fixed = |need: usize| {
            let d = c.data.get(..need);
            if d.is_none() {
                notes.push(format!("{}: ignored", too_short(c, need)));
            }
            d
        };
        match &c.id {
            b"BMHD" => bmhd = Some(parse_bmhd(c)?),
            b"CMAP" => cmap = Some(c),
            b"CAMG" => {
                if let Some(d) = fixed(4) {
                    camg = Some((be_u32(d), c.offset));
                }
            }
            b"CRNG" => {
                if let Some(d) = fixed(8) {
                    ranges.push(ColorRange {
                        rate: be_u16(&d[2..4]),
                        flags: be_u16(&d[4..6]),
                        low: d[6],
                        high: d[7],
                    });
                }
            }
            b"CCRT" => {
                if let Some(d) = fixed(14) {
                    ccrt.push(ccrt_to_range(d));
                }
            }
            b"GRAB" => {
                if let Some(d) = fixed(4) {
                    grab = Some((be_i16(&d[0..2]), be_i16(&d[2..4])));
                }
            }
            b"BODY" => {
                body = Some(c);
                let after: Vec<String> = form.chunks[i + 1..]
                    .iter()
                    .map(|c| iff::name(c.id))
                    .collect();
                if !after.is_empty() {
                    notes.push(format!(
                        "{} after the BODY not read: {}",
                        if after.len() == 1 { "chunk" } else { "chunks" },
                        list(&after)
                    ));
                }
                break;
            }
            b"DEEP" => {
                return Err(IlbmError::new(
                    ErrorKind::Unsupported,
                    "DEEP ILBM: not supported yet",
                    format!(
                        "The DEEP chunk at {} holds true-colour pixels. DeluxePainter reads \
                         1 to 8 bitplanes for now.",
                        at(c.offset)
                    ),
                ));
            }
            id if !iff::is_valid_id(id) => notes.push(format!(
                "chunk with the invalid id {} at {} skipped ({})",
                iff::quoted(*id),
                at(c.offset),
                iff::byte_count(c.data.len())
            )),
            _ => other_chunks.push(RawChunk {
                id: c.id,
                data: c.data.to_vec(),
            }),
        }
    }
    let Some(mut bmhd) = bmhd else {
        return Err(missing_bmhd(&form, body));
    };
    if ranges.is_empty() {
        ranges = ccrt;
    }
    check_bmhd(&bmhd)?;
    if bmhd.planes > 0 {
        // A palette-only file (no planes) has no pixels to interpret in a display mode.
        check_mode(bmhd.planes, camg)?;
    }
    if bmhd.masking == Masking::Lasso {
        notes.push(format!(
            "lasso masking is read as transparent colour {}; the outline fill is not done",
            bmhd.transparent_color
        ));
        bmhd.masking = Masking::TransparentColor;
    }
    if let Some(offset) = form.cut_header
        && body.is_some()
    {
        notes.push(format!(
            "{} stray bytes at the end of the file are ignored (at {})",
            bytes.len() - offset,
            at(offset)
        ));
    }

    let colors = if bmhd.planes == 0 {
        cmap.map_or(1, |c| (c.data.len() / 3).clamp(1, 256))
    } else {
        1usize << bmhd.planes
    };
    let (palette, cmap_4bit_fixed) = parse_cmap(cmap, colors, bmhd.planes, &mut notes);
    let (w, h) = (usize::from(bmhd.w), usize::from(bmhd.h));
    let (pixels, mask) = if bmhd.planes == 0 {
        (vec![0; w * h], None)
    } else {
        let body = body.ok_or_else(|| missing_body(&form, bmhd.planes))?;
        decode_body(&bmhd, body, &mut notes)?
    };
    let ilbm = Ilbm {
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
        camg: camg.map(|(v, _)| v),
        ranges,
        grab,
        other_chunks,
        cmap_4bit_fixed,
    };
    Ok((ilbm, notes))
}

/// `a, b, c`: at most eight names, then `and N more`.
fn list(names: &[String]) -> String {
    const MAX: usize = 8;
    let shown = &names[..names.len().min(MAX)];
    let mut s = shown.join(", ");
    if names.len() > MAX {
        s += &format!(" and {} more", names.len() - MAX);
    }
    s
}

/// The chunks of a FORM for an error message: `BMHD at 12, CMAP at 40`.
fn chunk_list(form: &Form<'_>) -> String {
    if form.chunks.is_empty() {
        return "none".into();
    }
    let names: Vec<String> = form
        .chunks
        .iter()
        .map(|c| format!("{} at {}", iff::name(c.id), c.offset))
        .collect();
    list(&names)
}

/// Why the file may have lost chunks at its end, as a sentence (or empty).
fn truncation(form: &Form<'_>) -> String {
    if let Some(offset) = form.cut_header {
        format!(
            "The file ends inside a chunk header at {}: it is truncated. ",
            at(offset)
        )
    } else if let Some(c) = form.chunks.last().filter(|c| c.is_cut()) {
        format!(
            "The file ends inside the {} chunk at {} ({} of {} bytes): it is truncated. ",
            iff::name(c.id),
            at(c.offset),
            c.data.len(),
            c.size
        )
    } else if form.missing > 0 {
        format!(
            "The file is {} shorter than its FORM header says: it is truncated. ",
            iff::byte_count(form.missing)
        )
    } else {
        String::new()
    }
}

fn missing_bmhd(form: &Form<'_>, body: Option<&Chunk<'_>>) -> IlbmError {
    let before = body.map_or(String::new(), |b| {
        format!(" before the BODY at {}", at(b.offset))
    });
    IlbmError::new(
        ErrorKind::MissingChunk,
        "No picture header (BMHD missing)",
        format!(
            "There is no BMHD chunk{before}. {}Chunks found: {}.",
            truncation(form),
            chunk_list(form)
        ),
    )
}

fn missing_body(form: &Form<'_>, planes: u8) -> IlbmError {
    IlbmError::new(
        ErrorKind::MissingChunk,
        "No pixel data (BODY missing)",
        format!(
            "BMHD says {planes} bitplanes, but there is no BODY chunk. {}Chunks found: {}.",
            truncation(form),
            chunk_list(form)
        ),
    )
}

/// The error for a FORM that is not an ILBM; known picture formats are named.
fn foreign_form(t: FourCc) -> IlbmError {
    let name = iff::name(t);
    let (summary, details) = match &t {
        b"PBM " => (
            "PBM pictures: not supported yet".to_string(),
            "This is a FORM PBM picture (chunky pixels, as written by DeluxePaint on the PC). \
             DeluxePainter reads FORM ILBM for now."
                .to_string(),
        ),
        b"ACBM" => (
            "ACBM pictures: not supported yet".to_string(),
            "This is a FORM ACBM picture (contiguous bitplanes, as written by AmigaBASIC \
             programs). DeluxePainter reads FORM ILBM for now."
                .to_string(),
        ),
        b"ANIM" => (
            "ANIM files arrive in Phase 6".to_string(),
            "This is an IFF ANIM animation. DeluxePainter loads animations from Phase 6 on; \
             until then it reads single FORM ILBM pictures."
                .to_string(),
        ),
        b"DEEP" | b"RGBN" | b"RGB8" | b"TVPP" => (
            format!("{name} pictures: not supported yet"),
            format!(
                "This is a true-colour FORM {name} picture. DeluxePainter reads FORM ILBM \
                 with 1 to 8 bitplanes."
            ),
        ),
        _ => {
            let what = match &t {
                b"8SVX" | b"AIFF" | b"16SV" => " (a sound)",
                b"SMUS" => " (a music score)",
                b"FTXT" => " (text)",
                _ => "",
            };
            (
                format!("IFF FORM {name}: not a picture"),
                format!(
                    "DeluxePainter loads FORM ILBM pictures; this file is an IFF FORM \
                     {name}{what}."
                ),
            )
        }
    };
    IlbmError::new(ErrorKind::NotIlbm, summary, details)
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

/// `plane 3` or `mask plane`.
fn plane_name(p: usize, planes: usize) -> String {
    if p == planes {
        "mask plane".into()
    } else {
        format!("plane {p}")
    }
}

/// Decodes interleaved bitplanes into indices (+ mask plane). A BODY that ends early is
/// tolerated: every plane row that is not complete stays 0 (colour 0, mask transparent) and
/// a note says where the data ended. A ByteRun1 run that does not fit its row is an error.
fn decode_body(
    bmhd: &Bmhd,
    body: &Chunk<'_>,
    notes: &mut Vec<String>,
) -> Result<(Vec<u8>, Option<Vec<bool>>), IlbmError> {
    let data = body.data;
    let (w, h) = (usize::from(bmhd.w), usize::from(bmhd.h));
    let rb = row_bytes(w);
    let planes = usize::from(bmhd.planes);
    let has_mask = bmhd.masking == Masking::HasMask;
    let rows_per_line = planes + usize::from(has_mask);
    let mut pixels = vec![0u8; w * h];
    let mut mask = has_mask.then(|| vec![false; w * h]);
    let mut pos = 0;
    let mut cut = None;
    'rows: for y in 0..h {
        for p in 0..rows_per_line {
            let row: std::borrow::Cow<[u8]> = match bmhd.compression {
                Compression::None => match data.get(pos..pos + rb) {
                    Some(r) => {
                        pos += rb;
                        r.into()
                    }
                    None => {
                        cut = Some((y, p));
                        break 'rows;
                    }
                },
                Compression::ByteRun1 => {
                    match byterun1::unpack_row(&data[pos.min(data.len())..], rb) {
                        Ok((r, used)) => {
                            pos += used;
                            r.into()
                        }
                        Err(UnpackError::Truncated { .. }) => {
                            cut = Some((y, p));
                            break 'rows;
                        }
                        Err(UnpackError::Overrun {
                            at: run_at,
                            count,
                            left,
                            repeat,
                        }) => {
                            let offset = body.offset + 8 + pos + run_at;
                            return Err(IlbmError::new(
                                ErrorKind::BadBody,
                                format!("BODY: bad ByteRun1 run, row {y}"),
                                format!(
                                    "At {} a ByteRun1 {} of {count} bytes does not fit: {} of row \
                                 {y} has {left} of its {rb} bytes left. The BODY is corrupt (or \
                                 packed across rows, which ILBM does not allow).",
                                    at(offset),
                                    if repeat { "repeat" } else { "literal" },
                                    plane_name(p, planes)
                                ),
                            ));
                        }
                    }
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
    if let Some((y, p)) = cut {
        let why = if body.is_cut() {
            "the file ends inside the BODY"
        } else {
            "the BODY chunk is too short"
        };
        let size = match bmhd.compression {
            Compression::None => format!(", {} of {} bytes", data.len(), rb * rows_per_line * h),
            Compression::ByteRun1 => String::new(),
        };
        notes.push(if y == 0 && p == 0 {
            format!(
                "BODY at {} holds no pixel data ({why}{size}): the picture is colour 0",
                at(body.offset)
            )
        } else {
            format!(
                "BODY ends in row {y} of {h}, {} ({why}{size}): the rest of the picture is \
                 colour 0",
                plane_name(p, planes)
            )
        });
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
    let invalid = |m: String| Err(IlbmError::invalid(m));
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
            let (pic, notes) = read_with_notes(&f.finish()).unwrap();
            assert!(notes.is_empty(), "{notes:?}");
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
                let (back, notes) = read_with_notes(&bytes).unwrap();
                assert!(notes.is_empty(), "{notes:?}");
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
    fn more_planes_than_the_body_holds_reads_with_notes() {
        let src = sample(16, 1, 4);
        let mut bytes = write(&src, Compression::None).unwrap();
        // Patch BMHD planes from 2 to 3: 8 colours expected, CMAP has 4, and the BODY now
        // lacks a plane row. Both are tolerated with a note instead of a panic.
        let planes_at = 12 + 8 + 8;
        bytes[planes_at] = 3;
        let (pic, notes) = read_with_notes(&bytes).unwrap();
        assert_eq!(pic.palette.len(), 8);
        assert_eq!(
            notes,
            [
                "CMAP has 4 colours for 8 (3 bitplanes): colours 4-7 are black",
                "BODY ends in row 0 of 1, plane 2 (the BODY chunk is too short, 4 of 6 bytes): \
                 the rest of the picture is colour 0",
            ]
        );
    }

    #[test]
    fn writer_refuses_silent_loss() {
        let mut src = sample(4, 4, 4);
        src.palette = Palette::new(vec![Rgb8::BLACK; 2]).unwrap();
        let invalid = |r: Result<Vec<u8>, IlbmError>| r.unwrap_err().kind == ErrorKind::Invalid;
        assert!(invalid(write(&src, Compression::None)));
        let mut src = sample(4, 4, 4);
        src.planes = 1;
        assert!(invalid(write(&src, Compression::None)));
        let mut src = sample(4, 4, 4);
        src.masking = Masking::HasMask;
        let e = write(&src, Compression::None).unwrap_err();
        assert_eq!(
            e.to_string(),
            "cannot write ILBM: mask plane does not match the masking type"
        );
    }

    #[test]
    fn refuses_unsupported_variants() {
        let mut src = sample(4, 2, 64);
        src.camg = Some(CAMG_EHB);
        let bytes = write(&src, Compression::None).unwrap();
        let kind = |b: &[u8]| read(b).unwrap_err().kind;
        assert_eq!(kind(&bytes), ErrorKind::Unsupported);
        src.camg = None;
        let bytes = write(&src, Compression::None).unwrap();
        assert_eq!(kind(&bytes), ErrorKind::Unsupported);
        src.camg = Some(CAMG_HAM);
        let bytes = write(&src, Compression::None).unwrap();
        assert_eq!(kind(&bytes), ErrorKind::Unsupported);

        let mut pbm = iff::FormWriter::new(*b"PBM ");
        pbm.chunk(*b"BMHD", &[0; 20]);
        let e = read(&pbm.finish()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotIlbm);
        assert_eq!(e.summary, "PBM pictures: not supported yet");
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
