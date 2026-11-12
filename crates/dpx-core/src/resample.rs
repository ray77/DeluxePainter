//! Resampling true-colour pictures (ADR-9): area averaging for pictures too big for the
//! screen, and the page size that shows them undistorted with the mode's pixel shape.

use crate::rgba::{Rgba8, RgbaImage};

/// Per-output-index contributions along one axis: first source index and normalised weights.
struct Axis {
    start: Vec<usize>,
    offset: Vec<usize>,
    weights: Vec<f32>,
}

fn axis_weights(src: usize, dst: usize) -> Axis {
    let mut start = Vec::with_capacity(dst);
    let mut offset = Vec::with_capacity(dst + 1);
    let mut weights = Vec::new();
    let ratio = src as f64 / dst as f64;
    for o in 0..dst {
        let lo = o as f64 * ratio;
        let hi = ((o + 1) as f64 * ratio).min(src as f64);
        let first = (lo.floor() as usize).min(src - 1);
        let last = (hi.ceil() as usize).clamp(first + 1, src);
        start.push(first);
        offset.push(weights.len());
        let span = hi - lo;
        for i in first..last {
            let overlap = hi.min((i + 1) as f64) - lo.max(i as f64);
            weights.push((overlap.max(0.0) / span) as f32);
        }
    }
    offset.push(weights.len());
    Axis {
        start,
        offset,
        weights,
    }
}

/// Scales `img` to `width`×`height` by area averaging (box filter with fractional coverage):
/// every output pixel is the coverage-weighted mean of the source pixels under its footprint.
/// Colour is averaged premultiplied by alpha, so fully transparent pixels never tint their
/// neighbours; the result is un-premultiplied and rounded. Works for down- and up-scaling
/// (up-scaling repeats source pixels and blends only where an output pixel straddles two).
/// Panics if `width` or `height` is 0.
pub fn scale_area(img: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    assert!(width > 0 && height > 0, "target size must be non-zero");
    if img.width() == width && img.height() == height {
        return img.clone();
    }
    let (sw, sh) = (img.width() as usize, img.height() as usize);
    let (dw, dh) = (width as usize, height as usize);
    if sw == 0 || sh == 0 {
        return RgbaImage::new(width, height, Rgba8::new(0, 0, 0, 0));
    }
    let ax = axis_weights(sw, dw);
    let ay = axis_weights(sh, dh);
    let src = img.pixels();

    // Horizontal pass: sh rows × dw columns of premultiplied [r, g, b, a].
    let mut mid = vec![0.0f32; sh * dw * 4];
    for y in 0..sh {
        let row = &src[y * sw..(y + 1) * sw];
        let out = &mut mid[y * dw * 4..(y + 1) * dw * 4];
        for x in 0..dw {
            let (w0, w1) = (ax.offset[x], ax.offset[x + 1]);
            let first = ax.start[x];
            let mut acc = [0.0f32; 4];
            for (k, &w) in ax.weights[w0..w1].iter().enumerate() {
                let p = row[first + k];
                let a = f32::from(p.a);
                let pa = a / 255.0;
                acc[0] += w * f32::from(p.r) * pa;
                acc[1] += w * f32::from(p.g) * pa;
                acc[2] += w * f32::from(p.b) * pa;
                acc[3] += w * a;
            }
            out[x * 4..x * 4 + 4].copy_from_slice(&acc);
        }
    }

    // Vertical pass and un-premultiply.
    let mut pixels = Vec::with_capacity(dw * dh);
    let mut acc = vec![0.0f32; dw * 4];
    for y in 0..dh {
        acc.iter_mut().for_each(|v| *v = 0.0);
        let (w0, w1) = (ay.offset[y], ay.offset[y + 1]);
        let first = ay.start[y];
        for (k, &w) in ay.weights[w0..w1].iter().enumerate() {
            let row = &mid[(first + k) * dw * 4..(first + k + 1) * dw * 4];
            for (a, &m) in acc.iter_mut().zip(row) {
                *a += w * m;
            }
        }
        for x in 0..dw {
            let a = acc[x * 4 + 3];
            if a <= 1e-4 {
                pixels.push(Rgba8::new(0, 0, 0, 0));
                continue;
            }
            let k = 255.0 / a;
            let q = |v: f32| (v * k + 0.5).clamp(0.0, 255.0) as u8;
            pixels.push(Rgba8::new(
                q(acc[x * 4]),
                q(acc[x * 4 + 1]),
                q(acc[x * 4 + 2]),
                (a + 0.5).clamp(0.0, 255.0) as u8,
            ));
        }
    }
    RgbaImage::from_pixels(width, height, pixels).expect("pixel count matches size")
}

