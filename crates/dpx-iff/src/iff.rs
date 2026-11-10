//! The IFF-85 chunk container: `FORM <size> <type>` followed by chunks, each
//! `<id> <size> <data>` padded to an even length (`docs/IFF_SUPPORT_MATRIX.md` §1).
//!
//! Reading is tolerant of common real-world quirks (ADR-17): they become notes in
//! [`Form::notes`] instead of errors whenever the chunks can still be found reliably.

use std::fmt;

pub type FourCc = [u8; 4];

/// One chunk inside a FORM, borrowing its data from the file bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub id: FourCc,
    /// The chunk data; shorter than `size` when the file ends inside the chunk.
    pub data: &'a [u8],
    /// File offset of the chunk header (the data starts 8 bytes later).
    pub offset: usize,
    /// Data size from the chunk header.
    pub size: usize,
}

impl Chunk<'_> {
    /// The file ends inside this chunk.
    pub fn is_cut(&self) -> bool {
        self.data.len() < self.size
    }
}

/// A parsed top-level FORM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form<'a> {
    pub form_type: FourCc,
    /// Chunks in file order.
    pub chunks: Vec<Chunk<'a>>,
    /// Container quirks that were tolerated, as user-readable notes.
    pub notes: Vec<String>,
    /// File offset of a chunk header cut off by the end of the file; parsing stopped there.
    pub cut_header: Option<usize>,
    /// Bytes the FORM size claims beyond the end of the file.
    pub missing: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IffError {
    /// Fewer bytes than a FORM header (12).
    TooShort(usize),
    /// The data does not start with `FORM`; holds the first four bytes.
    NotIff(FourCc),
    /// The chunk header at `offset` is cut off: only `have` of its 8 bytes are in the file.
    Truncated { offset: usize, have: usize },
}

impl IffError {
    /// One short line (at most 34 characters) for the file requester's message line.
    pub fn summary(&self) -> String {
        match self {
            IffError::TooShort(0) => "File is empty".into(),
            IffError::TooShort(_) => "File too short for IFF".into(),
            IffError::NotIff(id) if is_container(*id) => {
                format!("IFF {} files: not supported yet", name(*id))
            }
            IffError::NotIff(_) => "Not an IFF file".into(),
            IffError::Truncated { .. } => "IFF file is cut off".into(),
        }
    }

    /// What exactly is wrong and where.
    pub fn details(&self) -> String {
        match *self {
            IffError::TooShort(n) => {
                format!("The file has {n} bytes; an IFF FORM header alone needs 12.")
            }
            IffError::NotIff(id) if is_container(id) => format!(
                "The file is an IFF {} (several FORMs in one file). DeluxePainter reads a \
                 single FORM ILBM.",
                name(id)
            ),
            IffError::NotIff(id) => {
                format!("It starts with {} instead of FORM.", quoted(id))
            }
            IffError::Truncated { offset, have } => format!(
                "The chunk header at {} has only {have} of its 8 bytes: the file ends there.",
                at(offset)
            ),
        }
    }
}

impl fmt::Display for IffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{}", self.summary(), self.details())
    }
}

impl std::error::Error for IffError {}

