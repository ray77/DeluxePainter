//! The document: 1..N animation frames of one size, one palette and one hardware target
//! (ADR-1 subset; frames: ADR-28).
//!
//! Every frame is one indexed page (layers follow). The current frame is "the page": every
//! page command acts on it, and [`Document::page`] returns it, so single-frame code keeps
//! working unchanged. The other frames sit behind `Arc`s, so undo snapshots and frame copies
//! share their pixels. Documents are changed only through [`crate::Engine`], which keeps the
//! revision counter and the undo history.

use std::sync::Arc;

use crate::anim;
use crate::command::CommandError;
use crate::image::IndexedImage;
use crate::mode::Target;
use crate::palette::Palette;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// The current frame.
    pub(crate) page: IndexedImage,
    pub(crate) palette: Palette,
    pub(crate) target: Target,
    pub(crate) revision: u64,
    /// Every frame but the current one, in order; the current frame belongs between
    /// `others[current - 1]` and `others[current]`.
    pub(crate) others: Vec<Arc<IndexedImage>>,
    /// Index of the current frame (0-based; the interface counts from 1).
    pub(crate) current: usize,
    /// Playback rate in frames per second (Anim ▸ Control ▸ Set Rate..).
    pub(crate) fps: u8,
}

impl Document {
    /// A blank page (colour 0 everywhere) at revision 0 for the default target.
    pub fn new(width: u32, height: u32, palette: Palette) -> Self {
        Self::from_picture(
            IndexedImage::new(width, height, 0),
            palette,
            Target::default(),
        )
    }

    /// A document holding an existing picture (e.g. a loaded file) at revision 0.
    pub fn from_picture(page: IndexedImage, palette: Palette, target: Target) -> Self {
        Self {
            page,
            palette,
            target,
            revision: 0,
            others: Vec::new(),
            current: 0,
            fps: anim::DEFAULT_FPS,
        }
    }

    /// A document of several frames (e.g. a loaded project) at revision 0, showing frame
    /// `current`. All frames must have one size, and their number must stay within the frame
    /// limits ([`anim::max_frames_for`]); `fps` must be a Set Rate value.
    pub fn from_frames(
        frames: Vec<IndexedImage>,
        current: usize,
        fps: u8,
        palette: Palette,
        target: Target,
    ) -> Result<Self, CommandError> {
        let Some(first) = frames.first() else {
            return Err(CommandError::invalid(
                "a document needs at least one frame".into(),
            ));
        };
        let (w, h) = (first.width(), first.height());
        if let Some(i) = frames
            .iter()
            .position(|f| (f.width(), f.height()) != (w, h))
        {
            return Err(CommandError::invalid(format!(
                "frame {} is {}x{}, frame 1 is {w}x{h}: all frames must have one size",
                i + 1,
                frames[i].width(),
                frames[i].height()
            )));
        }
        check_frame_count(frames.len(), w, h)?;
        if current >= frames.len() {
            return Err(CommandError::invalid(format!(
                "current frame {} is outside 1..={}",
                current + 1,
                frames.len()
            )));
        }
        check_fps(fps)?;
        let mut others: Vec<Arc<IndexedImage>> = frames.into_iter().map(Arc::new).collect();
        let page = Arc::unwrap_or_clone(others.remove(current));
        Ok(Self {
            page,
            palette,
            target,
            revision: 0,
            others,
            current,
            fps,
        })
    }

    /// Chipset and display mode the document is made for.
    pub fn target(&self) -> Target {
        self.target
    }