/// Scales `img` to `width`×`height` by taking, for every output pixel, the source pixel under
/// its centre (`floor((o + ½) · src / dst)` on each axis). No colour is new: enlarged pixel
/// art stays crisp (whole factors repeat every pixel equally). Panics if `width` or `height`
/// is 0.
pub fn scale_nearest(img: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    assert!(width > 0 && height > 0, "target size must be non-zero");
    if img.width() == width && img.height() == height {
        return img.clone();
    }
    let (sw, sh) = (img.width() as u64, img.height() as u64);
    if sw == 0 || sh == 0 {
        return RgbaImage::new(width, height, Rgba8::new(0, 0, 0, 0));
    }
    let pick =
        |o: u32, src: u64, dst: u32| ((2 * u64::from(o) + 1) * src / (2 * u64::from(dst))) as usize;
    let xs: Vec<usize> = (0..width).map(|x| pick(x, sw, width)).collect();
    let src = img.pixels();
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        let row = pick(y, sh, height) * sw as usize;
        pixels.extend(xs.iter().map(|&x| src[row + x]));
    }
    RgbaImage::from_pixels(width, height, pixels).expect("pixel count matches size")
}

/// Scales a picture for the Import requester (ADR-42): along an axis that grows (or keeps
/// its size) the nearest source pixel is taken ([`scale_nearest`]), so pixel art stays crisp
/// and no colours are mixed; along an axis that shrinks, areas are averaged
/// ([`scale_area`]). Panics if `width` or `height` is 0.
pub fn scale_fit(img: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let grow_x = width >= img.width();
    let grow_y = height >= img.height();
    match (grow_x, grow_y) {
        (true, true) => scale_nearest(img, width, height),
        (false, false) => scale_area(img, width, height),
        (true, false) => scale_area(&scale_nearest(img, width, img.height()), width, height),
        (false, true) => scale_area(&scale_nearest(img, img.width(), height), width, height),
    }
}

/// Largest page that shows a `src_w`×`src_h` picture of square pixels undistorted on a screen
/// of `max_w`×`max_h` pixels whose pixels have the display shape `pixel_shape` = (x, y)
/// (an Amiga pixel is shown `x` units wide and `y` units high, e.g. (1, 2) for HiRes
/// non-laced). Never larger than `src` scaled to fit and never larger than `max`; each side at
/// least 1. Rounds to the nearest pixel.
///
/// Note: this may also enlarge a small picture. ADR-9's load path calls it only for pictures
/// too big for the screen; the Import requester (ADR-42) fits every picture with it.
pub fn fit_size(
    src_w: u32,
    src_h: u32,
    max_w: u32,
    max_h: u32,
    pixel_shape: (u32, u32),
) -> (u32, u32) {
    let (px, py) = (
        f64::from(pixel_shape.0.max(1)),
        f64::from(pixel_shape.1.max(1)),
    );
    let (sw, sh) = (f64::from(src_w.max(1)), f64::from(src_h.max(1)));
    let s = (f64::from(max_w) * px / sw).min(f64::from(max_h) * py / sh);
    let w = ((sw * s / px).round() as u32).clamp(1, max_w.max(1));
    let h = ((sh * s / py).round() as u32).clamp(1, max_h.max(1));
    (w, h)
}

/// The display proportions of a `page` shown with `pixel_shape` (ADR-45): `(w · x, h · y)`,
/// reduced. A 320×256 LoRes page gives (5, 4), a 640×256 HiRes page (pixels 1×2) too.
pub fn display_aspect(page: (u32, u32), pixel_shape: (u32, u32)) -> (u64, u64) {
    let w = u64::from(page.0.max(1)) * u64::from(pixel_shape.0.max(1));
    let h = u64::from(page.1.max(1)) * u64::from(pixel_shape.1.max(1));
    let g = gcd64(w, h);
    (w / g, h / g)
}

