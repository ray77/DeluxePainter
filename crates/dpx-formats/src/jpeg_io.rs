//! JPEG import (decode only; JPEG is never written). Every file is decoded to RGB and
//! returned as an opaque [`RgbaImage`] with the EXIF orientation already applied.

use std::fmt;

use dpx_core::{Rgba8, RgbaImage};
use zune_jpeg::JpegDecoder;
use zune_jpeg::errors::DecodeErrors;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

/// Maximum accepted picture size in pixels (64 megapixels).
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

/// Errors from [`read_jpeg`]. They display as a short first line (at most 34 characters, for
/// the file requester) followed by the details (ADR-17).
#[derive(Debug)]
pub enum JpegError {
    /// The data is not a decodable JPEG; holds the decoder's reason.
    Decode(String),
    /// The file ends before the decoder has what it needs; holds the decoder's reason.
    Truncated(String),
    /// The JPEG is valid but outside what we accept: short summary and details.
    Unsupported(&'static str, String),
}

impl JpegError {
    /// The short first line.
    pub fn summary(&self) -> &'static str {
        match self {
            JpegError::Decode(_) => "JPEG cannot be decoded",
            JpegError::Truncated(_) => "JPEG file is cut off",
            JpegError::Unsupported(summary, _) => summary,
        }
    }

    /// What exactly is wrong.
    pub fn details(&self) -> String {
        match self {
            JpegError::Decode(m) | JpegError::Truncated(m) => {
                format!("The JPEG decoder reports: {m}")
            }
            JpegError::Unsupported(_, details) => details.clone(),
        }
    }
}

/// Sorts the decoder's errors into cut-off files, unsupported variants and the rest.
fn decoder_error(e: DecodeErrors) -> JpegError {
    let reason = e.to_string().trim_end().to_string();
    match e {
        DecodeErrors::ExhaustedData | DecodeErrors::IoErrors(_) => JpegError::Truncated(reason),
        DecodeErrors::Unsupported(_) => JpegError::Unsupported(
            "JPEG variant not supported",
            format!("The JPEG decoder reports: {reason}"),
        ),
        _ => JpegError::Decode(reason),
    }
}

impl fmt::Display for JpegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{}", self.summary(), self.details())
    }
}

impl std::error::Error for JpegError {}

/// Decodes a JPEG file (baseline or progressive; greyscale, YCbCr, CMYK/YCCK) into an
/// opaque RGBA image, applying the EXIF orientation if present.
pub fn read_jpeg(bytes: &[u8]) -> Result<RgbaImage, JpegError> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut dec = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    dec.decode_headers().map_err(decoder_error)?;
    let info = dec
        .info()
        .ok_or_else(|| JpegError::Decode("missing image header".into()))?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(JpegError::Unsupported(
            "JPEG picture is too large",
            format!("{w}x{h} pixels exceed the 64 megapixel limit."),
        ));
    }
    let orientation = dec.exif().map_or(1, |e| exif_orientation(e));
    let raw = dec.decode().map_err(decoder_error)?;
    let expected = w as usize * h as usize * 3;
    if raw.len() != expected {
        return Err(JpegError::Decode(format!(
            "unexpected output size {} (expected {expected})",
            raw.len()
        )));
    }
    let pixels: Vec<Rgba8> = raw
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| Rgba8::opaque(c[0], c[1], c[2]))
        .collect();
    let img = RgbaImage::from_pixels(w, h, pixels)
        .ok_or_else(|| JpegError::Decode("pixel buffer size mismatch".into()))?;
    Ok(orient(img, orientation))
}

/// Reads the EXIF orientation (tag 0x0112) from raw EXIF bytes (with or without the
/// `Exif\0\0` prefix). Returns 1 for anything missing, malformed or out of range.
fn exif_orientation(exif: &[u8]) -> u16 {
    parse_orientation(exif)
        .filter(|v| (1..=8).contains(v))
        .unwrap_or(1)
}

