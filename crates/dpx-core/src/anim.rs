//! Animation frames (ADR-28): frame limits, the Light Table (onion skin) and playback timing.
//!
//! The frames themselves live in the [`crate::Document`] and change only through frame
//! commands. This module holds the pure functions the front-ends use to show them; nothing
//! here changes a document.

use crate::image::IndexedImage;
use crate::palette::{Palette, Rgb8};

/// Most frames a document may have. DeluxePaint names a saved frame series with three digits
/// (AGA p.231); its own limit was the Amiga's memory.
pub const MAX_FRAMES: usize = 999;
/// Most bytes all frames together may hold, one byte per pixel (128 MiB). A 320×256 page fits
/// [`MAX_FRAMES`] times, a 1008×1024 page 130 times. It keeps a saved project inside the
/// project reader's limits (`dpx_formats::project::Limits`).
pub const MAX_FRAME_BYTES: u64 = 128 * 1024 * 1024;
/// Playback rate of a new document (DeluxePaint's Set Rate default, AGA p.255).
pub const DEFAULT_FPS: u8 = 30;
/// Set Rate allows 1 to 60 frames per second (AGA p.255).
pub const MIN_FPS: u8 = 1;
pub const MAX_FPS: u8 = 60;

/// Most frames of `width`×`height` pixels within [`MAX_FRAMES`] and [`MAX_FRAME_BYTES`]
/// (at least one: a single page is always allowed).
pub fn max_frames_for(width: u32, height: u32) -> usize {
    let bytes = u64::from(width) * u64::from(height);
    if bytes == 0 {
        return MAX_FRAMES;
    }
    usize::try_from(MAX_FRAME_BYTES / bytes)
        .unwrap_or(MAX_FRAMES)
        .clamp(1, MAX_FRAMES)
}

/// Effect ▸ LightTable: which other images show behind the current frame, and whether they
/// are dimmed. Which images are on at first is not stated in the manual; Previous is the
/// usual onion skin, so it starts on. Dim starts on as in DeluxePaint (AGA p.274).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightTable {
    pub two_back: bool,
    pub previous: bool,
    pub next: bool,
    /// The spare page (DeluxePaint's "Scratch" page while animating).
    pub spare: bool,
    pub dim: bool,
}

impl Default for LightTable {
    fn default() -> Self {
        Self {
            two_back: false,
            previous: true,
            next: false,
            spare: false,
            dim: true,
        }
    }
}

impl LightTable {
    /// The frames to show behind frame `current` of `count`, nearest first (it lies on top):
    /// Previous, Next, Two Back. Frames wrap around like the Previous/Next keys; the current
    /// frame itself and repeats are left out. The spare page is the caller's.
    pub fn frames(self, current: usize, count: usize) -> Vec<usize> {
        let mut out = Vec::new();
        if count < 2 || current >= count {
            return out;
        }
        let back = |n: usize| (current + count - n % count) % count;
        let wanted = [
            (self.previous, back(1)),
            (self.next, (current + 1) % count),
            (self.two_back, back(2)),
        ];
        for (on, f) in wanted {
            if on && f != current && !out.contains(&f) {
                out.push(f);
            }
        }
        out
    }
}

/// Dimmed colours for the Light Table: `table[c]` is the palette entry nearest to the colour
/// halfway between `c` and the `empty` (background) colour, never `empty` itself, so dimmed
/// pixels stay visible. Entries outside the palette map to themselves. With an indexed
/// palette this is the only way to dim (DeluxePaint's own method is not documented).
pub fn dim_table(palette: &Palette, empty: u8) -> Vec<u8> {
    let bg = palette.get(empty).unwrap_or(Rgb8::BLACK);
    (0..=255u8)
        .map(|c| match palette.get(c) {
            Some(_) if c == empty => c,
            Some(col) => {
                let mid = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
                let half = Rgb8::new(mid(col.r, bg.r), mid(col.g, bg.g), mid(col.b, bg.b));
                palette.nearest_excluding(half, &[empty])
            }
            None => c,
        })
        .collect()
}

