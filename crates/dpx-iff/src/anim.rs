//! IFF ANIM: DeluxePaint animations (`FORM ANIM` of `FORM ILBM` frames), ADR-29.
//!
//! A file holds one full picture (first `FORM ILBM`, BMHD/CMAP/BODY) followed by delta
//! frames (`FORM ILBM` with ANHD + DLTA, optionally CMAP). Reading supports DLTA method 0
//! (full ILBM frame) and method 5 (Byte Vertical Delta, plain and XOR); writing produces
//! method 5 with the double-buffering rule and DPaint's two closing loop frames.
//! Errors follow ADR-17: a short first line plus details.

use std::fmt;

use dpx_core::{IndexedImage, Palette, Rgb8};

use crate::iff::{self, Chunk, FormWriter, IffError, at, be_i16, be_u16, be_u32};
use crate::ilbm::{self, ColorRange, Compression, Ilbm, IlbmError, RawChunk, SpecialMode};

/// Total pixels over all frames the reader accepts (guards against absurd allocations).
pub const MAX_TOTAL_PIXELS: usize = 1 << 30;

/// ANHD `operation` value for a full ILBM frame.
pub const OP_SET: u8 = 0;
/// ANHD `operation` value for Byte Vertical Delta (Jim Kent), the DPaint default.
pub const OP_BYTE_VERTICAL: u8 = 5;

/// Longest column an op-5 delta can describe: the op count is one byte and a literal run
/// covers at most 127 rows.
pub const MAX_OP5_HEIGHT: usize = 255 * 127;

/// What kind of problem an [`AnimError`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Not an IFF FORM, or the container is broken.
    Iff,
    /// A FORM of another type than ANIM.
    NotAnim,
    /// The ANIM has no usable first picture or no frames at all.
    MissingFrame,
    /// A frame header (ANHD) or frame structure that makes no sense.
    BadFrame,
    /// Valid ANIM that this reader does not handle (other DLTA methods, HAM frames ...).
    Unsupported,
    /// Corrupt DLTA data.
    BadDelta,
    /// The animation cannot be written as described.
    Invalid,
}

/// Why an ANIM could not be read or written. `Display` is the first line, then details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimError {
    pub kind: ErrorKind,
    /// One short line (at most 34 characters).
    pub summary: String,
    /// Which frame, chunk and offset, and what was expected; may be empty.
    pub details: String,
}

impl AnimError {
    fn new(kind: ErrorKind, summary: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
            details: details.into(),
        }
    }

    fn invalid(summary: impl Into<String>, details: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, summary, details)
    }
}

impl fmt::Display for AnimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary)?;
        if !self.details.is_empty() {
            write!(f, "\n{}", self.details)?;
        }
        Ok(())
    }
}

impl std::error::Error for AnimError {}

impl From<IffError> for AnimError {
    fn from(e: IffError) -> Self {
        AnimError::new(ErrorKind::Iff, e.summary(), e.details())
    }
}

impl From<IlbmError> for AnimError {
    fn from(e: IlbmError) -> Self {
        AnimError::new(ErrorKind::BadFrame, e.summary, e.details)
    }
}

/// The ANHD chunk (frame header), 40 bytes on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Anhd {
    /// DLTA method (0 = full ILBM body, 5 = byte vertical delta ...).
    pub operation: u8,
    /// XOR mode only: plane mask (read and written, not used for decoding).
    pub mask: u8,
    /// XOR mode only: changed area (read and written, not used for decoding).
    pub w: u16,
    pub h: u16,
    pub x: i16,
    pub y: i16,
    /// Unused by every known writer.
    pub abs_time: u32,
    /// Jiffies (1/60 s) since the previous frame was shown.
    pub rel_time: u32,
    /// Frames back that the delta applies to; 0 means 2 (double buffering).
    pub interleave: u8,
    /// Option flags for methods 4 and 5; for method 5 bit 1 or bit 2 means XOR.
    pub bits: u32,
}

impl Anhd {
    pub const SIZE: usize = 40;

    /// Frames back that the delta applies to (0 is the default 2).
    pub fn effective_interleave(&self) -> usize {
        if self.interleave == 0 {
            2
        } else {
            usize::from(self.interleave)
        }
    }

    /// Method 5 delta data are XORed onto the old bytes instead of replacing them. The
    /// sources disagree on the bit (spec table: bit 1 = value 2; DPaint anim-brush notes:
    /// value 4), so for method 5, which has no other option, either bit selects XOR.
    pub fn xor(&self) -> bool {
        self.bits & 0b110 != 0
    }

    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0] = self.operation;
        b[1] = self.mask;
        b[2..4].copy_from_slice(&self.w.to_be_bytes());
        b[4..6].copy_from_slice(&self.h.to_be_bytes());
        b[6..8].copy_from_slice(&self.x.to_be_bytes());
        b[8..10].copy_from_slice(&self.y.to_be_bytes());
        b[10..14].copy_from_slice(&self.abs_time.to_be_bytes());
        b[14..18].copy_from_slice(&self.rel_time.to_be_bytes());
        b[18] = self.interleave;
        b[20..24].copy_from_slice(&self.bits.to_be_bytes());
        b
    }

    /// Parses at least the first 24 bytes of an ANHD chunk.
    pub fn parse(d: &[u8]) -> Option<Self> {
        if d.len() < 24 {
            return None;
        }
        Some(Self {
            operation: d[0],
            mask: d[1],
            w: be_u16(&d[2..4]),
            h: be_u16(&d[4..6]),
            x: be_i16(&d[6..8]),
            y: be_i16(&d[8..10]),
            abs_time: be_u32(&d[10..14]),
            rel_time: be_u32(&d[14..18]),
            interleave: d[18],
            bits: be_u32(&d[20..24]),
        })
    }
}