fn gcd64(a: u64, b: u64) -> u64 {
    if b == 0 { a.max(1) } else { gcd64(b, a % b) }
}

/// The centred part of a `src_w`×`src_h` picture (square pixels) with the display
/// proportions `aspect` = (width, height), as large as the picture allows (crop-to-fill,
/// ADR-45): a wider picture loses columns left and right, a taller one rows above and
/// below; odd leftovers put the extra pixel right and below. Returns (x, y, width, height),
/// each side at least 1.
pub fn crop_to_aspect(src_w: u32, src_h: u32, aspect: (u64, u64)) -> (u32, u32, u32, u32) {
    let (sw, sh) = (u64::from(src_w.max(1)), u64::from(src_h.max(1)));
    let (aw, ah) = (aspect.0.max(1), aspect.1.max(1));
    let (w, h) = if sw * ah > sh * aw {
        // Wider than the target: keep the height.
        (((sh * aw * 2 + ah) / (2 * ah)).clamp(1, sw), sh)
    } else {
        (sw, ((sw * ah * 2 + aw) / (2 * aw)).clamp(1, sh))
    };
    let (w, h) = (w as u32, h as u32);
    ((src_w.max(1) - w) / 2, (src_h.max(1) - h) / 2, w, h)
}

/// Scales `img` to `width`×`height` for pixel art (ADR-45): every output pixel takes the
/// colour most of the source pixels under its cell have. A cell is the source pixels from
/// `floor(o · src / dst)` up to `ceil((o + 1) · src / dst)` on each axis (at least one).
/// Colours are counted in buckets of 8 levels per channel (5 bits), so the slight noise of
/// generated "pixel art" does not split one colour into many; the winning bucket gives the
/// mean of its pixels. Pixels with less alpha than [`crate::quantize::ALPHA_THRESHOLD`]
/// count as one "transparent" bucket, which gives a fully transparent pixel. Ties go to the
/// bucket met first in reading order. Enlarging takes the source pixel under each cell's
/// top-left corner. Panics if `width` or `height` is 0.
pub fn scale_majority(img: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    assert!(width > 0 && height > 0, "target size must be non-zero");
    if img.width() == width && img.height() == height {
        return img.clone();
    }
    let (sw, sh) = (u64::from(img.width()), u64::from(img.height()));
    if sw == 0 || sh == 0 {
        return RgbaImage::new(width, height, Rgba8::new(0, 0, 0, 0));
    }
    let span = |o: u32, src: u64, dst: u32| {
        let dst = u64::from(dst);
        let lo = u64::from(o) * src / dst;
        let hi = ((u64::from(o) + 1) * src).div_ceil(dst).clamp(lo + 1, src);
        (lo as usize, hi as usize)
    };
    const CLEAR: u32 = u32::MAX;
    let key = |p: Rgba8| {
        if p.a < crate::quantize::ALPHA_THRESHOLD {
            CLEAR
        } else {
            (u32::from(p.r >> 3) << 10) | (u32::from(p.g >> 3) << 5) | u32::from(p.b >> 3)
        }
    };
    let xs: Vec<(usize, usize)> = (0..width).map(|x| span(x, sw, width)).collect();
    let src = img.pixels();
    let stride = sw as usize;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    let mut keys: Vec<u32> = Vec::new();
    let mut sorted: Vec<u32> = Vec::new();
    // (bucket, pixels) in bucket order.
    let mut runs: Vec<(u32, usize)> = Vec::new();
    for y in 0..height {
        let (y0, y1) = span(y, sh, height);
        for &(x0, x1) in &xs {
            keys.clear();
            for row in y0..y1 {
                keys.extend(
                    src[row * stride + x0..row * stride + x1]
                        .iter()
                        .map(|&p| key(p)),
                );
            }
            sorted.clear();
            sorted.extend_from_slice(&keys);
            sorted.sort_unstable();
            runs.clear();
            for &k in &sorted {
                match runs.last_mut() {
                    Some((last, n)) if *last == k => *n += 1,
                    _ => runs.push((k, 1)),
                }
            }
            let most = runs.iter().map(|&(_, n)| n).max().unwrap_or(0);
            // The first bucket in reading order with the most pixels.
            let winner = if runs.len() == 1 {
                runs[0].0
            } else {
                keys.iter()
                    .copied()
                    .find(|&k| {
                        let i = runs.partition_point(|&(r, _)| r < k);
                        runs[i].1 == most
                    })
                    .unwrap_or(CLEAR)
            };
            if winner == CLEAR {
                pixels.push(Rgba8::new(0, 0, 0, 0));
                continue;
            }
            let mut acc = [0u64; 4];
            let mut n = 0u64;
            for row in y0..y1 {
                for &p in &src[row * stride + x0..row * stride + x1] {
                    if key(p) == winner {
                        acc[0] += u64::from(p.r);
                        acc[1] += u64::from(p.g);
                        acc[2] += u64::from(p.b);
                        acc[3] += u64::from(p.a);
                        n += 1;
                    }
                }
            }
            let mean = |v: u64| ((v + n / 2) / n) as u8;
            pixels.push(Rgba8::new(
                mean(acc[0]),
                mean(acc[1]),
                mean(acc[2]),
                mean(acc[3]),
            ));
        }
    }
    RgbaImage::from_pixels(width, height, pixels).expect("pixel count matches size")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(v: u8) -> Rgba8 {
        Rgba8::opaque(v, v, v)
    }

    #[test]
    fn constant_stays_constant() {
        let c = Rgba8::new(201, 37, 99, 255);
        for &(sw, sh, dw, dh) in &[(7, 5, 3, 2), (3, 2, 7, 5), (10, 10, 4, 6), (1, 1, 9, 9)] {
            let img = RgbaImage::new(sw, sh, c);
            let out = scale_area(&img, dw, dh);
            assert!(out.pixels().iter().all(|&p| p == c), "{sw}x{sh}->{dw}x{dh}");
        }
        let t = RgbaImage::new(7, 7, Rgba8::new(10, 20, 30, 77));
        let out = scale_area(&t, 3, 3);
        assert!(
            out.pixels()
                .iter()
                .all(|&p| p == Rgba8::new(10, 20, 30, 77))
        );
    }

    #[test]
    fn four_by_four_to_two_by_two() {
        let vals: Vec<u8> = vec![
            1, 2, 10, 20, //
            4, 6, 30, 41, //
            100, 101, 200, 201, //
            102, 104, 203, 205,
        ];
        let img = RgbaImage::from_pixels(4, 4, vals.iter().map(|&v| grey(v)).collect()).unwrap();
        let out = scale_area(&img, 2, 2);
        let expect = |a: u32, b: u32, c: u32, d: u32| ((a + b + c + d) as f64 / 4.0).round() as u8;
        assert_eq!(out.get(0, 0).unwrap(), grey(expect(1, 2, 4, 6)));
        assert_eq!(out.get(1, 0).unwrap(), grey(expect(10, 20, 30, 41)));
        assert_eq!(out.get(0, 1).unwrap(), grey(expect(100, 101, 102, 104)));
        assert_eq!(out.get(1, 1).unwrap(), grey(expect(200, 201, 203, 205)));
    }

    #[test]
    fn fractional_weights() {
        let img = RgbaImage::from_pixels(3, 1, vec![grey(0), grey(90), grey(180)]).unwrap();
        let out = scale_area(&img, 2, 1);
        assert_eq!(out.get(0, 0).unwrap(), grey(30));
        assert_eq!(out.get(1, 0).unwrap(), grey(150));
    }

    #[test]
    fn premultiplied_alpha() {
        let img = RgbaImage::from_pixels(
            2,
            1,
            vec![Rgba8::opaque(255, 255, 255), Rgba8::new(255, 0, 0, 0)],
        )
        .unwrap();
        let p = scale_area(&img, 1, 1).get(0, 0).unwrap();
        assert_eq!((p.r, p.g, p.b), (255, 255, 255));
        assert!(p.a == 127 || p.a == 128, "alpha {}", p.a);
    }

    #[test]
    fn fully_transparent_is_zero() {
        let img = RgbaImage::new(4, 4, Rgba8::new(9, 9, 9, 0));
        let out = scale_area(&img, 2, 3);
        assert!(out.pixels().iter().all(|&p| p == Rgba8::new(0, 0, 0, 0)));
    }

    #[test]
    fn identity() {
        let px: Vec<Rgba8> = (0..12)
            .map(|i| Rgba8::new(i * 20, i, 255 - i, 200))
            .collect();
        let img = RgbaImage::from_pixels(4, 3, px).unwrap();
        assert_eq!(scale_area(&img, 4, 3), img);
    }

    #[test]
    fn gradient_stays_monotonic() {
        let (w, h) = (400u32, 300u32);
        let mut px = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                px.push(grey((x * 255 / (w - 1)) as u8));
            }
        }
        let img = RgbaImage::from_pixels(w, h, px).unwrap();
        let out = scale_area(&img, 37, 23);
        for y in 0..23 {
            for x in 1..37 {
                assert!(out.get(x, y).unwrap().r >= out.get(x - 1, y).unwrap().r);
            }
        }
        assert!(out.get(36, 0).unwrap().r > out.get(0, 0).unwrap().r + 200);
    }

    #[test]
    fn nearest_repeats_pixels_in_whole_blocks_and_mixes_nothing() {
        let px: Vec<Rgba8> = (0..6).map(|i| grey(i * 40)).collect();
        let img = RgbaImage::from_pixels(3, 2, px).unwrap();
        let big = scale_nearest(&img, 12, 6);
        for y in 0..6 {
            for x in 0..12 {
                assert_eq!(big.get(x, y), img.get(x / 4, y / 3), "{x},{y}");
            }
        }
        // Uneven factors take only source colours.
        let odd = scale_nearest(&img, 7, 5);
        assert!(odd.pixels().iter().all(|p| img.pixels().contains(p)));
        assert_eq!(scale_nearest(&img, 3, 2), img);
    }

    #[test]
    fn scale_fit_enlarges_by_nearest_and_reduces_by_area_per_axis() {
        let img =
            RgbaImage::from_pixels(2, 2, vec![grey(0), grey(100), grey(200), grey(50)]).unwrap();
        // Both axes grow: crisp.
        let up = scale_fit(&img, 6, 6);
        assert!(up.pixels().iter().all(|p| img.pixels().contains(p)));
        // x grows, y shrinks: columns repeated, rows averaged.
        let mixed = scale_fit(&img, 4, 1);
        assert_eq!(mixed.get(0, 0), Some(grey(100)));
        assert_eq!(mixed.get(1, 0), Some(grey(100)));
        assert_eq!(mixed.get(2, 0), Some(grey(75)));
        assert_eq!(mixed.get(3, 0), Some(grey(75)));
        // Both shrink: area averaging.
        assert_eq!(scale_fit(&img, 1, 1), scale_area(&img, 1, 1));
    }

    #[test]
    fn fit_size_examples() {
        assert_eq!(fit_size(4032, 3024, 320, 256, (1, 1)), (320, 240));
        assert_eq!(fit_size(1920, 1080, 320, 256, (1, 1)), (320, 180));
        assert_eq!(fit_size(1920, 1080, 640, 256, (1, 2)), (640, 180));
        assert_eq!(fit_size(1000, 3000, 320, 256, (1, 1)), (85, 256));
        assert_eq!(fit_size(4032, 3024, 640, 512, (1, 1)), (640, 480));
        assert_eq!(fit_size(5000, 10, 320, 256, (1, 1)), (320, 1));
        // Laced HiRes (square pixels) and LoRes laced (2,1).
        assert_eq!(fit_size(1920, 1080, 640, 512, (1, 1)), (640, 360));
        assert_eq!(fit_size(1920, 1080, 320, 512, (2, 1)), (320, 360));
    }

    #[test]
    fn display_aspect_follows_the_pixel_shape() {
        assert_eq!(display_aspect((320, 240), (1, 1)), (4, 3));
        assert_eq!(display_aspect((320, 256), (1, 1)), (5, 4));
        assert_eq!(display_aspect((640, 256), (1, 2)), (5, 4));
        assert_eq!(display_aspect((320, 512), (2, 1)), (5, 4));
        assert_eq!(display_aspect((640, 200), (1, 2)), (8, 5));
        assert_eq!(display_aspect((1280, 256), (1, 4)), (5, 4));
    }

    #[test]
    fn crop_to_aspect_keeps_the_centre() {
        // Exact proportions: nothing is cut.
        assert_eq!(crop_to_aspect(1024, 768, (4, 3)), (0, 0, 1024, 768));
        // Square to 5:4 loses rows above and below (819.2 rounds to 819; the odd row
        // goes below).
        assert_eq!(crop_to_aspect(1024, 1024, (5, 4)), (0, 102, 1024, 819));
        // Square to 3:4 loses columns.
        assert_eq!(crop_to_aspect(1024, 1024, (3, 4)), (128, 0, 768, 1024));
        // 1040x832 is exactly 5:4.
        assert_eq!(crop_to_aspect(1040, 832, (5, 4)), (0, 0, 1040, 832));
        // Extreme proportions keep at least one pixel.
        assert_eq!(crop_to_aspect(10, 10, (1000, 1)), (0, 4, 10, 1));
    }

    #[test]
    fn majority_takes_the_most_common_colour_of_each_cell() {
        // A 2x2 picture blown up 3x with noise: every 3x3 cell is one colour except one
        // pixel, which must not win.
        let base = [
            grey(0),
            grey(200),
            Rgba8::opaque(200, 0, 0),
            Rgba8::opaque(0, 0, 200),
        ];
        let mut img = RgbaImage::new(6, 6, grey(0));
        for y in 0..6 {
            for x in 0..6 {
                img.set(x, y, base[(y / 3 * 2 + x / 3) as usize]);
            }
        }
        // Odd pixels and slight noise inside the bucket (8 levels).
        img.set(1, 1, Rgba8::opaque(255, 255, 0));
        img.set(4, 0, grey(203));
        let out = scale_majority(&img, 2, 2);
        assert_eq!(out.get(0, 0), Some(grey(0)));
        // The mean of 8 x 200 and one 203 rounds to 200.
        assert_eq!(out.get(1, 0), Some(grey(200)));
        assert_eq!(out.get(0, 1), Some(base[2]));
        assert_eq!(out.get(1, 1), Some(base[3]));
        // Area averaging mixes the odd pixel in.
        assert_ne!(scale_area(&img, 2, 2).get(0, 0), Some(grey(0)));
    }

    #[test]
    fn majority_ties_transparency_and_enlarging() {
        // A 2x1 cell with two colours: the first in reading order wins.
        let img = RgbaImage::from_pixels(2, 1, vec![grey(10), grey(250)]).unwrap();
        assert_eq!(scale_majority(&img, 1, 1).get(0, 0), Some(grey(10)));
        // Mostly transparent cells become fully transparent.
        let clear = Rgba8::new(9, 9, 9, 0);
        let img =
            RgbaImage::from_pixels(3, 1, vec![clear, Rgba8::new(1, 2, 3, 100), grey(7)]).unwrap();
        assert_eq!(
            scale_majority(&img, 1, 1).get(0, 0),
            Some(Rgba8::new(0, 0, 0, 0))
        );
        // Enlarging repeats pixels, like nearest.
        let img = RgbaImage::from_pixels(2, 1, vec![grey(10), grey(250)]).unwrap();
        let big = scale_majority(&img, 4, 2);
        assert_eq!(
            big.pixels(),
            &[grey(10), grey(10), grey(250), grey(250)].repeat(2)[..]
        );
        // Exact size, also for awkward ratios.
        let big = RgbaImage::new(1024, 768, grey(5));
        let small = scale_majority(&big, 320, 240);
        assert_eq!((small.width(), small.height()), (320, 240));
        assert!(small.pixels().iter().all(|&p| p == grey(5)));
        let odd = scale_majority(&RgbaImage::new(1000, 999, grey(5)), 321, 239);
        assert_eq!((odd.width(), odd.height()), (321, 239));
    }
}
