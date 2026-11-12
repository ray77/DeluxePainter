//! The picture sizes a provider makes and the one to ask for (ADR-45).
//!
//! Providers do not make 320×240 pictures: the request asks for the provider's size with the
//! page's display proportions (page size × pixel shape), as exactly as the provider allows,
//! and `dpx_core::import::convert` brings the result to the exact page afterwards.

/// The sizes a provider makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SizeRule {
    /// Only these sizes (width, height).
    Fixed(Vec<(u32, u32)>),
    /// Any size within limits.
    Custom(CustomSizes),
}

/// Limits of a provider that makes any size within them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CustomSizes {
    /// Width and height are multiples of this.
    pub multiple: u32,
    /// The long side is at most this many times the short one.
    pub max_ratio: u32,
    pub min_pixels: u64,
    pub max_pixels: u64,
    /// Longest side.
    pub max_edge: u32,
    /// The long side to aim for (larger only when the pixel minimum needs it).
    pub long_edge: u32,
}

impl CustomSizes {
    /// OpenAI's GPT image models (image generation guide, checked 2026-11-12): multiples of
    /// 16, at most 3:1, 655,360 to 8,294,400 pixels, sides up to 3840. The long side aims at
    /// 1024, the side of the standard sizes; anything above 2560×1440 is "experimental".
    pub const OPENAI: CustomSizes = CustomSizes {
        multiple: 16,
        max_ratio: 3,
        min_pixels: 655_360,
        max_pixels: 8_294_400,
        max_edge: 3840,
        long_edge: 1024,
    };

    fn allows(&self, (w, h): (u32, u32)) -> bool {
        let m = self.multiple.max(1);
        let (long, short) = (w.max(h), w.min(h));
        let pixels = u64::from(w) * u64::from(h);
        short > 0
            && w.is_multiple_of(m)
            && h.is_multiple_of(m)
            && long <= self.max_edge
            && u64::from(long) <= u64::from(short) * u64::from(self.max_ratio)
            && (self.min_pixels..=self.max_pixels).contains(&pixels)
    }

    /// The short side for a long side of `long` at proportions `long_a`:`short_a`: the
    /// nearest multiple, but within the ratio limit.
    fn short_side(&self, long: u32, long_a: u64, short_a: u64) -> u32 {
        let m = u64::from(self.multiple.max(1));
        let long = u64::from(long);
        let nearest = (long * short_a * 2 + long_a * m) / (2 * long_a * m) * m;
        let ratio = u64::from(self.max_ratio.max(1));
        let least = long.div_ceil(ratio).div_ceil(m) * m;
        nearest.clamp(least.max(m), long) as u32
    }

    fn choose(&self, aspect: (u64, u64)) -> (u32, u32) {
        let m = self.multiple.max(1);
        let ratio = u64::from(self.max_ratio.max(1));
        let (mut aw, mut ah) = (aspect.0.max(1), aspect.1.max(1));
        // Beyond the ratio limit the conversion crops anyway.
        if aw > ah * ratio {
            aw = ah * ratio;
        } else if ah > aw * ratio {
            ah = aw * ratio;
        }
        let landscape = aw >= ah;
        let (long_a, short_a) = if landscape { (aw, ah) } else { (ah, aw) };
        let size = |long: u32| (long, self.short_side(long, long_a, short_a));
        let pixels = |(l, s): (u32, u32)| u64::from(l) * u64::from(s);
        let max_long = self.max_edge / m * m;
        let mut long = (self.long_edge.div_ceil(m) * m).clamp(m, max_long);
        while pixels(size(long)) < self.min_pixels && long + m <= max_long {
            long += m;
        }
        while pixels(size(long)) > self.max_pixels && long > m {
            long -= m;
        }
        // Exact proportions a little larger are worth it: nothing to crop then.
        let exact = (long..=(long + long / 4).min(max_long))
            .step_by(m as usize)
            .map(size)
            .find(|&(l, s)| u64::from(l) * short_a == u64::from(s) * long_a && self.allows((l, s)));
        let (l, s) = exact.unwrap_or_else(|| size(long));
        if landscape { (l, s) } else { (s, l) }
    }
}

impl SizeRule {
    /// Whether the provider makes a `size` picture.
    pub fn allows(&self, size: (u32, u32)) -> bool {
        match self {
            SizeRule::Fixed(sizes) => sizes.contains(&size),
            SizeRule::Custom(c) => c.allows(size),
        }
    }