/// The DPAN chunk of DPaint: version, frame count, flags (8 bytes). It carries no frame rate;
/// timing lives in the ANHD `rel_time` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dpan {
    pub version: u16,
    pub frames: u16,
    pub flags: u32,
}

impl Dpan {
    fn parse(d: &[u8]) -> Option<Self> {
        (d.len() >= 4).then(|| Self {
            version: be_u16(&d[0..2]),
            frames: be_u16(&d[2..4]),
            flags: d.get(4..8).map_or(0, be_u32),
        })
    }

    fn to_bytes(self) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[0..2].copy_from_slice(&self.version.to_be_bytes());
        b[2..4].copy_from_slice(&self.frames.to_be_bytes());
        b[4..8].copy_from_slice(&self.flags.to_be_bytes());
        b
    }
}

/// One frame of a decoded animation, always a full picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimFrame {
    pub image: IndexedImage,
    /// ANHD `operation` of this frame in the file (0 for the first picture).
    pub operation: u8,
    /// ANHD `rel_time`: jiffies (1/60 s) between the previous frame and this one.
    pub rel_time: u32,
    /// Frames back the delta applied to (2 unless the file says otherwise).
    pub interleave: usize,
    /// The full palette in effect from this frame on, when the frame carried a CMAP.
    pub palette: Option<Palette>,
}

/// A decoded ANIM: every frame stored in the file, as a full indexed picture.
#[derive(Clone, Debug)]
pub struct Anim {
    pub width: u32,
    pub height: u32,
    pub planes: u8,
    /// Palette of the first picture.
    pub palette: Palette,
    pub special: Option<SpecialMode>,
    pub camg: Option<u32>,
    pub ranges: Vec<ColorRange>,
    pub x_aspect: u8,
    pub y_aspect: u8,
    /// All frames in file order, the closing loop frames included.
    pub frames: Vec<AnimFrame>,
    /// How many trailing frames of [`Anim::frames`] are DPaint's closing loop frames (0 or 2).
    pub loop_frames: usize,
    pub dpan: Option<Dpan>,
}

impl Anim {
    /// The frames of the animation proper, without the closing loop frames.
    pub fn logical_frames(&self) -> &[AnimFrame] {
        &self.frames[..self.frames.len() - self.loop_frames]
    }

    /// How long logical frame `i` is shown, in jiffies (1/60 s): the `rel_time` of the next
    /// file frame (for the last frame that is the first closing loop frame). Without closing
    /// frames the last frame uses its own `rel_time`. This reading of `rel_time` follows the
    /// ANHD description; it was not checked against DeluxePaint's own playback.
    pub fn hold_jiffies(&self, i: usize) -> u32 {
        match self.frames.get(i + 1) {
            Some(next) => next.rel_time,
            None => self.frames.get(i).map_or(0, |f| f.rel_time),
        }
    }

    /// The common hold time when every logical frame has the same one.
    pub fn jiffies_per_frame(&self) -> Option<u32> {
        let n = self.logical_frames().len();
        let first = self.hold_jiffies(0);
        (0..n)
            .all(|i| self.hold_jiffies(i) == first)
            .then_some(first)
    }

    /// The palette in effect at file frame `i` (the latest per-frame CMAP at or before it).
    pub fn palette_at(&self, i: usize) -> &Palette {
        self.frames
            .iter()
            .take(i + 1)
            .rev()
            .find_map(|f| f.palette.as_ref())
            .unwrap_or(&self.palette)
    }
}

// ----------------------------------------------------------------------------------------
// Planar helpers

fn row_bytes(w: usize) -> usize {
    w.div_ceil(16) * 2
}

/// Chunky to separate bitplanes (`row_bytes * h` bytes each, pad bits 0).
fn to_planes(img: &IndexedImage, planes: usize) -> Vec<Vec<u8>> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let rb = row_bytes(w);
    let mut out = vec![vec![0u8; rb * h]; planes];
    for y in 0..h {
        for x in 0..w {
            let c = img.pixels()[y * w + x];
            for (p, plane) in out.iter_mut().enumerate() {
                if c & (1 << p) != 0 {
                    plane[y * rb + (x >> 3)] |= 0x80 >> (x & 7);
                }
            }
        }
    }
    out
}

