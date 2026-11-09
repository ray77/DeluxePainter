//! The DPX 8×8 bitmap font (original, Topaz-style metrics).

use std::sync::OnceLock;

use dpx_core::IndexedImage;

use crate::bitmap;

pub const GLYPH_W: i32 = 8;
pub const GLYPH_H: i32 = 8;

/// Special glyph codes beyond ASCII.
pub mod glyph {
    pub const SUBMENU: u8 = 0x80;
    pub const CHECK: u8 = 0x81;
    pub const ARROW_LEFT: u8 = 0x82;
    pub const ARROW_RIGHT: u8 = 0x83;
    pub const AMIGA_KEY: u8 = 0x84;
}

pub struct Font {
    glyphs: Vec<Option<[u8; 8]>>,
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
            glyphs[usize::from(code)] = Some(rows);
        }
        Ok(Self { glyphs })
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

    pub fn text_width(text: &str) -> i32 {
        text.chars().count() as i32 * GLYPH_W
    }

    /// Draws a single glyph code; returns the x position after it.
    pub fn draw_code(&self, img: &mut IndexedImage, x: i32, y: i32, code: u8, pen: u8) -> i32 {
        if let Some(rows) = self.glyphs[usize::from(code)] {
            for (dy, bits) in rows.iter().enumerate() {
                for dx in 0..8 {
                    if bits & (0x80 >> dx) != 0 {
                        img.set(x + dx, y + dy as i32, pen);
                    }
                }
            }
        }
        x + GLYPH_W
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
    fn draws_within_cell() {
        let mut img = IndexedImage::new(16, 8, 0);
        let end = Font::builtin().draw(&mut img, 0, 0, "A", 1);
        assert_eq!(end, 8);
        assert!(img.pixels().contains(&1));
        for y in 0..8 {
            assert_eq!(img.get(15, y), Some(0));
        }
    }
}
