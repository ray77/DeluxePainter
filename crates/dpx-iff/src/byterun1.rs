//! ByteRun1 (PackBits) row codec used by IFF ILBM `BODY` chunks.
//!
//! See `docs/IFF_SUPPORT_MATRIX.md`, section 3.8.

use std::fmt;

/// Errors while unpacking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnpackError {
    /// The input ended before `len` bytes were produced.
    Truncated,
    /// A run or literal would write past `len` bytes (corrupt data).
    Overrun,
}

impl fmt::Display for UnpackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnpackError::Truncated => {
                f.write_str("ByteRun1 data ended before the row was complete")
            }
            UnpackError::Overrun => f.write_str("ByteRun1 run or literal exceeds the row length"),
        }
    }
}

impl std::error::Error for UnpackError {}

/// Unpacks exactly `len` bytes (one row) from the start of `src`.
/// Returns the unpacked bytes and the number of source bytes consumed.
/// Control byte n (as i8): 0..=127 copy n+1 literal bytes; -127..=-1 repeat next byte 1-n times; -128 is a no-op.
pub fn unpack_row(src: &[u8], len: usize) -> Result<(Vec<u8>, usize), UnpackError> {
    let mut out = Vec::with_capacity(len);
    let mut pos = 0usize;
    while out.len() < len {
        let &ctrl = src.get(pos).ok_or(UnpackError::Truncated)?;
        pos += 1;
        let n = ctrl as i8;
        if n == -128 {
            continue;
        }
        if n >= 0 {
            let count = n as usize + 1;
            if count > len - out.len() {
                return Err(UnpackError::Overrun);
            }
            let chunk = src.get(pos..pos + count).ok_or(UnpackError::Truncated)?;
            out.extend_from_slice(chunk);
            pos += count;
        } else {
            let count = 1 + (-(n as i16)) as usize;
            if count > len - out.len() {
                return Err(UnpackError::Overrun);
            }
            let &b = src.get(pos).ok_or(UnpackError::Truncated)?;
            pos += 1;
            out.resize(out.len() + count, b);
        }
    }
    Ok((out, pos))
}

const MAX: usize = 128;

fn flush_literal(out: &mut Vec<u8>, lit: &mut Vec<u8>) {
    if !lit.is_empty() {
        out.push((lit.len() - 1) as u8);
        out.append(lit);
    }
}

/// Length of the run of equal bytes starting at `i`, capped at 128.
fn run_len(row: &[u8], i: usize) -> usize {
    let b = row[i];
    row[i..].iter().take(MAX).take_while(|&&x| x == b).count()
}

