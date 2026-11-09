//! The DPX 8×8 bitmap font (original artwork), set proportionally like the DeluxePaint
//! screen font in the reference screenshots: each glyph is trimmed to its inked columns and
//! followed by a 1-pixel gap; the line height stays 8 pixels.

use std::sync::OnceLock;

use dpx_core::IndexedImage;

use crate::bitmap;

pub const GLYPH_H: i32 = 8;
/// Blank columns between two glyphs.
pub const GAP: i32 = 1;
/// Advance of the space character (measured: 7 px including the gap).
pub const SPACE_ADVANCE: i32 = 7;

/// Special glyph codes beyond ASCII.
pub mod glyph {
    pub const SUBMENU: u8 = 0x80;
    pub const CHECK: u8 = 0x81;
    pub const ARROW_LEFT: u8 = 0x82;
    pub const ARROW_RIGHT: u8 = 0x83;
    pub const AMIGA_KEY: u8 = 0x84;
}

#[derive(Clone, Copy)]
struct Glyph {
    rows: [u8; 8],
    /// First inked column.
    left: i32,
    /// Inked width (0 for blank glyphs).
    width: i32,
}

pub struct Font {
    glyphs: Vec<Option<Glyph>>,
    /// Inked width of the widest digit; digits are tabular so numbers do not jitter.
    digit_width: i32,
}

impl Font {
    /// Parses the font text format (`= 0xNN` + 8 rows of 8 pixels).
    pub fn parse(src: &str) -> Result<Self, String> {
        let mut glyphs = vec![None; 256];
        for (name, bmp) in bitmap::parse(src)? {
            let code = name
                .strip_prefix("0x")
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| format!("bad glyph code '{name}'"))?;
            if bmp.width != 8 || bmp.height != 8 {
                return Err(format!(
                    "glyph {name} is {}x{}, expected 8x8",
                    bmp.width, bmp.height
                ));
            }
            let mut rows = [0u8; 8];
            for (y, row) in rows.iter_mut().enumerate() {
                for x in 0..8 {
                    if bmp.get(x, y) {
                        *row |= 0x80 >> x;
                    }
                }
            }
            let ink = rows.iter().fold(0u8, |a, &r| a | r);
            let (left, width) = if ink == 0 {
                (0, 0)
            } else {
                let left = ink.leading_zeros() as i32;
                let right = 7 - ink.trailing_zeros() as i32;
                (left, right - left + 1)
            };
            glyphs[usize::from(code)] = Some(Glyph { rows, left, width });
        }
        let digit_width = (b'0'..=b'9')
            .filter_map(|c| glyphs[usize::from(c)].map(|g: Glyph| g.width))
            .max()
            .unwrap_or(0);
        Ok(Self {
            glyphs,
            digit_width,
        })
    }

    /// The font shipped in `assets/ui/font/dpx-8x8.txt`.
    pub fn builtin() -> &'static Font {
        static FONT: OnceLock<Font> = OnceLock::new();
        FONT.get_or_init(|| {
            Font::parse(include_str!("../../../assets/ui/font/dpx-8x8.txt"))
                .expect("built-in font is valid")
        })
    }

    fn code_for(c: char) -> u8 {
        match c {
            ' '..='~' => c as u8,
            '▸' => glyph::SUBMENU,
            '✓' => glyph::CHECK,
            '◂' => glyph::ARROW_LEFT,
            '▶' => glyph::ARROW_RIGHT,
            _ => b'?',
        }
    }

    /// Horizontal advance of one glyph code, including the trailing gap.
    pub fn advance(&self, code: u8) -> i32 {
        match self.glyphs[usize::from(code)] {
            _ if code == b' ' => SPACE_ADVANCE,
            _ if code.is_ascii_digit() => self.digit_width + GAP,
            Some(g) if g.width > 0 => g.width + GAP,
            _ => SPACE_ADVANCE,
        }
    }

    /// Width of `text` in pixels, without the gap after the last glyph.
    pub fn text_width(&self, text: &str) -> i32 {
        let total: i32 = text.chars().map(|c| self.advance(Self::code_for(c))).sum();
        (total - GAP).max(0)
    }

    /// Draws a single glyph code; returns the x position after it (including the gap).
    pub fn draw_code(&self, img: &mut IndexedImage, x: i32, y: i32, code: u8, pen: u8) -> i32 {
        if let Some(g) = self.glyphs[usize::from(code)] {
            // Tabular digits are centred in the common digit width.
            let pad = if code.is_ascii_digit() {
                (self.digit_width - g.width) / 2
            } else {
                0
            };
            for (dy, bits) in g.rows.iter().enumerate() {
                for col in 0..8 {
                    if bits & (0x80 >> col) != 0 {
                        img.set(x + pad + col - g.left, y + dy as i32, pen);
                    }
                }
            }
        }
        x + self.advance(code)
    }

    /// Draws `text` with its top-left at (`x`, `y`); returns the x position after it.
    pub fn draw(&self, img: &mut IndexedImage, x: i32, y: i32, text: &str, pen: u8) -> i32 {
        text.chars().fold(x, |cx, c| {
            self.draw_code(img, cx, y, Self::code_for(c), pen)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_font_covers_printable_ascii() {
        let f = Font::builtin();
        for c in b' '..=b'~' {
            assert!(
                f.glyphs[usize::from(c)].is_some(),
                "missing glyph {:?}",
                c as char
            );
        }
        for c in 0x80..=0x84u8 {
            assert!(
                f.glyphs[usize::from(c)].is_some(),
                "missing special glyph {c:#x}"
            );
        }
    }

    #[test]
    fn proportional_metrics() {
        let f = Font::builtin();
        // Narrow glyphs advance less than wide ones; the gap is one pixel.
        assert!(f.advance(b'l') < f.advance(b'm'));
        assert_eq!(f.advance(b' '), SPACE_ADVANCE);
        assert_eq!(f.text_width(""), 0);
        let i = f.text_width("i");
        assert_eq!(f.text_width("ii"), 2 * i + GAP);
        // Digits are tabular.
        assert_eq!(f.text_width("1111"), f.text_width("8888"));
    }

    #[test]
    fn draws_left_aligned_without_leading_blank_columns() {
        let mut img = IndexedImage::new(16, 8, 0);
        let f = Font::builtin();
        let end = f.draw(&mut img, 0, 0, "l", 1);
        assert_eq!(end, f.advance(b'l'));
        assert!(
            (0..8).any(|y| img.get(0, y) == Some(1)),
            "ink starts at x=0"
        );
    }
}
