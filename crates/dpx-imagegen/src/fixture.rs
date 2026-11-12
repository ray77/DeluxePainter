//! A deterministic test pattern behind [`ImageGenerationProvider`] (ADR-45), **for tests and
//! a clearly marked debug snapshot flag only**.
//!
//! It is no image generator: it draws the same landscape-like pattern (sky, sun, hills,
//! ground, varied by the prompt's hash and the variant) for every prompt, locally and at
//! once. It is never part of [`crate::user_providers`], so the GUI and MCP never offer it
//! as a way to "generate" pictures.

use dpx_core::{Rgba8, RgbaImage};

use crate::provider::{
    CancelToken, Capabilities, GenError, GenerationRequest, ImageGenerationProvider, ProviderStatus,
};
use crate::size::{CustomSizes, SizeRule};

/// The test pattern "provider".
#[derive(Clone, Copy, Debug, Default)]
pub struct TestPattern;

/// Its id; never a provider a user can pick.
pub const TEST_PATTERN_ID: &str = "test-pattern";

impl ImageGenerationProvider for TestPattern {
    fn id(&self) -> &str {
        TEST_PATTERN_ID
    }

    fn display_name(&self) -> &str {
        "Test pattern (not AI, debug only)"
    }

    fn needs_network(&self) -> bool {
        false
    }

    fn cost_note(&self) -> &str {
        "Draws a fixed test pattern on this computer: no AI, no network, no cost. For tests \
         and debug snapshots only."
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            sizes: SizeRule::Custom(CustomSizes {
                multiple: 1,
                max_ratio: 8,
                min_pixels: 1,
                max_pixels: 16 * 1024 * 1024,
                max_edge: 4096,
                long_edge: 1024,
            }),
            transparent_background: true,
            max_variants: 4,
            seed: true,
            max_prompt_chars: 32_000,
        }
    }

    fn status(&self) -> ProviderStatus {
        ProviderStatus::Ready
    }

    fn generate(
        &self,
        request: &GenerationRequest,
        cancel: &CancelToken,
    ) -> Result<Vec<RgbaImage>, GenError> {
        self.capabilities().check(request)?;
        let base = fnv1a(request.prompt.as_bytes()) ^ request.seed.unwrap_or(0);
        (0..request.variants)
            .map(|v| {
                cancel.check()?;
                Ok(pattern(
                    request.size,
                    base.wrapping_add(u64::from(v).wrapping_mul(0x9e37_79b9_7f4a_7c15)),
                    request.transparent_background,
                ))
            })
            .collect()
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The pattern for `seed` at `size`: a sky gradient (transparent with `clear_sky`), a sun,
/// two rows of hills and the ground.
pub fn pattern((w, h): (u32, u32), seed: u64, clear_sky: bool) -> RgbaImage {
    let mut img = RgbaImage::new(w, h, Rgba8::default());
    let (wf, hf) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let phase = (seed % 628) as f64 / 100.0;
    let sun = (
        wf * (0.2 + (seed >> 10) as f64 % 60.0 / 100.0),
        hf * 0.25,
        hf.min(wf) * 0.1,
    );
    let hue = (seed >> 20) % 3;
    for y in 0..h {
        for x in 0..w {
            let (xf, yf) = (f64::from(x) / wf, f64::from(y) / hf);
            let far = 0.55 + 0.08 * (xf * 9.0 + phase).sin();
            let near = 0.7 + 0.06 * (xf * 5.0 - phase * 0.7).cos();
            let (dx, dy) = (f64::from(x) - sun.0, f64::from(y) - sun.1);
            let p = if yf > 0.88 {
                Rgba8::opaque(90, 60, 30)
            } else if yf > near {
                Rgba8::opaque(30, (110.0 + 60.0 * yf) as u8, 40)
            } else if yf > far {
                Rgba8::opaque(70, 90, 140)
            } else if dx * dx + dy * dy < sun.2 * sun.2 {
                Rgba8::opaque(255, 220, 60)
            } else if clear_sky {
                Rgba8::new(0, 0, 0, 0)
            } else {
                let t = (yf / far * 255.0) as u8;
                match hue {
                    0 => Rgba8::opaque(t / 2, 80 + t / 3, 255 - t / 4),
                    1 => Rgba8::opaque(255 - t / 3, 120 + t / 4, 90 + t / 2),
                    _ => Rgba8::opaque(40 + t / 2, 40 + t / 3, 120 + t / 2),
                }
            };
            img.set(x, y, p);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pattern_is_deterministic_and_follows_the_request() {
        let p = TestPattern;
        let mut r = GenerationRequest::new("castle", (64, 48));
        r.variants = 2;
        let a = p.generate(&r, &CancelToken::new()).unwrap();
        let b = p.generate(&r, &CancelToken::new()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        assert_ne!(a[0], a[1]);
        assert_eq!((a[0].width(), a[0].height()), (64, 48));
        assert!(a[0].pixels().iter().all(|p| p.a == 255));
        r.transparent_background = true;
        let clear = p.generate(&r, &CancelToken::new()).unwrap();
        assert!(clear[0].pixels().iter().any(|p| p.a == 0));
        let other = GenerationRequest::new("a ship", (64, 48));
        assert_ne!(p.generate(&other, &CancelToken::new()).unwrap()[0], a[0]);
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(p.generate(&r, &cancel), Err(GenError::Cancelled));
        assert!(!p.needs_network());
        assert!(p.display_name().contains("not AI"));
    }
}
