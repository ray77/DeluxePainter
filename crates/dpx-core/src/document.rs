//! The document: 1..N animation frames of one size, one palette, one hardware target and one
//! layer stack (ADR-1 subset; frames: ADR-28; layers: ADR-44).
//!
//! Every frame holds one indexed picture per layer of the stack (bottom first). The active
//! layer of the current frame is "the page": every page command paints it, and
//! [`Document::page`] returns it, so single-layer, single-frame code keeps working
//! unchanged. What the document looks like (visible layers bottom to top, colour 0
//! transparent above the bottom layer) is [`Document::composite`] and
//! [`Document::composite_frame`]; a single-layer document's picture is its page itself,
//! borrowed, not copied. The other pictures sit behind `Arc`s, so undo snapshots, frame
//! copies and new layers share their pixels. Documents are changed only through
//! [`crate::Engine`], which keeps the revision counter and the undo history.

use std::borrow::Cow;
use std::sync::Arc;

use crate::anim;
use crate::command::CommandError;
use crate::image::IndexedImage;
use crate::layer::{self, Layer};
use crate::mode::Target;
use crate::palette::Palette;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// The active layer of the current frame.
    pub(crate) page: IndexedImage,
    pub(crate) palette: Palette,
    pub(crate) target: Target,
    pub(crate) revision: u64,
    /// Every picture but `page`, frame by frame and in each frame bottom layer first; `page`
    /// belongs at position `current * layers.len() + active` of that list. A single-layer
    /// document keeps exactly its other frames here.
    pub(crate) others: Vec<Arc<IndexedImage>>,
    /// Index of the current frame (0-based; the interface counts from 1).
    pub(crate) current: usize,
    /// Playback rate in frames per second (Anim ▸ Control ▸ Set Rate..).
    pub(crate) fps: u8,
    /// The layer stack shared by every frame, bottom first (ADR-44).
    pub(crate) layers: Vec<Layer>,
    /// Index of the active layer (0 = bottom).
    pub(crate) active: usize,
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

    /// A document holding an existing picture (e.g. a loaded file) at revision 0: one frame
    /// with the one "Background" layer.
    pub fn from_picture(page: IndexedImage, palette: Palette, target: Target) -> Self {
        Self {
            page,
            palette,
            target,
            revision: 0,
            others: Vec::new(),
            current: 0,
            fps: anim::DEFAULT_FPS,
            layers: vec![Layer::background()],
            active: 0,
        }
    }

    /// A document of several frames (e.g. a loaded project) at revision 0, showing frame
    /// `current`, with the one "Background" layer. All frames must have one size, and their
    /// number must stay within the frame limits ([`anim::max_frames_for`]); `fps` must be a
    /// Set Rate value.
    pub fn from_frames(
        frames: Vec<IndexedImage>,
        current: usize,
        fps: u8,
        palette: Palette,
        target: Target,
    ) -> Result<Self, CommandError> {
        let frames = frames.into_iter().map(|f| vec![f]).collect();
        Self::from_layers(
            frames,
            vec![Layer::background()],
            current,
            0,
            fps,
            palette,
            target,
        )
    }

    /// A layered document (e.g. a loaded project) at revision 0: `frames[f][l]` is layer `l`
    /// of frame `f` (bottom first), `layers` the stack, frame `current` and layer `active`
    /// shown and painted. Every frame needs one picture per layer, all of one size; 1 to
    /// [`layer::MAX_LAYERS`] layers with names of at most [`layer::MAX_NAME_CHARS`]
    /// characters; frames times layers within the memory limit
    /// ([`anim::max_frames_for_layers`]); `fps` a Set Rate value. Pixel values are not
    /// checked against the palette (as for [`Document::from_frames`]).
    pub fn from_layers(
        frames: Vec<Vec<IndexedImage>>,
        layers: Vec<Layer>,
        current: usize,
        active: usize,
        fps: u8,
        palette: Palette,
        target: Target,
    ) -> Result<Self, CommandError> {
        if layers.is_empty() || layers.len() > layer::MAX_LAYERS {
            return Err(CommandError::invalid(format!(
                "{} layers: 1 to {} allowed",
                layers.len(),
                layer::MAX_LAYERS
            )));
        }
        for l in &layers {
            layer::check_name(&l.name)?;
        }
        let Some(first) = frames.first().and_then(|f| f.first()) else {
            return Err(CommandError::invalid(
                "a document needs at least one frame".into(),
            ));
        };
        let (w, h) = (first.width(), first.height());
        for (f, frame) in frames.iter().enumerate() {
            if frame.len() != layers.len() {
                return Err(CommandError::invalid(format!(
                    "frame {} holds {} pictures for {} layers",
                    f + 1,
                    frame.len(),
                    layers.len()
                )));
            }
            if let Some(odd) = frame.iter().find(|p| (p.width(), p.height()) != (w, h)) {
                return Err(CommandError::invalid(format!(
                    "frame {} has a {}x{} picture, frame 1 is {w}x{h}: all frames and layers \
                     must have one size",
                    f + 1,
                    odd.width(),
                    odd.height()
                )));
            }
        }
        check_frame_count(frames.len(), layers.len(), w, h)?;
        if current >= frames.len() {
            return Err(CommandError::invalid(format!(
                "current frame {} is outside 1..={}",
                current + 1,
                frames.len()
            )));
        }
        if active >= layers.len() {
            return Err(CommandError::invalid(format!(
                "active layer {active} is outside 0..{}",
                layers.len()
            )));
        }
        check_fps(fps)?;
        let mut doc = Self {
            page: IndexedImage::new(0, 0, 0),
            palette,
            target,
            revision: 0,
            others: Vec::new(),
            current: 0,
            fps,
            layers: Vec::new(),
            active: 0,
        };
        let frames = frames
            .into_iter()
            .map(|f| f.into_iter().map(Arc::new).collect())
            .collect();
        doc.set_layered(frames, layers, current, active);
        Ok(doc)
    }

    /// Chipset and display mode the document is made for.
    pub fn target(&self) -> Target {
        self.target
    }

    /// The page every page command paints: the active layer of the current frame. What
    /// the frame looks like with all its layers is [`Document::composite`].
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
        (self.others.len() + 1) / self.layers.len()
    }

    /// Index of the current frame (0-based).
    pub fn current_frame(&self) -> usize {
        self.current
    }

    /// The active layer of frame `i` (0-based), the current frame included. What the frame
    /// looks like is [`Document::composite_frame`].
    pub fn frame(&self, i: usize) -> Option<&IndexedImage> {
        self.cel(i, self.active)
    }

    /// The active layer of every frame, in order (see [`Document::frame`]).
    pub fn frames(&self) -> impl Iterator<Item = &IndexedImage> {
        (0..self.frame_count()).filter_map(|i| self.frame(i))
    }

    /// Playback rate in frames per second.
    pub fn fps(&self) -> u8 {
        self.fps
    }

    /// The layer stack, bottom first (at least one layer).
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// Number of layers (at least 1).
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Index of the active layer (0 = bottom): [`Document::page`] is this layer of the
    /// current frame.
    pub fn active_layer(&self) -> usize {
        self.active
    }

    /// Layer `layer` of frame `frame` (both 0-based).
    pub fn cel(&self, frame: usize, layer: usize) -> Option<&IndexedImage> {
        if frame >= self.frame_count() || layer >= self.layers.len() {
            return None;
        }
        let at = self.slot(frame, layer);
        let page = self.slot(self.current, self.active);
        match at.cmp(&page) {
            std::cmp::Ordering::Less => self.others.get(at).map(|f| &**f),
            std::cmp::Ordering::Equal => Some(&self.page),
            std::cmp::Ordering::Greater => self.others.get(at - 1).map(|f| &**f),
        }
    }

    /// Layer `layer` of the current frame.
    pub fn layer(&self, layer: usize) -> Option<&IndexedImage> {
        self.cel(self.current, layer)
    }

    /// Every picture of every frame and layer, frame by frame, bottom layer first.
    pub fn cels(&self) -> impl Iterator<Item = &IndexedImage> {
        let layers = self.layers.len();
        (0..self.frame_count() * layers).filter_map(move |i| self.cel(i / layers, i % layers))
    }

    /// The current frame as it looks: its visible layers bottom to top, colour 0
    /// transparent on every layer above the bottom one, colour 0 where nothing shows. With
    /// one visible layer (every single-layer document) that layer itself, borrowed.
    pub fn composite(&self) -> Cow<'_, IndexedImage> {
        self.composite_frame_with(self.current, None)
            .expect("the current frame exists")
    }

    /// The current frame as it looks with `active` in place of its active layer (a stroke
    /// preview painted on a copy of [`Document::page`]; it must have the page's size).
    pub fn composite_with<'a>(&'a self, active: &'a IndexedImage) -> Cow<'a, IndexedImage> {
        self.composite_frame_with(self.current, Some(active))
            .expect("the current frame exists")
    }

    /// Frame `i` as it looks (see [`Document::composite`]).
    pub fn composite_frame(&self, i: usize) -> Option<Cow<'_, IndexedImage>> {
        self.composite_frame_with(i, None)
    }

    /// Frame `i` as it looks, with `active` (when given) in place of its active layer.
    pub fn composite_frame_with<'a>(
        &'a self,
        i: usize,
        active: Option<&'a IndexedImage>,
    ) -> Option<Cow<'a, IndexedImage>> {
        if i >= self.frame_count() {
            return None;
        }
        let picture = |l: usize| match active {
            Some(img) if l == self.active => Some(img),
            _ => self.cel(i, l),
        };
        let visible: Vec<&IndexedImage> = (0..self.layers.len())
            .filter(|&l| self.layers[l].visible)
            .filter_map(picture)
            .collect();
        Some(match visible.as_slice() {
            [only] => Cow::Borrowed(*only),
            _ => Cow::Owned(layer::flatten(
                &visible,
                self.page.width(),
                self.page.height(),
            )),
        })
    }

    /// Every frame as it looks, in order (see [`Document::composite`]).
    pub fn composite_frames(&self) -> impl Iterator<Item = Cow<'_, IndexedImage>> {
        (0..self.frame_count()).filter_map(|i| self.composite_frame(i))
    }

    /// The same frames, layers, current frame and rate with another palette, at revision 0
    /// (e.g. the spare page taking over the shown page's palette). The caller keeps the
    /// pixels inside the new palette.
    pub fn with_palette(&self, palette: Palette) -> Document {
        Document {
            palette,
            revision: 0,
            ..self.clone()
        }
    }

    /// Position of layer `layer` of frame `frame` in the list of all pictures.
    fn slot(&self, frame: usize, layer: usize) -> usize {
        frame * self.layers.len() + layer
    }

    /// Makes layer `layer` of frame `frame` the page. Call only with existing ones.
    pub(crate) fn focus(&mut self, frame: usize, layer: usize) {
        if (frame, layer) == (self.current, self.active)
            || frame >= self.frame_count()
            || layer >= self.layers.len()
        {
            return;
        }
        let page = std::mem::replace(&mut self.page, IndexedImage::new(0, 0, 0));
        let here = self.slot(self.current, self.active);
        self.others.insert(here, Arc::new(page));
        let there = self.slot(frame, layer);
        self.page = Arc::unwrap_or_clone(self.others.remove(there));
        self.current = frame;
        self.active = layer;
    }

    /// Makes frame `i` the current one (the active layer stays). Call only with
    /// `i < frame_count()`.
    pub(crate) fn go_to(&mut self, i: usize) {
        self.focus(i, self.active);
    }

    /// Changes every picture of every frame and layer, the page included; `f` gets the
    /// layer index with the picture.
    pub(crate) fn for_each_cel(&mut self, mut f: impl FnMut(usize, &mut IndexedImage)) {
        let layers = self.layers.len();
        let page = self.slot(self.current, self.active);
        f(self.active, &mut self.page);
        for (i, other) in self.others.iter_mut().enumerate() {
            let at = if i < page { i } else { i + 1 };
            f(at % layers, Arc::make_mut(other));
        }
    }

    /// Replaces all frames, keeping the layer stack: `frames[f]` holds one picture per layer
    /// (same rules as [`Document::from_layers`], checked by the caller); frame `current`
    /// becomes the current one.
    pub(crate) fn set_frames(&mut self, frames: Vec<Vec<Arc<IndexedImage>>>, current: usize) {
        let layers = std::mem::take(&mut self.layers);
        let active = self.active;
        self.set_layered(frames, layers, current, active);
    }

    /// Replaces all frames and the layer stack (checked by the caller): `frames[f][l]` is
    /// layer `l` of frame `f`; frame `current` and layer `active` become the page.
    pub(crate) fn set_layered(
        &mut self,
        frames: Vec<Vec<Arc<IndexedImage>>>,
        layers: Vec<Layer>,
        current: usize,
        active: usize,
    ) {
        debug_assert!(frames.iter().all(|f| f.len() == layers.len()));
        let mut others: Vec<Arc<IndexedImage>> = frames.into_iter().flatten().collect();
        self.layers = layers;
        self.current = current;
        self.active = active;
        let page = self.slot(current, active);
        self.page = Arc::unwrap_or_clone(others.remove(page));
        self.others = others;
    }

    /// All frames in order, each with its pictures bottom layer first; the page as a new
    /// `Arc` (its pixels are copied once).
    pub(crate) fn frame_cels(&self) -> Vec<Vec<Arc<IndexedImage>>> {
        let mut all = self.others.clone();
        all.insert(
            self.slot(self.current, self.active),
            Arc::new(self.page.clone()),
        );
        let layers = self.layers.len();
        let mut frames = Vec::with_capacity(all.len() / layers);
        let mut rest = all.into_iter();
        loop {
            let frame: Vec<_> = rest.by_ref().take(layers).collect();
            if frame.is_empty() {
                break;
            }
            frames.push(frame);
        }
        frames
    }

    /// One frame of the document's size and layers: the bottom layer in `fill`, every layer
    /// above it transparent (colour 0); the upper layers share one picture.
    pub(crate) fn blank_frame(&self, fill: u8) -> Vec<Arc<IndexedImage>> {
        let (w, h) = (self.page.width(), self.page.height());
        let clear = Arc::new(IndexedImage::new(w, h, 0));
        let mut frame = vec![Arc::new(IndexedImage::new(w, h, fill))];
        frame.extend((1..self.layers.len()).map(|_| Arc::clone(&clear)));
        frame
    }
}

