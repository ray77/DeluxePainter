//! Hold-And-Modify codecs for HAM6 (OCS/AGA, 6 planes) and HAM8 (AGA, 8 planes); ADR-25.
//!
//! Per pixel the plane value is split into control bits (the two top planes) and data bits
//! (the rest). Control `00` = set from palette; `01` = modify blue, `10` = modify red,
//! `11` = modify green, each keeping the other channels of the pixel to the left. Every row
//! starts from palette entry 0 (HARDWARE_MODES.md 7.3, likely, not verified against the HRM).
//!
//! HAM6 modify: 4 data bits replace the channel, expanded by nibble replication.
//! HAM8 modify: 6 data bits replace the upper 6 bits of the channel; the lower 2 bits stay.

use dpx_core::quantize::{Dither, PaletteDepth, ReduceOptions, reduce};
use dpx_core::{IndexedImage, Rgb8, Rgba8, RgbaImage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HamKind {
    Ham6,
    Ham8,
}

impl HamKind {
    /// Palette registers the Set code can address.
    pub fn palette_len(self) -> usize {
        match self {
            HamKind::Ham6 => 16,
            HamKind::Ham8 => 64,
        }
    }

    pub fn planes(self) -> u8 {
        match self {
            HamKind::Ham6 => 6,
            HamKind::Ham8 => 8,
        }
    }

    fn data_bits(self) -> u32 {
        match self {
            HamKind::Ham6 => 4,
            HamKind::Ham8 => 6,
        }
    }

    fn depth(self) -> PaletteDepth {
        match self {
            HamKind::Ham6 => PaletteDepth::Rgb4,
            HamKind::Ham8 => PaletteDepth::Rgb8,
        }
    }

    /// New channel value after a modify with data `d`, given the previous channel value.
    fn modify(self, prev: u8, d: u8) -> u8 {
        match self {
            HamKind::Ham6 => d * 17,
            HamKind::Ham8 => (d << 2) | (prev & 3),
        }
    }
}

const SET: u8 = 0;
const BLUE: u8 = 1;
const RED: u8 = 2;
const GREEN: u8 = 3;

fn channel(c: Rgb8, ctrl: u8) -> u8 {
    match ctrl {
        BLUE => c.b,
        RED => c.r,
        _ => c.g,
    }
}

fn with_channel(c: Rgb8, ctrl: u8, v: u8) -> Rgb8 {
    match ctrl {
        BLUE => Rgb8::new(c.r, c.g, v),
        RED => Rgb8::new(v, c.g, c.b),
        _ => Rgb8::new(c.r, v, c.b),
    }
}

fn pal_get(palette: &[Rgb8], i: usize) -> Rgb8 {
    palette.get(i).copied().unwrap_or(Rgb8::BLACK)
}

/// Decodes HAM plane values to RGB. `palette` holds the Set registers (missing = black).
pub fn decode_ham(kind: HamKind, indices: &IndexedImage, palette: &[Rgb8]) -> RgbaImage {
    let w = indices.width() as usize;
    let bits = kind.data_bits();
    let mut out = Vec::with_capacity(indices.pixels().len());
    if w > 0 {
        for row in indices.pixels().chunks(w) {
            let mut cur = pal_get(palette, 0);
            for &v in row {
                let ctrl = (v >> bits) & 3;
                let d = v & ((1 << bits) - 1);
                cur = if ctrl == SET {
                    pal_get(palette, usize::from(d))
                } else {
                    with_channel(cur, ctrl, kind.modify(channel(cur, ctrl), d))
                };
                out.push(Rgba8::opaque(cur.r, cur.g, cur.b));
            }
        }
    }
    RgbaImage::from_pixels(indices.width(), indices.height(), out).expect("same size")
}

/// How well an encode matched the source (distances are Euclidean in 8-bit RGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HamReport {
    pub pixels: usize,
    pub mean_error: f64,
    pub max_error: f64,
    /// Pixels whose error exceeds the threshold given to the encoder.
    pub over_threshold: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HamEncoded {
    pub indices: IndexedImage,
    pub report: HamReport,
}

fn sq(a: Rgb8, b: Rgb8) -> u32 {
    let d = |x: u8, y: u8| {
        let t = i32::from(x) - i32::from(y);
        (t * t) as u32
    };
    d(a.r, b.r) + d(a.g, b.g) + d(a.b, b.b)
}

/// Best data value for one channel: the one whose resulting channel value is closest to
/// `target` (ties: smaller data value).
fn best_data(kind: HamKind, prev: u8, target: u8) -> u8 {
    let max = ((1u16 << kind.data_bits()) - 1) as u8;
    let guess = match kind {
        HamKind::Ham6 => (u16::from(target) + 8) / 17,
        HamKind::Ham8 => u16::from(target) >> 2,
    }
    .min(u16::from(max)) as u8;
    let mut best = (u32::MAX, 0u8);
    for d in guess.saturating_sub(1)..=(guess + 1).min(max) {
        let v = kind.modify(prev, d);
        let e = (i32::from(v) - i32::from(target)).unsigned_abs();
        if e < best.0 {
            best = (e, d);
        }
    }
    best.1
}

/// Greedy HAM encoder. `palette` supplies the Set registers (padded with black to
/// `kind.palette_len()`); alpha is ignored. Pixels whose error exceeds `threshold` are counted
/// in the report. Deterministic: ties prefer Set, then red, green, blue.
pub fn encode_ham(kind: HamKind, img: &RgbaImage, palette: &[Rgb8], threshold: f64) -> HamEncoded {
    let n = kind.palette_len();
    let pal: Vec<Rgb8> = (0..n).map(|i| pal_get(palette, i)).collect();
    let bits = kind.data_bits();
    let w = img.width() as usize;
    let mut out = Vec::with_capacity(img.pixels().len());
    let (mut sum, mut max, mut over) = (0.0f64, 0.0f64, 0usize);
    let t2 = threshold * threshold;
    if w > 0 {
        for row in img.pixels().chunks(w) {
            let mut cur = pal[0];
            for p in row {
                let target = Rgb8::new(p.r, p.g, p.b);
                let mut best_i = 0usize;
                let mut best_e = u32::MAX;
                for (i, &c) in pal.iter().enumerate() {
                    let e = sq(c, target);
                    if e < best_e {
                        best_e = e;
                        best_i = i;
                    }
                }
                let mut best_v = best_i as u8;
                let mut best_c = pal[best_i];
                // Modify candidates win only when strictly better, so ties keep Set.
                for ctrl in [RED, GREEN, BLUE] {
                    let d = best_data(kind, channel(cur, ctrl), channel(target, ctrl));
                    let c = with_channel(cur, ctrl, kind.modify(channel(cur, ctrl), d));
                    let e = sq(c, target);
                    if e < best_e {
                        best_e = e;
                        best_v = (ctrl << bits) | d;
                        best_c = c;
                    }
                }
                cur = best_c;
                out.push(best_v);
                let err = f64::from(best_e).sqrt();
                sum += err;
                max = max.max(err);
                if f64::from(best_e) > t2 {
                    over += 1;
                }
            }
        }
    }
    let count = out.len();
    HamEncoded {
        indices: IndexedImage::from_pixels(img.width(), img.height(), out).expect("same size"),
        report: HamReport {
            pixels: count,
            mean_error: if count == 0 { 0.0 } else { sum / count as f64 },
            max_error: max,
            over_threshold: over,
        },
    }
}

/// Picks the Set palette (16 or 64 colours, OCS precision for HAM6) with the median-cut
/// quantizer. Transparent pixels are treated as opaque. Always returns exactly
/// `kind.palette_len()` entries (padded with black).
pub fn choose_base_palette(kind: HamKind, img: &RgbaImage) -> Vec<Rgb8> {
    let opaque: Vec<Rgba8> = img
        .pixels()
        .iter()
        .map(|p| Rgba8::opaque(p.r, p.g, p.b))
        .collect();
    let opaque = RgbaImage::from_pixels(img.width(), img.height(), opaque).expect("same size");
    let n = kind.palette_len();
    let r = reduce(
        &opaque,
        &ReduceOptions {
            max_colors: n as u16,
            depth: kind.depth(),
            dither: Dither::None,
            transparent: Rgb8::BLACK,
        },
    );
    let mut pal: Vec<Rgb8> = r.palette.colors().to_vec();
    pal.resize(n, Rgb8::BLACK);
    pal
}

/// Convenience: choose a palette, then encode.
pub fn convert_to_ham(kind: HamKind, img: &RgbaImage, threshold: f64) -> (HamEncoded, Vec<Rgb8>) {
    let pal = choose_base_palette(kind, img);
    (encode_ham(kind, img, &pal, threshold), pal)
}

/// Color ▸ Recompute (ADR-39, manual AGA p.270): the HAM picture data `raw` worked out again
/// for the base palette `new_base`, so that it shows the colours it showed with `old_base` as
/// closely as the encoder can ([`encode_ham`]). With `old_base == new_base` it re-encodes the
/// picture as it looks, which reduces the fringing of data written by another program's
/// encoder. Not wired into the app yet: documents are never HAM (ADR-26 reduces HAM pictures
/// on load), so the menu item stays ghosted.
pub fn recompute(
    kind: HamKind,
    raw: &IndexedImage,
    old_base: &[Rgb8],
    new_base: &[Rgb8],
    threshold: f64,
) -> HamEncoded {
    encode_ham(kind, &decode_ham(kind, raw, old_base), new_base, threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recompute_keeps_the_look_under_a_new_base_palette() {
        let img = gradient(48, 8);
        let (encoded, old) = convert_to_ham(HamKind::Ham6, &img, 10.0);
        let shown = decode_ham(HamKind::Ham6, &encoded.indices, &old);
        // Same palette: the picture looks the same afterwards (the encoder is deterministic,
        // so recomputing its own data changes nothing).
        let same = recompute(HamKind::Ham6, &encoded.indices, &old, &old, 0.0);
        assert_eq!(same.indices, encoded.indices);
        // Another base palette: the plain decode of the old data with it differs, the
        // recomputed data shows the old colours again (HAM can hold any 12-bit colour after
        // at most three modify steps, so the error stays small).
        let mut new = old.clone();
        new.reverse();
        let stale = decode_ham(HamKind::Ham6, &encoded.indices, &new);
        assert_ne!(rgb(&stale), rgb(&shown));
        let fixed = recompute(HamKind::Ham6, &encoded.indices, &old, &new, 0.0);
        let back = decode_ham(HamKind::Ham6, &fixed.indices, &new);
        let err = |a: &RgbaImage| -> u64 {
            a.pixels()
                .iter()
                .zip(shown.pixels())
                .map(|(p, q)| u64::from(sq(Rgb8::new(p.r, p.g, p.b), Rgb8::new(q.r, q.g, q.b))))
                .sum()
        };
        assert!(
            err(&back) * 4 < err(&stale),
            "{} vs {}",
            err(&back),
            err(&stale)
        );
        assert_eq!(fixed.report.pixels, 48 * 8);
    }

    fn rgb(img: &RgbaImage) -> Vec<(u8, u8, u8)> {
        img.pixels().iter().map(|p| (p.r, p.g, p.b)).collect()
    }

    fn pal6() -> Vec<Rgb8> {
        let mut p = vec![Rgb8::BLACK; 16];
        p[0] = Rgb8::from_rgb4(0x123);
        p[1] = Rgb8::from_rgb4(0xF80);
        p
    }

    #[test]
    fn ham6_decode_set_modify_and_line_reset() {
        let pal = pal6();
        // ctrl<<4 | data. Row 0: set 1, mod blue F, mod red 3, mod green A.
        // Row 1 starts from palette[0] again.
        let idx = IndexedImage::from_pixels(
            4,
            2,
            vec![
                0x01,
                0x10 | 0xF,
                0x20 | 0x3,
                0x30 | 0xA,
                0x20 | 0xF,
                0x01,
                0x00,
                0x30,
            ],
        )
        .unwrap();
        let out = rgb(&decode_ham(HamKind::Ham6, &idx, &pal));
        assert_eq!(out[0], (0xFF, 0x88, 0x00));
        assert_eq!(out[1], (0xFF, 0x88, 0xFF));
        assert_eq!(out[2], (0x33, 0x88, 0xFF));
        assert_eq!(out[3], (0x33, 0xAA, 0xFF));
        // Line reset: red modify on top of palette[0] = (0x11,0x22,0x33).
        assert_eq!(out[4], (0xFF, 0x22, 0x33));
        assert_eq!(out[5], (0xFF, 0x88, 0x00));
        assert_eq!(out[6], (0x11, 0x22, 0x33));
        assert_eq!(out[7], (0x11, 0x00, 0x33));
    }

    #[test]
    fn ham8_decode_keeps_low_two_bits() {
        let mut pal = vec![Rgb8::BLACK; 64];
        pal[5] = Rgb8::new(0b1010_1011, 0b0101_0110, 0b1111_0001);
        let idx = IndexedImage::from_pixels(
            4,
            1,
            vec![
                5,
                (2 << 6) | 0b11_1111, // red: upper 6 bits set, low 2 stay 11
                3 << 6,               // green: upper 6 bits 0, low 2 stay 10
                (1 << 6) | 1,         // blue: upper 6 bits 000001, low 2 stay 01
            ],
        )
        .unwrap();
        let out = rgb(&decode_ham(HamKind::Ham8, &idx, &pal));
        assert_eq!(out[0], (0b1010_1011, 0b0101_0110, 0b1111_0001));
        assert_eq!(out[1], (0b1111_1111, 0b0101_0110, 0b1111_0001));
        assert_eq!(out[2], (0b1111_1111, 0b0000_0010, 0b1111_0001));
        assert_eq!(out[3], (0b1111_1111, 0b0000_0010, 0b0000_0101));
    }

    fn gradient(w: u32, h: u32) -> RgbaImage {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                v.push(Rgba8::opaque(
                    (x * 255 / (w - 1)) as u8,
                    (y * 255 / (h - 1)) as u8,
                    ((x + y) * 255 / (w + h - 2)) as u8,
                ));
            }
        }
        RgbaImage::from_pixels(w, h, v).unwrap()
    }

    #[test]
    fn roundtrip_of_ham_legal_image_has_zero_error() {
        for kind in [HamKind::Ham6, HamKind::Ham8] {
            let bits = kind.data_bits();
            let pal: Vec<Rgb8> = (0..kind.palette_len())
                .map(|i| Rgb8::new((i * 13) as u8, (i * 7) as u8, (i * 29) as u8))
                .collect();
            let mut s = 12345u32;
            let px: Vec<u8> = (0..64 * 8)
                .map(|_| {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let ctrl = ((s >> 24) & 3) as u8;
                    let d = ((s >> 8) & ((1 << bits) - 1)) as u8;
                    (ctrl << bits) | d
                })
                .collect();
            let idx = IndexedImage::from_pixels(64, 8, px).unwrap();
            let decoded = decode_ham(kind, &idx, &pal);
            let enc = encode_ham(kind, &decoded, &pal, 0.0);
            assert_eq!(enc.report.max_error, 0.0, "{kind:?}");
            assert_eq!(enc.report.over_threshold, 0);
            assert_eq!(decode_ham(kind, &enc.indices, &pal), decoded, "{kind:?}");
        }
    }

    #[test]
    fn gradient_encodes_with_low_error() {
        let img = gradient(320, 32);
        let (enc, pal) = convert_to_ham(HamKind::Ham6, &img, 24.0);
        assert!(enc.report.mean_error < 10.0, "{:?}", enc.report);
        let (enc8, _) = convert_to_ham(HamKind::Ham8, &img, 8.0);
        assert!(enc8.report.mean_error < 2.5, "{:?}", enc8.report);
        assert_eq!(pal.len(), 16);
    }

    #[test]
    fn encoding_is_deterministic_and_ties_prefer_set() {
        let img = gradient(40, 10);
        let (a, pa) = convert_to_ham(HamKind::Ham6, &img, 10.0);
        let (b, pb) = convert_to_ham(HamKind::Ham6, &img, 10.0);
        assert_eq!(a, b);
        assert_eq!(pa, pb);
        let pal = pal6();
        let c = pal[0];
        let one = RgbaImage::from_pixels(1, 1, vec![Rgba8::opaque(c.r, c.g, c.b)]).unwrap();
        assert_eq!(
            encode_ham(HamKind::Ham6, &one, &pal, 0.0).indices.pixels(),
            &[0]
        );
    }

    #[test]
    fn timing_320x256_manual() {
        // Run with `cargo test --release -p dpx-formats timing -- --nocapture` and read the numbers.
        let img = gradient(320, 256);
        let t = std::time::Instant::now();
        let (e6, _) = convert_to_ham(HamKind::Ham6, &img, 16.0);
        let d6 = t.elapsed();
        let t = std::time::Instant::now();
        let (e8, _) = convert_to_ham(HamKind::Ham8, &img, 16.0);
        println!(
            "ham6 {d6:?} ({:?}), ham8 {:?} ({:?})",
            e6.report,
            t.elapsed(),
            e8.report
        );
    }
}