fn from_planes(w: usize, h: usize, planes: &[Vec<u8>]) -> IndexedImage {
    let rb = row_bytes(w);
    let mut px = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut c = 0u8;
            for (p, plane) in planes.iter().enumerate() {
                if plane[y * rb + (x >> 3)] & (0x80 >> (x & 7)) != 0 {
                    c |= 1 << p;
                }
            }
            px[y * w + x] = c;
        }
    }
    IndexedImage::from_pixels(w as u32, h as u32, px).expect("size matches")
}

// ----------------------------------------------------------------------------------------
// Method 5, Byte Vertical Delta

/// Applies an op-5 DLTA chunk to bitplanes in place. `xor` XORs the data onto the old bytes.
/// `context` names the frame in error texts.
pub fn apply_op5(
    dlta: &[u8],
    planes: &mut [Vec<u8>],
    w: usize,
    h: usize,
    xor: bool,
    context: &str,
) -> Result<(), AnimError> {
    let bad = |summary: String, details: String| {
        AnimError::new(
            ErrorKind::BadDelta,
            summary,
            format!("{context}: {details}"),
        )
    };
    if dlta.len() < 64 {
        return Err(bad(
            "DLTA too short for method 5".into(),
            format!(
                "the offset table needs 64 bytes, the chunk has {}",
                iff::byte_count(dlta.len())
            ),
        ));
    }
    let rb = row_bytes(w);
    for (p, plane) in planes.iter_mut().enumerate() {
        let off = be_u32(&dlta[p * 4..p * 4 + 4]) as usize;
        if off == 0 {
            continue; // plane unchanged
        }
        if off < 64 || off >= dlta.len() {
            return Err(bad(
                format!("DLTA: bad offset, plane {p}"),
                format!(
                    "plane {p} data starts at byte {off} of a {} chunk (valid: 64 to {})",
                    iff::byte_count(dlta.len()),
                    dlta.len().saturating_sub(1)
                ),
            ));
        }
        let mut pos = off;
        for col in 0..rb {
            let cut = |pos: usize| {
                bad(
                    format!("DLTA cut off, plane {p}"),
                    format!(
                        "plane {p}, column {col}: the data end at byte {} before the column does",
                        pos.min(dlta.len())
                    ),
                )
            };
            let ops = *dlta.get(pos).ok_or_else(|| cut(pos))?;
            pos += 1;
            let mut row = 0usize;
            let over = |row: usize, n: usize| {
                bad(
                    format!("DLTA runs past row {h}, plane {p}"),
                    format!(
                        "plane {p}, column {col}: an operation reaches row {} of {h}",
                        row + n
                    ),
                )
            };
            for _ in 0..ops {
                let b = *dlta.get(pos).ok_or_else(|| cut(pos))?;
                pos += 1;
                if b == 0 {
                    // Same: count, value
                    let n = usize::from(*dlta.get(pos).ok_or_else(|| cut(pos))?);
                    let v = *dlta.get(pos + 1).ok_or_else(|| cut(pos + 1))?;
                    pos += 2;
                    if row + n > h {
                        return Err(over(row, n));
                    }
                    for r in row..row + n {
                        let d = &mut plane[r * rb + col];
                        *d = if xor { *d ^ v } else { v };
                    }
                    row += n;
                } else if b & 0x80 != 0 {
                    let n = usize::from(b & 0x7F);
                    if pos + n > dlta.len() {
                        return Err(cut(dlta.len()));
                    }
                    if row + n > h {
                        return Err(over(row, n));
                    }
                    for (i, &v) in dlta[pos..pos + n].iter().enumerate() {
                        let d = &mut plane[(row + i) * rb + col];
                        *d = if xor { *d ^ v } else { v };
                    }
                    pos += n;
                    row += n;
                } else {
                    let n = usize::from(b);
                    if row + n > h {
                        return Err(over(row, n));
                    }
                    row += n;
                }
            }
        }
    }
    Ok(())
}

/// Encodes one column; `None` when more than 255 operations would be needed.
fn encode_column(reference: &[u8], cur: &[u8], gap: usize, use_same: bool) -> Option<Vec<u8>> {
    let h = cur.len();
    // Segments of rows to write; unchanged gaps of at most `gap` rows are written as well.
    let mut segs = Vec::new();
    let mut r = 0;
    while r < h {
        if cur[r] == reference[r] {
            r += 1;
            continue;
        }
        let s = r;
        let mut e = r + 1;
        loop {
            let mut k = e;
            while k < h && cur[k] == reference[k] {
                k += 1;
            }
            if k < h && k - e <= gap {
                e = k + 1;
            } else {
                break;
            }
        }
        segs.push((s, e));
        r = e;
    }
    let mut out = vec![0u8];
    let mut ops = 0usize;
    let mut pos = 0usize;
    let flush = |out: &mut Vec<u8>, ops: &mut usize, lit: &[u8]| {
        for chunk in lit.chunks(127) {
            out.push(0x80 | chunk.len() as u8);
            out.extend_from_slice(chunk);
            *ops += 1;
        }
    };
    for (s, e) in segs {
        let mut skip = s - pos;
        while skip > 0 {
            let n = skip.min(127);
            out.push(n as u8);
            ops += 1;
            skip -= n;
        }
        let mut i = s;
        let mut lit_start = s;
        while i < e {
            if use_same {
                let mut l = 1;
                while i + l < e && cur[i + l] == cur[i] {
                    l += 1;
                }
                if l >= 4 {
                    flush(&mut out, &mut ops, &cur[lit_start..i]);
                    let n = l.min(255);
                    out.extend_from_slice(&[0, n as u8, cur[i]]);
                    ops += 1;
                    i += n;
                    lit_start = i;
                    continue;
                }
            }
            i += 1;
        }
        flush(&mut out, &mut ops, &cur[lit_start..e]);
        pos = e;
    }
    if ops > 255 {
        return None;
    }
    out[0] = ops as u8;
    Some(out)
}

