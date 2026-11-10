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
    /// Text of disabled (ghosted) items: a grey between `dark` and `face`, if the palette
    /// has a usable one. Without it, disabled items fall back to a checkerboard pattern.
    pub ghost: Option<u8>,
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
            ghost: ghost_pen(palette, face, dark),
        }
    }
}

/// Picks the ghost pen: the entry closest to a grey 60 % of the way from `dark` to `face`
/// (owner decision 2026-11-10: ghosted options are grey, not a dot pattern). Any entry at
/// least 25 % of the dark–face distance away from both `face` and `dark` qualifies, so
/// ghosted text stays visible and differs from normal text. Only palettes without such an
/// entry (two colours, or all colours near face/dark) get `None` and the dot pattern.
fn ghost_pen(palette: &Palette, face: u8, dark: u8) -> Option<u8> {
    let (f, d) = (palette.get(face)?, palette.get(dark)?);
    let mix = |a: u8, b: u8| ((u16::from(a) * 2 + u16::from(b) * 3) / 5) as u8;
    let ideal = Rgb8::new(mix(d.r, f.r), mix(d.g, f.g), mix(d.b, f.b));
    let span = u64::from(f.distance_sq(d));
    let apart = |c: Rgb8| {
        16 * u64::from(c.distance_sq(f)) >= span && 16 * u64::from(c.distance_sq(d)) >= span
    };
    palette
        .colors()
        .iter()
        .enumerate()
        .filter(|&(i, &c)| i != usize::from(face) && i != usize::from(dark) && apart(c))
        .min_by_key(|&(_, &c)| c.distance_sq(ideal))
        .map(|(i, _)| i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_gives_grey_ui() {
        let p = Pens::from_palette(&Palette::dpaint_default_32());
        assert_eq!((p.dark, p.face, p.shine), (0, 1, 3));
        // 0x666 is exactly 60 % of the way from black to 0xAAA.
        let ghost = p.ghost.expect("default palette has a grey ghost pen");
        assert_eq!(
            Palette::dpaint_default_32().get(ghost),
            Some(Rgb8::new(0x66, 0x66, 0x66))
        );
    }

    #[test]
    fn ghost_pen_needs_a_real_grey() {
        // Black/white only: no colour in between, so disabled items use the dot pattern.
        let bw = Palette::from_rgb4(&[0x000, 0xFFF]).unwrap();
        assert_eq!(Pens::from_palette(&bw).ghost, None);
        // Without a grey, the colour nearest the ideal mid grey is used (no dot pattern).
        let vivid = Palette::from_rgb4(&[0x000, 0xAAA, 0x05A, 0xFFF]).unwrap();
        assert_eq!(Pens::from_palette(&vivid).ghost, Some(2));
        // Colours too close to face or dark do not qualify.
        let close = Palette::from_rgb4(&[0x000, 0xAAA, 0x111, 0x999]).unwrap();
        assert_eq!(Pens::from_palette(&close).ghost, None);
        // A mid grey is.
        let grey = Palette::from_rgb4(&[0x000, 0xAAA, 0x666, 0xFFF]).unwrap();
        assert_eq!(Pens::from_palette(&grey).ghost, Some(2));
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