/// The Light Table picture: `current` with the images `under` showing through its `empty`
/// pixels. `under` is ordered nearest first: at each empty pixel the first image whose pixel
/// is not `empty` shows, through `dim` when given (see [`dim_table`]). Images may have other
/// sizes (the spare page); outside an image counts as empty. The result has `current`'s size.
/// Display only: the document never changes.
pub fn light_table(
    current: &IndexedImage,
    under: &[&IndexedImage],
    empty: u8,
    dim: Option<&[u8]>,
) -> IndexedImage {
    let (w, h) = (current.width() as i32, current.height() as i32);
    let mut pixels = current.pixels().to_vec();
    if under.is_empty() {
        return current.clone();
    }
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if pixels[i] != empty {
                continue;
            }
            let shown = under
                .iter()
                .filter_map(|img| img.get(x, y))
                .find(|&p| p != empty);
            if let Some(p) = shown {
                pixels[i] = dim
                    .and_then(|t| t.get(usize::from(p)).copied())
                    .unwrap_or(p);
            }
        }
    }
    IndexedImage::from_pixels(current.width(), current.height(), pixels).expect("same size")
}

/// Anim ▸ Control ▸ Play, Play once and Ping-pong (keys 4, 5, 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    /// Loops forward until stopped.
    Loop,
    /// Plays once to the last frame and stops there.
    Once,
    /// Forward to the last frame, back to the first, until stopped.
    PingPong,
}

/// The step position after `elapsed` seconds at `fps` frames per second, counted from
/// position `start`. One step per frame time; negative times count as zero.
pub fn play_position(start: u64, fps: u8, elapsed: f64) -> u64 {
    let steps = (elapsed.max(0.0) * f64::from(fps.max(1))).floor();
    start.saturating_add(steps as u64)
}

