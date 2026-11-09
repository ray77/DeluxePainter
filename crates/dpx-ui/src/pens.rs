//! UI pens taken from the picture palette, like DeluxePaint does on the Amiga.
//!
//! See `docs/design/AMIGA_GUI_FIDELITY.md` §2: every pen is the palette entry closest to an
//! ideal colour, so with few colours the interface appears in picture colours.

use dpx_core::{Palette, Rgb8};

pub const IDEAL_FACE: Rgb8 = Rgb8::new(0xAA, 0xAA, 0xAA);
pub const IDEAL_DARK: Rgb8 = Rgb8::BLACK;
pub const IDEAL_SHINE: Rgb8 = Rgb8::WHITE;
pub const IDEAL_SELECT: Rgb8 = Rgb8::new(0x00, 0x55, 0xAA);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pens {
    /// Bars, buttons and requester backgrounds.
    pub face: u8,
    /// Text, icons, outlines and gaps.
    pub dark: u8,
    /// Bevel highlights.
    pub shine: u8,
    /// Highlighted menu items and selected toggles.
    pub select: u8,
}

impl Pens {
    pub fn from_palette(palette: &Palette) -> Self {
        let dark = palette.nearest(IDEAL_DARK);
        // `face` must differ from `dark` so text stays readable.
        let face = palette.nearest_excluding(IDEAL_FACE, &[dark]);
        let shine = palette.nearest_excluding(IDEAL_SHINE, &[dark]);
        let select = palette.nearest_excluding(IDEAL_SELECT, &[face]);
        Self {
            face,
            dark,
            shine,
            select,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_gives_grey_ui() {
        let p = Pens::from_palette(&Palette::dpaint_default_32());
        assert_eq!((p.dark, p.face, p.shine), (0, 1, 3));
    }

    #[test]
    fn few_colours_use_picture_colours() {
        // Black, dark blue, gold, light gold (like the DPaint II Tutankhamun reference):
        // bars and buttons become light gold with black text.
        let pal = Palette::from_rgb4(&[0x000, 0x115, 0xC90, 0xFE8]).unwrap();
        let p = Pens::from_palette(&pal);
        assert_eq!(p.dark, 0);
        assert_eq!(p.face, 3);
        assert_eq!(p.shine, 3);
    }

    #[test]
    fn two_colour_screen_keeps_text_readable() {
        let pal = Palette::from_rgb4(&[0x000, 0xFFF]).unwrap();
        let p = Pens::from_palette(&pal);
        assert_ne!(p.face, p.dark);
    }
}