/// Packs one row. Rules (from the spec guidance): runs and literals are at most 128 bytes;
/// 3 or more equal bytes always become a run; a 2-byte repeat becomes a run only when it is
/// not between two literal runs (otherwise it stays inside the literal); never emit -128.
pub fn pack_row(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(row.len() + row.len() / MAX + 2);
    let mut lit: Vec<u8> = Vec::with_capacity(MAX);
    let mut i = 0usize;
    while i < row.len() {
        let r = run_len(row, i);
        let as_run = match r {
            1 => false,
            2 => {
                // Keep a pair inside the literal only if literals surround it.
                let next = i + 2;
                lit.is_empty() || next >= row.len() || run_len(row, next) >= 3
            }
            _ => true,
        };
        if as_run {
            flush_literal(&mut out, &mut lit);
            out.push((1i16 - r as i16) as i8 as u8);
            out.push(row[i]);
            i += r;
        } else {
            let take = r.min(MAX - lit.len());
            // A pair must not be split across literal boundaries needlessly; splitting is
            // still lossless, so just push what fits.
            lit.extend(std::iter::repeat_n(row[i], take));
            i += take;
            if lit.len() == MAX {
                flush_literal(&mut out, &mut lit);
            }
        }
    }
    flush_literal(&mut out, &mut lit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    fn roundtrip(r: &[u8]) {
        let p = pack_row(r);
        assert_eq!(
            unpack_row(&p, r.len()),
            Ok((r.to_vec(), p.len())),
            "row {r:?}"
        );
        check_structure(&p);
    }

    /// Walks packed output: no -128, no run/literal over 128.
    fn check_structure(p: &[u8]) {
        let mut i = 0;
        while i < p.len() {
            let n = p[i] as i8;
            assert_ne!(n, -128, "emitted -128");
            if n >= 0 {
                i += 1 + n as usize + 1;
            } else {
                assert!(1 - i32::from(n) <= 128);
                i += 2;
            }
        }
        assert_eq!(i, p.len());
    }

    #[test]
    fn known_vector() {
        let src = [
            0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
            0xAA,
        ];
        let want = [
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];
        assert_eq!(unpack_row(&src, 24), Ok((want.to_vec(), 15)));
    }

    #[test]
    fn noop_control_byte_skipped() {
        assert_eq!(
            unpack_row(&[0x80, 0x01, 7, 8, 0x80], 2),
            Ok((vec![7, 8], 4))
        );
    }

    #[test]
    fn truncated_input() {
        assert_eq!(unpack_row(&[], 1), Err(UnpackError::Truncated));
        assert_eq!(unpack_row(&[0x02, 1, 2], 3), Err(UnpackError::Truncated));
        assert_eq!(unpack_row(&[0xFE], 3), Err(UnpackError::Truncated));
        assert_eq!(unpack_row(&[0x80], 1), Err(UnpackError::Truncated));
    }

    #[test]
    fn overrun_detected() {
        assert_eq!(
            unpack_row(&[0x03, 1, 2, 3, 4], 2),
            Err(UnpackError::Overrun)
        );
        assert_eq!(unpack_row(&[0xFE, 9], 2), Err(UnpackError::Overrun));
        assert_eq!(
            unpack_row(&[0x00, 1, 0xFF, 2], 2),
            Err(UnpackError::Overrun)
        );
    }

    #[test]
    fn unpack_zero_len_consumes_nothing() {
        assert_eq!(unpack_row(&[1, 2, 3], 0), Ok((vec![], 0)));
    }

    #[test]
    fn roundtrip_fixed_cases() {
        roundtrip(&[]);
        roundtrip(&[5]);
        roundtrip(&[5, 5]);
        for n in [128usize, 129, 130, 257] {
            roundtrip(&vec![9; n]);
        }
        for n in [128usize, 129, 300] {
            let v: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(7)).collect();
            roundtrip(&v);
            let inc: Vec<u8> = (0..n).map(|i| if i % 2 == 0 { 1 } else { 2 }).collect();
            roundtrip(&inc);
        }
        roundtrip(&[1, 2, 2, 3, 4, 4, 5]);
        roundtrip(&[1, 1, 2, 3, 3]);
        roundtrip(&[1, 1, 1, 2, 2, 3, 3, 3, 4, 5, 5, 6]);
        roundtrip(&[0, 0, 1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn roundtrip_pseudo_random() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for _ in 0..2000 {
            let len = (rng.next() % 301) as usize;
            let range = [1u64, 2, 3, 4, 16, 256][(rng.next() % 6) as usize];
            let row: Vec<u8> = (0..len).map(|_| (rng.next() % range) as u8).collect();
            roundtrip(&row);
        }
    }

    #[test]
    fn encoder_limits_and_no_noop() {
        let mut rng = XorShift(12345);
        for _ in 0..500 {
            let len = (rng.next() % 600) as usize;
            let row: Vec<u8> = (0..len)
                .map(|_| {
                    if rng.next().is_multiple_of(4) {
                        0x80
                    } else {
                        (rng.next() % 3) as u8
                    }
                })
                .collect();
            check_structure(&pack_row(&row));
        }
        check_structure(&pack_row(&vec![0x80; 1000]));
    }

    #[test]
    fn compression_sanity() {
        assert_eq!(pack_row(&[0u8; 40]), vec![(1 - 40i16) as i8 as u8, 0]);
        for len in [1usize, 2, 127, 128, 129, 300, 1000] {
            let row: Vec<u8> = (0..len).map(|i| i as u8).collect();
            assert!(pack_row(&row).len() <= len + len.div_ceil(128));
        }
        let worst: Vec<u8> = (0..300).map(|i| if i % 2 == 0 { 1 } else { 2 }).collect();
        assert!(pack_row(&worst).len() <= 300 + 300usize.div_ceil(128));
    }

    #[test]
    fn pair_between_literals_stays_literal() {
        assert_eq!(pack_row(&[1, 2, 2, 3]), vec![3, 1, 2, 2, 3]);
        assert_eq!(pack_row(&[2, 2, 3]), vec![0xFF, 2, 0, 3]);
    }
}
