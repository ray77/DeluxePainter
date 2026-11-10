//! Extra-Half-Brite (EHB) codec: 6 bitplanes, 32 base colours plus 32 derived half-bright
//! colours (ADR-25, `docs/HARDWARE_MODES.md` section 6).
//!
//! Pixel index `i` (0..=63): `i < 32` shows `base[i]`, `i >= 32` shows `base[i - 32]` at half
//! brightness. Half brightness is a right shift by one of each channel: on OCS (4 bit per
//! channel) the shift is applied to the 4-bit value, on AGA (8 bit) to the 8-bit value.

use dpx_core::quantize::PaletteDepth;
use dpx_core::{IndexedImage, Rgb8, Rgba8, RgbaImage};

/// Half brightness of one colour at the given palette precision.
pub fn half_bright(c: Rgb8, depth: PaletteDepth) -> Rgb8 {
    match depth {
        PaletteDepth::Rgb8 => Rgb8::new(c.r >> 1, c.g >> 1, c.b >> 1),
        PaletteDepth::Rgb4 => {
            // The 4-bit value is the high nibble (nibble replication keeps it there).
            let h = |v: u8| ((v >> 4) >> 1) * 17;
            Rgb8::new(h(c.r), h(c.g), h(c.b))
        }
    }
}

/// The full 64-entry EHB palette. `base` supplies entries 0-31 (missing ones are black, extra
/// ones ignored); entries 32-63 are derived.
pub fn ehb_palette(base: &[Rgb8], depth: PaletteDepth) -> [Rgb8; 64] {
    let mut out = [Rgb8::BLACK; 64];
    for i in 0..32 {
        let c = base.get(i).copied().unwrap_or(Rgb8::BLACK);
        out[i] = c;
        out[i + 32] = half_bright(c, depth);
    }
    out
}

/// Decodes 6-plane EHB index data (the chunky value of planes 0-5) to RGB. Indices above 63
/// use only their low 6 bits.
pub fn decode_ehb(indices: &IndexedImage, base: &[Rgb8], depth: PaletteDepth) -> RgbaImage {
    let pal = ehb_palette(base, depth);
    let pixels = indices
        .pixels()
        .iter()
        .map(|&i| {
            let c = pal[usize::from(i & 63)];
            Rgba8::opaque(c.r, c.g, c.b)
        })
        .collect();
    RgbaImage::from_pixels(indices.width(), indices.height(), pixels).expect("same size")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_bright_values() {
        // AGA: plain shift, odd values round down.
        assert_eq!(
            half_bright(Rgb8::new(255, 128, 1), PaletteDepth::Rgb8),
            Rgb8::new(127, 64, 0)
        );
        // OCS: 0xF -> 0x7 -> 0x77; 0x8 -> 0x4 -> 0x44; 0x1 -> 0x0.
        assert_eq!(
            half_bright(Rgb8::from_rgb4(0xF81), PaletteDepth::Rgb4),
            Rgb8::from_rgb4(0x740)
        );
    }

    #[test]
    fn palette_derivation_and_padding() {
        let base = [Rgb8::from_rgb4(0xFFF), Rgb8::from_rgb4(0xA52)];
        let p = ehb_palette(&base, PaletteDepth::Rgb4);
        assert_eq!(p[0], Rgb8::from_rgb4(0xFFF));
        assert_eq!(p[32], Rgb8::from_rgb4(0x777));
        assert_eq!(p[33], Rgb8::from_rgb4(0x521)); // A>>1=5, 5>>1=2, 2>>1=1
        assert_eq!(p[2], Rgb8::BLACK);
        assert_eq!(p[34], Rgb8::BLACK);
    }

    #[test]
    fn decode_uses_plane5_as_half() {
        let base = [Rgb8::new(200, 100, 50), Rgb8::new(10, 20, 30)];
        let img = IndexedImage::from_pixels(4, 1, vec![0, 1, 32, 33]).unwrap();
        let out = decode_ehb(&img, &base, PaletteDepth::Rgb8);
        let px: Vec<_> = out.pixels().iter().map(|p| (p.r, p.g, p.b, p.a)).collect();
        assert_eq!(
            px,
            vec![
                (200, 100, 50, 255),
                (10, 20, 30, 255),
                (100, 50, 25, 255),
                (5, 10, 15, 255)
            ]
        );
    }
}