fn encode_column_best(reference: &[u8], cur: &[u8]) -> Option<Vec<u8>> {
    const ATTEMPTS: [(usize, bool); 7] = [
        (2, true),
        (8, true),
        (32, true),
        (128, true),
        (512, true),
        (usize::MAX, true),
        (usize::MAX, false),
    ];
    ATTEMPTS
        .iter()
        .find_map(|&(gap, same)| encode_column(reference, cur, gap, same))
}

/// Encodes the op-5 DLTA chunk that turns `reference` into `cur` (set mode, not XOR).
///
/// Per plane and per byte column: ops Skip (1..=127 rows), Uniq (literal bytes, 1..=127) and
/// Same (0, count 1..=255, value). Unchanged gaps of up to two rows are written as literals
/// (cheaper than a skip plus a new literal header); when a column would need more than 255
/// operations, gaps are merged more and more until it fits. Planes without changes get the
/// offset 0; plane data start on even offsets.
pub fn encode_op5(
    reference: &IndexedImage,
    cur: &IndexedImage,
    planes: u8,
) -> Result<Vec<u8>, AnimError> {
    if (reference.width(), reference.height()) != (cur.width(), cur.height()) {
        return Err(AnimError::invalid(
            "cannot write ANIM: frame sizes",
            "the two pictures of a delta differ in size",
        ));
    }
    let (w, h) = (cur.width() as usize, cur.height() as usize);
    if h > MAX_OP5_HEIGHT {
        return Err(AnimError::invalid(
            "cannot write ANIM: too tall",
            format!(
                "method 5 stores at most {MAX_OP5_HEIGHT} rows per column (255 operations of \
                 127 rows); the animation is {h} rows high"
            ),
        ));
    }
    let rb = row_bytes(w);
    let pr = to_planes(reference, usize::from(planes));
    let pc = to_planes(cur, usize::from(planes));
    let mut out = vec![0u8; 64];
    let mut rcol = vec![0u8; h];
    let mut ccol = vec![0u8; h];
    for p in 0..usize::from(planes) {
        if pr[p] == pc[p] {
            continue;
        }
        let mut data = Vec::new();
        for col in 0..rb {
            for r in 0..h {
                rcol[r] = pr[p][r * rb + col];
                ccol[r] = pc[p][r * rb + col];
            }
            let c = encode_column_best(&rcol, &ccol).ok_or_else(|| {
                AnimError::invalid("cannot write ANIM: column too busy", "op 5 column overflow")
            })?;
            data.extend_from_slice(&c);
        }
        if data.len() % 2 == 1 {
            data.push(0);
        }
        let off = (out.len() as u32).to_be_bytes();
        out[p * 4..p * 4 + 4].copy_from_slice(&off);
        out.extend_from_slice(&data);
    }
    Ok(out)
}

// ----------------------------------------------------------------------------------------
// Reading

fn method_name(op: u8) -> String {
    match op {
        1 => "method 1 (XOR ILBM)".into(),
        2 => "method 2 (long delta)".into(),
        3 => "method 3 (short delta)".into(),
        4 => "method 4 (generalized delta)".into(),
        6 => "method 6 (stereo op 5)".into(),
        7 => "method 7 (short/long vertical delta)".into(),
        8 => "method 8 (vertical delta)".into(),
        74 => "method J (Eric Graham)".into(),
        n => format!("method {n}"),
    }
}

fn find<'a, 'b>(chunks: &'b [Chunk<'a>], id: &[u8; 4]) -> Option<&'b Chunk<'a>> {
    chunks.iter().find(|c| &c.id == id)
}

/// A frame CMAP: the previous palette with its first entries replaced.
fn apply_cmap(prev: &Palette, c: &Chunk<'_>, notes: &mut Vec<String>, frame: usize) -> Palette {
    let n = c.data.len() / 3;
    if !c.data.len().is_multiple_of(3) {
        notes.push(format!(
            "frame {frame}: CMAP at {} has {} (not a multiple of 3), the rest is ignored",
            at(c.offset),
            iff::byte_count(c.data.len())
        ));
    }
    let mut colors = prev.colors().to_vec();
    if n > colors.len() {
        notes.push(format!(
            "frame {frame}: CMAP has {n} colours, the palette has {}; the extra ones are ignored",
            colors.len()
        ));
    }
    for (i, rgb) in c
        .data
        .as_chunks::<3>()
        .0
        .iter()
        .take(colors.len())
        .enumerate()
    {
        colors[i] = Rgb8 {
            r: rgb[0],
            g: rgb[1],
            b: rgb[2],
        };
    }
    Palette::new(colors).expect("same length as before")
}