/// The frame shown at step `position` of a playback of `count` frames, and whether the
/// playback is over. Loop and Once count positions as frame numbers (a loop may start at any
/// frame); Ping-pong positions run around the cycle 0, 1, …, last, last−1, …, 1.
pub fn frame_at(mode: PlayMode, count: usize, position: u64) -> (usize, bool) {
    if count <= 1 {
        return (0, mode == PlayMode::Once && position >= 1);
    }
    let n = count as u64;
    match mode {
        PlayMode::Loop => ((position % n) as usize, false),
        PlayMode::Once if position >= n => (count - 1, true),
        PlayMode::Once => (position as usize, false),
        PlayMode::PingPong => {
            let period = 2 * (n - 1);
            let p = position % period;
            let frame = if p < n { p } else { period - p };
            (frame as usize, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_limits_follow_the_page_size() {
        assert_eq!(max_frames_for(320, 256), MAX_FRAMES);
        assert_eq!(max_frames_for(1008, 1024), 130);
        assert_eq!(max_frames_for(4096, 4096), 8);
        assert_eq!(max_frames_for(0, 10), MAX_FRAMES);
    }

    #[test]
    fn light_table_picks_neighbours_with_wrap_and_no_repeats() {
        let lt = LightTable {
            two_back: true,
            previous: true,
            next: true,
            spare: false,
            dim: true,
        };
        assert_eq!(lt.frames(3, 10), vec![2, 4, 1]);
        assert_eq!(lt.frames(0, 10), vec![9, 1, 8], "wraps");
        // Two frames: previous and next are the same frame, two back is the current one.
        assert_eq!(lt.frames(0, 2), vec![1]);
        assert_eq!(lt.frames(0, 1), Vec::<usize>::new());
        assert_eq!(LightTable::default().frames(5, 10), vec![4]);
    }

    #[test]
    fn light_table_shows_other_frames_only_where_the_current_is_empty() {
        let current = IndexedImage::from_pixels(4, 1, vec![0, 5, 0, 0]).unwrap();
        let prev = IndexedImage::from_pixels(4, 1, vec![7, 7, 0, 0]).unwrap();
        let next = IndexedImage::from_pixels(4, 1, vec![9, 9, 9, 0]).unwrap();
        let out = light_table(&current, &[&prev, &next], 0, None);
        assert_eq!(out.pixels(), &[7, 5, 9, 0], "nearest first, current on top");
        // A smaller image (spare page) counts as empty outside.
        let small = IndexedImage::from_pixels(1, 1, vec![3]).unwrap();
        let out = light_table(&current, &[&small], 0, None);
        assert_eq!(out.pixels(), &[3, 5, 0, 0]);
        // Dimmed through the table.
        let mut table: Vec<u8> = (0..=255).collect();
        table[7] = 2;
        let out = light_table(&current, &[&prev], 0, Some(&table));
        assert_eq!(out.pixels(), &[2, 5, 0, 0]);
        // Nothing under it: unchanged.
        assert_eq!(light_table(&current, &[], 0, None), current);
    }

    #[test]
    fn dim_table_halves_towards_the_background_and_never_hides() {
        let pal = Palette::new(vec![
            Rgb8::BLACK,
            Rgb8::WHITE,
            Rgb8::new(128, 128, 128),
            Rgb8::new(250, 0, 0),
            Rgb8::new(120, 0, 0),
        ])
        .unwrap();
        let t = dim_table(&pal, 0);
        assert_eq!(t.len(), 256);
        assert_eq!(t[0], 0, "background stays");
        assert_eq!(t[1], 2, "white dims to grey");
        assert_eq!(t[3], 4, "red dims to dark red");
        assert_eq!(
            t[4], 4,
            "dark red has nothing darker but black, which is excluded"
        );
        assert_eq!(t[200], 200, "outside the palette");
        let two = Palette::new(vec![Rgb8::BLACK, Rgb8::WHITE]).unwrap();
        assert_eq!(dim_table(&two, 0)[1], 1, "two colours: stays visible");
    }

    #[test]
    fn playback_timing() {
        assert_eq!(play_position(0, 30, 0.0), 0);
        assert_eq!(play_position(0, 30, 0.99 / 30.0), 0);
        assert_eq!(play_position(0, 30, 1.0), 30);
        assert_eq!(play_position(4, 10, 0.25), 6);
        assert_eq!(play_position(4, 10, -1.0), 4);
        assert_eq!(play_position(0, 0, 1.0), 1, "rate 0 counts as 1");
        // Loop wraps around, starting at any frame.
        assert_eq!(frame_at(PlayMode::Loop, 5, 3), (3, false));
        assert_eq!(frame_at(PlayMode::Loop, 5, 7), (2, false));
        // Once stops on the last frame after it has shown for one frame time.
        assert_eq!(frame_at(PlayMode::Once, 5, 4), (4, false));
        assert_eq!(frame_at(PlayMode::Once, 5, 5), (4, true));
        assert_eq!(frame_at(PlayMode::Once, 5, 99), (4, true));
        // Ping-pong: 0 1 2 3 2 1 0 1 …
        let seq: Vec<usize> = (0..8)
            .map(|p| frame_at(PlayMode::PingPong, 4, p).0)
            .collect();
        assert_eq!(seq, [0, 1, 2, 3, 2, 1, 0, 1]);
        assert!(!frame_at(PlayMode::PingPong, 4, 1000).1);
        // One frame.
        assert_eq!(frame_at(PlayMode::Loop, 1, 9), (0, false));
        assert_eq!(frame_at(PlayMode::PingPong, 1, 9), (0, false));
        assert_eq!(frame_at(PlayMode::Once, 1, 0), (0, false));
        assert_eq!(frame_at(PlayMode::Once, 1, 1), (0, true));
    }
}