/// Shows a chunk id like `BMHD`; non-printable bytes as `?`.
pub fn id_str(id: FourCc) -> String {
    id.iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

/// A chunk or FORM id for messages: printable, trailing spaces removed (`PBM `→`PBM`).
pub fn name(id: FourCc) -> String {
    id_str(id).trim_end().to_string()
}

/// A chunk id as hex bytes plus its printable form, e.g. `47 49 46 38 ("GIF8")`.
pub fn quoted(id: FourCc) -> String {
    format!(
        "{:02X} {:02X} {:02X} {:02X} (\"{}\")",
        id[0],
        id[1],
        id[2],
        id[3],
        id_str(id)
    )
}

/// A file offset for messages, decimal and in Amiga-style hex: `offset 60 ($3C)`.
pub fn at(offset: usize) -> String {
    format!("offset {offset} (${offset:X})")
}

/// `1 byte`, `2 bytes`.
pub fn byte_count(n: usize) -> String {
    if n == 1 {
        "1 byte".into()
    } else {
        format!("{n} bytes")
    }
}

/// IFF-85 ids are four printable ASCII characters without a leading space.
pub fn is_valid_id(id: &[u8]) -> bool {
    id.len() == 4 && id[0] != b' ' && id.iter().all(|b| (0x20..0x7F).contains(b))
}

/// The grouping containers that hold several FORMs.
fn is_container(id: FourCc) -> bool {
    matches!(&id, b"LIST" | b"CAT " | b"PROP")
}

/// Parses a top-level `FORM` and returns its type and chunks in file order.
///
/// Tolerant like common readers (see [`read_form`]); a cut-off chunk header is an error.
pub fn parse_form(bytes: &[u8]) -> Result<(FourCc, Vec<Chunk<'_>>), IffError> {
    let form = read_form(bytes)?;
    if let Some(offset) = form.cut_header {
        return Err(IffError::Truncated {
            offset,
            have: bytes.len() - offset,
        });
    }
    Ok((form.form_type, form.chunks))
}

/// Parses a top-level `FORM`, tolerating (with a note) what common writers get wrong:
/// - a FORM size larger than the file (chunks get the bytes that are there; the caller
///   decides whether a cut-off chunk is enough, see [`Chunk::is_cut`]);
/// - a FORM size too small for the chunks that follow it in the file (read to the end);
/// - a missing pad byte after an odd-sized chunk, at the end or in the middle of the file;
/// - data after the end of the FORM.
///
/// A chunk header cut off by the end of the file ends the chunk list ([`Form::cut_header`]).
pub fn read_form(bytes: &[u8]) -> Result<Form<'_>, IffError> {
    if bytes.len() >= 4 && &bytes[..4] != b"FORM" {
        return Err(IffError::NotIff(bytes[..4].try_into().expect("4 bytes")));
    }
    if bytes.len() < 12 {
        return Err(IffError::TooShort(bytes.len()));
    }
    let form_size = be_u32(&bytes[4..8]) as usize;
    let declared_end = 8usize.saturating_add(form_size);
    let missing = declared_end.saturating_sub(bytes.len());
    let mut end = bytes.len().min(declared_end);
    let form_type: FourCc = bytes[8..12].try_into().expect("4 bytes");
    let mut notes = Vec::new();
    let mut chunks: Vec<Chunk<'_>> = Vec::new();
    let mut cut_header = None;
    let mut extended = false;
    let too_small = |notes: &mut Vec<String>| {
        notes.push(format!(
            "the FORM size ({form_size} bytes) is too small for its chunks; the file is read \
             to its end"
        ));
    };
    let mut pos: usize = 12;
    loop {
        if pos.saturating_add(8) > end {
            // No room for another chunk header inside the FORM. If a whole chunk follows in
            // the file anyway, the FORM size is wrong (e.g. never patched by the writer).
            let next_is_chunk = bytes.get(pos..pos.saturating_add(8)).is_some_and(|h| {
                let fits = (pos + 8).saturating_add(be_u32(&h[4..]) as usize) <= bytes.len();
                is_valid_id(&h[..4]) && !is_form_start(&h[..4]) && fits
            });
            if !extended && end < bytes.len() && next_is_chunk {
                too_small(&mut notes);
                end = bytes.len();
                extended = true;
                continue;
            }
            if end == bytes.len() && end.saturating_sub(pos) > 1 {
                cut_header = Some(pos);
            }
            break;
        }
        let id: FourCc = bytes[pos..pos + 4].try_into().expect("4 bytes");
        let size = be_u32(&bytes[pos + 4..pos + 8]) as usize;
        let start = pos + 8;
        let data_end = start.saturating_add(size);
        if !extended && data_end > end && data_end <= bytes.len() {
            too_small(&mut notes);
            end = bytes.len();
            extended = true;
        }
        chunks.push(Chunk {
            id,
            data: &bytes[start..end.min(data_end)],
            offset: pos,
            size,
        });
        let mut next = data_end.saturating_add(size & 1);
        if size & 1 == 1 && data_end.saturating_add(8) <= end {
            // A writer that forgot the pad byte: the next header starts right after the data.
            let padded = is_valid_id(&bytes[next..next + 4]);
            if !padded && is_valid_id(&bytes[data_end..data_end + 4]) {
                notes.push(format!(
                    "the pad byte after the odd-sized {} chunk at {} is missing",
                    name(id),
                    at(pos)
                ));
                next = data_end;
            }
        }
        pos = next;
    }

    let last = chunks.last();
    let pad_missing_at_end = last
        .is_some_and(|c| c.size % 2 == 1 && !c.is_cut() && c.offset + 8 + c.size == bytes.len());
    if pad_missing_at_end {
        notes.push(format!(
            "the pad byte after the last chunk ({}) is missing at the end of the file",
            name(last.expect("a last chunk").id)
        ));
    } else if missing > 0 && cut_header.is_none() && !last.is_some_and(Chunk::is_cut) {
        notes.push(format!(
            "the FORM size says {} more than the file has (ignored)",
            byte_count(missing)
        ));
    }
    let consumed = pos.max(end).min(bytes.len());
    let trailing = bytes.len() - consumed;
    // A single byte after an odd FORM size is the FORM's own pad byte.
    if trailing > 0 && !(trailing == 1 && form_size % 2 == 1) {
        if bytes.get(consumed..consumed + 4).is_some_and(is_form_start) {
            notes.push(format!(
                "a second IFF file after the end of the FORM is ignored ({} at {})",
                byte_count(trailing),
                at(consumed)
            ));
        } else {
            notes.push(format!(
                "{} after the end of the FORM ignored (at {})",
                byte_count(trailing),
                at(consumed)
            ));
        }
    }
    Ok(Form {
        form_type,
        chunks,
        notes,
        cut_header,
        missing,
    })
}