    /// The current frame's page.
    pub fn page(&self) -> &IndexedImage {
        &self.page
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Increases by one for every executed, undone or redone command.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Number of animation frames (at least 1).
    pub fn frame_count(&self) -> usize {
        self.others.len() + 1
    }

    /// Index of the current frame (0-based).
    pub fn current_frame(&self) -> usize {
        self.current
    }

    /// Frame `i` (0-based), the current one included.
    pub fn frame(&self, i: usize) -> Option<&IndexedImage> {
        match i.cmp(&self.current) {
            std::cmp::Ordering::Less => self.others.get(i).map(|f| &**f),
            std::cmp::Ordering::Equal => Some(&self.page),
            std::cmp::Ordering::Greater => self.others.get(i - 1).map(|f| &**f),
        }
    }

    /// All frames in order.
    pub fn frames(&self) -> impl Iterator<Item = &IndexedImage> {
        (0..self.frame_count()).filter_map(|i| self.frame(i))
    }

    /// Playback rate in frames per second.
    pub fn fps(&self) -> u8 {
        self.fps
    }

    /// The same frames, current frame and rate with another palette, at revision 0 (e.g. the
    /// spare page taking over the shown page's palette). The caller keeps the pixels inside
    /// the new palette.
    pub fn with_palette(&self, palette: Palette) -> Document {
        Document {
            palette,
            revision: 0,
            ..self.clone()
        }
    }

    /// Makes frame `i` the current one. Call only with `i < frame_count()`.
    pub(crate) fn go_to(&mut self, i: usize) {
        if i == self.current || i >= self.frame_count() {
            return;
        }
        let page = std::mem::replace(&mut self.page, IndexedImage::new(0, 0, 0));
        self.others.insert(self.current, Arc::new(page));
        self.page = Arc::unwrap_or_clone(self.others.remove(i));
        self.current = i;
    }

    /// Changes every frame, the current one included.
    pub(crate) fn for_each_frame(&mut self, mut f: impl FnMut(&mut IndexedImage)) {
        f(&mut self.page);
        for other in &mut self.others {
            f(Arc::make_mut(other));
        }
    }

    /// Replaces all frames (same rules as [`Document::from_frames`], checked by the caller);
    /// frame `current` becomes the current one.
    pub(crate) fn set_frames(&mut self, frames: Vec<Arc<IndexedImage>>, current: usize) {
        let mut others = frames;
        self.page = Arc::unwrap_or_clone(others.remove(current));
        self.others = others;
        self.current = current;
    }

    /// All frames in order, the current one as a new `Arc` (its pixels are copied once).
    pub(crate) fn frame_arcs(&self) -> Vec<Arc<IndexedImage>> {
        let mut frames = self.others.clone();
        frames.insert(self.current, Arc::new(self.page.clone()));
        frames
    }
}

/// `count` frames of `width`×`height` pixels must be 1..=[`anim::max_frames_for`].
pub(crate) fn check_frame_count(count: usize, width: u32, height: u32) -> Result<(), CommandError> {
    let max = anim::max_frames_for(width, height);
    if (1..=max).contains(&count) {
        return Ok(());
    }
    let why = if max < anim::MAX_FRAMES {
        format!(
            " ({width}x{height} pages: at most {} MiB of frames)",
            anim::MAX_FRAME_BYTES / (1024 * 1024)
        )
    } else {
        String::new()
    };
    Err(CommandError::invalid(format!(
        "{count} frames: 1 to {max} allowed{why}"
    )))
}

/// Set Rate values: [`anim::MIN_FPS`]..=[`anim::MAX_FPS`] frames per second.
pub(crate) fn check_fps(fps: u8) -> Result<(), CommandError> {
    if (anim::MIN_FPS..=anim::MAX_FPS).contains(&fps) {
        Ok(())
    } else {
        Err(CommandError::invalid(format!(
            "rate {fps}: {} to {} frames per second",
            anim::MIN_FPS,
            anim::MAX_FPS
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(n: u8) -> Vec<IndexedImage> {
        (0..n).map(|i| IndexedImage::new(4, 2, i)).collect()
    }

    #[test]
    fn frames_keep_their_order_while_the_current_one_moves() {
        let mut d = Document::from_frames(
            frames(5),
            2,
            12,
            Palette::dpaint_default_32(),
            Target::default(),
        )
        .unwrap();
        assert_eq!((d.frame_count(), d.current_frame(), d.fps()), (5, 2, 12));
        assert_eq!(d.page().get(0, 0), Some(2));
        for target in [4, 0, 3, 3, 1] {
            d.go_to(target);
            assert_eq!(d.current_frame(), target);
            assert_eq!(d.page().get(0, 0), Some(target as u8));
            let order: Vec<u8> = d.frames().map(|f| f.get(0, 0).unwrap()).collect();
            assert_eq!(order, [0, 1, 2, 3, 4]);
        }
        assert!(d.frame(5).is_none());
    }

    #[test]
    fn from_frames_checks_sizes_counts_and_rate() {
        let pal = Palette::dpaint_default_32();
        let t = Target::default();
        assert!(Document::from_frames(vec![], 0, 30, pal.clone(), t).is_err());
        let mut odd = frames(2);
        odd.push(IndexedImage::new(5, 2, 0));
        assert!(Document::from_frames(odd, 0, 30, pal.clone(), t).is_err());
        assert!(Document::from_frames(frames(2), 2, 30, pal.clone(), t).is_err());
        assert!(Document::from_frames(frames(2), 0, 0, pal.clone(), t).is_err());
        assert!(Document::from_frames(frames(2), 0, 61, pal.clone(), t).is_err());
        let many = vec![IndexedImage::new(1, 1, 0); anim::MAX_FRAMES + 1];
        assert!(Document::from_frames(many, 0, 30, pal.clone(), t).is_err());
        // One frame is the same as a picture.
        let one = Document::from_frames(frames(1), 0, 30, pal.clone(), t).unwrap();
        assert_eq!(
            one,
            Document::from_picture(IndexedImage::new(4, 2, 0), pal, t)
        );
    }
}
