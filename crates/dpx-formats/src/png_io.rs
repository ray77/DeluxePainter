//! PNG import/export. Indexed pictures keep their palette in PLTE, so a PNG round trip keeps
//! pixels and colours exactly. True-colour and greyscale PNGs (any bit depth, with or without
//! alpha) are read as RGBA for colour reduction (ADR-9). ILBM-only metadata (CAMG, CRNG,
//! masking, aspect) has no place in PNG; callers report what they drop.

use std::fmt;
use std::io::Cursor;

use dpx_core::{IndexedImage, Palette, Rgb8, Rgba8, RgbaImage};

#[derive(Debug)]
pub enum PngError {
    Decode(png::DecodingError),
    Encode(png::EncodingError),
    /// Valid PNG that cannot be loaded (e.g. too large).
    Unsupported(String),
    /// The picture cannot be written as described.
    Invalid(String),
}

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngError::Decode(e) => write!(f, "PNG decoding failed: {e}"),
            PngError::Encode(e) => write!(f, "PNG encoding failed: {e}"),
            PngError::Unsupported(what) => write!(f, "not supported yet: {what}"),
            PngError::Invalid(what) => write!(f, "cannot write PNG: {what}"),
        }
    }
}

impl std::error::Error for PngError {}

/// An indexed PNG picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PngPicture {
    pub image: IndexedImage,
    pub palette: Palette,
    /// Palette entries with alpha < 255 (tRNS). DeluxePainter pages are opaque, so callers
    /// must tell the user when this is not empty.
    pub transparent_entries: Vec<u8>,
}

/// Largest accepted picture (64 megapixels, as for JPEG).
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

/// A PNG as read: indexed with its palette, or true colour for colour reduction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PngImage {
    Indexed(PngPicture),
    TrueColor(RgbaImage),
}

fn decoder(bytes: &[u8]) -> png::Decoder<Cursor<&[u8]>> {
    // RGBA output of the largest accepted picture, plus room for the raw rows.
    let limits = png::Limits {
        bytes: (MAX_PIXELS * 8) as usize,
    };
    png::Decoder::new_with_limits(Cursor::new(bytes), limits)
}

/// Reads a PNG: indexed (1, 2, 4 or 8 bits per pixel) as it is, everything else as RGBA.
pub fn read_png(bytes: &[u8]) -> Result<PngImage, PngError> {
    let mut decoder = decoder(bytes);
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(PngError::Decode)?;
    let info = reader.info();
    if u64::from(info.width) * u64::from(info.height) > MAX_PIXELS {
        return Err(PngError::Unsupported(format!(
            "{}x{} exceeds the 64 megapixel limit",
            info.width, info.height
        )));
    }
    if info.color_type != png::ColorType::Indexed {
        return read_true_color(bytes).map(PngImage::TrueColor);
    }
    let depth = info.bit_depth as u32;
    let plte = info
        .palette
        .as_deref()
        .ok_or_else(|| PngError::Unsupported("indexed PNG without PLTE".into()))?
        .to_vec();
    let trns = info.trns.as_deref().map(<[u8]>::to_vec).unwrap_or_default();
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| PngError::Unsupported("picture too large".into()))?
    ];
    let frame = reader.next_frame(&mut buf).map_err(PngError::Decode)?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut pixels = Vec::with_capacity(w * h);
    let per_byte = 8 / depth as usize;
    let mask = ((1u16 << depth) - 1) as u8;
    for row in buf.chunks(frame.line_size).take(h) {
        for x in 0..w {
            let byte = row[x / per_byte];
            let shift = 8 - depth as usize * (x % per_byte + 1);
            pixels.push((byte >> shift) & mask);
        }
    }
    let colors: Vec<Rgb8> = plte
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| Rgb8::new(c[0], c[1], c[2]))
        .collect();
    let palette = Palette::new(colors).map_err(|e| PngError::Unsupported(format!("PLTE: {e}")))?;
    if let Some(&max) = pixels.iter().max()
        && usize::from(max) >= palette.len()
    {
        return Err(PngError::Unsupported(format!(
            "pixel index {max} outside the {}-entry PLTE",
            palette.len()
        )));
    }
    let transparent_entries = trns
        .iter()
        .enumerate()
        .filter(|&(_, &a)| a < 255)
        .map(|(i, _)| i as u8)
        .collect();
    Ok(PngImage::Indexed(PngPicture {
        image: IndexedImage::from_pixels(frame.width, frame.height, pixels).expect("w*h pixels"),
        palette,
        transparent_entries,
    }))
}

/// Greyscale or RGB PNG (with alpha or a tRNS colour key) → 8-bit RGBA.
fn read_true_color(bytes: &[u8]) -> Result<RgbaImage, PngError> {
    let mut decoder = decoder(bytes);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(PngError::Decode)?;
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| PngError::Unsupported("picture too large".into()))?
    ];
    let frame = reader.next_frame(&mut buf).map_err(PngError::Decode)?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err(PngError::Unsupported("unexpected indexed output".into()));
        }
    };
    if frame.bit_depth != png::BitDepth::Eight {
        return Err(PngError::Unsupported(format!(
            "unexpected {:?} output",
            frame.bit_depth
        )));
    }
    let mut pixels = Vec::with_capacity(w * h);
    for row in buf.chunks(frame.line_size).take(h) {
        for px in row[..w * channels].chunks_exact(channels) {
            pixels.push(match *px {
                [v] => Rgba8::opaque(v, v, v),
                [v, a] => Rgba8::new(v, v, v, a),
                [r, g, b] => Rgba8::opaque(r, g, b),
                [r, g, b, a] => Rgba8::new(r, g, b, a),
                _ => unreachable!("chunks of 1 to 4 channels"),
            });
        }
    }
    RgbaImage::from_pixels(frame.width, frame.height, pixels)
        .ok_or_else(|| PngError::Unsupported("truncated picture data".into()))
}

