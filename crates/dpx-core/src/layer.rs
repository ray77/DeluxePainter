//! Layers (ADR-44): one layer stack per document, shared by every animation frame.
//!
//! Every frame holds one indexed picture per layer. Layer 0 is the bottom layer and is
//! opaque; on every layer above it palette index 0 is transparent, so painting with colour 0
//! there erases. Layers are visible or hidden, nothing in between, and there are no groups.
//! All layers share the document's palette, size and hardware target.
//!
//! This module holds the [`Layer`] entry, the limits, the compositing rule
//! ([`flatten`]) and the validation and application of the layer commands, kept apart from
//! the page commands in `command.rs` (as `frame_ops.rs` does for frames). Layer numbers are
//! 0-based from the bottom.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::command::{Command, CommandError};
use crate::document::{Document, check_frame_count};
use crate::image::IndexedImage;

/// Most layers a document may have. Together with [`crate::anim::MAX_FRAME_BYTES`] (frames
/// times layers times page pixels) it bounds the document's memory.
pub const MAX_LAYERS: usize = 32;
/// Most characters of a layer name (the project format's limit since ADR-23).
pub const MAX_NAME_CHARS: usize = 255;
/// The name of the one layer of a new document (and of every project before ADR-44).
pub const BACKGROUND: &str = "Background";

/// One layer of the stack: what every frame's picture of that layer is called and whether it
/// shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
}

impl Layer {
    /// A visible layer called `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible: true,
        }
    }

    /// The single visible "Background" layer every new document has.
    pub fn background() -> Self {
        Self::new(BACKGROUND)
    }
}

/// True for the default stack of every document before ADR-44: one visible "Background".
pub fn is_default_stack(layers: &[Layer]) -> bool {
    matches!(layers, [only] if *only == Layer::background())
}

/// A layer name must have at most [`MAX_NAME_CHARS`] characters.
pub fn check_name(name: &str) -> Result<(), CommandError> {
    let n = name.chars().count();
    if n > MAX_NAME_CHARS {
        return Err(CommandError::invalid(format!(
            "layer name has {n} characters, at most {MAX_NAME_CHARS}"
        )));
    }
    Ok(())
}

/// "Layer N" with the smallest N from 1 that no layer of `layers` is called yet.
pub fn default_name(layers: &[Layer]) -> String {
    (1..)
        .map(|n| format!("Layer {n}"))
        .find(|name| layers.iter().all(|l| l.name != *name))
        .expect("an unused number")
}

/// The name of a copy of `name`: "<name> copy", cut to [`MAX_NAME_CHARS`] characters.
pub fn copy_name(name: &str) -> String {
    format!("{name} copy")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect()
}

/// The note a picture export (ILBM, ANIM, PNG) adds about the layers it flattens: an
/// exported picture is what the frames show, so with more than one layer, or a hidden one,
/// the file holds less than the document. `None` for one visible layer.
pub fn flatten_note(layers: &[Layer]) -> Option<String> {
    let hidden = layers.iter().filter(|l| !l.visible).count();
    match layers {
        [] => None,
        [only] if only.visible => None,
        [only] => Some(format!(
            "layer \"{}\" is hidden: saved as colour 0 (save as .dpxproj to keep it)",
            only.name
        )),
        _ => {
            let left_out = match hidden {
                0 => String::new(),
                1 => ", 1 hidden layer left out".into(),
                n => format!(", {n} hidden layers left out"),
            };
            Some(format!(
                "{} layers saved as one picture, as shown{left_out} (save as .dpxproj to keep \
                 them)",
                layers.len()
            ))
        }
    }
}

/// Lays `top` over `base` (same size): every pixel of `top` but colour 0 covers `base`.
pub fn overlay(base: &mut IndexedImage, top: &IndexedImage) {
    debug_assert_eq!(
        (base.width(), base.height()),
        (top.width(), top.height()),
        "layers share one size"
    );
    for (b, &t) in base.pixels_mut().iter_mut().zip(top.pixels()) {
        if t != 0 {
            *b = t;
        }
    }
}

/// The picture of visible layer pictures `visible`, bottom first: the lowest is copied as it
/// is (the bottom layer is opaque, and over a hidden bottom layer an upper layer's colour 0
/// shows colour 0), the others lie over it with colour 0 transparent. No layer at all gives
/// a `width`×`height` picture of colour 0.
pub fn flatten(visible: &[&IndexedImage], width: u32, height: u32) -> IndexedImage {
    let Some((first, rest)) = visible.split_first() else {
        return IndexedImage::new(width, height, 0);
    };
    let mut out = (*first).clone();
    for top in rest {
        overlay(&mut out, top);
    }
    out
}

/// `active` with the pixels of `after` wherever `after` differs from `before` (all three of
/// one size): a change worked out on the picture as it shows (`before`, the composite) put
/// into the active layer, e.g. LightTable ▸ Merge. Every other pixel of `active` stays.
pub fn with_changes(
    active: &IndexedImage,
    before: &IndexedImage,
    after: &IndexedImage,
) -> IndexedImage {
    debug_assert_eq!(
        (active.width(), active.height()),
        (after.width(), after.height()),
        "one size"
    );
    let mut out = active.clone();
    let changed = before.pixels().iter().zip(after.pixels());
    for (p, (&b, &a)) in out.pixels_mut().iter_mut().zip(changed) {
        if a != b {
            *p = a;
        }
    }
    out
}

/// The current frame of `doc` as it shows once `picture` (the page's size) is a new
/// visible layer where [`Command::AddPictureLayer`] puts it: right above the active layer,
/// under the visible layers above that one (ADR-45: the Generate requester's New layer
/// thumbnail). The same picture as the composite after that command.
pub fn composite_with_new_layer(doc: &Document, picture: &IndexedImage) -> IndexedImage {
    let frame = doc.current_frame();
    let mut visible: Vec<&IndexedImage> = Vec::with_capacity(doc.layer_count() + 1);
    for (l, layer) in doc.layers().iter().enumerate() {
        if layer.visible
            && let Some(cel) = doc.cel(frame, l)
        {
            visible.push(cel);
        }
        if l == doc.active_layer() {
            visible.push(picture);
        }
    }
    flatten(&visible, doc.page().width(), doc.page().height())
}

// ---- Layer commands.