/// Reads an ANIM file; tolerated quirks are dropped. See [`read_with_notes`].
pub fn read(bytes: &[u8]) -> Result<Anim, AnimError> {
    read_with_notes(bytes).map(|(a, _)| a)
}

/// Reads an ANIM file and returns notes about everything tolerated rather than refused.
///
/// Frame decoding follows the double-buffering rule: a delta applies to the frame
/// `interleave` frames back (0 means 2). The first two delta frames have no such frame
/// (frame -1 does not exist): both buffers start as copies of the first picture, so a delta
/// whose reference lies before frame 0 applies to frame 0. A cut-off file keeps the frames
/// before the cut (note).
pub fn read_with_notes(bytes: &[u8]) -> Result<(Anim, Vec<String>), AnimError> {
    let form = iff::read_form(bytes)?;
    if &form.form_type != b"ANIM" {
        let t = iff::name(form.form_type);
        return Err(AnimError::new(
            ErrorKind::NotAnim,
            if &form.form_type == b"ILBM" {
                "Not an ANIM file (single picture)".to_string()
            } else {
                format!("Not an ANIM file: FORM {t}")
            },
            format!(
                "The file is an IFF FORM {t}; an animation is a FORM ANIM holding one FORM ILBM \
                 per frame."
            ),
        ));
    }
    let mut notes = form.notes.clone();
    let mut dpan = None;
    let mut frame_forms: Vec<&Chunk<'_>> = Vec::new();
    for c in &form.chunks {
        match &c.id {
            b"FORM" => frame_forms.push(c),
            b"DPAN" => dpan = Dpan::parse(c.data),
            _ => notes.push(format!(
                "chunk {} at {} between the frames skipped",
                iff::name(c.id),
                at(c.offset)
            )),
        }
    }
    if let Some(off) = form.cut_header {
        notes.push(format!(
            "the file ends inside a chunk header at {}",
            at(off)
        ));
    }
    let Some(first_form) = frame_forms.first() else {
        return Err(AnimError::new(
            ErrorKind::MissingFrame,
            "ANIM has no frames",
            format!(
                "The FORM ANIM holds no FORM ILBM; {} chunks found.",
                form.chunks.len()
            ),
        ));
    };
    let sub = |c: &Chunk<'_>| &bytes[c.offset..(c.offset + 8 + c.size).min(bytes.len())];
    if first_form.is_cut() {
        return Err(AnimError::new(
            ErrorKind::MissingFrame,
            "First ANIM picture is cut off",
            format!(
                "The first FORM ILBM at {} claims {} but the file ends after {}.",
                at(first_form.offset),
                iff::byte_count(first_form.size),
                iff::byte_count(first_form.data.len())
            ),
        ));
    }
    let first_bytes = sub(first_form);
    let first_chunks = iff::read_form(first_bytes)?;
    if &first_chunks.form_type != b"ILBM" {
        return Err(AnimError::new(
            ErrorKind::MissingFrame,
            "First ANIM frame is not an ILBM",
            format!(
                "The first FORM at {} is a FORM {}; the first frame must be a full FORM ILBM.",
                at(first_form.offset),
                iff::name(first_chunks.form_type)
            ),
        ));
    }
    if find(&first_chunks.chunks, b"BODY").is_none() {
        return Err(AnimError::new(
            ErrorKind::Unsupported,
            "First ANIM frame has no BODY",
            format!(
                "The first FORM ILBM at {} has no BODY. A first frame stored as a delta against \
                 a black screen is not supported.",
                at(first_form.offset)
            ),
        ));
    }
    let (first, first_notes) = ilbm::read_with_notes(first_bytes)?;
    notes.extend(first_notes.into_iter().map(|n| format!("first frame: {n}")));
    if first.mask.is_some() {
        notes.push("the mask plane of the first frame is not kept".into());
    }
    let (w, h) = (first.image.width() as usize, first.image.height() as usize);
    let planes = usize::from(first.planes);
    let first_anhd = find(&first_chunks.chunks, b"ANHD").and_then(|c| Anhd::parse(c.data));
    if dpan.is_none() {
        dpan = find(&first_chunks.chunks, b"DPAN").and_then(|c| Dpan::parse(c.data));
    }

    let mut frames = vec![AnimFrame {
        image: first.image.clone(),
        operation: OP_SET,
        rel_time: first_anhd.map_or(0, |a| a.rel_time),
        interleave: 2,
        palette: None,
    }];
    let mut current_palette = first.palette.clone();
    for (n, c) in frame_forms.iter().enumerate().skip(1) {
        if c.is_cut() {
            notes.push(format!(
                "the file ends inside frame {n} (FORM at {}): the {} frames before it are kept",
                at(c.offset),
                frames.len()
            ));
            break;
        }
        if frames.len() * w * h > MAX_TOTAL_PIXELS {
            return Err(AnimError::new(
                ErrorKind::Unsupported,
                "ANIM too big",
                format!(
                    "{} frames of {w}x{h} exceed the limit of {MAX_TOTAL_PIXELS} pixels in all.",
                    frames.len()
                ),
            ));
        }
        let fb = sub(c);
        let f = iff::read_form(fb)?;
        if &f.form_type != b"ILBM" {
            notes.push(format!(
                "frame {n}: FORM {} at {} is not an ILBM and was skipped",
                iff::name(f.form_type),
                at(c.offset)
            ));
            continue;
        }
        let ctx = format!("frame {n} (FORM at {})", at(c.offset));
        let anhd_chunk = find(&f.chunks, b"ANHD");
        let anhd = match anhd_chunk {
            Some(a) => Some(Anhd::parse(a.data).ok_or_else(|| {
                AnimError::new(
                    ErrorKind::BadFrame,
                    "ANHD too short",
                    format!(
                        "{ctx}: ANHD at {} has {}, 24 needed (40 expected)",
                        at(c.offset + a.offset),
                        iff::byte_count(a.data.len())
                    ),
                )
            })?),
            None => None,
        };
        if let Some(a) = anhd_chunk
            && a.data.len() < Anhd::SIZE
        {
            notes.push(format!(
                "frame {n}: ANHD has {}, 40 expected",
                iff::byte_count(a.data.len())
            ));
        }
        let body = find(&f.chunks, b"BODY");
        let dlta = find(&f.chunks, b"DLTA");
        let op = anhd.map_or_else(
            || if body.is_some() { OP_SET } else { 255 },
            |a| a.operation,
        );
        let interleave = anhd.map_or(2, |a| a.effective_interleave());
        let image = match op {
            OP_SET => {
                let Some(_) = body else {
                    return Err(AnimError::new(
                        ErrorKind::BadFrame,
                        "Frame has no BODY or DLTA",
                        format!("{ctx}: method 0 needs a BODY chunk"),
                    ));
                };
                if anhd.is_none() {
                    notes.push(format!(
                        "frame {n}: no ANHD, read as a full picture without timing"
                    ));
                }
                let (pic, pic_notes) = ilbm::read_with_notes(fb)?;
                notes.extend(pic_notes.into_iter().map(|m| format!("frame {n}: {m}")));
                if (pic.image.width() as usize, pic.image.height() as usize) != (w, h)
                    || pic.planes != first.planes
                {
                    return Err(AnimError::new(
                        ErrorKind::BadFrame,
                        "Frame size differs from frame 0",
                        format!(
                            "{ctx}: {}x{} with {} planes, frame 0 has {w}x{h} with {planes}",
                            pic.image.width(),
                            pic.image.height(),
                            pic.planes
                        ),
                    ));
                }
                pic.image
            }
            OP_BYTE_VERTICAL => {
                let Some(d) = dlta else {
                    return Err(AnimError::new(
                        ErrorKind::BadFrame,
                        "Frame has no DLTA chunk",
                        format!("{ctx}: ANHD asks for method 5 but there is no DLTA"),
                    ));
                };
                if d.is_cut() {
                    return Err(AnimError::new(
                        ErrorKind::BadDelta,
                        "DLTA is cut off",
                        format!(
                            "{ctx}: the DLTA claims {} but {} are there",
                            iff::byte_count(d.size),
                            d.data.len()
                        ),
                    ));
                }
                let reference_index = n.checked_sub(interleave).filter(|&r| r < frames.len());
                let reference = &frames[reference_index.unwrap_or(0)].image;
                let mut buf = to_planes(reference, planes);
                let xor = anhd.is_some_and(|a| a.xor());
                apply_op5(d.data, &mut buf, w, h, xor, &ctx)?;
                from_planes(w, h, &buf)
            }
            255 => {
                return Err(AnimError::new(
                    ErrorKind::BadFrame,
                    "Frame has no ANHD chunk",
                    format!("{ctx}: neither an ANHD nor a BODY; the method is unknown"),
                ));
            }
            other => {
                return Err(AnimError::new(
                    ErrorKind::Unsupported,
                    format!("ANIM {}: not supported", method_short(other)),
                    format!(
                        "{ctx}: ANHD asks for {}. DeluxePainter reads method 0 (full ILBM) and \
                         method 5 (byte vertical delta, the DeluxePaint default).",
                        method_name(other)
                    ),
                ));
            }
        };
        let palette = find(&f.chunks, b"CMAP").map(|cm| {
            let p = apply_cmap(&current_palette, cm, &mut notes, n);
            current_palette = p.clone();
            p
        });
        frames.push(AnimFrame {
            image,
            operation: op,
            rel_time: anhd.map_or(0, |a| a.rel_time),
            interleave,
            palette,
        });
    }

    let total = frames.len();
    let content_loops = total >= 4
        && frames[total - 2].image == frames[0].image
        && frames[total - 1].image == frames[1].image;
    let loop_frames = match dpan {
        Some(d) if total >= 4 && usize::from(d.frames) + 2 == total => 2,
        Some(d) if usize::from(d.frames) == total => usize::from(content_loops) * 2,
        Some(d) => {
            notes.push(format!(
                "DPAN says {} frames, the file holds {total}",
                d.frames
            ));
            usize::from(content_loops) * 2
        }
        None => usize::from(content_loops) * 2,
    };
    let anim = Anim {
        width: w as u32,
        height: h as u32,
        planes: first.planes,
        palette: first.palette,
        special: first.special,
        camg: first.camg,
        ranges: first.ranges,
        x_aspect: first.x_aspect,
        y_aspect: first.y_aspect,
        frames,
        loop_frames,
        dpan,
    };
    Ok((anim, notes))
}