fn parse_orientation(data: &[u8]) -> Option<u16> {
    let tiff = data.strip_prefix(b"Exif\0\0").unwrap_or(data);
    let le = match tiff.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(o..o.checked_add(2)?)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(o..o.checked_add(4)?)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = u32_at(4)? as usize;
    let count = usize::from(u16_at(ifd)?);
    for i in 0..count {
        let e = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        if u16_at(e)? == 0x0112 {
            // Type must be SHORT with count 1; the value sits in the first two bytes.
            if u16_at(e + 2)? != 3 || u32_at(e + 4)? != 1 {
                return None;
            }
            return u16_at(e + 8);
        }
    }
    None
}

/// Applies an EXIF orientation (1..=8) so the result is displayed upright.
/// Unknown values return the image unchanged.
fn orient(img: RgbaImage, orientation: u16) -> RgbaImage {
    if !(2..=8).contains(&orientation) {
        return img;
    }
    let (w, h) = (img.width(), img.height());
    let swap = orientation >= 5;
    let (dw, dh) = if swap { (h, w) } else { (w, h) };
    let mut out = RgbaImage::new(dw, dh, Rgba8::opaque(0, 0, 0));
    for y in 0..dh {
        for x in 0..dw {
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x),
            };
            if let Some(p) = img.get(sx, sy) {
                out.set(x, y, p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_BLUE: &[u8] = include_bytes!("../tests/data/red-blue-16x8.jpg");
    const GREY: &[u8] = include_bytes!("../tests/data/grey-8x8.jpg");

    fn close(p: Rgba8, r: u8, g: u8, b: u8, tol: i32) -> bool {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= tol;
        d(p.r, r) && d(p.g, g) && d(p.b, b) && p.a == 255
    }

    fn tiff(le: bool, value: u16) -> Vec<u8> {
        let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
        let mut t = Vec::new();
        t.extend_from_slice(if le { b"II" } else { b"MM" });
        t.extend_from_slice(&u16b(42));
        t.extend_from_slice(&u32b(8));
        t.extend_from_slice(&u16b(1));
        t.extend_from_slice(&u16b(0x0112));
        t.extend_from_slice(&u16b(3));
        t.extend_from_slice(&u32b(1));
        t.extend_from_slice(&u16b(value));
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&u32b(0));
        t
    }

    fn with_exif(jpeg: &[u8], le: bool, value: u16) -> Vec<u8> {
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend_from_slice(&tiff(le, value));
        let len = u16::try_from(seg.len() + 2).unwrap();
        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&seg);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[test]
    fn decodes_red_blue() {
        let img = read_jpeg(RED_BLUE).unwrap();
        assert_eq!((img.width(), img.height()), (16, 8));
        assert!(close(img.get(2, 2).unwrap(), 255, 0, 0, 12));
        assert!(close(img.get(13, 5).unwrap(), 0, 0, 255, 12));
    }

    #[test]
    fn decodes_greyscale_to_rgb() {
        let img = read_jpeg(GREY).unwrap();
        assert_eq!((img.width(), img.height()), (8, 8));
        let p = img.get(4, 4).unwrap();
        assert!(close(p, 128, 128, 128, 6), "{p:?}");
    }

    #[test]
    fn exif_orientation_applied() {
        for le in [true, false] {
            let red = |p: Rgba8| close(p, 255, 0, 0, 12);
            let blue = |p: Rgba8| close(p, 0, 0, 255, 12);

            let i = read_jpeg(&with_exif(RED_BLUE, le, 1)).unwrap();
            assert_eq!((i.width(), i.height()), (16, 8));
            assert!(red(i.get(2, 2).unwrap()));

            let i = read_jpeg(&with_exif(RED_BLUE, le, 6)).unwrap();
            assert_eq!((i.width(), i.height()), (8, 16));
            assert!(red(i.get(4, 2).unwrap()));
            assert!(blue(i.get(4, 13).unwrap()));

            let i = read_jpeg(&with_exif(RED_BLUE, le, 3)).unwrap();
            assert_eq!((i.width(), i.height()), (16, 8));
            assert!(blue(i.get(2, 2).unwrap()));
            assert!(red(i.get(13, 5).unwrap()));

            let i = read_jpeg(&with_exif(RED_BLUE, le, 2)).unwrap();
            assert_eq!((i.width(), i.height()), (16, 8));
            assert!(blue(i.get(2, 2).unwrap()));
            assert!(red(i.get(13, 5).unwrap()));

            let i = read_jpeg(&with_exif(RED_BLUE, le, 8)).unwrap();
            assert_eq!((i.width(), i.height()), (8, 16));
            assert!(blue(i.get(4, 2).unwrap()));
            assert!(red(i.get(4, 13).unwrap()));
        }
    }

    #[test]
    fn exif_orientation_rejects_garbage() {
        assert_eq!(exif_orientation(&[]), 1);
        assert_eq!(exif_orientation(b"Exif\0\0"), 1);
        assert_eq!(exif_orientation(b"XX\0*\0\0\0\x08\0\0"), 1);
        let mut t = tiff(true, 6);
        assert_eq!(exif_orientation(&t), 6);
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&t);
        assert_eq!(exif_orientation(&prefixed), 6);
        assert_eq!(exif_orientation(&tiff(false, 9)), 1);
        assert_eq!(exif_orientation(&tiff(false, 0)), 1);
        // IFD offset past the end.
        t[4..8].copy_from_slice(&1000u32.to_le_bytes());
        assert_eq!(exif_orientation(&t), 1);
        // Huge offset must not overflow.
        t[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(exif_orientation(&t), 1);
        // Truncated at every length.
        let full = tiff(true, 6);
        for n in 0..20 {
            assert_eq!(exif_orientation(&full[..n]), 1, "len {n}");
        }
    }

    #[test]
    fn orient_all_values() {
        let px: Vec<Rgba8> = (0..6u8).map(|v| Rgba8::opaque(v, 0, 0)).collect();
        let src = RgbaImage::from_pixels(3, 2, px).unwrap();
        let cases: [(u16, (u32, u32), [u8; 6]); 8] = [
            (1, (3, 2), [0, 1, 2, 3, 4, 5]),
            (2, (3, 2), [2, 1, 0, 5, 4, 3]),
            (3, (3, 2), [5, 4, 3, 2, 1, 0]),
            (4, (3, 2), [3, 4, 5, 0, 1, 2]),
            (5, (2, 3), [0, 3, 1, 4, 2, 5]),
            (6, (2, 3), [3, 0, 4, 1, 5, 2]),
            (7, (2, 3), [5, 2, 4, 1, 3, 0]),
            (8, (2, 3), [2, 5, 1, 4, 0, 3]),
        ];
        for (o, (w, h), want) in cases {
            let out = orient(src.clone(), o);
            assert_eq!((out.width(), out.height()), (w, h), "orientation {o}");
            let got: Vec<u8> = out.pixels().iter().map(|p| p.r).collect();
            assert_eq!(got, want, "orientation {o}");
        }
    }

    #[test]
    fn garbage_is_decode_error() {
        let junk: Vec<u8> = (0..300u32).map(|i| (i * 7919 % 251) as u8).collect();
        for bytes in [&junk[..], &[], &RED_BLUE[..40]] {
            let e = read_jpeg(bytes).unwrap_err();
            assert!(
                matches!(e, JpegError::Decode(_) | JpegError::Truncated(_)),
                "{e}"
            );
            assert!(e.summary().len() <= 34);
        }
    }

    #[test]
    fn decode_errors_keep_the_decoder_reason() {
        let e = read_jpeg(&RED_BLUE[..40]).unwrap_err();
        assert_eq!(
            e.to_string(),
            "JPEG file is cut off\nThe JPEG decoder reports: I/O errors Not enough bytes, \
             expected 64 but found 15"
        );
        let junk: Vec<u8> = (0..300u32).map(|i| (i * 7919 % 251) as u8).collect();
        let text = read_jpeg(&junk).unwrap_err().to_string();
        assert!(
            text.starts_with("JPEG cannot be decoded\nThe JPEG decoder reports: "),
            "{text}"
        );
    }
}
