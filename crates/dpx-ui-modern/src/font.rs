//! The bundled font of the Modern skin: Go Regular and Go Medium (`assets/fonts/`, BSD-style
//! licence in `assets/fonts/LICENSE-Go.txt`). No system font is used, so text has the same
//! metrics on every platform and in tests. The layout code and the rasteriser share
//! [`measure`], so text is always as wide on screen as the layout assumed.

use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};

use crate::paint::Weight;

/// The font files, as bundled.
pub const REGULAR_TTF: &[u8] = include_bytes!("../../../assets/fonts/Go-Regular.ttf");
pub const MEDIUM_TTF: &[u8] = include_bytes!("../../../assets/fonts/Go-Medium.ttf");

/// Em size of the bar's text and of menus, in points.
pub const SIZE_BAR: f32 = 13.5;
/// Em size of panel text and tooltips, in points.
pub const SIZE_PANEL: f32 = 12.0;

/// The parsed font of `weight`.
pub fn face(weight: Weight) -> &'static FontRef<'static> {
    static FACES: OnceLock<[FontRef<'static>; 2]> = OnceLock::new();
    let faces = FACES.get_or_init(|| {
        [REGULAR_TTF, MEDIUM_TTF]
            .map(|data| FontRef::try_from_slice(data).expect("the bundled font is a valid TTF"))
    });
    match weight {
        Weight::Regular => &faces[0],
        Weight::Medium => &faces[1],
    }
}

/// `ab_glyph` scales a font by its line height (ascent minus descent); sizes here are em
/// sizes, as in every design tool.
pub(crate) fn px_scale(font: &FontRef<'_>, em: f32) -> PxScale {
    let units = font.units_per_em().unwrap_or(2048.0);
    PxScale::from(em * font.height_unscaled() / units)
}

/// The advance width of `text` at em size `size`, kerning included. Linear in `size`: the
/// font is used unhinted.
pub fn measure(text: &str, size: f32, weight: Weight) -> f32 {
    let font = face(weight);
    let scaled = font.as_scaled(px_scale(font, size));
    let mut width = 0.0;
    let mut previous = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(p) = previous {
            width += scaled.kern(p, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

/// The height of a capital letter at em size `size`.
pub fn cap_height(size: f32, weight: Weight) -> f32 {
    static CAPS: OnceLock<[f32; 2]> = OnceLock::new();
    let caps = CAPS.get_or_init(|| {
        [Weight::Regular, Weight::Medium].map(|w| {
            let font = face(w);
            let units = font.units_per_em().unwrap_or(2048.0);
            font.outline(font.glyph_id('H'))
                .map_or(0.7, |o| (o.bounds.max.y - o.bounds.min.y).abs() / units)
        })
    });
    size * match weight {
        Weight::Regular => caps[0],
        Weight::Medium => caps[1],
    }
}

/// The baseline that centres capital letters of em size `size` on the line `center_y`.
pub fn baseline_centered(center_y: f32, size: f32, weight: Weight) -> f32 {
    center_y + cap_height(size, weight) / 2.0
}

/// The longest prefix of `text` that is at most `room` wide, with "…" in place of what was
/// cut; the whole text when it fits, an empty string when not even one letter and the
/// ellipsis fit.
pub fn fit(text: &str, room: f32, size: f32, weight: Weight) -> String {
    if measure(text, size, weight) <= room {
        return text.to_string();
    }
    let mut cut: Vec<char> = text.chars().collect();
    while cut.pop().is_some() && !cut.is_empty() {
        let candidate: String = cut.iter().chain(&['…']).collect();
        if measure(&candidate, size, weight) <= room {
            return candidate;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The licence text stored in a TrueType file: name record 13 of the Windows platform.
    fn licence_in(ttf: &[u8]) -> String {
        let u16_at = |o: usize| usize::from(u16::from_be_bytes([ttf[o], ttf[o + 1]]));
        let u32_at =
            |o: usize| u32::from_be_bytes([ttf[o], ttf[o + 1], ttf[o + 2], ttf[o + 3]]) as usize;
        let tables = u16_at(4);
        let name = (0..tables)
            .map(|i| 12 + 16 * i)
            .find(|&o| &ttf[o..o + 4] == b"name")
            .map(|o| u32_at(o + 8))
            .expect("a name table");
        let (count, strings) = (u16_at(name + 2), name + u16_at(name + 4));
        (0..count)
            .map(|i| name + 6 + 12 * i)
            .find(|&r| u16_at(r) == 3 && u16_at(r + 6) == 13)
            .map(|r| {
                let (len, start) = (u16_at(r + 8), strings + u16_at(r + 10));
                let units: Vec<u16> = ttf[start..start + len]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_be_bytes(*b))
                    .collect();
                String::from_utf16(&units).expect("UTF-16 licence text")
            })
            .expect("a licence record")
    }

    /// The licence file beside the fonts is the text the fonts carry, and that text is the
    /// BSD-style licence the project allows. A replaced font file fails here.
    #[test]
    fn the_bundled_fonts_carry_the_licence_that_is_shipped_beside_them() {
        let shipped = include_str!("../../../assets/fonts/LICENSE-Go.txt").replace("\r\n", "\n");
        for (name, ttf) in [("Go-Regular", REGULAR_TTF), ("Go-Medium", MEDIUM_TTF)] {
            let licence = licence_in(ttf);
            assert_eq!(licence.trim(), shipped.trim(), "{name}");
            assert!(licence.contains("Bigelow & Holmes"), "{name}");
            assert!(
                licence.contains("Redistribution and use in source and binary forms"),
                "{name}"
            );
            for forbidden in ["Open Font License", "OFL", "Ubuntu Font Licence"] {
                assert!(!licence.contains(forbidden), "{name}: {forbidden}");
            }
        }
    }

    #[test]
    fn measuring_is_linear_in_the_size_and_additive_enough() {
        let w = measure("Picture", SIZE_BAR, Weight::Regular);
        assert!(w > 30.0 && w < 60.0, "Picture at 13.5 pt is {w} wide");
        let double = measure("Picture", 2.0 * SIZE_BAR, Weight::Regular);
        assert!((double - 2.0 * w).abs() < 1e-3, "{double} vs {w}");
        assert_eq!(measure("", SIZE_BAR, Weight::Regular), 0.0);
        // Medium is the heavier cut: not narrower.
        assert!(measure("Picture", SIZE_BAR, Weight::Medium) >= w);
    }

    #[test]
    fn capitals_are_about_seven_tenths_of_the_em() {
        for weight in [Weight::Regular, Weight::Medium] {
            let cap = cap_height(10.0, weight);
            assert!((6.0..8.0).contains(&cap), "{weight:?}: {cap}");
            assert_eq!(baseline_centered(15.0, 10.0, weight), 15.0 + cap / 2.0);
        }
    }

    #[test]
    fn fitting_cuts_with_an_ellipsis() {
        let (size, weight) = (SIZE_PANEL, Weight::Regular);
        let full = "Layer: Background";
        let wide = measure(full, size, weight);
        assert_eq!(fit(full, wide, size, weight), full);
        let cut = fit(full, wide - 20.0, size, weight);
        assert!(cut.ends_with('…') && cut.len() < full.len(), "{cut}");
        assert!(measure(&cut, size, weight) <= wide - 20.0);
        assert_eq!(fit(full, 2.0, size, weight), "");
    }
}