impl Command {
    /// True for the layer commands of ADR-44.
    pub(crate) fn is_layer_command(&self) -> bool {
        matches!(
            self,
            Command::AddLayer { .. }
                | Command::AddPictureLayer { .. }
                | Command::DuplicateLayer { .. }
                | Command::DeleteLayer { .. }
                | Command::MoveLayer { .. }
                | Command::SelectLayer { .. }
                | Command::SetLayerVisible { .. }
                | Command::RenameLayer { .. }
                | Command::MergeLayerDown { .. }
                | Command::FlattenLayers
                | Command::SetPicture { .. }
        )
    }

    /// The [`Command::SetPicture`] that makes a document of `doc`'s palette and target show
    /// and hold what `doc` does: every frame with every layer, the layer stack, the current
    /// frame and the active layer (the rate is a setting and stays).
    pub fn set_picture(doc: &Document) -> Command {
        let layers = doc.layer_count();
        let frames = (0..doc.frame_count())
            .map(|f| {
                (0..layers)
                    .map(|l| {
                        let cel = doc.cel(f, l).expect("an existing frame and layer");
                        cel.pixels().to_vec()
                    })
                    .collect()
            })
            .collect();
        Command::SetPicture {
            width: doc.page.width(),
            height: doc.page.height(),
            layers: doc.layers.clone(),
            frames,
            current: doc.current_frame() as u32,
            active: doc.active_layer() as u32,
        }
    }
}

/// Layer `l` must exist.
fn layer_exists(doc: &Document, l: u32, what: &str) -> Result<usize, CommandError> {
    let count = doc.layer_count();
    let i = l as usize;
    if i < count {
        Ok(i)
    } else {
        Err(CommandError::invalid(format!(
            "{what} layer {l} is outside 0..{count} (0 is the bottom layer)"
        )))
    }
}

/// One more layer must fit [`MAX_LAYERS`] and the memory limit of all frames' pictures.
fn room_for_another(doc: &Document) -> Result<(), CommandError> {
    let count = doc.layer_count();
    if count >= MAX_LAYERS {
        return Err(CommandError::invalid(format!(
            "{count} layers: at most {MAX_LAYERS} allowed"
        )));
    }
    let (w, h) = (doc.page.width(), doc.page.height());
    check_frame_count(doc.frame_count(), count + 1, w, h)
}

pub(crate) fn validate(cmd: &Command, doc: &Document) -> Result<(), CommandError> {
    match cmd {
        Command::AddLayer { name } => {
            if let Some(name) = name {
                check_name(name)?;
            }
            room_for_another(doc)
        }
        Command::AddPictureLayer { name, pixels } => {
            if let Some(name) = name {
                check_name(name)?;
            }
            room_for_another(doc)?;
            let (w, h) = (doc.page.width(), doc.page.height());
            crate::command::other_page(w, h, pixels, doc.palette.len())
        }
        Command::DuplicateLayer { layer } => {
            layer_exists(doc, *layer, "duplicate")?;
            room_for_another(doc)
        }
        Command::DeleteLayer { layer } => {
            layer_exists(doc, *layer, "delete")?;
            if doc.layer_count() == 1 {
                return Err(CommandError::invalid(
                    "the last layer cannot be deleted".into(),
                ));
            }
            Ok(())
        }
        Command::MoveLayer { from, to } => {
            layer_exists(doc, *from, "move")?;
            layer_exists(doc, *to, "target")?;
            Ok(())
        }
        Command::SelectLayer { layer } => layer_exists(doc, *layer, "select").map(|_| ()),
        Command::SetLayerVisible { layer, .. } => layer_exists(doc, *layer, "show").map(|_| ()),
        Command::RenameLayer { layer, name } => {
            layer_exists(doc, *layer, "rename")?;
            check_name(name)
        }
        Command::MergeLayerDown { layer } => {
            let l = layer_exists(doc, *layer, "merge")?;
            if l == 0 {
                return Err(CommandError::invalid(
                    "the bottom layer has no layer below to merge into".into(),
                ));
            }
            Ok(())
        }
        Command::FlattenLayers => Ok(()),
        Command::SetPicture {
            width,
            height,
            layers,
            frames,
            current,
            active,
        } => validate_picture(doc, (*width, *height), layers, frames, *current, *active),
        _ => unreachable!("not a layer command"),
    }
}

/// [`Command::SetPicture`]: a stack of 1 to [`MAX_LAYERS`] named layers, at least one frame
/// with one picture per layer, every picture `width`×`height` in the palette, a size the
/// target allows (or the page's own), frames times layers within the memory limit, and a
/// current frame and active layer that exist.
fn validate_picture(
    doc: &Document,
    (width, height): (u32, u32),
    layers: &[Layer],
    frames: &[Vec<Vec<u8>>],
    current: u32,
    active: u32,
) -> Result<(), CommandError> {
    if layers.is_empty() || layers.len() > MAX_LAYERS {
        return Err(CommandError::invalid(format!(
            "{} layers: 1 to {MAX_LAYERS} allowed",
            layers.len()
        )));
    }
    for l in layers {
        check_name(&l.name)?;
    }
    if frames.is_empty() {
        return Err(CommandError::invalid(
            "a picture needs at least one frame".into(),
        ));
    }
    let colors = doc.palette.len();
    for (f, frame) in frames.iter().enumerate() {
        if frame.len() != layers.len() {
            return Err(CommandError::invalid(format!(
                "frame {} holds {} pictures for {} layers",
                f + 1,
                frame.len(),
                layers.len()
            )));
        }
        for pixels in frame {
            crate::command::other_page(width, height, pixels, colors)?;
        }
    }
    let same = (width, height) == (doc.page.width(), doc.page.height());
    if !same && !doc.target.page_fits(width, height) {
        let ((min_w, min_h), (max_w, max_h)) = doc.target.page_limits();
        return Err(CommandError::invalid(format!(
            "Page must be {min_w}x{min_h} to {max_w}x{max_h}"
        )));
    }
    check_frame_count(frames.len(), layers.len(), width, height)?;
    if current as usize >= frames.len() {
        return Err(CommandError::invalid(format!(
            "current frame {} is outside 1..={}",
            current + 1,
            frames.len()
        )));
    }
    if active as usize >= layers.len() {
        return Err(CommandError::invalid(format!(
            "active layer {active} is outside 0..{}",
            layers.len()
        )));
    }
    Ok(())
}

