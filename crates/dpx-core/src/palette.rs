//! Colour palettes (OCS RGB4 / AGA RGB8).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A 24-bit colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub const BLACK: Rgb8 = Rgb8::new(0, 0, 0);
    pub const WHITE: Rgb8 = Rgb8::new(255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Expands an OCS 12-bit colour (`0xRGB`) to 24 bit by nibble replication (`0xA` → `0xAA`).
    pub const fn from_rgb4(rgb4: u16) -> Self {
        let r = ((rgb4 >> 8) & 0xF) as u8;
        let g = ((rgb4 >> 4) & 0xF) as u8;
        let b = (rgb4 & 0xF) as u8;
        Self::new(r * 17, g * 17, b * 17)
    }

    /// Nearest OCS 12-bit colour (`0xRGB`), rounding each channel.
    pub fn to_rgb4(self) -> u16 {
        let q = |v: u8| ((u16::from(v) + 8) / 17).min(15);
        (q(self.r) << 8) | (q(self.g) << 4) | q(self.b)
    }

    /// Perceptual-ish squared distance ("redmean" weighting); cheap and stable.
    pub fn distance_sq(self, other: Rgb8) -> u32 {
        let rmean = (i32::from(self.r) + i32::from(other.r)) / 2;
        let dr = i32::from(self.r) - i32::from(other.r);
        let dg = i32::from(self.g) - i32::from(other.g);
        let db = i32::from(self.b) - i32::from(other.b);
        let v = (((512 + rmean) * dr * dr) >> 8) + 4 * dg * dg + (((767 - rmean) * db * db) >> 8);
        v as u32
    }
}

impl fmt::Display for Rgb8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteError {
    /// A palette must hold between 1 and 256 entries.
    InvalidSize(usize),
}

impl fmt::Display for PaletteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PaletteError::InvalidSize(n) => write!(f, "palette size {n} is outside 1..=256"),
        }
    }
}

impl std::error::Error for PaletteError {}

/// An indexed palette with 1–256 entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    colors: Vec<Rgb8>,
}

impl Palette {
    pub fn new(colors: Vec<Rgb8>) -> Result<Self, PaletteError> {
        if colors.is_empty() || colors.len() > 256 {
            return Err(PaletteError::InvalidSize(colors.len()));
        }
        Ok(Self { colors })
    }

    pub fn from_rgb4(values: &[u16]) -> Result<Self, PaletteError> {
        Self::new(values.iter().map(|&v| Rgb8::from_rgb4(v)).collect())
    }

    /// The DeluxePaint IV start-up palette for 32-colour OCS screens.
    ///
    /// Measured from reference screenshots and quantised to RGB4; to be confirmed against a
    /// lossless emulator capture (see `docs/design/AMIGA_GUI_FIDELITY.md`).
    pub fn dpaint_default_32() -> Self {
        const RGB4: [u16; 32] = [
            0x000, 0xAAA, 0xE00, 0xFFF, 0xD80, 0xFE0, 0x8F0, 0x080, //
            0x0B6, 0x0DD, 0x0AF, 0x07C, 0x00F, 0x70F, 0xC0E, 0xC08, //
            0x620, 0xE52, 0xA52, 0xFCA, 0x333, 0x444, 0x555, 0x666, //
            0x777, 0x888, 0x999, 0xAAA, 0x000, 0xDDD, 0x068, 0xFFF, //
        ];
        Self::from_rgb4(&RGB4).expect("32 entries")
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn get(&self, index: u8) -> Option<Rgb8> {
        self.colors.get(usize::from(index)).copied()
    }

    pub fn colors(&self) -> &[Rgb8] {
        &self.colors
    }

    /// Replaces one entry; returns `false` if the index is out of range.
    pub fn set(&mut self, index: u8, color: Rgb8) -> bool {
        match self.colors.get_mut(usize::from(index)) {
            Some(c) => {
                *c = color;
                true
            }
            None => false,
        }
    }

    /// Index of the entry closest to `target`; ties resolve to the lowest index.
    pub fn nearest(&self, target: Rgb8) -> u8 {
        self.nearest_excluding(target, &[])
    }

    /// Like [`Palette::nearest`] but skips the given indices when another entry exists.
    pub fn nearest_excluding(&self, target: Rgb8, exclude: &[u8]) -> u8 {
        let mut best: Option<(u32, usize)> = None;
        for (i, &c) in self.colors.iter().enumerate() {
            if exclude.contains(&(i as u8)) {
                continue;
            }
            let d = c.distance_sq(target);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        best.map_or(0, |(_, i)| i as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb4_roundtrip() {
        for v in [0x000, 0xFFF, 0xA5C, 0x123] {
            assert_eq!(Rgb8::from_rgb4(v).to_rgb4(), v);
        }
        assert_eq!(Rgb8::from_rgb4(0xA0F), Rgb8::new(0xAA, 0x00, 0xFF));
    }

    #[test]
    fn nearest_prefers_lowest_index_on_tie() {
        let p = Palette::dpaint_default_32();
        // 0xAAA appears at index 1 and 27.
        assert_eq!(p.nearest(Rgb8::new(0xAA, 0xAA, 0xAA)), 1);
        assert_eq!(p.nearest(Rgb8::BLACK), 0);
        assert_eq!(p.nearest(Rgb8::WHITE), 3);
    }

    #[test]
    fn size_limits() {
        assert!(Palette::new(vec![]).is_err());
        assert!(Palette::new(vec![Rgb8::BLACK; 257]).is_err());
        assert!(Palette::new(vec![Rgb8::BLACK; 256]).is_ok());
    }
}
