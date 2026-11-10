//! What an IFF ANIM means for a DeluxePainter document, shared by the app and the MCP server
//! (ADR-32): the frames to show, the Set Rate value for the file's timing, the jiffy holds to
//! write for a Set Rate value, and the notes about what a document cannot keep.

use dpx_core::anim::{DEFAULT_FPS, MAX_FPS, MIN_FPS};
use dpx_core::{IndexedImage, Palette};

use crate::anim::Anim;
use crate::ilbm::ColorRange;

/// Jiffies (the ANIM time unit) per second.
pub const JIFFIES_PER_SECOND: u32 = 60;

/// The parts of an [`Anim`] a document keeps.
#[derive(Clone, Debug)]
pub struct AnimContent {
    /// The logical frames (the two closing loop frames are dropped).
    pub frames: Vec<IndexedImage>,
    /// The palette of the first frame.
    pub palette: Palette,
    /// The nearest Set Rate value (1..=60 frames per second).
    pub fps: u8,
    pub ranges: Vec<ColorRange>,
    pub camg: Option<u32>,
    /// Everything the document does not keep or had to approximate.
    pub notes: Vec<String>,
}

/// Takes the document parts out of a decoded ANIM.
///
/// - Frames: the logical frames, palette from the first frame, CRNG kept.
/// - Rate: the average over all logical frames, `60 * frames / total jiffies`, rounded to the
///   nearest Set Rate value. A note says so when the timing varies by more than one jiffy (a
///   writer rounding a rate to whole jiffies varies by one) or the rate had to be rounded
///   by more than 0.05 frames per second. A file without timing gets the default rate.
/// - Per-frame CMAP changes cannot be kept (one palette per document): noted.
pub fn content_of(anim: &Anim) -> AnimContent {
    let logical = anim.logical_frames();
    let n = logical.len();
    let mut notes = Vec::new();
    let holds: Vec<u32> = (0..n).map(|i| anim.hold_jiffies(i)).collect();
    let total: u64 = holds.iter().map(|&j| u64::from(j)).sum();
    let fps = if total == 0 {
        notes.push(format!(
            "the file has no frame timing, rate set to {DEFAULT_FPS} per second"
        ));
        DEFAULT_FPS
    } else {
        let ideal = f64::from(JIFFIES_PER_SECOND) * n as f64 / total as f64;
        let fps = (ideal.round() as i64).clamp(i64::from(MIN_FPS), i64::from(MAX_FPS)) as u8;
        let (min, max) = (
            holds.iter().min().copied().unwrap_or(0),
            holds.iter().max().copied().unwrap_or(0),
        );
        if max.saturating_sub(min) > 1 {
            notes.push(format!(
                "frame timing differs ({min} to {max} jiffies per frame, 1/60 s); one rate for \
                 all frames: the average, {fps} per second"
            ));
        } else if (ideal - f64::from(fps)).abs() > 0.05 {
            notes.push(format!(
                "frame rate {ideal:.2} per second rounded to {fps} per second"
            ));
        }
        fps
    };
    let changed = (1..n)
        .filter(|&i| {
            anim.frames[i]
                .palette
                .as_ref()
                .is_some_and(|p| *p != anim.palette)
        })
        .count();
    if changed > 0 {
        notes.push(format!(
            "palette changes in {changed} frames not kept (a document has one palette); the \
             palette of the first frame is used"
        ));
    }
    AnimContent {
        frames: logical.iter().map(|f| f.image.clone()).collect(),
        palette: anim.palette.clone(),
        fps,
        ranges: anim.ranges.clone(),
        camg: anim.camg,
        notes,
    }
}

/// How long each of `frames` frames is shown at `fps` frames per second, in whole jiffies:
/// the rounding error is spread over the frames, so the total is exact for the whole
/// animation (25 per second gives 2, 3, 2, 3, ... jiffies) and reads back as `fps`.
pub fn holds_for_fps(fps: u8, frames: usize) -> Vec<u32> {
    let fps = u64::from(fps.clamp(MIN_FPS, MAX_FPS));
    let edge = |i: u64| (i * u64::from(JIFFIES_PER_SECOND) * 2 + fps) / (fps * 2);
    (0..frames as u64)
        .map(|i| (edge(i + 1) - edge(i)) as u32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_are_at_least_one_jiffy_and_average_exactly() {
        for fps in MIN_FPS..=MAX_FPS {
            let h = holds_for_fps(fps, 60);
            assert!(h.iter().all(|&j| j >= 1), "{fps}");
            let exact = (3600.0 / f64::from(fps)).round() as u32;
            assert_eq!(h.iter().sum::<u32>(), exact, "{fps}");
            let (min, max) = (h.iter().min().unwrap(), h.iter().max().unwrap());
            assert!(max - min <= 1, "{fps}");
        }
        assert_eq!(holds_for_fps(30, 3), vec![2, 2, 2]);
        assert_eq!(holds_for_fps(25, 5), vec![2, 3, 2, 3, 2]);
    }
}
