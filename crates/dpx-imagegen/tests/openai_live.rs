//! The real OpenAI Images API (ADR-45). Ignored by default: it needs `OPENAI_API_KEY`, the
//! internet, and it costs money (one picture at quality `low`). The owner runs it with
//!
//! ```text
//! OPENAI_API_KEY=sk-... cargo test -p dpx-imagegen --test openai_live -- --ignored --nocapture
//! ```
//!
//! `DPX_OPENAI_MODEL` picks another model id. The converted page is written to
//! `target/tmp/openai_live_320x240.png` for looking at. This test has not been run by the
//! agent that wrote it (no key): the real call is UNVERIFIED.

use std::sync::Arc;
use std::time::Duration;

use dpx_core::import::{ConvertOptions, ConvertTarget, Scaler, convert};
use dpx_core::{Chipset, DisplayMode, Palette, Rgb8, Target};
use dpx_imagegen::{
    ApiKeys, GenerationRequest, ImageGenerationProvider, JobManager, JobState, OpenAiConfig,
    OpenAiProvider, Quality, Secret, openai,
};

#[test]
#[ignore = "calls the real OpenAI API (costs money); needs OPENAI_API_KEY"]
fn generates_a_castle_for_an_ocs_320x240_page() {
    let Some(key) = std::env::var(openai::ENV_KEY)
        .ok()
        .and_then(|k| Secret::new(&k))
    else {
        eprintln!("{} is not set: nothing was sent", openai::ENV_KEY);
        return;
    };
    let keys = ApiKeys::in_memory();
    keys.set(openai::KEY_ACCOUNT, &key);
    let config = OpenAiConfig {
        model: std::env::var("DPX_OPENAI_MODEL").unwrap_or_else(|_| openai::DEFAULT_MODEL.into()),
        quality: Quality::Low,
        ..OpenAiConfig::default()
    };
    let provider = Arc::new(OpenAiProvider::new(config, Arc::new(keys)));
    let target = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES).with_screen(320, 240);
    let request = GenerationRequest::for_page(
        "A medieval castle at sunset, classic Amiga pixel art",
        &provider.capabilities(),
        (320, 240),
        target.mode.pixel_shape(),
    );
    assert_eq!(request.size, (1024, 768));
    let jobs = JobManager::new();
    let id = jobs.submit(provider, request).unwrap();
    let status = jobs.wait(id, Duration::from_secs(300)).unwrap();
    eprintln!(
        "{}: {} after {:.1} s",
        status.id,
        status.state.as_str(),
        status.elapsed.as_secs_f64()
    );
    assert_eq!(status.state, JobState::Done, "{:?}", status.error);
    let images = jobs.images(id).unwrap();
    let image = &images[0];
    eprintln!("got {}x{}", image.width(), image.height());

    let palette = Palette::dpaint_default_32();
    let page = convert(
        image,
        &ConvertOptions {
            scaler: Scaler::PixelArt,
            colors: 32,
            ..ConvertOptions::default()
        },
        &ConvertTarget {
            target,
            page: (320, 240),
            palette: &palette,
            bg: 0,
            transparent: Rgb8::new(255, 0, 255),
        },
    )
    .unwrap();
    assert_eq!((page.page.width(), page.page.height()), (320, 240));
    assert_eq!(page.palette.len(), 32);
    assert!(page.page.pixels().iter().all(|&p| p < 32));
    assert!(
        page.palette
            .colors()
            .iter()
            .all(|&c| Rgb8::from_rgb4(c.to_rgb4()) == c)
    );
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("openai_live_320x240.png");
    std::fs::write(
        &out,
        dpx_formats::write_png(&page.page, &page.palette).unwrap(),
    )
    .unwrap();
    eprintln!("wrote {}", out.display());
}