/// Writes an 8-bit indexed PNG with the full palette in PLTE.
pub fn write_png(image: &IndexedImage, palette: &Palette) -> Result<Vec<u8>, PngError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(PngError::Invalid("empty picture".into()));
    }
    if let Some(&max) = image.pixels().iter().max()
        && usize::from(max) >= palette.len()
    {
        return Err(PngError::Invalid(format!(
            "pixel colour {max} is outside the {}-colour palette",
            palette.len()
        )));
    }
    let plte: Vec<u8> = palette
        .colors()
        .iter()
        .flat_map(|c| [c.r, c.g, c.b])
        .collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, image.width(), image.height());
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(plte);
        let mut writer = enc.write_header().map_err(PngError::Encode)?;
        writer
            .write_image_data(image.pixels())
            .map_err(PngError::Encode)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(w: u32, h: u32, colors: usize) -> (IndexedImage, Palette) {
        let mut img = IndexedImage::new(w, h, 0);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                img.set(x, y, ((x + 3 * y) as usize % colors) as u8);
            }
        }
        let pal = Palette::new(
            (0..colors)
                .map(|i| Rgb8::new(i as u8, (i * 7) as u8, 255 - i as u8))
                .collect(),
        )
        .unwrap();
        (img, pal)
    }

    #[test]
    fn indexed_round_trip() {
        for (w, h, colors) in [(1, 1, 1), (7, 3, 2), (320, 200, 32), (33, 5, 256)] {
            let (img, pal) = sample(w, h, colors);
            let PngImage::Indexed(back) = read_png(&write_png(&img, &pal).unwrap()).unwrap() else {
                panic!("expected an indexed PNG");
            };
            assert_eq!(back.image, img);
            assert_eq!(back.palette, pal);
            assert!(back.transparent_entries.is_empty());
        }
    }

    fn encode(
        color: png::ColorType,
        depth: png::BitDepth,
        w: u32,
        h: u32,
        data: &[u8],
        plte: Option<Vec<u8>>,
        trns: Option<Vec<u8>>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(depth);
            if let Some(p) = plte {
                enc.set_palette(p);
            }
            if let Some(t) = trns {
                enc.set_trns(t);
            }
            enc.write_header().unwrap().write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn low_bit_depths_and_transparency() {
        // 2 bits per pixel, 5 pixels per row → 2 bytes per row.
        let data = [0b00_01_10_11, 0b01_000000, 0b11_10_01_00, 0b10_000000];
        let plte = vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
        let png = encode(
            png::ColorType::Indexed,
            png::BitDepth::Two,
            5,
            2,
            &data,
            Some(plte),
            Some(vec![0, 255]),
        );
        let PngImage::Indexed(pic) = read_png(&png).unwrap() else {
            panic!("expected an indexed PNG");
        };
        assert_eq!(pic.image.pixels(), &[0, 1, 2, 3, 1, 3, 2, 1, 0, 2]);
        assert_eq!(pic.palette.len(), 4);
        assert_eq!(pic.transparent_entries, vec![0]);
    }

    fn true_color(png: &[u8]) -> RgbaImage {
        match read_png(png).unwrap() {
            PngImage::TrueColor(img) => img,
            PngImage::Indexed(_) => panic!("expected true colour"),
        }
    }

    #[test]
    fn true_colour_and_greyscale_read_as_rgba() {
        use png::{BitDepth, ColorType};
        let rgb = encode(
            ColorType::Rgb,
            BitDepth::Eight,
            2,
            1,
            &[1, 2, 3, 4, 5, 6],
            None,
            None,
        );
        let img = true_color(&rgb);
        assert_eq!(
            img.pixels(),
            &[Rgba8::opaque(1, 2, 3), Rgba8::opaque(4, 5, 6)]
        );
        let rgba = encode(
            ColorType::Rgba,
            BitDepth::Eight,
            1,
            1,
            &[9, 8, 7, 6],
            None,
            None,
        );
        assert_eq!(true_color(&rgba).pixels(), &[Rgba8::new(9, 8, 7, 6)]);
        // 16-bit samples keep their high byte.
        let rgb16 = encode(
            ColorType::Rgb,
            BitDepth::Sixteen,
            1,
            1,
            &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC],
            None,
            None,
        );
        assert_eq!(
            true_color(&rgb16).pixels(),
            &[Rgba8::opaque(0x12, 0x56, 0x9A)]
        );
        // 1-bit greyscale with a tRNS key: black is transparent.
        let grey = encode(
            ColorType::Grayscale,
            BitDepth::One,
            3,
            1,
            &[0b0100_0000],
            None,
            Some(vec![0, 0]),
        );
        assert_eq!(
            true_color(&grey).pixels(),
            &[
                Rgba8::new(0, 0, 0, 0),
                Rgba8::opaque(255, 255, 255),
                Rgba8::new(0, 0, 0, 0)
            ]
        );
        let ga = encode(
            ColorType::GrayscaleAlpha,
            BitDepth::Eight,
            1,
            1,
            &[77, 128],
            None,
            None,
        );
        assert_eq!(true_color(&ga).pixels(), &[Rgba8::new(77, 77, 77, 128)]);
    }

    #[test]
    fn garbage_is_a_decode_error() {
        assert!(matches!(read_png(b"not a png"), Err(PngError::Decode(_))));
    }

    #[test]
    fn writer_refuses_out_of_palette_pixels() {
        let (img, _) = sample(4, 4, 4);
        let small = Palette::new(vec![Rgb8::BLACK; 2]).unwrap();
        assert!(matches!(write_png(&img, &small), Err(PngError::Invalid(_))));
    }
}