/// `count` frames of `layers` pictures of `width`×`height` pixels must be
/// 1..=[`anim::max_frames_for_layers`].
pub(crate) fn check_frame_count(
    count: usize,
    layers: usize,
    width: u32,
    height: u32,
) -> Result<(), CommandError> {
    let max = anim::max_frames_for_layers(width, height, layers);
    if (1..=max).contains(&count) {
        return Ok(());
    }
    let mib = anim::MAX_FRAME_BYTES / (1024 * 1024);
    if layers > 1 {
        if max == 0 {
            return Err(CommandError::invalid(format!(
                "{layers} layers of {width}x{height} pixels: more than {mib} MiB of pictures"
            )));
        }
        return Err(CommandError::invalid(format!(
            "{count} frames of {layers} layers: 1 to {max} allowed ({width}x{height} pages: at \
             most {mib} MiB of pictures)"
        )));
    }
    let why = if max < anim::MAX_FRAMES {
        format!(" ({width}x{height} pages: at most {mib} MiB of frames)")
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

    /// Three frames of three layers: layer `l` of frame `f` is filled with `10 * f + l + 1`.
    fn layered(current: usize, active: usize) -> Document {
        let frames = (0..3u8)
            .map(|f| {
                (0..3u8)
                    .map(|l| IndexedImage::new(2, 1, 10 * f + l + 1))
                    .collect()
            })
            .collect();
        let layers = ["Background", "Middle", "Top"].map(Layer::new).to_vec();
        Document::from_layers(
            frames,
            layers,
            current,
            active,
            30,
            Palette::dpaint_default_32(),
            Target::default(),
        )
        .unwrap()
    }

    #[test]
    fn every_frame_and_layer_keeps_its_place_while_the_page_moves() {
        let mut d = layered(1, 2);
        assert_eq!((d.frame_count(), d.layer_count()), (3, 3));
        assert_eq!((d.current_frame(), d.active_layer()), (1, 2));
        assert_eq!(d.page().get(0, 0), Some(13));
        let all = |d: &Document| -> Vec<u8> { d.cels().map(|c| c.get(0, 0).unwrap()).collect() };
        let expect = [1, 2, 3, 11, 12, 13, 21, 22, 23];
        assert_eq!(all(&d), expect);
        for (f, l) in [(0, 0), (2, 1), (2, 2), (1, 0), (0, 2)] {
            d.focus(f, l);
            assert_eq!((d.current_frame(), d.active_layer()), (f, l));
            assert_eq!(d.page().get(0, 0), Some(10 * f as u8 + l as u8 + 1));
            assert_eq!(all(&d), expect);
            // Frames are the active layer of every frame.
            let frames: Vec<u8> = d.frames().map(|p| p.get(0, 0).unwrap()).collect();
            assert_eq!(frames, [l as u8 + 1, 11 + l as u8, 21 + l as u8]);
            assert_eq!(d.layer(1).unwrap().get(0, 0), Some(10 * f as u8 + 2));
        }
        assert!(d.cel(3, 0).is_none() && d.cel(0, 3).is_none());
        // Frame groups round-trip.
        let groups = d.frame_cels();
        assert_eq!(groups.len(), 3);
        let mut e = d.clone();
        e.set_frames(groups, 2);
        assert_eq!(all(&e), expect);
        assert_eq!((e.current_frame(), e.active_layer()), (2, 2));
    }

    #[test]
    fn the_composite_lays_visible_layers_bottom_to_top() {
        let pal = Palette::dpaint_default_32();
        let t = Target::default();
        let pic = |p: [u8; 3]| IndexedImage::from_pixels(3, 1, p.to_vec()).unwrap();
        let frames = vec![vec![pic([1, 1, 1]), pic([0, 2, 0]), pic([0, 0, 3])]];
        let mut layers = ["A", "B", "C"].map(Layer::new).to_vec();
        let d = Document::from_layers(frames.clone(), layers.clone(), 0, 1, 30, pal.clone(), t)
            .unwrap();
        assert_eq!(d.composite().pixels(), [1, 2, 3]);
        assert!(matches!(d.composite(), Cow::Owned(_)));
        // A stroke preview takes the active layer's place.
        assert_eq!(d.composite_with(&pic([5, 0, 0])).pixels(), [5, 1, 3]);
        layers[0].visible = false;
        let d = Document::from_layers(frames.clone(), layers.clone(), 0, 1, 30, pal.clone(), t)
            .unwrap();
        assert_eq!(d.composite().pixels(), [0, 2, 3]);
        layers[2].visible = false;
        let d = Document::from_layers(frames.clone(), layers.clone(), 0, 1, 30, pal.clone(), t)
            .unwrap();
        assert!(matches!(d.composite(), Cow::Borrowed(_)), "one layer shows");
        assert_eq!(d.composite().pixels(), [0, 2, 0]);
        layers[1].visible = false;
        let d = Document::from_layers(frames, layers, 0, 1, 30, pal.clone(), t).unwrap();
        assert_eq!(d.composite().pixels(), [0, 0, 0]);
        // A single-layer document's picture is its page, borrowed.
        let one = Document::new(4, 2, pal);
        assert!(matches!(one.composite(), Cow::Borrowed(p) if std::ptr::eq(p, one.page())));
        assert!(matches!(one.composite_frame(0), Some(Cow::Borrowed(_))));
        assert!(one.composite_frame(1).is_none());
    }

    #[test]
    fn from_layers_checks_the_stack() {
        let pal = Palette::dpaint_default_32();
        let t = Target::default();
        let frame = |n: usize| vec![IndexedImage::new(2, 1, 0); n];
        let two = vec![Layer::background(), Layer::new("Top")];
        let ok = Document::from_layers(vec![frame(2)], two.clone(), 0, 1, 30, pal.clone(), t);
        assert!(ok.is_ok());
        let bad = [
            Document::from_layers(vec![frame(1)], two.clone(), 0, 0, 30, pal.clone(), t),
            Document::from_layers(vec![frame(2)], two.clone(), 0, 2, 30, pal.clone(), t),
            Document::from_layers(vec![frame(2)], vec![], 0, 0, 30, pal.clone(), t),
            Document::from_layers(vec![], two.clone(), 0, 0, 30, pal.clone(), t),
            Document::from_layers(
                vec![vec![IndexedImage::new(2, 1, 0), IndexedImage::new(1, 1, 0)]],
                two.clone(),
                0,
                0,
                30,
                pal.clone(),
                t,
            ),
            Document::from_layers(
                vec![frame(1)],
                vec![Layer::new("x".repeat(256))],
                0,
                0,
                30,
                pal.clone(),
                t,
            ),
            Document::from_layers(
                vec![frame(layer::MAX_LAYERS + 1)],
                vec![Layer::background(); layer::MAX_LAYERS + 1],
                0,
                0,
                30,
                pal,
                t,
            ),
        ];
        for (i, b) in bad.iter().enumerate() {
            assert!(b.is_err(), "case {i}");
        }
    }
}