    /// The size to ask for a picture of the display proportions `aspect` (width, height):
    /// - Fixed: the size with the nearest proportions (the first of equals).
    /// - Custom: the long side at [`CustomSizes::long_edge`] (larger if the pixel minimum
    ///   needs it), the short side the nearest multiple; if a long side up to a quarter
    ///   larger gives the proportions exactly, that one. Proportions beyond the ratio limit
    ///   are taken at the limit. 4:3 gives 1024×768, 16:10 1024×640, 5:4 1040×832 for
    ///   OpenAI.
    pub fn choose(&self, aspect: (u64, u64)) -> (u32, u32) {
        match self {
            SizeRule::Fixed(sizes) => {
                let want = (aspect.0.max(1) as f64 / aspect.1.max(1) as f64).ln();
                sizes
                    .iter()
                    .copied()
                    .min_by(|a, b| {
                        let d = |s: (u32, u32)| {
                            ((f64::from(s.0.max(1)) / f64::from(s.1.max(1))).ln() - want).abs()
                        };
                        d(*a).total_cmp(&d(*b))
                    })
                    .unwrap_or((1024, 1024))
            }
            SizeRule::Custom(c) => c.choose(aspect),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::resample::display_aspect;

    const O: SizeRule = SizeRule::Custom(CustomSizes::OPENAI);

    #[test]
    fn openai_sizes_for_amiga_pages() {
        let cases = [
            // page, pixel shape, size
            ((320, 240), (1, 1), (1024, 768)),
            ((320, 256), (1, 1), (1040, 832)),
            ((320, 200), (1, 1), (1024, 640)),
            ((640, 256), (1, 2), (1040, 832)),
            ((640, 200), (1, 2), (1024, 640)),
            ((640, 512), (1, 1), (1040, 832)),
            ((640, 400), (1, 1), (1024, 640)),
            ((320, 512), (2, 1), (1040, 832)),
            ((1280, 256), (1, 4), (1040, 832)),
            ((368, 283), (1, 1), (1024, 784)),
            ((256, 256), (1, 1), (1024, 1024)),
            ((240, 320), (1, 1), (768, 1024)),
        ];
        for (page, shape, want) in cases {
            let got = O.choose(display_aspect(page, shape));
            assert_eq!(got, want, "{page:?} {shape:?}");
            assert!(O.allows(got), "{got:?}");
        }
    }

    #[test]
    fn extreme_proportions_stay_within_the_limits() {
        for aspect in [(3, 1), (10, 1), (1, 3), (1, 10), (1000, 999), (7, 2)] {
            let (w, h) = O.choose(aspect);
            assert!(O.allows((w, h)), "{aspect:?} -> {w}x{h}");
        }
        // 3:1 exactly: 1440x480 (the smallest 3:1 above the pixel minimum).
        assert_eq!(O.choose((3, 1)), (1440, 480));
        assert_eq!(O.choose((1, 10)), (480, 1440));
    }

    #[test]
    fn allows_checks_every_openai_limit() {
        assert!(O.allows((1024, 1024)));
        assert!(O.allows((1536, 1024)));
        assert!(O.allows((3840, 2160)));
        // Not a multiple of 16, below the pixel minimum, above 3:1, edge, pixels.
        assert!(!O.allows((1000, 1000)));
        assert!(!O.allows((512, 512)));
        assert!(!O.allows((2048, 672)));
        assert!(!O.allows((4096, 1600)));
        assert!(!O.allows((3840, 3840)));
        assert!(!O.allows((0, 1024)));
    }

    #[test]
    fn fixed_sizes_take_the_nearest_proportions() {
        let f = SizeRule::Fixed(vec![(1024, 1024), (1536, 1024), (1024, 1536)]);
        assert_eq!(f.choose((4, 3)), (1536, 1024));
        // ln 1.25 is nearer ln 1.5 than ln 1.
        assert_eq!(f.choose((5, 4)), (1536, 1024));
        assert_eq!(f.choose((9, 10)), (1024, 1024));
        assert_eq!(f.choose((3, 4)), (1024, 1536));
        assert!(f.allows((1536, 1024)));
        assert!(!f.allows((1024, 768)));
    }
}
