//! PNG import/export of indexed pictures. The palette travels in PLTE, so a PNG round trip
//! keeps pixels and colours exactly. ILBM-only metadata (CAMG, CRNG, masking, aspect) has no
//! place in PNG; callers report what they drop.

use std::fmt;
use std::io::Cursor;

use dpx_core::{IndexedImage, Palette, Rgb8};

#[derive(Debug)]
pub enum PngError {
    Decode(png::DecodingError),
    Encode(png::EncodingError),
    /// Valid PNG that cannot be loaded yet (e.g. true-colour without colour reduction).
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

/// Reads an indexed (palette) PNG with 1, 2, 4 or 8 bits per pixel.
pub fn read_png(bytes: &[u8]) -> Result<PngPicture, PngError> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(PngError::Decode)?;
    let info = reader.info();
    if info.color_type != png::ColorType::Indexed {
        return Err(PngError::Unsupported(format!(
            "{:?} PNG (true-colour import needs colour reduction, planned)",
            info.color_type
        )));
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
    Ok(PngPicture {
        image: IndexedImage::from_pixels(frame.width, frame.height, pixels).expect("w*h pixels"),
        palette,
        transparent_entries,
    })
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
            let back = read_png(&write_png(&img, &pal).unwrap()).unwrap();
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
        let pic = read_png(&png).unwrap();
        assert_eq!(pic.image.pixels(), &[0, 1, 2, 3, 1, 3, 2, 1, 0, 2]);
        assert_eq!(pic.palette.len(), 4);
        assert_eq!(pic.transparent_entries, vec![0]);
    }

    #[test]
    fn true_colour_is_refused_clearly() {
        let png = encode(
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            1,
            1,
            &[1, 2, 3],
            None,
            None,
        );
        assert!(matches!(read_png(&png), Err(PngError::Unsupported(_))));
    }

    #[test]
    fn writer_refuses_out_of_palette_pixels() {
        let (img, _) = sample(4, 4, 4);
        let small = Palette::new(vec![Rgb8::BLACK; 2]).unwrap();
        assert!(matches!(write_png(&img, &small), Err(PngError::Invalid(_))));
    }
}