fn is_form_start(id: &[u8]) -> bool {
    matches!(id, b"FORM" | b"LIST" | b"CAT ")
}

/// Builds a `FORM` in memory.
pub struct FormWriter {
    buf: Vec<u8>,
}

impl FormWriter {
    pub fn new(form_type: FourCc) -> Self {
        let mut buf = Vec::with_capacity(4096);
        buf.extend_from_slice(b"FORM");
        buf.extend_from_slice(&[0; 4]);
        buf.extend_from_slice(&form_type);
        Self { buf }
    }

    /// Appends a chunk plus the pad byte for odd sizes.
    pub fn chunk(&mut self, id: FourCc, data: &[u8]) {
        self.buf.extend_from_slice(&id);
        self.buf
            .extend_from_slice(&(data.len() as u32).to_be_bytes());
        self.buf.extend_from_slice(data);
        if data.len() % 2 == 1 {
            self.buf.push(0);
        }
    }

    /// Fills in the FORM size and returns the file bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let size = (self.buf.len() - 8) as u32;
        self.buf[4..8].copy_from_slice(&size.to_be_bytes());
        self.buf
    }
}

pub(crate) fn be_u16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

pub(crate) fn be_i16(b: &[u8]) -> i16 {
    i16::from_be_bytes([b[0], b[1]])
}

