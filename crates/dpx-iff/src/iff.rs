//! The IFF-85 chunk container: `FORM <size> <type>` followed by chunks, each
//! `<id> <size> <data>` padded to an even length (`docs/IFF_SUPPORT_MATRIX.md` §1).

use std::fmt;

pub type FourCc = [u8; 4];

/// One chunk inside a FORM, borrowing its data from the file bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub id: FourCc,
    pub data: &'a [u8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IffError {
    /// The data does not start with a `FORM` header.
    NotIff,
    /// A chunk header is cut off.
    Truncated,
}

impl fmt::Display for IffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IffError::NotIff => write!(f, "not an IFF file (no FORM header)"),
            IffError::Truncated => write!(f, "IFF file is truncated"),
        }
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

/// Parses a top-level `FORM` and returns its type and chunks in file order.
///
/// Tolerant like common readers: a FORM size larger than the file is clamped to the file, and
/// a last chunk whose size runs past the end gets the bytes that are there (the caller decides
/// whether that is enough). A cut-off chunk header is an error.
pub fn parse_form(bytes: &[u8]) -> Result<(FourCc, Vec<Chunk<'_>>), IffError> {
    if bytes.len() < 12 || &bytes[..4] != b"FORM" {
        return Err(IffError::NotIff);
    }
    let form_size = be_u32(&bytes[4..8]) as usize;
    let end = bytes.len().min(8usize.saturating_add(form_size));
    let form_type: FourCc = bytes[8..12].try_into().expect("4 bytes");
    let mut chunks = Vec::new();
    let mut pos = 12;
    while pos < end {
        if end - pos < 8 {
            // A single pad byte at the end is fine; anything else is a cut-off header.
            return if end - pos == 1 {
                Ok((form_type, chunks))
            } else {
                Err(IffError::Truncated)
            };
        }
        let id: FourCc = bytes[pos..pos + 4].try_into().expect("4 bytes");
        let size = be_u32(&bytes[pos + 4..pos + 8]) as usize;
        let start = pos + 8;
        let stop = end.min(start.saturating_add(size));
        chunks.push(Chunk {
            id,
            data: &bytes[start..stop],
        });
        pos = start.saturating_add(size).saturating_add(size & 1);
    }
    Ok((form_type, chunks))
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
        let got: Vec<(String, Vec<u8>)> = chunks
            .iter()
            .map(|c| (id_str(c.id), c.data.to_vec()))
            .collect();
        assert_eq!(
            got,
            [
                ("ODD ".to_string(), vec![1, 2, 3]),
                ("EVEN".to_string(), vec![4, 5]),
                ("NONE".to_string(), vec![]),
            ]
        );
    }

    #[test]
    fn tolerates_oversized_sizes_and_rejects_garbage() {
        assert_eq!(parse_form(b"JUNKJUNKJUNK"), Err(IffError::NotIff));
        assert_eq!(parse_form(b"FORM"), Err(IffError::NotIff));
        let mut w = FormWriter::new(*b"TEST");
        w.chunk(*b"DATA", &[9; 10]);
        let mut bytes = w.finish();
        // FORM size and chunk size claim more than the file has.
        bytes[4..8].copy_from_slice(&1000u32.to_be_bytes());
        bytes[16..20].copy_from_slice(&50u32.to_be_bytes());
        let (_, chunks) = parse_form(&bytes).unwrap();
        assert_eq!(chunks[0].data, &[9; 10]);
        // A cut-off chunk header.
        let mut cut = FormWriter::new(*b"TEST").finish();
        cut.extend_from_slice(b"AB");
        cut[4..8].copy_from_slice(&6u32.to_be_bytes());
        assert_eq!(parse_form(&cut), Err(IffError::Truncated));
    }
}