pub(crate) fn apply(cmd: &Command, doc: &mut Document) {
    let active = doc.active;
    let current = doc.current;
    let (w, h) = (doc.page.width(), doc.page.height());
    match cmd {
        Command::AddLayer { name } => {
            // One transparent picture shared by every frame until a frame is painted.
            let blank = Arc::new(IndexedImage::new(w, h, 0));
            let at = active + 1;
            let mut frames = doc.frame_cels();
            for f in &mut frames {
                f.insert(at, Arc::clone(&blank));
            }
            let mut layers = doc.layers.clone();
            let name = name.clone().unwrap_or_else(|| default_name(&layers));
            layers.insert(at, Layer::new(name));
            doc.set_layered(frames, layers, current, at);
        }
        Command::AddPictureLayer { name, pixels } => {
            // Transparent in every frame but the current one, which holds the picture.
            let blank = Arc::new(IndexedImage::new(w, h, 0));
            let picture = IndexedImage::from_pixels(w, h, pixels.clone());
            let picture = Arc::new(picture.expect("size checked by validate"));
            let at = active + 1;
            let mut frames = doc.frame_cels();
            for (f, cels) in frames.iter_mut().enumerate() {
                let cel = if f == current { &picture } else { &blank };
                cels.insert(at, Arc::clone(cel));
            }
            let mut layers = doc.layers.clone();
            let name = name.clone().unwrap_or_else(|| default_name(&layers));
            layers.insert(at, Layer::new(name));
            doc.set_layered(frames, layers, current, at);
        }
        Command::DuplicateLayer { layer } => {
            let l = *layer as usize;
            let mut frames = doc.frame_cels();
            for f in &mut frames {
                let copy = Arc::clone(&f[l]);
                f.insert(l + 1, copy);
            }
            let mut layers = doc.layers.clone();
            let copy = Layer {
                name: copy_name(&layers[l].name),
                visible: layers[l].visible,
            };
            layers.insert(l + 1, copy);
            doc.set_layered(frames, layers, current, l + 1);
        }
        Command::DeleteLayer { layer } => {
            let l = *layer as usize;
            let mut frames = doc.frame_cels();
            for f in &mut frames {
                f.remove(l);
            }
            let mut layers = doc.layers.clone();
            layers.remove(l);
            // The layer below takes over (the new bottom layer when the bottom one goes).
            let active = match active.cmp(&l) {
                std::cmp::Ordering::Less => active,
                std::cmp::Ordering::Equal => l.saturating_sub(1),
                std::cmp::Ordering::Greater => active - 1,
            };
            doc.set_layered(frames, layers, current, active);
        }
        Command::MoveLayer { from, to } => {
            let (from, to) = (*from as usize, *to as usize);
            let mut order: Vec<usize> = (0..doc.layer_count()).collect();
            let moved = order.remove(from);
            order.insert(to, moved);
            let frames = doc
                .frame_cels()
                .into_iter()
                .map(|f| order.iter().map(|&i| Arc::clone(&f[i])).collect())
                .collect();
            let layers = order.iter().map(|&i| doc.layers[i].clone()).collect();
            let active = order
                .iter()
                .position(|&i| i == active)
                .expect("a permutation");
            doc.set_layered(frames, layers, current, active);
        }
        Command::SelectLayer { layer } => doc.focus(current, *layer as usize),
        Command::SetLayerVisible { layer, visible } => {
            doc.layers[*layer as usize].visible = *visible;
        }
        Command::RenameLayer { layer, name } => {
            doc.layers[*layer as usize].name = name.clone();
        }
        Command::MergeLayerDown { layer } => {
            let l = *layer as usize;
            let mut frames = doc.frame_cels();
            for f in &mut frames {
                let top = f.remove(l);
                if top.pixels().iter().any(|&p| p != 0) {
                    let mut merged = (*f[l - 1]).clone();
                    overlay(&mut merged, &top);
                    f[l - 1] = Arc::new(merged);
                }
            }
            let mut layers = doc.layers.clone();
            layers.remove(l);
            let active = if active >= l { active - 1 } else { active };
            doc.set_layered(frames, layers, current, active);
        }
        Command::FlattenLayers => {
            let frames = (0..doc.frame_count())
                .map(|f| {
                    let picture = doc.composite_frame(f).expect("an existing frame");
                    vec![Arc::new(picture.into_owned())]
                })
                .collect();
            let bottom = Layer {
                visible: true,
                ..doc.layers[0].clone()
            };
            doc.set_layered(frames, vec![bottom], current, 0);
        }
        Command::SetPicture {
            width,
            height,
            layers,
            frames,
            current,
            active,
        } => {
            let frames = frames
                .iter()
                .map(|f| {
                    f.iter()
                        .map(|p| {
                            let picture = IndexedImage::from_pixels(*width, *height, p.clone());
                            Arc::new(picture.expect("size checked by validate"))
                        })
                        .collect()
                })
                .collect();
            doc.set_layered(frames, layers.clone(), *current as usize, *active as usize);
        }
        _ => unreachable!("not a layer command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, ErrorCode, Palette, Target};

    fn pic(w: u32, pixels: &[u8]) -> IndexedImage {
        IndexedImage::from_pixels(w, pixels.len() as u32 / w, pixels.to_vec()).unwrap()
    }

    #[test]
    fn flatten_lays_upper_layers_over_with_colour_0_transparent() {
        let bottom = pic(4, &[1, 1, 1, 1]);
        let middle = pic(4, &[0, 2, 2, 0]);
        let top = pic(4, &[0, 0, 3, 0]);
        assert_eq!(
            flatten(&[&bottom, &middle, &top], 4, 1).pixels(),
            [1, 2, 3, 1]
        );
        // Over a hidden bottom layer an upper layer's colour 0 shows colour 0.
        assert_eq!(flatten(&[&middle, &top], 4, 1).pixels(), [0, 2, 3, 0]);
        assert_eq!(flatten(&[], 4, 1).pixels(), [0, 0, 0, 0]);
        // The bottom layer's own colour 0 stays (it is opaque).
        assert_eq!(
            flatten(&[&pic(2, &[0, 5]), &pic(2, &[0, 0])], 2, 1).pixels(),
            [0, 5]
        );
    }

    #[test]
    fn names_defaults_and_copies() {
        let mut layers = vec![Layer::background()];
        assert_eq!(default_name(&layers), "Layer 1");
        layers.push(Layer::new("Layer 1"));
        layers.push(Layer::new("Layer 3"));
        assert_eq!(default_name(&layers), "Layer 2");
        assert_eq!(copy_name("Sky"), "Sky copy");
        let long = "x".repeat(MAX_NAME_CHARS);
        assert_eq!(copy_name(&long).chars().count(), MAX_NAME_CHARS);
        assert!(check_name(&long).is_ok());
        assert!(check_name(&format!("{long}y")).is_err());
        assert!(check_name("").is_ok());
        assert!(is_default_stack(&[Layer::background()]));
        assert!(!is_default_stack(&[Layer::new("Sky")]));
        assert!(!is_default_stack(&[Layer {
            visible: false,
            ..Layer::background()
        }]));
    }

    fn engine() -> Engine {
        Engine::new(Document::new(4, 2, Palette::dpaint_default_32()))
    }

    fn names(e: &Engine) -> Vec<String> {
        e.document()
            .layers()
            .iter()
            .map(|l| l.name.clone())
            .collect()
    }

    #[test]
    fn layer_commands_are_checked() {
        let mut e = engine();
        let bad = |e: &mut Engine, cmd: Command| e.execute(cmd).unwrap_err().code;
        assert_eq!(
            bad(&mut e, Command::DeleteLayer { layer: 0 }),
            ErrorCode::InvalidArgument,
            "the last layer stays"
        );
        assert_eq!(
            bad(&mut e, Command::MergeLayerDown { layer: 0 }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            bad(&mut e, Command::SelectLayer { layer: 1 }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            bad(
                &mut e,
                Command::RenameLayer {
                    layer: 0,
                    name: "x".repeat(256)
                }
            ),
            ErrorCode::InvalidArgument
        );
        for _ in 1..MAX_LAYERS {
            e.execute(Command::AddLayer { name: None }).unwrap();
        }
        assert_eq!(e.document().layer_count(), MAX_LAYERS);
        assert_eq!(
            bad(&mut e, Command::AddLayer { name: None }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            bad(&mut e, Command::DuplicateLayer { layer: 0 }),
            ErrorCode::InvalidArgument
        );
        assert_eq!(e.document().revision(), MAX_LAYERS as u64 - 1);
    }

    #[test]
    fn layers_count_against_the_frame_memory_limit() {
        // 2048x1024 pages hold 2 MiB: 64 pictures fit into 128 MiB, 21 layers of 3 frames.
        let mut e = Engine::new(Document::from_picture(
            IndexedImage::new(2048, 1024, 0),
            Palette::dpaint_default_32(),
            Target::default(),
        ));
        e.execute(Command::AddFrames { count: 2 }).unwrap();
        for _ in 1..21 {
            e.execute(Command::AddLayer { name: None }).unwrap();
        }
        let err = e.execute(Command::AddLayer { name: None }).unwrap_err();
        assert!(
            err.message.contains("21 layers") || err.message.contains("22 layers"),
            "{}",
            err.message
        );
        // Frames count too.
        assert!(e.execute(Command::AddFrames { count: 1 }).is_err());
        for l in (16..21).rev() {
            e.execute(Command::DeleteLayer { layer: l }).unwrap();
        }
        e.execute(Command::AddFrames { count: 1 }).unwrap();
        assert_eq!(e.document().frame_count(), 4);
        assert_eq!(e.document().layer_count(), 16);
        // A single frame of a single layer is always allowed.
        assert!(check_frame_count(1, 1, 1 << 20, 1 << 20).is_ok());
        assert!(check_frame_count(1, 2, 1 << 20, 1 << 20).is_err());
    }

    #[test]
    fn add_duplicate_move_and_delete_keep_the_active_layer_sensible() {
        let mut e = engine();
        e.execute(Command::Clear { color: 1 }).unwrap();
        e.execute(Command::AddLayer {
            name: Some("Sky".into()),
        })
        .unwrap();
        assert_eq!(names(&e), ["Background", "Sky"]);
        assert_eq!(e.document().active_layer(), 1);
        assert!(e.document().page().pixels().iter().all(|&p| p == 0));
        e.execute(Command::Clear { color: 2 }).unwrap();
        // Add goes above the active layer; the new one becomes active.
        e.execute(Command::SelectLayer { layer: 0 }).unwrap();
        e.execute(Command::AddLayer { name: None }).unwrap();
        assert_eq!(names(&e), ["Background", "Layer 1", "Sky"]);
        assert_eq!(e.document().active_layer(), 1);
        e.execute(Command::DuplicateLayer { layer: 2 }).unwrap();
        assert_eq!(names(&e), ["Background", "Layer 1", "Sky", "Sky copy"]);
        assert_eq!(e.document().active_layer(), 3);
        assert_eq!(e.document().page().get(0, 0), Some(2), "a copy of Sky");
        // Moving: the active layer follows its picture.
        e.execute(Command::MoveLayer { from: 3, to: 0 }).unwrap();
        assert_eq!(names(&e), ["Sky copy", "Background", "Layer 1", "Sky"]);
        assert_eq!(e.document().active_layer(), 0);
        e.execute(Command::SelectLayer { layer: 2 }).unwrap();
        e.execute(Command::MoveLayer { from: 0, to: 3 }).unwrap();
        assert_eq!(names(&e), ["Background", "Layer 1", "Sky", "Sky copy"]);
        assert_eq!(e.document().active_layer(), 1, "Layer 1 stays active");
        // Deleting the active layer activates the one below.
        e.execute(Command::DeleteLayer { layer: 1 }).unwrap();
        assert_eq!(names(&e), ["Background", "Sky", "Sky copy"]);
        assert_eq!(e.document().active_layer(), 0);
        e.execute(Command::SelectLayer { layer: 2 }).unwrap();
        e.execute(Command::DeleteLayer { layer: 0 }).unwrap();
        assert_eq!(names(&e), ["Sky", "Sky copy"]);
        assert_eq!(e.document().active_layer(), 1, "above the deleted one");
        e.execute(Command::DeleteLayer { layer: 1 }).unwrap();
        assert_eq!(e.document().active_layer(), 0);
        // Every step undoes back to the start.
        while e.can_undo() {
            e.undo().unwrap();
        }
        let start = engine();
        assert_eq!(e.document().layers(), start.document().layers());
        assert_eq!(e.document().page(), start.document().page());
    }

    #[test]
    fn colour_0_is_transparent_above_the_bottom_layer() {
        let mut e = engine();
        e.execute(Command::Clear { color: 4 }).unwrap();
        e.execute(Command::AddLayer { name: None }).unwrap();
        assert_eq!(e.document().composite().pixels(), [4; 8], "a new layer");
        e.execute(Command::Clear { color: 7 }).unwrap();
        assert_eq!(e.document().composite().pixels(), [7; 8]);
        // Painting with colour 0 erases the upper layer: the bottom one shows again.
        e.execute(Command::FloodFill {
            x: 0,
            y: 0,
            color: 0,
            fill: Default::default(),
        })
        .unwrap();
        assert_eq!(e.document().composite().pixels(), [4; 8]);
        // On the bottom layer colour 0 is a colour.
        e.execute(Command::SelectLayer { layer: 0 }).unwrap();
        e.execute(Command::Clear { color: 0 }).unwrap();
        assert_eq!(e.document().composite().pixels(), [0; 8]);
    }

    #[test]
    fn visibility_rename_merge_and_flatten() {
        let mut e = engine();
        e.execute(Command::Clear { color: 1 }).unwrap();
        e.execute(Command::AddLayer { name: None }).unwrap();
        e.execute(Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![0, 2, 2, 0, 0, 0, 0, 0],
        })
        .unwrap();
        e.execute(Command::AddLayer { name: None }).unwrap();
        e.execute(Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![0, 0, 3, 3, 0, 0, 0, 0],
        })
        .unwrap();
        let doc = e.document();
        assert_eq!(doc.composite().pixels(), [1, 2, 3, 3, 1, 1, 1, 1]);
        // Hidden layers do not show; a hidden bottom layer shows colour 0.
        e.execute(Command::SetLayerVisible {
            layer: 1,
            visible: false,
        })
        .unwrap();
        assert_eq!(e.document().composite().pixels(), [1, 1, 3, 3, 1, 1, 1, 1]);
        e.execute(Command::SetLayerVisible {
            layer: 0,
            visible: false,
        })
        .unwrap();
        assert_eq!(e.document().composite().pixels(), [0, 0, 3, 3, 0, 0, 0, 0]);
        // One visible layer is the picture itself: no copy.
        assert!(matches!(
            e.document().composite(),
            std::borrow::Cow::Borrowed(_)
        ));
        e.execute(Command::RenameLayer {
            layer: 2,
            name: "Top".into(),
        })
        .unwrap();
        assert_eq!(names(&e), ["Background", "Layer 1", "Top"]);
        e.undo().unwrap();
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(names(&e), ["Background", "Layer 1", "Layer 2"]);
        assert!(e.document().layers().iter().all(|l| l.visible));
        // Merge down: the upper layer's colours cover the lower one; it keeps its name.
        e.execute(Command::MergeLayerDown { layer: 2 }).unwrap();
        assert_eq!(names(&e), ["Background", "Layer 1"]);
        assert_eq!(e.document().active_layer(), 1);
        assert_eq!(e.document().page().pixels(), [0, 2, 3, 3, 0, 0, 0, 0]);
        assert_eq!(e.document().composite().pixels(), [1, 2, 3, 3, 1, 1, 1, 1]);
        e.undo().unwrap();
        // Flatten: what shows becomes the one layer; hidden layers go.
        e.execute(Command::SetLayerVisible {
            layer: 1,
            visible: false,
        })
        .unwrap();
        let shown = e.document().composite().into_owned();
        e.execute(Command::FlattenLayers).unwrap();
        assert_eq!(names(&e), ["Background"]);
        assert_eq!(e.document().page(), &shown);
        assert_eq!(e.document().page().pixels(), [1, 1, 3, 3, 1, 1, 1, 1]);
        e.undo().unwrap();
        assert_eq!(e.document().layer_count(), 3);
        // JSON names.
        let json = serde_json::to_string(&Command::FlattenLayers).unwrap();
        assert_eq!(json, r#"{"op":"flatten_layers"}"#);
        let cmd: Command = serde_json::from_str(r#"{"op":"add_layer"}"#).unwrap();
        assert_eq!(cmd, Command::AddLayer { name: None });
        let cmd: Command =
            serde_json::from_str(r#"{"op":"set_layer_visible","layer":1,"visible":false}"#)
                .unwrap();
        assert_eq!(
            cmd,
            Command::SetLayerVisible {
                layer: 1,
                visible: false
            }
        );
    }
    /// A 4x2 document of `frames` frames (frame f cleared to colour f + 1) with a second
    /// layer "Top", active.
    fn two_layers(frames: u32) -> Engine {
        let mut e = engine();
        e.execute(Command::Clear { color: 1 }).unwrap();
        if frames > 1 {
            e.execute(Command::SetFrameCount { count: frames }).unwrap();
            for f in 1..frames {
                e.execute(Command::GotoFrame { frame: f }).unwrap();
                e.execute(Command::Clear { color: f as u8 + 1 }).unwrap();
            }
            e.execute(Command::GotoFrame { frame: 0 }).unwrap();
        }
        e.execute(Command::AddLayer {
            name: Some("Top".into()),
        })
        .unwrap();
        e
    }

    fn dot(color: u8, at: (i32, i32)) -> Command {
        Command::Stroke {
            brush: vec![(0, 0)],
            color,
            points: vec![at],
            continuous: false,
            custom: None,
            mode: Default::default(),
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    #[test]
    fn undo_and_redo_go_back_to_the_layer_a_step_painted() {
        let mut e = two_layers(2);
        e.execute(dot(9, (1, 0))).unwrap();
        assert_eq!(e.document().layer(1).unwrap().get(1, 0), Some(9));
        assert_eq!(
            e.document().layer(0).unwrap().get(1, 0),
            Some(1),
            "bottom untouched"
        );
        // Selecting is a setting: no undo step, the redo list stays.
        e.execute(Command::SelectLayer { layer: 0 }).unwrap();
        e.execute(Command::GotoFrame { frame: 1 }).unwrap();
        let out = e.undo().unwrap();
        assert_eq!(
            (e.document().current_frame(), e.document().active_layer()),
            (0, 1)
        );
        assert_eq!(
            out.affected,
            Some(e.document().page().bounds()),
            "switched layer"
        );
        assert_eq!(e.document().page().get(1, 0), Some(0));
        e.execute(Command::SelectLayer { layer: 0 }).unwrap();
        assert!(e.can_redo());
        e.redo().unwrap();
        assert_eq!(e.document().active_layer(), 1);
        assert_eq!(e.document().page().get(1, 0), Some(9));
        assert_eq!(e.document().composite().get(1, 0), Some(9));
        assert_eq!(e.document().composite().get(0, 0), Some(1));
    }

    #[test]
    fn frame_commands_carry_every_layer() {
        let mut e = two_layers(2);
        e.execute(dot(7, (0, 0))).unwrap();
        // New frames copy all layers of the current frame.
        e.execute(Command::AddFrames { count: 1 }).unwrap();
        let d = e.document();
        assert_eq!(
            (d.frame_count(), d.current_frame(), d.active_layer()),
            (3, 1, 1)
        );
        assert_eq!(d.cel(1, 0).unwrap().get(1, 1), Some(1));
        assert_eq!(d.cel(1, 1).unwrap().get(0, 0), Some(7));
        assert_eq!(d.cel(2, 0).unwrap().get(1, 1), Some(2), "frame 2 moved on");
        assert_eq!(d.composite_frame(1).unwrap().get(0, 0), Some(7));
        // Deleting every frame leaves one: BG on the bottom layer, the upper ones clear.
        e.execute(Command::DeleteFrames {
            first: 0,
            last: 2,
            fill: 5,
        })
        .unwrap();
        let d = e.document();
        assert_eq!(
            (d.frame_count(), d.layer_count(), d.active_layer()),
            (1, 2, 1)
        );
        assert_eq!(d.cel(0, 0).unwrap().pixels(), [5; 8]);
        assert_eq!(d.cel(0, 1).unwrap().pixels(), [0; 8]);
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(e.document().frame_count(), 2);
        // Copy Frames copies every layer.
        e.execute(Command::CopyFrame { first: 1, last: 1 }).unwrap();
        assert_eq!(e.document().cel(1, 1).unwrap().get(0, 0), Some(7));
        assert_eq!(e.document().cel(1, 0).unwrap().get(1, 1), Some(1));
    }

    #[test]
    fn page_size_and_screen_format_change_every_layer() {
        use crate::DisplayMode;
        let mut e = Engine::new(Document::from_picture(
            IndexedImage::new(320, 256, 3),
            Palette::dpaint_default_32(),
            Target::default(),
        ));
        e.execute(Command::SetFrameCount { count: 2 }).unwrap();
        e.execute(Command::AddLayer { name: None }).unwrap();
        e.execute(Command::Clear { color: 31 }).unwrap();
        e.execute(Command::SetPageSize {
            width: 330,
            height: 260,
            fill: 4,
        })
        .unwrap();
        let d = e.document();
        for f in 0..2 {
            assert_eq!(d.cel(f, 0).unwrap().get(325, 258), Some(4), "BG below");
            assert_eq!(d.cel(f, 1).unwrap().get(325, 258), Some(0), "clear above");
            assert_eq!(d.composite_frame(f).unwrap().get(325, 258), Some(4));
        }
        assert_eq!(d.cel(0, 1).unwrap().get(0, 0), Some(31));
        // Retain keeps the layers; the colours of every layer count.
        let format = |colors, retain| Command::SetFormat {
            mode: DisplayMode::PAL_LORES,
            colors,
            width: 330,
            height: 260,
            retain,
            screen: None,
            chipset: None,
        };
        assert_eq!(
            e.execute(format(16, true)).unwrap_err().code,
            ErrorCode::PaletteRangeInvalid,
            "the upper layer uses colour 31"
        );
        e.execute(format(32, true)).unwrap();
        assert_eq!(e.document().layer_count(), 2);
        // Without retain the page is cleared to one Background layer.
        e.execute(format(16, false)).unwrap();
        let d = e.document();
        assert_eq!(d.layers(), [Layer::background()]);
        assert_eq!((d.frame_count(), d.active_layer()), (2, 0));
        assert!(d.cels().all(|c| c.pixels().iter().all(|&p| p == 0)));
        e.undo().unwrap();
        assert_eq!(e.document().layer_count(), 2);
        assert_eq!(e.document().active_layer(), 1);
        // Another page size by SetPage would split the layers: refused.
        let other = Command::SetPage {
            width: 320,
            height: 256,
            pixels: vec![0; 320 * 256],
        };
        e.execute(Command::DeleteFrames {
            first: 1,
            last: 1,
            fill: 0,
        })
        .unwrap();
        assert_eq!(
            e.execute(other).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn arrange_palette_keeps_colour_0_transparent_above_the_bottom_layer() {
        let mut e = two_layers(1);
        e.execute(dot(2, (0, 0))).unwrap();
        e.execute(dot(3, (1, 0))).unwrap();
        let shown = e.document().composite().into_owned();
        let looks = |e: &Engine| -> Vec<crate::Rgb8> {
            let d = e.document();
            d.composite()
                .pixels()
                .iter()
                .map(|&p| d.palette().get(p).unwrap())
                .collect()
        };
        let before = looks(&e);
        // Entries 0 and 1 swap: the bottom layer's colour 1 becomes 0, the top layer keeps
        // its transparent pixels.
        let mut order: Vec<u8> = (0..32).collect();
        order.swap(0, 1);
        e.execute(Command::ArrangePalette {
            order: order.clone(),
        })
        .unwrap();
        assert_eq!(looks(&e), before, "the picture looks the same");
        assert_eq!(e.document().layer(1).unwrap().pixels()[2..], [0; 6]);
        e.undo().unwrap();
        assert_eq!(e.document().composite().as_ref(), &shown);
        // Colour 3 becoming entry 0 would make the top layer's colour 3 transparent.
        let mut order: Vec<u8> = (0..32).collect();
        order.swap(0, 3);
        assert_eq!(
            e.execute(Command::ArrangePalette {
                order: order.clone()
            })
            .unwrap_err()
            .code,
            ErrorCode::PaletteRangeInvalid
        );
        // On the bottom layer it is fine.
        e.execute(Command::FlattenLayers).unwrap();
        e.execute(Command::ArrangePalette { order }).unwrap();
    }

    #[test]
    fn moves_and_animbrush_pick_ups_use_the_active_layer() {
        let mut e = two_layers(3);
        e.execute(dot(9, (2, 1))).unwrap();
        let brush = crate::anim_brush::pick_up(e.document(), crate::Rect::new(0, 0, 4, 2), 2, None)
            .unwrap();
        assert_eq!(brush.cels[0].pixels[6], 9, "frame 1 of the top layer");
        assert_eq!(
            brush.cels[1].pixels[0], 0,
            "frame 2 of the top layer is clear"
        );
        // The frames a page command sees are the active layer of every frame.
        let tops: Vec<u8> = e
            .document()
            .frames()
            .map(|f| f.get(0, 0).unwrap())
            .collect();
        assert_eq!(tops, [0, 0, 0]);
        let shown: Vec<u8> = e
            .document()
            .composite_frames()
            .map(|f| f.get(0, 0).unwrap())
            .collect();
        assert_eq!(shown, [1, 2, 3]);
    }

    #[test]
    fn set_picture_replaces_every_frame_and_layer_as_one_step() {
        // The source: 2 frames, 3 layers (the middle one hidden), frame 2 and layer 2 current.
        let mut src = engine();
        src.execute(Command::Clear { color: 1 }).unwrap();
        src.execute(Command::AddLayer {
            name: Some("Hills".into()),
        })
        .unwrap();
        src.execute(dot(2, (1, 1))).unwrap();
        src.execute(Command::SetLayerVisible {
            layer: 1,
            visible: false,
        })
        .unwrap();
        src.execute(Command::AddLayer {
            name: Some("Sun".into()),
        })
        .unwrap();
        src.execute(Command::AddFrames { count: 1 }).unwrap();
        src.execute(dot(3, (2, 0))).unwrap();
        let command = Command::set_picture(src.document());
        // The target: one plain frame and layer.
        let mut e = engine();
        e.execute(Command::Clear { color: 9 }).unwrap();
        let before = e.document().clone();
        e.execute(command.clone()).unwrap();
        let (d, s) = (e.document(), src.document());
        assert_eq!(d.layers(), s.layers());
        assert_eq!((d.frame_count(), d.current_frame()), (2, 1));
        assert_eq!(d.active_layer(), 2);
        assert_eq!(d.cels().collect::<Vec<_>>(), s.cels().collect::<Vec<_>>());
        assert_eq!(
            d.composite_frames().collect::<Vec<_>>(),
            s.composite_frames().collect::<Vec<_>>()
        );
        assert_eq!(d.palette(), before.palette());
        // One step: undo gives the old picture back, redo the new one.
        e.undo().unwrap();
        assert_eq!(e.document().cels().collect::<Vec<_>>(), [before.page()]);
        assert_eq!(e.document().layers(), before.layers());
        e.redo().unwrap();
        assert_eq!(e.document().layers(), src.document().layers());
        assert_eq!(
            e.document().cels().collect::<Vec<_>>(),
            src.document().cels().collect::<Vec<_>>()
        );
        e.undo().unwrap();
        e.undo().unwrap();
        assert!(!e.can_undo(), "the Clear and the new picture: two steps");
        // JSON.
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["op"], "set_picture");
        assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    }

    #[test]
    fn set_picture_is_checked() {
        let e = engine();
        let bad = |c: Command| {
            let mut e = e.clone();
            e.execute(c).unwrap_err().code
        };
        let picture = |layers: Vec<Layer>, frames: Vec<Vec<Vec<u8>>>, (w, h)| Command::SetPicture {
            width: w,
            height: h,
            layers,
            frames,
            current: 0,
            active: 0,
        };
        let one = || vec![Layer::background()];
        assert_eq!(
            bad(picture(vec![], vec![vec![]], (4, 2))),
            ErrorCode::InvalidArgument,
            "no layer"
        );
        assert_eq!(
            bad(picture(one(), vec![], (4, 2))),
            ErrorCode::InvalidArgument,
            "no frame"
        );
        assert_eq!(
            bad(picture(one(), vec![vec![vec![0; 8], vec![0; 8]]], (4, 2))),
            ErrorCode::InvalidArgument,
            "two pictures for one layer"
        );
        assert_eq!(
            bad(picture(one(), vec![vec![vec![0; 7]]], (4, 2))),
            ErrorCode::InvalidArgument,
            "a short picture"
        );
        assert_eq!(
            bad(picture(one(), vec![vec![vec![40; 8]]], (4, 2))),
            ErrorCode::PaletteRangeInvalid
        );
        assert_eq!(
            bad(picture(
                vec![Layer::new("x".repeat(256))],
                vec![vec![vec![0; 8]]],
                (4, 2)
            )),
            ErrorCode::InvalidArgument
        );
        // Another size must be a page the target allows (the 4x2 page itself is not one).
        assert_eq!(
            bad(picture(one(), vec![vec![vec![0; 4]]], (2, 2))),
            ErrorCode::InvalidArgument
        );
        let mut e = e.clone();
        let big = vec![vec![vec![3; 320 * 256]]];
        let out = e.execute(picture(one(), big, (320, 256))).unwrap();
        assert!(out.format_changed, "the page size changed");
        assert_eq!(e.document().page().width(), 320);
        e.undo().unwrap();
        assert_eq!(e.document().page().width(), 4);
        // The current frame and the active layer must exist.
        let mut c = picture(one(), vec![vec![vec![0; 8]]], (4, 2));
        if let Command::SetPicture { current, .. } = &mut c {
            *current = 1;
        }
        assert_eq!(bad(c), ErrorCode::InvalidArgument);
        let mut c = picture(one(), vec![vec![vec![0; 8]]], (4, 2));
        if let Command::SetPicture { active, .. } = &mut c {
            *active = 1;
        }
        assert_eq!(bad(c), ErrorCode::InvalidArgument);
    }

    #[test]
    fn merge_in_back_goes_behind_every_visible_layer() {
        // Bottom: 1 1 0 0 / 0 0 0 0; the active upper layer: 0 2 0 0 / 0 0 0 5.
        let mut e = engine();
        e.execute(Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![1, 1, 0, 0, 0, 0, 0, 0],
        })
        .unwrap();
        e.execute(Command::AddLayer { name: None }).unwrap();
        e.execute(Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![0, 2, 0, 0, 0, 0, 0, 5],
        })
        .unwrap();
        let merge = |front| Command::MergePage {
            width: 4,
            height: 2,
            pixels: vec![7; 8],
            front,
            transparent: 0,
        };
        // In back: only where the picture shows colour 0, not where the bottom layer shows
        // through the upper layer's transparent pixels.
        e.execute(merge(false)).unwrap();
        assert_eq!(e.document().page().pixels(), [0, 2, 7, 7, 7, 7, 7, 5]);
        assert_eq!(e.document().composite().pixels(), [1, 2, 7, 7, 7, 7, 7, 5]);
        assert_eq!(
            e.document().layer(0).unwrap().pixels(),
            [1, 1, 0, 0, 0, 0, 0, 0]
        );
        e.undo().unwrap();
        // In front: the other page covers the active layer.
        e.execute(merge(true)).unwrap();
        assert_eq!(e.document().composite().pixels(), [7; 8]);
        e.undo().unwrap();
        // A hidden active layer keeps what it holds where the picture shows colour 0.
        e.execute(Command::SetLayerVisible {
            layer: 1,
            visible: false,
        })
        .unwrap();
        e.execute(merge(false)).unwrap();
        assert_eq!(e.document().page().pixels(), [0, 2, 7, 7, 7, 7, 7, 5]);
    }

    #[test]
    fn helpers_for_arranging_and_merging_changes() {
        let pic = |p: &[u8]| IndexedImage::from_pixels(4, 1, p.to_vec()).unwrap();
        // Arrange: entries 0 and 3 change places; above the bottom layer 0 stays 0.
        let order = [3, 1, 2, 0];
        let bottom = crate::remap::arranged_picture(&pic(&[0, 1, 2, 3]), &order, false);
        assert_eq!(bottom.pixels(), [3, 1, 2, 0]);
        let upper = crate::remap::arranged_picture(&pic(&[0, 1, 2, 1]), &order, true);
        assert_eq!(upper.pixels(), [0, 1, 2, 1]);
        // Changes found on the composite go into the active layer, nothing else does.
        let active = pic(&[0, 5, 0, 0]);
        let before = pic(&[4, 5, 0, 4]);
        let after = pic(&[4, 5, 9, 6]);
        assert_eq!(
            with_changes(&active, &before, &after).pixels(),
            [0, 5, 9, 6]
        );
    }

    /// How long one composite takes (the screen makes one per redraw of a layered page).
    /// `cargo test [--release] -p dpx-core -- --ignored --nocapture composite_timing`.
    #[test]
    #[ignore = "timing"]
    fn composite_timing() {
        use std::time::Instant;
        for (w, h, layers) in [(320, 256, 4), (1008, 1024, 4), (4096, 4096, 4)] {
            let mut e = Engine::new(Document::from_picture(
                IndexedImage::new(w, h, 1),
                Palette::dpaint_default_32(),
                Target::default(),
            ));
            for l in 1..layers {
                e.execute(Command::AddLayer { name: None }).unwrap();
                e.execute(Command::Clear { color: l as u8 + 1 }).unwrap();
                e.execute(Command::FloodFill {
                    x: 0,
                    y: 0,
                    color: 0,
                    fill: Default::default(),
                })
                .unwrap();
                e.execute(dot(l as u8 + 1, (l, l))).unwrap();
            }
            let runs = 10;
            let t = Instant::now();
            for _ in 0..runs {
                std::hint::black_box(e.document().composite());
            }
            println!(
                "{w}x{h}, {layers} layers: {:.2} ms per composite",
                t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs)
            );
        }
    }

    #[test]
    fn a_picture_layer_holds_its_picture_in_the_current_frame_only() {
        // ADR-45: "New layer" for a generated picture, one undo step.
        let mut e = engine();
        e.execute(Command::AddFrames { count: 2 }).unwrap();
        e.execute(Command::GotoFrame { frame: 1 }).unwrap();
        e.execute(Command::Clear { color: 4 }).unwrap();
        let rev = e.document().revision();
        let pixels = vec![0, 5, 6, 7, 0, 0, 9, 9];
        e.execute(Command::AddPictureLayer {
            name: Some("Generated".into()),
            pixels: pixels.clone(),
        })
        .unwrap();
        let d = e.document();
        assert_eq!(d.revision(), rev + 1);
        assert_eq!(names(&e), ["Background", "Generated"]);
        assert_eq!((d.active_layer(), d.current_frame()), (1, 1));
        assert_eq!(d.page().pixels(), pixels);
        for f in [0, 2] {
            assert!(
                d.cel(f, 1).unwrap().pixels().iter().all(|&p| p == 0),
                "frame {f}"
            );
        }
        // Colour 0 is transparent: the composite shows the bottom layer there.
        assert_eq!(d.composite().pixels(), [4, 5, 6, 7, 4, 4, 9, 9]);
        e.undo().unwrap();
        assert_eq!(names(&e), ["Background"]);
        assert_eq!(e.document().page().pixels(), [4; 8]);
        e.redo().unwrap();
        assert_eq!(e.document().page().pixels(), pixels);
        // Without a name it is "Layer N", like add_layer.
        e.execute(Command::AddPictureLayer {
            name: None,
            pixels: vec![1; 8],
        })
        .unwrap();
        assert_eq!(names(&e), ["Background", "Generated", "Layer 1"]);
        let json = serde_json::to_value(Command::AddPictureLayer {
            name: None,
            pixels: vec![0],
        })
        .unwrap();
        assert_eq!(json["op"], "add_picture_layer");
    }

    #[test]
    fn a_picture_layer_is_checked() {
        let mut e = engine();
        let bad = |e: &mut Engine, pixels: Vec<u8>| {
            e.execute(Command::AddPictureLayer { name: None, pixels })
                .unwrap_err()
                .code
        };
        assert_eq!(bad(&mut e, vec![1; 7]), ErrorCode::InvalidArgument);
        assert_eq!(bad(&mut e, vec![32; 8]), ErrorCode::PaletteRangeInvalid);
        assert_eq!(
            e.execute(Command::AddPictureLayer {
                name: Some("x".repeat(256)),
                pixels: vec![1; 8],
            })
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        for _ in 1..MAX_LAYERS {
            e.execute(Command::AddLayer { name: None }).unwrap();
        }
        let err = e
            .execute(Command::AddPictureLayer {
                name: None,
                pixels: vec![1; 8],
            })
            .unwrap_err();
        assert!(err.message.contains("at most 32"), "{}", err.message);
        assert_eq!(e.document().layer_count(), MAX_LAYERS);
    }

    #[test]
    fn the_new_layer_preview_is_the_composite_after_add_picture_layer() {
        // [Background, Top] with Top's pixel 0 opaque, in every choice of active and
        // hidden layer: the new layer goes right above the active one, under Top.
        let picture = pic(4, &[0, 7, 7, 7, 0, 0, 7, 7]);
        for (active, top_visible, bottom_visible) in [
            (0, true, true),
            (1, true, true),
            (0, false, true),
            (0, true, false),
            (1, true, false),
        ] {
            let mut e = engine();
            e.execute(Command::Clear { color: 4 }).unwrap();
            e.execute(Command::AddLayer { name: None }).unwrap();
            e.execute(Command::SetPage {
                width: 4,
                height: 2,
                pixels: vec![9, 9, 0, 0, 0, 0, 0, 0],
            })
            .unwrap();
            e.execute(Command::SelectLayer { layer: active }).unwrap();
            for (layer, visible) in [(0, bottom_visible), (1, top_visible)] {
                e.execute(Command::SetLayerVisible { layer, visible })
                    .unwrap();
            }
            let shown = composite_with_new_layer(e.document(), &picture);
            e.execute(Command::AddPictureLayer {
                name: None,
                pixels: picture.pixels().to_vec(),
            })
            .unwrap();
            let case = (active, top_visible, bottom_visible);
            assert_eq!(&shown, e.document().composite().as_ref(), "{case:?}");
            if active == 0 && top_visible {
                assert_eq!(shown.pixels()[..2], [9, 9], "Top stays on top: {case:?}");
            }
        }
    }
}