fn method_short(op: u8) -> String {
    match op {
        74 => "method J".into(),
        n => format!("method {n}"),
    }
}

// ----------------------------------------------------------------------------------------
// Writing

/// How long each logical frame is shown, in jiffies (1/60 s).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hold {
    Uniform(u32),
    PerFrame(Vec<u32>),
}

/// Options for [`encode`].
#[derive(Clone, Debug)]
pub struct EncodeOptions {
    pub hold: Hold,
    /// Append DPaint's two closing frames (only with at least two frames).
    pub loop_frames: bool,
    /// Colour cycling ranges for the first picture.
    pub ranges: Vec<ColorRange>,
    /// Screen mode ID of the first picture (CAMG), if any.
    pub camg: Option<u32>,
    pub x_aspect: u8,
    pub y_aspect: u8,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            hold: Hold::Uniform(6),
            loop_frames: true,
            ranges: Vec::new(),
            camg: None,
            x_aspect: 1,
            y_aspect: 1,
        }
    }
}

/// Writes an ANIM from full indexed frames and one palette.
///
/// Layout: `FORM ANIM { FORM ILBM {BMHD CMAP [CRNG] [CAMG] ANHD DPAN BODY}, FORM ILBM {ANHD
/// DLTA} ... }`. The first picture is a ByteRun1 ILBM (method 0). Every further frame `i` is
/// a method-5 delta against frame `i - 2`; frame 1 has no frame -1, so it is a delta against
/// frame 0 (both display buffers start as copies of the first picture). With `loop_frames`
/// and N >= 2 frames, two more deltas follow: frame N is a copy of frame 0 (delta against
/// frame N-2) and frame N+1 a copy of frame 1 (delta against frame N-1), so a player that
/// restarts at the third frame loops seamlessly. DPAN holds N (the animation without the
/// two closing frames, version 4). ANHD: interleave 0 (= 2), bits 0 (set, not XOR); the
/// first frame has `rel_time` 0 (its hold time when it is the only frame), a later frame has the
/// hold time of the frame before it; without closing frames the last frame's hold time is
/// therefore not stored separately (the reader falls back to the last `rel_time`).
pub fn encode(
    frames: &[IndexedImage],
    palette: &Palette,
    opts: &EncodeOptions,
) -> Result<Vec<u8>, AnimError> {
    let n = frames.len();
    if n == 0 {
        return Err(AnimError::invalid("cannot write ANIM: no frames", ""));
    }
    if n > usize::from(u16::MAX) - 2 {
        return Err(AnimError::invalid(
            "cannot write ANIM: too many frames",
            format!("{n} frames; DPAN counts frames in 16 bits (at most 65533)"),
        ));
    }
    let (w, h) = (frames[0].width(), frames[0].height());
    if let Some((i, f)) = frames
        .iter()
        .enumerate()
        .find(|(_, f)| (f.width(), f.height()) != (w, h))
    {
        return Err(AnimError::invalid(
            "cannot write ANIM: frame sizes",
            format!(
                "frame {i} is {}x{}, frame 0 is {w}x{h}",
                f.width(),
                f.height()
            ),
        ));
    }
    let holds: Vec<u32> = match &opts.hold {
        Hold::Uniform(j) => vec![*j; n],
        Hold::PerFrame(v) if v.len() == n => v.clone(),
        Hold::PerFrame(v) => {
            return Err(AnimError::invalid(
                "cannot write ANIM: frame delays",
                format!("{} delays for {n} frames", v.len()),
            ));
        }
    };
    let planes = ilbm::planes_for(palette.len());
    let dpan = Dpan {
        version: 4,
        frames: n as u16,
        flags: 0,
    };
    let anhd = |op: u8, rel: u32| {
        Anhd {
            operation: op,
            w: w as u16,
            h: h as u16,
            rel_time: rel,
            ..Anhd::default()
        }
        .to_bytes()
    };

    let mut first = Ilbm::from_indexed(frames[0].clone(), palette.clone());
    first.planes = planes;
    first.camg = opts.camg;
    first.ranges = opts.ranges.clone();
    first.x_aspect = opts.x_aspect;
    first.y_aspect = opts.y_aspect;
    first.other_chunks = vec![
        RawChunk {
            id: *b"ANHD",
            data: anhd(OP_SET, if n == 1 { holds[0] } else { 0 }).to_vec(),
        },
        RawChunk {
            id: *b"DPAN",
            data: dpan.to_bytes().to_vec(),
        },
    ];
    let first_bytes = ilbm::write(&first, Compression::ByteRun1).map_err(|e| {
        AnimError::invalid(e.summary, e.details) // ILBM validation (colours, size)
    })?;

    let mut out = FormWriter::new(*b"ANIM");
    out.chunk(*b"FORM", &first_bytes[8..]);
    let total = if opts.loop_frames && n >= 2 { n + 2 } else { n };
    for i in 1..total {
        let source = |k: usize| &frames[k % n];
        let cur = source(i);
        let reference = if i < 2 { source(0) } else { source(i - 2) };
        let rel = if i < n {
            holds[i - 1]
        } else {
            holds[(i - 1) % n]
        };
        let dlta = encode_op5(reference, cur, planes)?;
        let mut f = FormWriter::new(*b"ILBM");
        f.chunk(*b"ANHD", &anhd(OP_BYTE_VERTICAL, rel));
        f.chunk(*b"DLTA", &dlta);
        out.chunk(*b"FORM", &f.finish()[8..]);
    }
    Ok(out.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> IndexedImage {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.push(f(x, y));
            }
        }
        IndexedImage::from_pixels(w, h, px).unwrap()
    }

    #[test]
    fn golden_op5_skip_and_uniq() {
        // 8x4, one plane, row_bytes 2. Column 0 changes in rows 1 and 2 only.
        let a = img(8, 4, |_, _| 0);
        let b = img(8, 4, |x, y| u8::from(x == 0 && (y == 1 || y == 2)));
        let d = encode_op5(&a, &b, 1).unwrap();
        let mut want = vec![0u8; 64];
        want[3] = 64; // plane 0 data at offset 64
        // column 0: 2 ops: skip 1, uniq 2 (0x80 0x80); column 1: no ops. Padded to even.
        want.extend_from_slice(&[2, 1, 0x82, 0x80, 0x80, 0]);
        assert_eq!(d, want);
        let mut planes = to_planes(&a, 1);
        apply_op5(&d, &mut planes, 8, 4, false, "t").unwrap();
        assert_eq!(from_planes(8, 4, &planes), b);
    }

    #[test]
    fn golden_op5_same_run_and_second_plane() {
        // 8x6, two planes. Plane 1 (value bit 1) is set in column 0 rows 0..6 -> Same 6.
        let a = img(8, 6, |_, _| 0);
        let b = img(8, 6, |x, _| if x == 0 { 2 } else { 0 });
        let d = encode_op5(&a, &b, 2).unwrap();
        let mut want = vec![0u8; 64];
        want[7] = 64; // plane 1 at 64, plane 0 unchanged (offset 0)
        // column 0: 1 op: same, count 6, value 0x80; column 1: none; 4 bytes, even already
        want.extend_from_slice(&[1, 0, 6, 0x80, 0, 0]);
        assert_eq!(d, want);
    }

    #[test]
    fn xor_mode_applies_xor() {
        let a = img(8, 2, |_, _| 1);
        let mut planes = to_planes(&a, 1); // all 0xFF in column 0 (x<8)
        let mut d = vec![0u8; 64];
        d[3] = 64;
        d.extend_from_slice(&[1, 0x81, 0x0F, 0, 0, 0]);
        apply_op5(&d, &mut planes, 8, 2, true, "t").unwrap();
        assert_eq!(planes[0][0], 0xF0);
        assert_eq!(planes[0][2], 0xFF);
    }

    #[test]
    fn column_overflow_falls_back_to_merged_literals() {
        // alternating changes: 600 rows, every other row changes -> far over 255 ops unless merged
        let a = img(8, 600, |_, _| 0);
        let b = img(8, 600, |x, y| u8::from(x == 0 && y % 2 == 0));
        let d = encode_op5(&a, &b, 1).unwrap();
        let mut planes = to_planes(&a, 1);
        apply_op5(&d, &mut planes, 8, 600, false, "t").unwrap();
        assert_eq!(from_planes(8, 600, &planes), b);
    }

    #[test]
    fn too_tall_is_refused() {
        let a = img(1, 32386, |_, _| 0);
        let e = encode_op5(&a, &a, 1).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Invalid);
        assert!(e.summary.chars().count() <= 34);
    }

    #[test]
    fn anhd_round_trips() {
        let a = Anhd {
            operation: 5,
            mask: 3,
            w: 320,
            h: 200,
            x: -1,
            y: 2,
            abs_time: 9,
            rel_time: 6,
            interleave: 0,
            bits: 4,
        };
        let b = a.to_bytes();
        assert_eq!(b.len(), 40);
        assert_eq!(Anhd::parse(&b), Some(a));
        assert_eq!(a.effective_interleave(), 2);
        assert!(a.xor());
    }

    #[test]
    fn summaries_fit_the_requester() {
        let all = [
            "DLTA too short for method 5",
            "DLTA: bad offset, plane 7",
            "DLTA cut off, plane 7",
            "DLTA runs past row 65535, plane 7",
            "Not an ANIM file (single picture)",
            "First ANIM picture is cut off",
            "First ANIM frame is not an ILBM",
            "Frame size differs from frame 0",
            "ANIM method 74: not supported",
            "cannot write ANIM: column too busy",
        ];
        for s in all {
            assert!(s.chars().count() <= 34, "{s}");
        }
    }
}