pub(crate) fn be_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_u32(bytes: &mut [u8], at: usize, v: u32) {
        bytes[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }

    #[test]
    fn write_then_parse_with_padding() {
        let mut w = FormWriter::new(*b"TEST");
        w.chunk(*b"ODD ", &[1, 2, 3]);
        w.chunk(*b"EVEN", &[4, 5]);
        w.chunk(*b"NONE", &[]);
        let bytes = w.finish();
        assert_eq!(bytes.len(), 12 + 8 + 4 + 8 + 2 + 8);
        assert_eq!(be_u32(&bytes[4..8]) as usize, bytes.len() - 8);
        let (ty, chunks) = parse_form(&bytes).unwrap();
        assert_eq!(&ty, b"TEST");
        let got: Vec<(String, Vec<u8>, usize)> = chunks
            .iter()
            .map(|c| (id_str(c.id), c.data.to_vec(), c.offset))
            .collect();
        assert_eq!(
            got,
            [
                ("ODD ".to_string(), vec![1, 2, 3], 12),
                ("EVEN".to_string(), vec![4, 5], 24),
                ("NONE".to_string(), vec![], 34),
            ]
        );
        assert!(read_form(&bytes).unwrap().notes.is_empty());
    }

    #[test]
    fn tolerates_oversized_sizes_and_rejects_garbage() {
        assert_eq!(parse_form(b"JUNKJUNKJUNK"), Err(IffError::NotIff(*b"JUNK")));
        assert_eq!(parse_form(b"FORM"), Err(IffError::TooShort(4)));
        assert_eq!(parse_form(b""), Err(IffError::TooShort(0)));
        let mut w = FormWriter::new(*b"TEST");
        w.chunk(*b"DATA", &[9; 10]);
        let mut bytes = w.finish();
        // FORM size and chunk size claim more than the file has.
        set_u32(&mut bytes, 4, 1000);
        set_u32(&mut bytes, 16, 50);
        let form = read_form(&bytes).unwrap();
        assert_eq!(form.chunks[0].data, &[9; 10]);
        assert!(form.chunks[0].is_cut());
        assert_eq!(form.missing, 1008 - 30);
        assert!(form.notes.is_empty(), "the cut chunk speaks for itself");
        // A cut-off chunk header.
        let mut cut = FormWriter::new(*b"TEST").finish();
        cut.extend_from_slice(b"AB");
        set_u32(&mut cut, 4, 6);
        assert_eq!(
            parse_form(&cut),
            Err(IffError::Truncated {
                offset: 12,
                have: 2
            })
        );
    }

    #[test]
    fn error_texts_are_short_first_lines_with_details() {
        assert_eq!(
            IffError::NotIff(*b"GIF8").to_string(),
            "Not an IFF file\nIt starts with 47 49 46 38 (\"GIF8\") instead of FORM."
        );
        assert_eq!(
            IffError::NotIff(*b"CAT ").to_string(),
            "IFF CAT files: not supported yet\nThe file is an IFF CAT (several FORMs in one \
             file). DeluxePainter reads a single FORM ILBM."
        );
        assert_eq!(
            IffError::Truncated {
                offset: 60,
                have: 3
            }
            .to_string(),
            "IFF file is cut off\nThe chunk header at offset 60 ($3C) has only 3 of its 8 \
             bytes: the file ends there."
        );
        assert_eq!(IffError::TooShort(0).summary(), "File is empty");
    }

    #[test]
    fn missing_pad_bytes_are_noted() {
        let mut w = FormWriter::new(*b"TEST");
        w.chunk(*b"ODD ", &[1, 2, 3]);
        w.chunk(*b"LAST", &[7, 7, 7]);
        let mut bytes = w.finish();
        // Mid-file: drop the pad after ODD.
        bytes.remove(12 + 8 + 3);
        // At the end: drop the final pad, FORM size still counts it.
        bytes.pop();
        let form = read_form(&bytes).unwrap();
        assert_eq!(form.chunks.len(), 2);
        assert_eq!(form.chunks[1].data, &[7, 7, 7]);
        assert_eq!(
            form.notes,
            [
                "the pad byte after the odd-sized ODD chunk at offset 12 ($C) is missing",
                "the pad byte after the last chunk (LAST) is missing at the end of the file",
            ]
        );
    }

    #[test]
    fn wrong_form_sizes_and_trailing_data() {
        let mut w = FormWriter::new(*b"TEST");
        w.chunk(*b"AAAA", &[1, 2]);
        w.chunk(*b"BBBB", &[3, 4]);
        let good = w.finish();

        // FORM size never patched (0): the chunks are read anyway.
        let mut zero = good.clone();
        set_u32(&mut zero, 4, 0);
        let form = read_form(&zero).unwrap();
        assert_eq!(form.chunks.len(), 2);
        assert_eq!(
            form.notes,
            ["the FORM size (0 bytes) is too small for its chunks; the file is read to its end"]
        );

        // Trailing garbage, trailing text that looks like a chunk id, a second FORM.
        let mut junk = good.clone();
        junk.extend_from_slice(&[0x1A; 5]);
        assert_eq!(
            read_form(&junk).unwrap().notes,
            ["5 bytes after the end of the FORM ignored (at offset 32 ($20))"]
        );
        let mut text = good.clone();
        text.extend_from_slice(b"Saved by a tool\n");
        let form = read_form(&text).unwrap();
        assert_eq!(form.chunks.len(), 2);
        assert_eq!(
            form.notes,
            ["16 bytes after the end of the FORM ignored (at offset 32 ($20))"]
        );
        let mut one = good.clone();
        one.push(0);
        assert_eq!(
            read_form(&one).unwrap().notes,
            ["1 byte after the end of the FORM ignored (at offset 32 ($20))"]
        );
        let mut two = good.clone();
        two.extend_from_slice(&good);
        assert_eq!(
            read_form(&two).unwrap().notes,
            [
                "a second IFF file after the end of the FORM is ignored (32 bytes at offset 32 \
              ($20))"
            ]
        );

        // FORM size too large while every chunk is complete.
        let mut big = good.clone();
        set_u32(&mut big, 4, 40);
        let form = read_form(&big).unwrap();
        assert_eq!(form.missing, 16);
        assert_eq!(
            form.notes,
            ["the FORM size says 16 bytes more than the file has (ignored)"]
        );
    }
}
