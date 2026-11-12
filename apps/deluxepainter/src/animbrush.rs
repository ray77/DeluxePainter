//! AnimBrush and Metamorph in the app (ADR-36): Anim ▸ AnimBrush (Pick up.., Load..,
//! Save.., Settings.., Use, Free), Brush ▸ Spare ▸ Metamorph.., Alt with the Brush Selector,
//! the keys `7` / `8` (previous / next cel), Shift+`7` / Shift+`8` (first / last cel),
//! Alt+`m` (Metamorph), Alt+`n` (Brush->Spare) and Alt+`b` (Brush<->Spare).
//!
//! The cels, their timing and the morph live in `dpx_core::anim_brush`; painting goes
//! through `Command::AnimBrushPaint`. This module keeps the AnimBrush, makes its current cel
//! the custom brush while it is in use, and moves its clock on after every committed stroke.
//! Whether it is in use is kept as a flag: picking up or loading another brush, a built-in
//! brush, Brush ▸ Restore of the picked-up brush, Brush<->Spare and the right-click away leave
//! the AnimBrush unused but kept, also when that brush looks like its current cel (Use, `B`
//! or a right-click on the Brush Selector bring it back).

use std::path::Path;

use dpx_core::anim_brush::{self, AnimBrush};
use dpx_core::{BrushImage, Command, Document, IndexedImage, Palette, Rect, brush_ops};
use dpx_iff::anim::{self as iff_anim, EncodeOptions};
use dpx_iff::ilbm::ColorRange;
use dpx_ui::{Action, AnimBrushReq, AnimBrushRequester, FileKind, PaintMode, Tool, ToolRequester};
use eframe::egui;

use crate::tools::{XformKind, pick_transparent};
use crate::{PainterApp, files, note_requester};

/// Title of the note when an AnimBrush cannot be picked up.
pub const NO_ANIM_BRUSH: &str = "No AnimBrush";
/// Title of the note after Anim ▸ AnimBrush ▸ Load..
pub const ANIM_BRUSH_LOADED: &str = "AnimBrush loaded";
/// Title of the note when an AnimBrush would hold more than [`anim_brush::MAX_CEL_BYTES`].
pub const ANIM_BRUSH_TOO_BIG: &str = "AnimBrush too big";
/// Cels Make AnimBrush suggests when the picture has a single frame (the manual gives no
/// default; with several frames it suggests one cel per frame).
pub const DEFAULT_MORPH_CELS: usize = 8;

/// The Brush menu items that change the brush at once; on an AnimBrush they change every cel
/// (Flip turns every frame of an AnimBrush, AGA p.244; Remap covers its cels too).
const TRANSFORMS: [&str; 15] = [
    "brush.flip_horizontal",
    "brush.flip_vertical",
    "brush.rotate_90",
    "brush.halve",
    "brush.double",
    "brush.double_horizontal",
    "brush.double_vertical",
    "brush.outline",
    "brush.trim",
    "brush.handle_center",
    "brush.handle_corner",
    "brush.bg_to_fg",
    "brush.swap_bg_fg",
    "brush.change_transparency",
    "brush.remap",
];

/// The app's AnimBrush state.
#[derive(Clone, Debug, Default)]
pub struct AnimBrushState {
    /// The AnimBrush (cels, settings, clock); `None` before one is made and after Free.
    pub brush: Option<AnimBrush>,
    /// The AnimBrush is the brush: set when it is made, loaded or brought back (Use, `B`,
    /// right-click), cleared when another brush becomes the brush
    /// ([`PainterApp::leave_anim_brush`]).
    in_use: bool,
    /// `picked_brush` when the AnimBrush was made or loaded: while it has not changed, the
    /// AnimBrush is the brush made last and `B` restores it (`B` brings back the last custom
    /// brush or AnimBrush, AGA p.293).
    made_after: Option<Option<BrushImage>>,
    /// Anim ▸ AnimBrush ▸ Pick up.. chose the Brush Selector for an AnimBrush pick-up.
    pub armed: bool,
    /// Alt is held in this pass.
    pub alt: bool,
    /// The frame waiting for the Pick up AnimBrush requester.
    pending: Option<Rect>,
    /// After Rotate ▸ Any Angle on the AnimBrush: the cels it made and those it was made
    /// from, so that a second use starts from the original orientation (as ADR-30 does for
    /// one brush).
    rotate_base: Option<(Vec<BrushImage>, Vec<BrushImage>)>,
    /// The AnimBrush file it was loaded from (Anim ▸ AnimBrush ▸ Load..): what Use Brush
    /// Palette takes (ADR-39); `None` for a picked-up or morphed AnimBrush.
    pub loaded: Option<LoadedAnimBrush>,
}

/// An AnimBrush as its file has it: the cels in the file's colour numbers, its palette (CMAP)
/// and colour ranges (CRNG), and the cels made from them for the picture (fitted to its
/// palette, AGA p.249). While the AnimBrush's cels are `fitted`, they come from the file
/// unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedAnimBrush {
    pub cels: Vec<BrushImage>,
    pub palette: Palette,
    pub ranges: Vec<ColorRange>,
    pub fitted: Vec<BrushImage>,
}

/// An AnimBrush file read for a picture ([`decode_anim_brush`]).
#[derive(Clone, Debug)]
pub struct DecodedAnimBrush {
    /// The AnimBrush for the picture (its colours fitted to the picture palette).
    pub brush: AnimBrush,
    /// How many colours fitting changed.
    pub changed: usize,
    /// The reader's notes.
    pub notes: Vec<String>,
    /// The file's own cels, palette and ranges.
    pub file: LoadedAnimBrush,
}

/// An AnimBrush as an IFF ANIM (the AnimBrush files of DeluxePaint are ANIMs, sample
/// extension `.anbr`, AGA p.249): one frame per cel on one canvas, the transparent colour in
/// BMHD masking 2 and the handle in GRAB of the first picture, the palette in CMAP, no
/// closing loop frames. Duration, direction and the current cel are not stored.
pub fn encode_anim_brush(brush: &AnimBrush, palette: &Palette) -> Result<Vec<u8>, String> {
    let canvas = brush.on_one_canvas(palette.len());
    let frames: Vec<IndexedImage> = canvas.cels.iter().map(BrushImage::to_image).collect();
    let first = &canvas.cels[0];
    let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let opts = EncodeOptions {
        loop_frames: false,
        transparent: first.transparent,
        grab: Some((clamp(first.handle.0), clamp(first.handle.1))),
        ..EncodeOptions::default()
    };
    iff_anim::encode(&frames, palette, &opts).map_err(|e| e.summary)
}

/// An AnimBrush from an IFF ANIM: one cel per frame (closing loop frames left out), handle
/// from GRAB (else the centre), transparent colour from BMHD masking 2 or 3. Its colours are
/// fitted to `page_palette` where the palettes differ (the picture palette stays, AGA
/// p.249); also how many colours changed, the reader's notes and the file's own cels,
/// palette and ranges (for Use Brush Palette).
pub fn decode_anim_brush(bytes: &[u8], page_palette: &Palette) -> Result<DecodedAnimBrush, String> {
    let (anim, notes) = iff_anim::read_with_notes(bytes).map_err(|e| e.summary)?;
    let frames = anim.logical_frames();
    if frames.len() < anim_brush::MIN_CELS {
        return Err("An AnimBrush needs 2 or more cels".into());
    }
    if frames.len() > anim_brush::MAX_CELS {
        return Err(format!("More than {} cels", anim_brush::MAX_CELS));
    }
    let (w, h) = (anim.width, anim.height);
    if w > dpx_core::command::MAX_BRUSH_SIDE || h > dpx_core::command::MAX_BRUSH_SIDE {
        return Err(format!("AnimBrush {w}x{h} is too big"));
    }
    let transparent = anim.transparent.and_then(|t| u8::try_from(t).ok());
    let handle = anim.grab.map_or((w as i32 / 2, h as i32 / 2), |(x, y)| {
        (
            i32::from(x).clamp(0, w as i32 - 1),
            i32::from(y).clamp(0, h as i32 - 1),
        )
    });
    let cels = frames
        .iter()
        .map(|f| BrushImage {
            handle,
            ..BrushImage::from_image(&f.image, transparent)
        })
        .collect();
    let brush = AnimBrush::new(cels);
    if brush.bytes() > anim_brush::MAX_CEL_BYTES {
        return Err("AnimBrush too big".into());
    }
    let (fitted, changed) = brush.remap(&anim.palette, page_palette);
    let file = LoadedAnimBrush {
        cels: brush.cels,
        palette: anim.palette,
        ranges: anim.ranges,
        fitted: fitted.cels.clone(),
    };
    Ok(DecodedAnimBrush {
        brush: fitted,
        changed,
        notes,
        file,
    })
}

/// `--animbrush-requester KIND` for snapshots: `pick` (Pick up AnimBrush for the frames from
/// the current one on), `make` (Make AnimBrush) or `settings` (AnimBrush Settings of a 20-cel
/// brush on its third cel, as the manual's figure 8.28).
pub fn snapshot_requester(args: &[String], doc: &Document) -> Option<ToolRequester> {
    let i = args.iter().position(|a| a == "--animbrush-requester")?;
    let req = match args.get(i + 1).map(String::as_str) {
        Some("pick") => {
            let (count, current) = (doc.frame_count(), doc.current_frame());
            AnimBrushRequester::pick_up(suggested_cels(count, current), count.max(2))
        }
        Some("make") => {
            AnimBrushRequester::make(morph_cels(doc.frame_count()), anim_brush::MAX_CELS)
        }
        Some("settings") => {
            AnimBrushRequester::settings(20, 20, 3, dpx_core::CelDirection::Forward)
        }
        _ => {
            eprintln!("--animbrush-requester expects pick, make or settings");
            std::process::exit(2);
        }
    };
    Some(ToolRequester::AnimBrush(req))
}

/// Cels the Pick up AnimBrush requester suggests: the frames from the current one to the
/// last (the tutorial expects 20 when picking up on frame 1 of 20, AGA p.164); on the last
/// frame (where that would be one) every frame.
fn suggested_cels(count: usize, current: usize) -> usize {
    let rest = count.saturating_sub(current);
    if rest >= anim_brush::MIN_CELS {
        rest
    } else {
        count
    }
    .min(anim_brush::MAX_CELS)
}

/// MiB of cels an AnimBrush may hold, for the notes.
const MAX_CEL_MIB: u64 = anim_brush::MAX_CEL_BYTES / (1024 * 1024);

/// The note when `cels` cels of a morph of `from` into `to` would hold too much.
fn morph_too_big(from: &BrushImage, to: &BrushImage, cels: usize) -> String {
    let c = anim_brush::morph_canvas(from, to);
    format!(
        "{cels} cels of {}x{} pixels (brush and spare brush around the handle) would hold \
         more than {MAX_CEL_MIB} MiB",
        c.w, c.h
    )
}

/// Cels Make AnimBrush suggests: one per frame, or [`DEFAULT_MORPH_CELS`] on one frame.
fn morph_cels(frames: usize) -> usize {
    if frames >= anim_brush::MIN_CELS {
        frames.min(anim_brush::MAX_CELS)
    } else {
        DEFAULT_MORPH_CELS
    }
}

impl PainterApp {
    /// Once per pass, before anything else: the menu flags, Alt, and a Pick up.. armed for a
    /// tool that is no longer the Brush Selector.
    pub(crate) fn animbrush_sync(&mut self, ctx: &egui::Context) {
        self.ui.has_anim_brush = self.animbrush.brush.is_some();
        self.ui.has_spare_brush = self.tools.spare_brush.is_some();
        // Brush ▸ Restore (`B`) brings back the AnimBrush made last, also before any plain
        // brush was picked up.
        self.ui.can_restore |= self.animbrush_is_last();
        self.animbrush.alt = ctx.input(|i| i.modifiers.alt);
        if self.animbrush.armed && self.ui.tool != Tool::BrushSelect {
            self.animbrush.armed = false;
        }
    }

    /// The AnimBrush is the brush. The flag decides; the custom brush must also be the
    /// current cel, so a path that replaces the brush without
    /// [`Self::leave_anim_brush`] can never make a stroke paint the cels.
    pub(crate) fn anim_brush_in_use(&self) -> bool {
        self.animbrush.in_use
            && self
                .animbrush
                .brush
                .as_ref()
                .is_some_and(|b| self.custom_brush.as_ref() == Some(b.current_cel()))
    }

    /// Another brush became the brush (a plain pick-up or load, Brush ▸ Restore of the
    /// picked-up brush, a built-in brush, Brush<->Spare): the AnimBrush is kept but not in
    /// use, even when that brush looks like its current cel.
    pub(crate) fn leave_anim_brush(&mut self) {
        self.animbrush.in_use = false;
    }

    /// Color ▸ Palette ▸ Arrange renumbered the colours with `lut` (`lut[old] = new`): the
    /// AnimBrush's cels, the brush it was made after and the Any Angle originals follow, so
    /// it stays in use and keeps its look (ADR-31: everything that holds colour numbers
    /// follows an Arrange).
    pub(crate) fn animbrush_arrange(&mut self, lut: &[u8; 256]) {
        let s = &mut self.animbrush;
        if let Some(b) = &mut s.brush {
            b.renumber(lut);
        }
        if let Some(Some(b)) = &mut s.made_after {
            anim_brush::renumber_cel(b, lut);
        }
        if let Some((made, original)) = &mut s.rotate_base {
            for c in made.iter_mut().chain(original.iter_mut()) {
                anim_brush::renumber_cel(c, lut);
            }
        }
    }

    /// The note that a change would make the AnimBrush hold too much.
    fn too_big_note(&mut self, note: String) {
        self.ui.note_requester = note_requester(ANIM_BRUSH_TOO_BIG, &[note]);
        self.dirty = true;
    }

    /// The AnimBrush a new gesture paints with: while it is in use and every cel's colours
    /// are in the palette (after a Screen Format with fewer colours the current cel alone
    /// paints, as a plain brush).
    pub(crate) fn animbrush_cels(&self) -> Option<AnimBrush> {
        let colors = self.engine.document().palette().len();
        let fits = |c: &BrushImage| {
            c.pixels.iter().all(|&p| usize::from(p) < colors)
                && c.transparent.is_none_or(|t| usize::from(t) < colors)
        };
        self.animbrush
            .brush
            .as_ref()
            .filter(|b| self.anim_brush_in_use() && b.cels.iter().all(fits))
            .cloned()
    }

    /// The AnimBrush was made after the last brush was picked up or loaded.
    fn animbrush_is_last(&self) -> bool {
        self.animbrush.brush.is_some()
            && self.animbrush.made_after.as_ref() == Some(&self.picked_brush)
    }

    /// A new AnimBrush (picked up, loaded or morphed) becomes the brush; the mode becomes
    /// Matte as after a brush pick-up.
    fn set_new_anim_brush(&mut self, brush: AnimBrush) {
        self.custom_brush = Some(brush.current_cel().clone());
        self.animbrush.brush = Some(brush);
        self.animbrush.in_use = true;
        self.animbrush.made_after = Some(self.picked_brush.clone());
        self.animbrush.rotate_base = None;
        // A load sets it again after this.
        self.animbrush.loaded = None;
        self.tools.brush_palette = Some(self.engine.document().palette().clone());
        self.ui.mode = PaintMode::Matte;
        self.dirty = true;
    }

    /// The changed AnimBrush stays the brush (it was in use).
    pub(crate) fn replace_anim_brush(&mut self, brush: AnimBrush) {
        self.custom_brush = Some(brush.current_cel().clone());
        self.animbrush.brush = Some(brush);
        self.animbrush.in_use = true;
        self.dirty = true;
    }

    /// Use Brush Palette gave the AnimBrush's cels new colour numbers (ADR-39): they replace
    /// its cels, it stays in use with its settings and clock, and the file it came from
    /// counts it as unchanged (a second Use Brush Palette starts from the file again).
    pub(crate) fn adopt_anim_brush_cels(&mut self, cels: Vec<BrushImage>) {
        let Some(b) = &self.animbrush.brush else {
            return;
        };
        let new = AnimBrush {
            cels: cels.clone(),
            ..b.clone()
        };
        if let Some(l) = &mut self.animbrush.loaded {
            l.fitted = cels;
        }
        self.animbrush.rotate_base = None;
        self.replace_anim_brush(new);
    }

    /// Anim ▸ AnimBrush ▸ Use (also `B` and the Brush Selector's right-click).
    fn use_anim_brush(&mut self) {
        if let Some(b) = &self.animbrush.brush {
            self.custom_brush = Some(b.current_cel().clone());
            self.animbrush.in_use = true;
            self.dirty = true;
        }
    }

    /// The menu ids this module answers: the AnimBrush items, Metamorph, Restore while the
    /// AnimBrush is the brush made last, and the instant Brush transforms while it is in use.
    pub(crate) fn animbrush_takes(&self, id: &str) -> bool {
        id.starts_with("animbrush.")
            || id == "brush.metamorph"
            || (id == "brush.restore" && self.animbrush_is_last())
            || (TRANSFORMS.contains(&id) && self.anim_brush_in_use())
    }

    /// Runs one of [`Self::animbrush_takes`]'s ids.
    pub(crate) fn animbrush_action(&mut self, id: &str) {
        self.finish_stroke();
        match id {
            "animbrush.pick_up" => self.arm_anim_pick_up(),
            "animbrush.use" | "brush.restore" => self.use_anim_brush(),
            "animbrush.free" => {
                if self.anim_brush_in_use() {
                    self.custom_brush = None;
                }
                self.animbrush = AnimBrushState::default();
                self.dirty = true;
            }
            "animbrush.settings" => {
                if let Some(b) = &self.animbrush.brush {
                    let req = AnimBrushRequester::settings(
                        b.len(),
                        b.duration,
                        b.current() + 1,
                        b.direction,
                    );
                    self.ui.tool_requester = Some(ToolRequester::AnimBrush(req));
                    self.dirty = true;
                }
            }
            "animbrush.load" => self.open_file_requester(FileKind::LoadAnimBrush),
            "animbrush.save" => {
                if self.animbrush.brush.is_some() {
                    self.open_file_requester(FileKind::SaveAnimBrush);
                }
            }
            "brush.metamorph" => self.open_metamorph(),
            id => self.anim_brush_transform(id),
        }
    }

    /// Brush ▸ Spare ▸ Metamorph..: the Make AnimBrush requester, its cel count limited to
    /// what fits [`anim_brush::MAX_CEL_BYTES`]; a note when not even two cels fit.
    fn open_metamorph(&mut self) {
        let (Some(from), Some(to)) = (&self.custom_brush, &self.tools.spare_brush) else {
            return;
        };
        let max = anim_brush::max_morph_cels(from, to);
        if max < anim_brush::MIN_CELS {
            let note = morph_too_big(from, to, anim_brush::MIN_CELS);
            return self.too_big_note(note);
        }
        let n = morph_cels(self.engine.document().frame_count());
        let req = AnimBrushRequester::make(n, max);
        self.ui.tool_requester = Some(ToolRequester::AnimBrush(req));
        self.dirty = true;
    }

    /// An instant Brush menu transform on every cel; the current cel stays current. A
    /// transform that would make the cels hold more than [`anim_brush::MAX_CEL_BYTES`] (Double,
    /// Outline) leaves the AnimBrush as it is and says so.
    fn anim_brush_transform(&mut self, id: &str) {
        let (fg, bg) = (self.ui.fg, self.ui.bg);
        let Some(b) = &self.animbrush.brush else {
            return;
        };
        let new = match id {
            // Trim only shrinks; with nothing left in a cel nothing happens, as for one brush.
            "brush.trim" => match b.try_map(brush_ops::trim) {
                Some(t) => t,
                None => return,
            },
            "brush.remap" => {
                let Some(from) = &self.tools.brush_palette else {
                    return;
                };
                let now = self.engine.document().palette().clone();
                let (new, _) = b.remap(from, &now);
                self.tools.brush_palette = Some(now);
                new
            }
            _ => {
                let f: Box<dyn Fn(&BrushImage) -> BrushImage> = match id {
                    "brush.flip_horizontal" => Box::new(brush_ops::flip_horizontal),
                    "brush.flip_vertical" => Box::new(brush_ops::flip_vertical),
                    "brush.rotate_90" => Box::new(brush_ops::rotate_90),
                    "brush.halve" => Box::new(brush_ops::halve),
                    "brush.double" => Box::new(|c| brush_ops::double(c, true, true)),
                    "brush.double_horizontal" => Box::new(|c| brush_ops::double(c, true, false)),
                    "brush.double_vertical" => Box::new(|c| brush_ops::double(c, false, true)),
                    "brush.outline" => Box::new(move |c| brush_ops::outline(c, fg)),
                    "brush.handle_center" => Box::new(brush_ops::handle_center),
                    "brush.handle_corner" => Box::new(brush_ops::handle_next_corner),
                    "brush.bg_to_fg" => Box::new(move |c| brush_ops::replace_color(c, bg, fg)),
                    "brush.swap_bg_fg" => Box::new(move |c| brush_ops::swap_colors(c, bg, fg)),
                    "brush.change_transparency" => {
                        Box::new(move |c| brush_ops::set_transparent(c, bg))
                    }
                    _ => return,
                };
                match b.map_within(f) {
                    Some(new) => new,
                    None => return self.cels_too_big_note(),
                }
            }
        };
        self.animbrush.rotate_base = None;
        self.replace_anim_brush(new);
    }

    fn cels_too_big_note(&mut self) {
        let note = format!(
            "The cels would hold more than {MAX_CEL_MIB} MiB: the AnimBrush stays as it was"
        );
        self.too_big_note(note);
    }

    /// The end of a Brush menu drag gesture (Stretch, Any Angle, Shear, Bend, Place) while the
    /// AnimBrush is in use: the same gesture changes every cel. Returns whether it did.
    pub(crate) fn animbrush_xform(&mut self, g: &crate::tools::Xform, colors: usize) -> bool {
        if !self.anim_brush_in_use() {
            return false;
        }
        let Some(b) = &self.animbrush.brush else {
            return false;
        };
        // A second Any Angle starts again from the cels before the first one.
        let bases = match (&self.animbrush.rotate_base, g.kind()) {
            (Some((made, original)), XformKind::Rotate) if *made == b.cels => original.clone(),
            _ => b.cels.clone(),
        };
        let base = AnimBrush {
            cels: bases,
            duration: b.duration,
            direction: b.direction,
            clock: b.clock,
        };
        // Within the byte limit only (Stretch can make every cel far bigger).
        let Some(new) = base.map_within(|c| g.apply_to(c, colors)) else {
            self.cels_too_big_note();
            return true;
        };
        self.animbrush.rotate_base =
            (g.kind() == XformKind::Rotate).then(|| (new.cels.clone(), base.cels));
        self.replace_anim_brush(new);
        true
    }

    /// Commits a gesture's command; after an AnimBrush stroke the AnimBrush's clock moves on by
    /// its stamps and the next cel becomes the brush (the next stroke, on this frame or
    /// another, goes on from there). The stroke may lie under an area stencil, and it may
    /// have gone to the Stencil ▸ Paint mask (ADR-39): it moves the AnimBrush on all the same.
    pub(crate) fn execute_painted(&mut self, cmd: Command) {
        let inner = match &cmd {
            Command::Stenciled { command, .. } => &**command,
            c => c,
        };
        let stamps = match inner {
            Command::AnimBrushPaint { .. } => inner.stamps(),
            _ => 0,
        };
        let in_use = self.anim_brush_in_use();
        let revisions = |app: &Self| {
            (
                app.engine.document().revision(),
                app.stencil_mask_revision(),
            )
        };
        let before = revisions(self);
        self.execute(cmd);
        if stamps > 0 && in_use && revisions(self) != before {
            self.advance_anim_brush(stamps);
        }
    }

    /// The AnimBrush in use moves on by `stamps` stamps: its next cel becomes the brush (also
    /// after a perspective stamp, ADR-37).
    pub(crate) fn advance_anim_brush(&mut self, stamps: u64) {
        if !self.anim_brush_in_use() {
            return;
        }
        if let Some(b) = &mut self.animbrush.brush {
            b.advance(stamps);
        }
        self.use_anim_brush();
    }

    /// Anim ▸ AnimBrush ▸ Pick up..: the Brush Selector picks up an AnimBrush next. One
    /// frame cannot make one (AGA p.250).
    fn arm_anim_pick_up(&mut self) {
        if self.engine.document().frame_count() < anim_brush::MIN_CELS {
            self.single_frame_note();
            return;
        }
        self.select_tool(Tool::BrushSelect, false);
        self.animbrush.armed = true;
    }

    fn single_frame_note(&mut self) {
        let note = ["Only one frame: an AnimBrush needs 2 or more (Anim ▸ Frames ▸ Set #)".into()];
        self.ui.note_requester = note_requester(NO_ANIM_BRUSH, &note);
        self.dirty = true;
    }

    /// The Brush Selector let go of frame `r` (on the page): with Alt held or after Pick
    /// up.., the Pick up AnimBrush requester asks for the cels (taken from the current frame
    /// on) and the AnimBrush is picked up after Ok. Returns whether this was such a pick-up.
    pub(crate) fn animbrush_take_pick(&mut self, r: Rect) -> bool {
        if !(self.animbrush.armed || self.animbrush.alt) {
            return false;
        }
        self.animbrush.armed = false;
        let doc = self.engine.document();
        let (count, current) = (doc.frame_count(), doc.current_frame());
        if count < anim_brush::MIN_CELS {
            self.single_frame_note();
            return true;
        }
        self.animbrush.pending = Some(r);
        let req = AnimBrushRequester::pick_up(
            suggested_cels(count, current),
            count.min(anim_brush::MAX_CELS),
        );
        self.ui.tool_requester = Some(ToolRequester::AnimBrush(req));
        self.dirty = true;
        true
    }

    /// Ok in an AnimBrush requester.
    pub(crate) fn animbrush_requester_ok(&mut self, r: AnimBrushRequester) {
        match r.kind() {
            AnimBrushReq::PickUp => {
                let Some(rect) = self.animbrush.pending.take() else {
                    return;
                };
                let doc = self.engine.document();
                let img = doc.page().crop(rect.intersect(doc.page().bounds()));
                let transparent = pick_transparent(&img, self.ui.prefs, self.ui.bg);
                if let Some(b) = anim_brush::pick_up(doc, rect, r.cels(), Some(transparent)) {
                    self.set_new_anim_brush(b);
                }
            }
            AnimBrushReq::Make => {
                let (Some(from), Some(to)) = (&self.custom_brush, &self.tools.spare_brush) else {
                    return;
                };
                let palette = self.engine.document().palette();
                match anim_brush::metamorph(from, to, r.cels(), palette) {
                    Some(b) => self.set_new_anim_brush(b),
                    None => {
                        let note = morph_too_big(from, to, r.cels());
                        self.too_big_note(note);
                    }
                }
            }
            AnimBrushReq::Settings => {
                let in_use = self.anim_brush_in_use();
                let Some(b) = &mut self.animbrush.brush else {
                    return;
                };
                b.set_direction(r.direction());
                b.set_duration(r.duration());
                b.set_current(r.current().saturating_sub(1));
                if in_use {
                    self.use_anim_brush();
                }
            }
        }
        self.dirty = true;
    }

    /// `B` (Brush ▸ Restore): the AnimBrush when it was made after the last brush was
    /// picked up. Returns whether it did that.
    pub(crate) fn animbrush_restore(&mut self) -> bool {
        if !self.animbrush_is_last() {
            return false;
        }
        self.use_anim_brush();
        true
    }

    /// Right-click on the Brush Selector while an AnimBrush exists: it switches between the
    /// plain custom brush and the AnimBrush (AGA p.251). From the AnimBrush it goes to the
    /// last picked-up brush (the built-in brush when there is none). Returns whether an
    /// AnimBrush exists.
    pub(crate) fn animbrush_toggle(&mut self) -> bool {
        if self.animbrush.brush.is_none() {
            return false;
        }
        if self.anim_brush_in_use() {
            self.custom_brush.clone_from(&self.picked_brush);
            self.leave_anim_brush();
            self.dirty = true;
        } else {
            self.use_anim_brush();
        }
        true
    }

    /// Steps the AnimBrush while it is in use: `to` maps the current cel and the cel count
    /// to the new current cel.
    fn step_cel(&mut self, to: impl Fn(usize, usize) -> usize) {
        if !self.anim_brush_in_use() {
            return;
        }
        if let Some(b) = &mut self.animbrush.brush {
            let n = b.len();
            b.set_current(to(b.current(), n));
        }
        self.use_anim_brush();
    }

    /// The AnimBrush keys (AGA p.296, p.251-252, p.292): `7` / `8` previous / next cel (they
    /// wrap; holding steps on), Shift+`7` / Shift+`8` first / last cel (the digit keys of the
    /// main keyboard, whatever the layout puts on them with Shift), Alt+`m` Metamorph, Alt+`n`
    /// Brush->Spare, Alt+`b` Brush<->Spare.
    pub(crate) fn animbrush_keys(&mut self, ctx: &egui::Context) {
        if self.typing() || self.modal_open() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat,
                modifiers,
            } = event
            else {
                continue;
            };
            // The digit keys by their place (Shift changes what they type); letters as typed.
            let k = match physical_key {
                Some(p @ (egui::Key::Num7 | egui::Key::Num8)) => p,
                _ => key,
            };
            let plain = !modifiers.any();
            let shift = modifiers.shift && !(modifiers.alt || modifiers.ctrl || modifiers.command);
            let alt = modifiers.alt && !(modifiers.shift || modifiers.ctrl || modifiers.command);
            match k {
                egui::Key::Num7 if plain => self.step_cel(|c, n| (c + n - 1) % n),
                egui::Key::Num8 if plain => self.step_cel(|c, n| (c + 1) % n),
                _ if repeat => {}
                egui::Key::Num7 if shift => self.step_cel(|_, _| 0),
                egui::Key::Num8 if shift => self.step_cel(|_, n| n - 1),
                egui::Key::M | egui::Key::N | egui::Key::B if alt => {
                    let id = match k {
                        egui::Key::M => "brush.metamorph",
                        egui::Key::N => "brush.to_spare",
                        _ => "brush.swap_spare",
                    };
                    // Keep the menu flags current for the check (Alt+n may come first).
                    self.ui.has_brush = self.custom_brush.is_some();
                    self.ui.has_spare_brush = self.tools.spare_brush.is_some();
                    // Shortcut keys keep Stencil ▸ Paint on (ADR-39): the brush changes, the
                    // mask stays.
                    if Action::Command(id).enabled(&self.ui.menu_context()) {
                        self.dispatch_action(Action::Command(id), ctx);
                    }
                    if self.modal_open() {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    /// The file requester's answer for Anim ▸ AnimBrush ▸ Load.. and Save..
    pub(crate) fn animbrush_file_accept(&mut self, kind: FileKind, path: &Path, overwrite: bool) {
        match kind {
            FileKind::LoadAnimBrush => self.load_anim_brush(path),
            FileKind::SaveAnimBrush => self.save_anim_brush(path, overwrite),
            _ => {}
        }
    }

    fn load_anim_brush(&mut self, path: &Path) {
        let palette = self.engine.document().palette().clone();
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| decode_anim_brush(&b, &palette));
        match result {
            Ok(DecodedAnimBrush {
                brush,
                changed,
                mut notes,
                file,
            }) => {
                self.ui.file_requester = None;
                self.set_new_anim_brush(brush);
                self.animbrush.loaded = Some(file);
                notes
                    .iter()
                    .for_each(|n| eprintln!("{}: {n}", path.display()));
                if changed > 0 {
                    notes.push(format!(
                        "{changed} AnimBrush colours were fitted to the picture palette"
                    ));
                }
                self.ui.note_requester = note_requester(ANIM_BRUSH_LOADED, &notes);
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e);
            }
        }
    }

    fn save_anim_brush(&mut self, path: &Path, overwrite: bool) {
        if path.is_dir() {
            return self.requester_message("That is a drawer".into());
        }
        if path.exists() && !overwrite {
            if let Some(req) = &mut self.ui.file_requester {
                req.ask_overwrite();
            }
            return;
        }
        let Some(brush) = &self.animbrush.brush else {
            return;
        };
        let bytes = encode_anim_brush(brush, self.engine.document().palette());
        match bytes.and_then(|b| files::write_atomic(path, &b)) {
            Ok(()) => self.ui.file_requester = None,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e);
            }
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{key_event, menu_path, ui_pass, ui_point};
    use dpx_core::CelDirection;
    use dpx_ui::Hit;
    use egui::PointerButton::{Primary, Secondary};

    const ALT: egui::Modifiers = egui::Modifiers::ALT;
    const NONE: egui::Modifiers = egui::Modifiers::NONE;

    /// One `ui()` pass with `modifiers` held (as egui-winit reports them, before the
    /// pass's other events).
    fn pass(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        let mut all = vec![egui::Event::ModifiersChanged(modifiers)];
        all.extend(events);
        ui_pass(app, ctx, t, all);
    }

    fn button(
        pos: egui::Pos2,
        button: egui::PointerButton,
        pressed: bool,
        modifiers: egui::Modifiers,
    ) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers,
        }
    }

    /// `key` pressed and let go (egui counts a second press without a release as a repeat).
    fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        [true, false]
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            })
            .to_vec()
    }

    fn with(mut events: Vec<egui::Event>, more: egui::Event) -> Vec<egui::Event> {
        events.push(more);
        events
    }

    fn text(s: &str) -> egui::Event {
        egui::Event::Text(s.into())
    }

    /// The window point over page pixel `p` (lores screen, not scrolled).
    fn page_pos(app: &PainterApp, p: (i32, i32)) -> egui::Pos2 {
        let v = app.view();
        assert_eq!((v.ux, v.uy, app.scroll), (1, 1, (0, 0)));
        ui_point(app, p)
    }

    /// A left-button drag on the page along `path` (one move per pass), `modifiers` held
    /// throughout; returns the time after it.
    fn drag(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        path: &[(i32, i32)],
        modifiers: egui::Modifiers,
    ) -> f64 {
        let points: Vec<egui::Pos2> = path.iter().map(|&p| page_pos(app, p)).collect();
        let mut t = t;
        let moved = egui::Event::PointerMoved;
        pass(app, ctx, t, vec![moved(points[0])], modifiers);
        t += 0.02;
        let down = button(points[0], Primary, true, modifiers);
        pass(app, ctx, t, vec![down], modifiers);
        for &p in &points[1..] {
            t += 0.02;
            pass(app, ctx, t, vec![moved(p)], modifiers);
        }
        t += 0.02;
        let last = *points.last().unwrap();
        pass(
            app,
            ctx,
            t,
            vec![button(last, Primary, false, modifiers)],
            modifiers,
        );
        t + 0.02
    }

    /// Picks menu item `id` with the right button (held on the title, let go on the item).
    fn menu_pick(app: &mut PainterApp, ctx: &egui::Context, t: f64, id: &'static str) -> f64 {
        let [title, item, sub] = menu_path(app, id).map(|p| ui_point(app, p));
        let moved = egui::Event::PointerMoved;
        let down = button(title, Secondary, true, NONE);
        pass(app, ctx, t, vec![moved(title), down], NONE);
        pass(app, ctx, t + 0.02, vec![moved(item)], NONE);
        pass(app, ctx, t + 0.04, vec![moved(sub)], NONE);
        let up = button(sub, Secondary, false, NONE);
        pass(app, ctx, t + 0.06, vec![up], NONE);
        t + 0.08
    }

    /// A right-click on the Brush Selector in the toolbox.
    fn right_click_brush_selector(app: &mut PainterApp, ctx: &egui::Context, t: f64) -> f64 {
        let view = app.view();
        let (w, h) = view.ui_size();
        let screen = view.ui_screen();
        let at = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .find(|&(x, y)| {
                matches!(
                    screen.hit_test(app.palette().len(), &app.ui, x, y),
                    Hit::Tool {
                        tool: Tool::BrushSelect,
                        ..
                    }
                )
            })
            .expect("the Brush Selector");
        let p = ui_point(app, at);
        pass(app, ctx, t, vec![egui::Event::PointerMoved(p)], NONE);
        pass(
            app,
            ctx,
            t + 0.02,
            vec![button(p, Secondary, true, NONE)],
            NONE,
        );
        pass(
            app,
            ctx,
            t + 0.04,
            vec![button(p, Secondary, false, NONE)],
            NONE,
        );
        t + 0.06
    }

    /// `n` frames; frame `i` shows a 5x5 square of colour `i + 1` at (40, 40) on colour 0.
    fn frames(n: u32) -> PainterApp {
        let mut app = PainterApp::new();
        app.engine
            .execute(Command::SetFrameCount { count: n })
            .unwrap();
        for frame in 0..n {
            app.engine.execute(Command::GotoFrame { frame }).unwrap();
            let mut pixels = vec![0; 45 * 45];
            for y in 40..45 {
                for x in 40..45 {
                    pixels[y * 45 + x] = frame as u8 + 1;
                }
            }
            app.engine
                .execute(Command::MergePage {
                    width: 45,
                    height: 45,
                    pixels,
                    front: true,
                    transparent: 0,
                })
                .unwrap();
        }
        app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
        app
    }

    fn solid(w: u32, h: u32, c: u8) -> BrushImage {
        BrushImage {
            width: w,
            height: h,
            pixels: vec![c; (w * h) as usize],
            handle: (w as i32 / 2, h as i32 / 2),
            transparent: Some(0),
        }
    }

    /// An AnimBrush of `n` one-pixel cels of colours 1, 2, ...
    fn dots(n: u8) -> AnimBrush {
        AnimBrush::new((1..=n).map(|c| solid(1, 1, c)).collect())
    }

    fn requester(app: &PainterApp) -> &AnimBrushRequester {
        match &app.ui.tool_requester {
            Some(ToolRequester::AnimBrush(r)) => r,
            _ => panic!("no AnimBrush requester"),
        }
    }

    #[test]
    fn alt_drag_with_the_brush_selector_picks_up_an_animbrush_from_this_frame_on() {
        let ctx = egui::Context::default();
        let mut app = frames(4);
        app.engine.execute(Command::GotoFrame { frame: 1 }).unwrap();
        let before = app.page().into_owned();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.1, vec![text("b")], NONE);
        assert_eq!(app.ui.tool, Tool::BrushSelect);
        let t = drag(&mut app, &ctx, 0.2, &[(37, 37), (47, 47)], ALT);
        let r = requester(&app);
        assert_eq!(r.kind(), AnimBrushReq::PickUp);
        assert_eq!(r.cels(), 3, "frames 2 to 4");
        assert_ne!(app.ui.tool, Tool::BrushSelect, "back to the tool before");
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Enter, true)]);
        let b = app.animbrush.brush.clone().expect("an AnimBrush");
        assert_eq!(b.len(), 3);
        for (i, cel) in b.cels.iter().enumerate() {
            assert_eq!(cel.transparent, Some(0));
            assert!(cel.pixels.contains(&(i as u8 + 2)), "cel {i}");
        }
        assert!(app.anim_brush_in_use());
        assert_eq!(app.ui.mode, PaintMode::Matte);
        assert_eq!(&*app.page(), &before, "picking up an AnimBrush never cuts");
        assert_eq!(app.engine.document().current_frame(), 1);
        // The menu items that need an AnimBrush are on now.
        ui_pass(&mut app, &ctx, t + 0.1, vec![]);
        assert!(Action::Command("animbrush.settings").enabled(&app.ui.menu_context()));
        // Without Alt the Brush Selector picks up a plain brush; the AnimBrush stays.
        pass(&mut app, &ctx, t + 0.2, vec![text("b")], NONE);
        drag(&mut app, &ctx, t + 0.3, &[(38, 38), (46, 46)], NONE);
        assert!(app.ui.tool_requester.is_none());
        assert!(!app.anim_brush_in_use());
        assert!(app.custom_brush.as_ref().unwrap().pixels.contains(&2));
        assert_eq!(app.animbrush.brush.as_ref(), Some(&b));
    }

    #[test]
    fn pick_up_from_the_menu_arms_the_brush_selector_and_one_frame_says_why_not() {
        let ctx = egui::Context::default();
        let mut app = frames(3);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let t = menu_pick(&mut app, &ctx, 0.1, "animbrush.pick_up");
        assert_eq!(app.ui.tool, Tool::BrushSelect);
        let t = drag(&mut app, &ctx, t, &[(39, 39), (45, 45)], NONE);
        assert_eq!(requester(&app).cels(), 3, "all frames from frame 1");
        // Esc: no AnimBrush.
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Escape, true)]);
        assert!(app.ui.tool_requester.is_none());
        assert!(app.animbrush.brush.is_none());
        // Armed once only: the pick-up after the next one is a plain brush.
        let t = menu_pick(&mut app, &ctx, t + 0.1, "animbrush.pick_up");
        let t = drag(&mut app, &ctx, t, &[(39, 39), (45, 45)], NONE);
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Enter, true)]);
        assert_eq!(app.animbrush.brush.as_ref().map(AnimBrush::len), Some(3));
        pass(&mut app, &ctx, t + 0.1, vec![text("b")], NONE);
        drag(&mut app, &ctx, t + 0.2, &[(38, 38), (46, 46)], NONE);
        assert!(app.ui.tool_requester.is_none() && !app.anim_brush_in_use());

        // One frame: the menu item and Alt with the Brush Selector both explain.
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 1.0, vec![]);
        menu_pick(&mut app, &ctx, 1.1, "animbrush.pick_up");
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.title(), NO_ANIM_BRUSH);
        assert_ne!(app.ui.tool, Tool::BrushSelect);
        pass(&mut app, &ctx, 1.3, vec![text("b")], NONE);
        drag(&mut app, &ctx, 1.4, &[(39, 39), (45, 45)], ALT);
        assert_eq!(app.ui.note_requester.take().unwrap().title(), NO_ANIM_BRUSH);
        assert!(app.animbrush.brush.is_none() && app.custom_brush.is_none());
    }

    #[test]
    fn keys_7_and_8_step_through_the_cels_and_shift_jumps_to_the_ends() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        app.set_new_anim_brush(dots(5));
        let current = |app: &PainterApp| {
            let b = app.animbrush.brush.as_ref().unwrap();
            assert_eq!(app.custom_brush.as_ref(), Some(b.current_cel()));
            b.current()
        };
        let shift = egui::Modifiers::SHIFT;
        let steps = [
            (with(key(egui::Key::Num8, NONE), text("8")), 1),
            (with(key(egui::Key::Num7, NONE), text("7")), 0),
            (with(key(egui::Key::Num7, NONE), text("7")), 4),
            (with(key(egui::Key::Num8, NONE), text("8")), 0),
            (with(key(egui::Key::Num8, shift), text("*")), 4),
            (with(key(egui::Key::Num7, shift), text("&")), 0),
        ];
        let mut t = 0.1;
        for (events, want) in steps {
            pass(&mut app, &ctx, t, events, NONE);
            assert_eq!(current(&app), want, "at {t}");
            t += 0.1;
        }
        // Held: the key repeats (pressed again without a release) and keeps stepping.
        let held = key(egui::Key::Num8, NONE)[0].clone();
        pass(&mut app, &ctx, t, vec![held.clone(), held], NONE);
        assert_eq!(current(&app), 2);
        let up = key_event(egui::Key::Num8, false);
        pass(&mut app, &ctx, t + 0.05, vec![up], NONE);
        // Shift+8 where the layout makes it another key (egui's `key`) still goes by the
        // key's place.
        let shifted = egui::Event::Key {
            key: egui::Key::OpenBracket,
            physical_key: Some(egui::Key::Num8),
            pressed: true,
            repeat: false,
            modifiers: shift,
        };
        pass(&mut app, &ctx, t + 0.1, vec![shifted], NONE);
        assert_eq!(current(&app), 4);
        // Only while the AnimBrush is the brush.
        app.custom_brush = None;
        pass(&mut app, &ctx, t + 0.2, key(egui::Key::Num8, NONE), NONE);
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 4);
        assert!(app.custom_brush.is_none());
    }

    #[test]
    fn painting_stamps_one_cel_after_the_other_and_goes_on_across_frames() {
        let ctx = egui::Context::default();
        let mut app = frames(2);
        app.run(Command::Clear { color: 0 });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.1, vec![text("s")], NONE);
        assert_eq!(app.ui.tool, Tool::DottedFreehand);
        app.set_new_anim_brush(dots(3));
        let mut t = 0.2;
        for (i, x) in [40, 50, 60, 70].into_iter().enumerate() {
            t = drag(&mut app, &ctx, t, &[(x, 40)], NONE);
            assert_eq!(app.page().get(x, 40), Some(i as u8 % 3 + 1), "click {i}");
        }
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 1);
        // Undo takes back one stamp; the cels go on regardless.
        app.undo();
        assert_eq!(app.page().get(70, 40), Some(0));
        // `2`: the next frame; the AnimBrush goes on with the next cel.
        pass(
            &mut app,
            &ctx,
            t,
            with(key(egui::Key::Num2, NONE), text("2")),
            NONE,
        );
        assert_eq!(app.engine.document().current_frame(), 1);
        let t = drag(&mut app, &ctx, t + 0.1, &[(30, 30)], NONE);
        assert_eq!(app.page().get(30, 30), Some(2));

        // A continuous freehand stroke: the committed stroke is the AnimBrush stroke over the
        // whole path, from the clock at its start.
        pass(&mut app, &ctx, t, vec![text("d")], NONE);
        let start = app.animbrush.brush.clone().unwrap();
        let doc = app.engine.document().clone();
        let path: Vec<(i32, i32)> = (0..8).map(|i| (20 + i * 3, 60 + i)).collect();
        drag(&mut app, &ctx, t + 0.1, &path, NONE);
        let stroke = Command::Stroke {
            brush: Vec::new(),
            color: app.ui.fg,
            points: path,
            continuous: true,
            custom: Some(start.current_cel().clone()),
            mode: dpx_core::DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        };
        let cmd = Command::AnimBrushPaint {
            brush: start.clone(),
            paint: Box::new(stroke),
        };
        let mut want = start.clone();
        want.advance(cmd.stamps());
        let mut reference = dpx_core::Engine::new(doc);
        reference.execute(cmd).unwrap();
        assert_eq!(&*app.page(), reference.document().page());
        assert_eq!(app.animbrush.brush.as_ref().unwrap().clock, want.clock);
        let shown: std::collections::BTreeSet<u8> = (20..45)
            .flat_map(|x| (60..68).map(move |y| (x, y)))
            .filter_map(|(x, y)| app.page().get(x, y))
            .filter(|&p| p != 0)
            .collect();
        assert_eq!(shown.len(), 3, "every cel shows: {shown:?}");
    }

    #[test]
    fn b_and_a_right_click_on_the_brush_selector_bring_back_the_animbrush() {
        let ctx = egui::Context::default();
        let mut app = frames(2);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // A plain brush first, then an AnimBrush (made last).
        pass(&mut app, &ctx, 0.1, vec![text("b")], NONE);
        let t = drag(&mut app, &ctx, 0.2, &[(39, 39), (45, 45)], NONE);
        let plain = app.custom_brush.clone().expect("a brush");
        pass(&mut app, &ctx, t, vec![text("b")], NONE);
        let t = drag(&mut app, &ctx, t + 0.1, &[(39, 39), (45, 45)], ALT);
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.anim_brush_in_use());
        // Right-click: to the plain brush and back.
        let t = right_click_brush_selector(&mut app, &ctx, t + 0.1);
        assert_eq!(app.custom_brush.as_ref(), Some(&plain));
        let t = right_click_brush_selector(&mut app, &ctx, t);
        assert!(app.anim_brush_in_use());
        // The built-in brush, then `B`: the AnimBrush was made last.
        app.custom_brush = None;
        pass(&mut app, &ctx, t, vec![text("B")], egui::Modifiers::SHIFT);
        assert!(app.anim_brush_in_use());
        // A plain brush picked up after it: `B` restores that one.
        pass(&mut app, &ctx, t + 0.1, vec![text("b")], NONE);
        let t = drag(&mut app, &ctx, t + 0.2, &[(40, 40), (44, 44)], NONE);
        let newer = app.custom_brush.clone().unwrap();
        app.custom_brush = None;
        pass(&mut app, &ctx, t, vec![text("B")], egui::Modifiers::SHIFT);
        assert_eq!(app.custom_brush.as_ref(), Some(&newer));
        // Anim ▸ AnimBrush ▸ Use still brings it back; Free drops it for good.
        let t = menu_pick(&mut app, &ctx, t + 0.1, "animbrush.use");
        assert!(app.anim_brush_in_use());
        let t = menu_pick(&mut app, &ctx, t, "animbrush.free");
        assert!(app.animbrush.brush.is_none());
        assert!(app.custom_brush.is_none(), "the built-in brush");
        ui_pass(&mut app, &ctx, t, vec![]);
        for id in ["animbrush.use", "animbrush.free", "animbrush.settings"] {
            assert!(!Action::Command(id).enabled(&app.ui.menu_context()), "{id}");
        }
    }

    #[test]
    fn the_settings_requester_sets_duration_current_cel_and_direction() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        app.set_new_anim_brush(dots(5));
        ui_pass(&mut app, &ctx, 0.05, vec![]);
        let t = menu_pick(&mut app, &ctx, 0.1, "animbrush.settings");
        let r = requester(&app);
        assert_eq!(r.kind(), AnimBrushReq::Settings);
        assert_eq!((r.duration(), r.current()), (5, 1));
        assert_eq!(r.direction(), CelDirection::Forward);
        // Duration up to 6, Tab, Current up to 3, click the ping-pong button, Return.
        let up = || key_event(egui::Key::ArrowUp, true);
        pass(&mut app, &ctx, t, vec![up()], NONE);
        let tab = key_event(egui::Key::Tab, true);
        pass(&mut app, &ctx, t + 0.02, vec![tab], NONE);
        pass(&mut app, &ctx, t + 0.04, vec![up(), up()], NONE);
        let req = app.ui.tool_requester.as_ref().unwrap();
        let (w, h) = app.view().ui_size();
        let (ox, oy) = req.origin(w, h);
        let r = AnimBrushRequester::direction_rect(CelDirection::PingPong);
        let p = ui_point(&app, (ox + r.x + r.w / 2, oy + r.y + r.h / 2));
        pass(
            &mut app,
            &ctx,
            t + 0.06,
            vec![egui::Event::PointerMoved(p)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            t + 0.08,
            vec![button(p, Primary, true, NONE)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            t + 0.1,
            vec![button(p, Primary, false, NONE)],
            NONE,
        );
        assert_eq!(requester(&app).direction(), CelDirection::PingPong);
        ui_pass(
            &mut app,
            &ctx,
            t + 0.12,
            vec![key_event(egui::Key::Enter, true)],
        );
        assert!(app.ui.tool_requester.is_none());
        let b = app.animbrush.brush.as_ref().unwrap();
        let settings = (b.duration, b.current(), b.direction);
        assert_eq!(settings, (6, 2, CelDirection::PingPong));
        assert_eq!(app.custom_brush.as_ref(), Some(&b.cels[2]));
    }

    #[test]
    fn alt_n_alt_m_morph_the_brush_into_the_spare_and_alt_b_reverses() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let small = solid(3, 3, 1);
        let big = solid(7, 7, 2);
        app.custom_brush = Some(small.clone());
        app.picked_brush = Some(small.clone());
        // Alt+n: Brush->Spare; another brush; Alt+m: Make AnimBrush (8 cels on one frame).
        pass(
            &mut app,
            &ctx,
            0.1,
            with(key(egui::Key::N, ALT), text("~")),
            ALT,
        );
        assert_eq!(app.tools.spare_brush.as_ref(), Some(&small));
        app.custom_brush = Some(big.clone());
        app.picked_brush = Some(big.clone());
        pass(
            &mut app,
            &ctx,
            0.2,
            with(key(egui::Key::M, ALT), text("µ")),
            ALT,
        );
        let r = requester(&app);
        assert_eq!(
            (r.kind(), r.cels()),
            (AnimBrushReq::Make, DEFAULT_MORPH_CELS)
        );
        let down = key_event(egui::Key::ArrowDown, true);
        pass(&mut app, &ctx, 0.3, vec![down.clone(), down], NONE);
        ui_pass(&mut app, &ctx, 0.4, vec![key_event(egui::Key::Enter, true)]);
        let b = app.animbrush.brush.clone().expect("an AnimBrush");
        assert_eq!(b.len(), 6);
        let colours = |c: &BrushImage| {
            c.pixels
                .iter()
                .copied()
                .filter(|&p| Some(p) != c.transparent)
                .collect::<std::collections::BTreeSet<u8>>()
        };
        // From the brush (current) to the spare (AGA addendum p.11-12).
        assert_eq!(colours(&b.cels[0]), [2].into());
        assert_eq!(colours(&b.cels[5]), [1].into());
        assert!(app.anim_brush_in_use());
        assert!(app.animbrush_is_last(), "B restores it");
        // Alt+b swaps brush and spare: the morph goes the other way. Brush ▸ Spare ▸
        // Metamorph.. opens the same requester.
        app.custom_brush = Some(big.clone());
        pass(
            &mut app,
            &ctx,
            0.5,
            with(key(egui::Key::B, ALT), text("∫")),
            ALT,
        );
        assert_eq!(app.custom_brush.as_ref(), Some(&small));
        assert_eq!(app.tools.spare_brush.as_ref(), Some(&big));
        ui_pass(&mut app, &ctx, 0.6, vec![]);
        let t = menu_pick(&mut app, &ctx, 0.7, "brush.metamorph");
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Enter, true)]);
        let b = app.animbrush.brush.clone().unwrap();
        assert_eq!(b.len(), DEFAULT_MORPH_CELS);
        assert_eq!(colours(&b.cels[0]), [1].into());
        assert_eq!(colours(&b.cels[7]), [2].into());
        // Without a spare brush Alt+m does nothing.
        let mut app = PainterApp::new();
        app.custom_brush = Some(small);
        pass(&mut app, &ctx, 1.0, key(egui::Key::M, ALT), ALT);
        assert!(app.ui.tool_requester.is_none());
    }

    #[test]
    fn brush_transforms_change_every_cel() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let cel = |c: u8| BrushImage {
            width: 3,
            height: 1,
            pixels: vec![c, 0, 0],
            handle: (1, 0),
            transparent: Some(0),
        };
        app.set_new_anim_brush(AnimBrush::new(vec![cel(1), cel(2), cel(3)]));
        app.run_action(Action::Command("brush.flip_horizontal"), &ctx);
        let b = app.animbrush.brush.clone().unwrap();
        for (i, c) in b.cels.iter().enumerate() {
            assert_eq!(c.pixels, vec![0, 0, i as u8 + 1]);
        }
        assert!(app.anim_brush_in_use());
        app.run_action(Action::Command("brush.double"), &ctx);
        let b = app.animbrush.brush.clone().unwrap();
        assert!(b.cels.iter().all(|c| (c.width, c.height) == (6, 2)));
        // Brush ▸ Stretch: one drag stretches every cel the same way.
        app.run_action(Action::Command("brush.stretch"), &ctx);
        drag(&mut app, &ctx, 0.1, &[(100, 100), (112, 108)], NONE);
        let b = app.animbrush.brush.clone().unwrap();
        let size = (b.cels[0].width, b.cels[0].height);
        assert_ne!(size, (6, 2));
        assert!(b.cels.iter().all(|c| (c.width, c.height) == size));
        for (i, c) in b.cels.iter().enumerate() {
            assert!(c.pixels.contains(&(i as u8 + 1)), "cel {i}");
        }
        assert!(app.anim_brush_in_use());
        // A plain brush is transformed alone again.
        app.custom_brush = Some(cel(9));
        app.run_action(Action::Command("brush.flip_horizontal"), &ctx);
        assert_eq!(app.custom_brush.as_ref().unwrap().pixels, vec![0, 0, 9]);
        assert_eq!(app.animbrush.brush.as_ref(), Some(&b));
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-animbrush-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_and_load_through_the_menu_keep_cels_transparency_and_handle() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let cel = |c: u8| BrushImage {
            width: 4,
            height: 3,
            pixels: vec![c, 0, 0, c, 0, c, c, 0, 5, 5, 5, 5],
            handle: (1, 2),
            transparent: Some(0),
        };
        let original = AnimBrush::new(vec![cel(1), cel(2), cel(3)]);
        app.set_new_anim_brush(original.clone());
        ui_pass(&mut app, &ctx, 0.05, vec![]);
        let dir = temp_dir("roundtrip");
        let path = dir.join("walk.anbr");
        let t = menu_pick(&mut app, &ctx, 0.1, "animbrush.save");
        let kind = app.ui.file_requester.as_ref().map(|r| r.kind());
        assert_eq!(kind, Some(FileKind::SaveAnimBrush));
        app.menu_file_accept(FileKind::SaveAnimBrush, &path, false);
        assert!(app.ui.file_requester.is_none());
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[8..12], b"ANIM");
        // Again: the file requester asks before it overwrites.
        menu_pick(&mut app, &ctx, t, "animbrush.save");
        app.menu_file_accept(FileKind::SaveAnimBrush, &path, false);
        assert!(app.ui.file_requester.is_some(), "asks first");
        app.ui.file_requester = None;
        // Free, then Load.
        let t = menu_pick(&mut app, &ctx, t + 0.2, "animbrush.free");
        ui_pass(&mut app, &ctx, t, vec![]);
        let t = menu_pick(&mut app, &ctx, t + 0.1, "animbrush.load");
        let kind = app.ui.file_requester.as_ref().map(|r| r.kind());
        assert_eq!(kind, Some(FileKind::LoadAnimBrush));
        app.menu_file_accept(FileKind::LoadAnimBrush, &path, false);
        assert!(app.ui.file_requester.is_none());
        assert!(app.ui.note_requester.is_none(), "nothing to say");
        let loaded = app.animbrush.brush.clone().expect("loaded");
        assert_eq!(loaded.cels, original.cels);
        assert!(app.anim_brush_in_use());
        ui_pass(&mut app, &ctx, t + 0.1, vec![]);
        // Into a picture whose palette differs: the colours are fitted, and the note says so.
        let mut colors = app.palette().colors().to_vec();
        colors.swap(1, 7);
        app.run(Command::SetPalette { colors });
        menu_pick(&mut app, &ctx, t + 0.2, "animbrush.load");
        app.menu_file_accept(FileKind::LoadAnimBrush, &path, false);
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.title(), ANIM_BRUSH_LOADED);
        let fitted = app.animbrush.brush.clone().unwrap();
        assert_eq!(fitted.cels[0].pixels[0], 7, "colour 1 is now at 7");
        // A single-frame ANIM is not an AnimBrush.
        let opts = EncodeOptions::default();
        let one = iff_anim::encode(&[cel(1).to_image()], app.palette(), &opts).unwrap();
        assert!(decode_anim_brush(&one, app.palette()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn anim_brush_files_fit_their_colours_to_the_picture_palette() {
        let pal = Palette::dpaint_default_32();
        let cel = |c: u8| BrushImage {
            width: 2,
            height: 2,
            pixels: vec![c, 0, 0, c],
            handle: (0, 0),
            transparent: Some(0),
        };
        let b = AnimBrush::new(vec![cel(1), cel(2), cel(3)]);
        let bytes = encode_anim_brush(&b, &pal).unwrap();
        let same = decode_anim_brush(&bytes, &pal).unwrap();
        assert_eq!((same.brush.cels, same.changed), (b.cels.clone(), 0));
        // The page palette has colour 1 at index 7 instead: the cels follow.
        let mut colors = pal.colors().to_vec();
        colors.swap(1, 7);
        let page = Palette::new(colors).unwrap();
        let fitted = decode_anim_brush(&bytes, &page).unwrap();
        assert!(fitted.changed > 0);
        assert_eq!(fitted.brush.cels[0].pixels, vec![7, 0, 0, 7]);
        // The file's own numbers and palette stay for Use Brush Palette (ADR-39).
        assert_eq!(fitted.file.cels, b.cels);
        assert_eq!(fitted.file.palette, pal);
        assert_eq!(fitted.file.fitted, fitted.brush.cels);
    }

    /// A morphed AnimBrush stamped click by click along two rows of the page, through the real
    /// pointer; with `DPX_ANIMBRUSH_PNG=out.png` the screen is written at 3x (ADR-36 shot).
    #[test]
    fn metamorph_clicks_show_every_cel_on_screen() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.1, vec![text("s")], NONE);
        let mut disc = solid(9, 9, 0);
        for y in 0..9 {
            for x in 0..9 {
                if (x - 4) * (x - 4) + (y - 4) * (y - 4) <= 16 {
                    disc.pixels[(y * 9 + x) as usize] = 7;
                }
            }
        }
        let bar = solid(19, 5, 13);
        app.custom_brush = Some(disc);
        pass(
            &mut app,
            &ctx,
            0.2,
            with(key(egui::Key::N, ALT), text("~")),
            ALT,
        );
        app.custom_brush = Some(bar);
        pass(
            &mut app,
            &ctx,
            0.3,
            with(key(egui::Key::M, ALT), text("µ")),
            ALT,
        );
        let up = key_event(egui::Key::ArrowUp, true);
        pass(&mut app, &ctx, 0.4, vec![up.clone(), up], NONE);
        ui_pass(&mut app, &ctx, 0.5, vec![key_event(egui::Key::Enter, true)]);
        assert_eq!(app.animbrush.brush.as_ref().map(AnimBrush::len), Some(10));
        let mut t = 0.6;
        for i in 0..20 {
            let (x, y) = (24 + (i % 10) * 26, 60 + (i / 10) * 40);
            t = drag(&mut app, &ctx, t, &[(x, y)], NONE);
        }
        let colours: std::collections::BTreeSet<u8> = app.page().pixels().iter().copied().collect();
        assert!(colours.contains(&7) && colours.contains(&13), "{colours:?}");
        assert!(colours.len() > 3, "colours between the two: {colours:?}");
        // The second row repeats the first (10 cels, duration 10).
        for x in 0..280 {
            for dy in -6..7 {
                assert_eq!(app.page().get(x, 60 + dy), app.page().get(x, 100 + dy));
            }
        }
        if let Ok(path) = std::env::var("DPX_ANIMBRUSH_PNG") {
            std::fs::write(path, screen_png(&app)).unwrap();
        }
    }

    /// The screen as a PNG at 3x.
    fn screen_png(app: &PainterApp) -> Vec<u8> {
        let screen = app.compose_screen();
        let scale = 3;
        let (w, h) = (screen.width() * scale, screen.height() * scale);
        let big: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| screen.get((x / scale) as i32, (y / scale) as i32).unwrap())
            .collect();
        let big = IndexedImage::from_pixels(w, h, big).unwrap();
        dpx_formats::write_png(&big, app.palette()).unwrap()
    }

    /// With `DPX_ANIMBRUSH_PNG_DIR=dir`, the screen is written to `dir/name` (ADR-36 shots).
    fn shot(app: &PainterApp, name: &str) {
        if let Ok(dir) = std::env::var("DPX_ANIMBRUSH_PNG_DIR") {
            std::fs::write(Path::new(&dir).join(name), screen_png(app)).unwrap();
        }
    }

    /// Picks top-level menu item `id` with the right button (held on the title, let go on
    /// the item); the path is checked with the menu's own pointer tracking.
    fn menu_pick_item(app: &mut PainterApp, ctx: &egui::Context, t: f64, id: &'static str) {
        use dpx_ui::menu::{self, MENUS};
        let (w, h) = app.view().ui_size();
        let wanted = Action::Command(id);
        let (m, item) = MENUS
            .iter()
            .enumerate()
            .find_map(|(m, menu)| {
                let i = menu
                    .items
                    .iter()
                    .position(|it| matches!(it.entry, menu::Entry::Action(a) if a == wanted));
                i.map(|i| (m, i))
            })
            .expect("a top-level item");
        let title_x = (0..w)
            .find(|&x| {
                let mut state = dpx_ui::MenuState::default();
                state.open(w, h, x, 1);
                state.menu == Some(m)
            })
            .expect("menu title");
        let mbox = menu::menu_box(m, w, h);
        let path = [(title_x, 1), (mbox.x + mbox.w / 2, mbox.item_top(item) + 1)];
        let mut state = dpx_ui::MenuState::default();
        state.open(w, h, path[0].0, path[0].1);
        state.pointer(w, h, path[1].0, path[1].1);
        assert_eq!(state.hovered_action(&app.ui.menu_context()), Some(wanted));
        let [title, at] = path.map(|p| ui_point(app, p));
        let moved = egui::Event::PointerMoved;
        let down = button(title, Secondary, true, NONE);
        pass(app, ctx, t, vec![moved(title), down], NONE);
        pass(app, ctx, t + 0.02, vec![moved(at)], NONE);
        pass(
            app,
            ctx,
            t + 0.04,
            vec![button(at, Secondary, false, NONE)],
            NONE,
        );
    }

    /// The window point over point `p` of the open tool requester.
    fn requester_pos(app: &PainterApp, p: (i32, i32)) -> egui::Pos2 {
        let req = app.ui.tool_requester.as_ref().expect("a requester");
        let (w, h) = app.view().ui_size();
        let (ox, oy) = req.origin(w, h);
        ui_point(app, (ox + p.0, oy + p.1))
    }

    /// What `b`'s cels show through `palette`: each pixel's colour, `None` where transparent.
    fn looks(b: &AnimBrush, palette: &Palette) -> Vec<Vec<Option<dpx_core::Rgb8>>> {
        b.cels
            .iter()
            .map(|c| {
                c.pixels
                    .iter()
                    .map(|&p| (Some(p) != c.transparent).then(|| palette.get(p).unwrap()))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn arrange_renumbers_the_animbrush_so_it_stays_in_use_and_looks_the_same() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 0 });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.1, vec![text("s")], NONE);
        // A plain brush picked up first, then an AnimBrush of colours 1, 2 and 3 (made last).
        app.picked_brush = Some(solid(2, 2, 1));
        app.set_new_anim_brush(dots(3));
        let before = looks(app.animbrush.brush.as_ref().unwrap(), app.palette());
        ui_pass(&mut app, &ctx, 0.15, vec![]);
        // Color ▸ Palette ▸ Arrange: colour 2 dragged to the front (0 and 1 move up), Return.
        let t = menu_pick(&mut app, &ctx, 0.2, "palette.arrange");
        let Some(ToolRequester::Arrange(r)) = &app.ui.tool_requester else {
            panic!("the Arrange requester");
        };
        let centre = |c: Rect| (c.x + c.w / 2, c.y + c.h / 2);
        let (from, to) = (centre(r.cell_rect(2)), centre(r.cell_rect(0)));
        let (from, to) = (requester_pos(&app, from), requester_pos(&app, to));
        pass(
            &mut app,
            &ctx,
            t,
            vec![egui::Event::PointerMoved(from)],
            NONE,
        );
        let down = button(from, Primary, true, NONE);
        pass(&mut app, &ctx, t + 0.02, vec![down], NONE);
        pass(
            &mut app,
            &ctx,
            t + 0.04,
            vec![egui::Event::PointerMoved(to)],
            NONE,
        );
        let up = button(to, Primary, false, NONE);
        pass(&mut app, &ctx, t + 0.06, vec![up], NONE);
        let enter = key_event(egui::Key::Enter, true);
        ui_pass(&mut app, &ctx, t + 0.08, vec![enter]);
        assert!(app.ui.tool_requester.is_none());
        let original = Palette::dpaint_default_32();
        assert_eq!(
            app.palette().get(0),
            original.get(2),
            "colour 2 is at 0 now"
        );
        // The AnimBrush followed: still the brush, the same look, and `B` still restores it.
        assert!(app.anim_brush_in_use());
        let b = app.animbrush.brush.clone().unwrap();
        assert_eq!(looks(&b, app.palette()), before);
        assert!(
            app.animbrush_is_last(),
            "made after the (renumbered) picked-up brush"
        );
        // Two clicks stamp cel 1 and cel 2 in their old colours.
        let mut t = t + 0.1;
        for (i, x) in [40, 50].into_iter().enumerate() {
            t = drag(&mut app, &ctx, t, &[(x, 40)], NONE);
            let p = app.page().get(x, 40).unwrap();
            assert_eq!(app.palette().get(p), before[i][0], "click {i}");
        }
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 2);
    }

    #[test]
    fn metamorph_never_makes_more_cels_than_the_byte_limit_holds() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // Handles 8192 pixels apart: not even two cels of their canvas fit. Alt+m explains.
        let mut far = solid(1, 1, 2);
        far.handle = (-8192, -8192);
        app.custom_brush = Some(far);
        pass(
            &mut app,
            &ctx,
            0.1,
            with(key(egui::Key::N, ALT), text("~")),
            ALT,
        );
        app.custom_brush = Some(solid(1, 1, 1));
        pass(&mut app, &ctx, 0.2, key(egui::Key::M, ALT), ALT);
        assert!(app.ui.tool_requester.is_none());
        shot(&app, "animbrush-too-big.png");
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.title(), ANIM_BRUSH_TOO_BIG);
        assert!(app.animbrush.brush.is_none());
        // Two 1000x1000 brushes: Make AnimBrush takes 134 cels at most (134 MB of 128 MiB).
        app.tools.spare_brush = Some(solid(1000, 1000, 2));
        app.custom_brush = Some(solid(1000, 1000, 1));
        pass(&mut app, &ctx, 0.3, key(egui::Key::M, ALT), ALT);
        assert_eq!(requester(&app).cels(), DEFAULT_MORPH_CELS);
        let bs = key_event(egui::Key::Backspace, true);
        pass(&mut app, &ctx, 0.4, vec![bs, text("135")], NONE);
        let enter = key_event(egui::Key::Enter, true);
        ui_pass(&mut app, &ctx, 0.5, vec![enter]);
        assert!(app.ui.tool_requester.is_some(), "135 cels are refused");
        shot(&app, "animbrush-make-limit.png");
        assert!(app.animbrush.brush.is_none());
        ui_pass(
            &mut app,
            &ctx,
            0.6,
            vec![key_event(egui::Key::Escape, true)],
        );
        assert!(app.ui.tool_requester.is_none());
        // An Ok for more cels than fit makes nothing either; it says why.
        app.animbrush_requester_ok(AnimBrushRequester::make(999, 999));
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.title(), ANIM_BRUSH_TOO_BIG);
        assert!(app.animbrush.brush.is_none());
    }

    #[test]
    fn brush_transforms_that_would_pass_the_byte_limit_leave_the_animbrush_as_it_was() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // 33 cels of 1024x1024 (33 MiB): doubled they would hold 132 MiB, more than 128.
        let cels = (0..33u8).map(|i| solid(1024, 1024, i % 5 + 1)).collect();
        app.set_new_anim_brush(AnimBrush::new(cels));
        ui_pass(&mut app, &ctx, 0.05, vec![]);
        let t = menu_pick(&mut app, &ctx, 0.1, "brush.double");
        let b = app.animbrush.brush.as_ref().unwrap();
        assert!(b.cels.iter().all(|c| (c.width, c.height) == (1024, 1024)));
        assert!(app.anim_brush_in_use());
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.title(), ANIM_BRUSH_TOO_BIG);
        // Brush ▸ Stretch with Shift (proportional): 400 cels of 1024x1 dragged 100 pixels
        // down would become 4096x100 each, 156 MiB together.
        let cels = (0..400)
            .map(|i| solid(1024, 1, (i % 5 + 1) as u8))
            .collect();
        app.set_new_anim_brush(AnimBrush::new(cels));
        let t = menu_pick(&mut app, &ctx, t, "brush.stretch");
        let shift = egui::Modifiers::SHIFT;
        drag(&mut app, &ctx, t, &[(100, 60), (100, 159)], shift);
        let b = app.animbrush.brush.as_ref().unwrap();
        assert!(b.cels.iter().all(|c| (c.width, c.height) == (1024, 1)));
        assert!(app.anim_brush_in_use());
        assert_eq!(app.custom_brush.as_ref(), Some(b.current_cel()));
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.title(), ANIM_BRUSH_TOO_BIG);
    }

    #[test]
    fn the_right_click_leaves_an_animbrush_whose_cel_looks_like_the_picked_up_brush() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 0 });
        // Two 5x5 squares: colour 2 at (20, 20) and colour 1 at (40, 40).
        for (at, c) in [(20, 2u8), (40, 1)] {
            app.run(Command::Shape {
                shape: dpx_core::Shape::Rect {
                    from: (at, at),
                    to: (at + 4, at + 4),
                },
                brush: Vec::new(),
                color: c,
                filled: true,
                symmetry: None,
                custom: None,
                mode: dpx_core::DrawMode::Matte,
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            });
        }
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // Pick up P (colour 2), Alt+n to the spare, then pick up Q (colour 1): the manual's
        // same-size brushes for Metamorph.
        pass(&mut app, &ctx, 0.1, vec![text("b")], NONE);
        let t = drag(&mut app, &ctx, 0.2, &[(20, 20), (24, 24)], NONE);
        pass(
            &mut app,
            &ctx,
            t,
            with(key(egui::Key::N, ALT), text("~")),
            ALT,
        );
        pass(&mut app, &ctx, t + 0.1, vec![text("b")], NONE);
        let t = drag(&mut app, &ctx, t + 0.2, &[(40, 40), (44, 44)], NONE);
        let q = app.custom_brush.clone().expect("Q");
        assert_eq!(app.picked_brush.as_ref(), Some(&q));
        // Brush ▸ Spare ▸ Metamorph.., Return: the first cel is Q itself.
        ui_pass(&mut app, &ctx, t, vec![]);
        let t = menu_pick(&mut app, &ctx, t + 0.1, "brush.metamorph");
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Enter, true)]);
        let b = app.animbrush.brush.clone().expect("an AnimBrush");
        assert_eq!(b.current_cel(), &q, "same-size brushes: cel 1 is the brush");
        assert!(app.anim_brush_in_use());
        // Right-click on the Brush Selector: the plain brush Q, not the AnimBrush.
        let t = right_click_brush_selector(&mut app, &ctx, t + 0.1);
        assert_eq!(app.custom_brush.as_ref(), Some(&q));
        assert!(!app.anim_brush_in_use());
        // A click paints Q and leaves the AnimBrush's clock alone.
        pass(&mut app, &ctx, t, vec![text("s")], NONE);
        let t = drag(&mut app, &ctx, t + 0.1, &[(100, 100)], NONE);
        assert_eq!(app.page().get(100, 100), Some(1));
        assert_eq!(app.animbrush.brush.as_ref(), Some(&b));
        // The second right-click brings the AnimBrush back; a click then moves it on.
        let t = right_click_brush_selector(&mut app, &ctx, t);
        assert!(app.anim_brush_in_use());
        let t = drag(&mut app, &ctx, t, &[(120, 100)], NONE);
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 1);
        // A plain pick-up of a brush that looks like the current cel is a plain brush too.
        app.animbrush.brush.as_mut().unwrap().set_current(0);
        app.use_anim_brush();
        pass(&mut app, &ctx, t, vec![text("b")], NONE);
        let t = drag(&mut app, &ctx, t + 0.1, &[(40, 40), (44, 44)], NONE);
        assert_eq!(app.custom_brush.as_ref(), Some(&q));
        assert!(!app.anim_brush_in_use());
        // So is Alt+b twice (the cel goes to the spare and comes back as a plain brush).
        app.use_anim_brush();
        pass(
            &mut app,
            &ctx,
            t,
            with(key(egui::Key::B, ALT), text("∫")),
            ALT,
        );
        assert!(!app.anim_brush_in_use());
        pass(
            &mut app,
            &ctx,
            t + 0.1,
            with(key(egui::Key::B, ALT), text("∫")),
            ALT,
        );
        assert_eq!(app.custom_brush.as_ref(), Some(&q));
        assert!(!app.anim_brush_in_use());
    }

    #[test]
    fn brush_restore_is_on_while_b_would_restore_the_animbrush() {
        let ctx = egui::Context::default();
        let mut app = frames(2);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // An AnimBrush before any plain brush was picked up.
        pass(&mut app, &ctx, 0.1, vec![text("b")], NONE);
        let t = drag(&mut app, &ctx, 0.2, &[(39, 39), (45, 45)], ALT);
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.anim_brush_in_use() && app.picked_brush.is_none());
        // Right-click on the Brush Selector: the built-in brush (there is no other).
        let t = right_click_brush_selector(&mut app, &ctx, t + 0.1);
        assert!(app.custom_brush.is_none());
        ui_pass(&mut app, &ctx, t, vec![]);
        assert!(Action::Command("brush.restore").enabled(&app.ui.menu_context()));
        // Brush ▸ Restore does what `B` does.
        menu_pick_item(&mut app, &ctx, t + 0.1, "brush.restore");
        assert!(app.anim_brush_in_use());
    }

    #[test]
    fn cel_suggestions_follow_the_frames() {
        assert_eq!(suggested_cels(20, 0), 20);
        assert_eq!(suggested_cels(20, 5), 15);
        assert_eq!(suggested_cels(20, 19), 20, "on the last frame: all");
        assert_eq!(morph_cels(1), DEFAULT_MORPH_CELS);
        assert_eq!(morph_cels(12), 12);
    }

    // ---- Interactions with Stencil ▸ Paint, area stencils and Perspective (review of the
    // wave-7 merge).

    const CTRL: egui::Modifiers = egui::Modifiers::CTRL;

    /// The screen as the next upload composes it (the views of ADR-28, -37 and -39).
    fn composed(app: &mut PainterApp) -> IndexedImage {
        app.prepare_perspective_view();
        app.prepare_anim_view();
        app.prepare_stencil_view();
        let screen = app.compose_screen();
        app.anim.shown = None;
        app.perspective.shown = None;
        screen
    }

    /// The Stencil ▸ Paint mask.
    fn mask(app: &PainterApp) -> dpx_core::AreaStencil {
        let p = app.menu_data.palette_stencil.painting.as_ref();
        dpx_core::AreaStencil::from_image(p.expect("Stencil Paint").engine.document().page())
    }

    /// Three cels: a dot, a bar three pixels wide (handle in its middle), a dot.
    fn dot_bar_dot() -> AnimBrush {
        let bar = BrushImage {
            width: 3,
            height: 1,
            pixels: vec![5; 3],
            handle: (1, 0),
            transparent: Some(0),
        };
        AnimBrush::new(vec![solid(1, 1, 4), bar, solid(1, 1, 6)])
    }

    #[test]
    fn stencil_paint_with_an_animbrush_paints_the_mask_in_its_cels_shapes() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 3 });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.1, vec![text("s")], NONE);
        app.set_new_anim_brush(dot_bar_dot());
        let revision = app.engine.document().revision();
        pass(&mut app, &ctx, 0.2, key(egui::Key::S, CTRL), CTRL);
        assert!(app.stencil_painting());
        // A dotted stroke of two stamps: the dot at (40, 40), the bar at (49..=51, 40).
        let (a, b) = (page_pos(&app, (40, 40)), page_pos(&app, (50, 40)));
        pass(
            &mut app,
            &ctx,
            0.3,
            vec![egui::Event::PointerMoved(a)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            0.32,
            vec![button(a, Primary, true, NONE)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            0.34,
            vec![egui::Event::PointerMoved(b)],
            NONE,
        );
        // While dragging, the stencil view shows the stroke in the mask at full brightness.
        let dim = dpx_core::stencil::half_bright_table(app.palette())[3];
        assert_ne!(dim, 3);
        let shown = composed(&mut app);
        let row =
            |img: &IndexedImage| -> Vec<u8> { (38..54).map(|x| img.get(x, 40).unwrap()).collect() };
        let (d, f) = (dim, 3);
        assert_eq!(
            row(&shown),
            [d, d, f, d, d, d, d, d, d, d, d, f, f, f, d, d],
            "the cels' shapes, previewed in the mask"
        );
        pass(
            &mut app,
            &ctx,
            0.36,
            vec![button(b, Primary, false, NONE)],
            NONE,
        );
        // The picture never changed; the mask holds the dot and the bar.
        assert_eq!(app.engine.document().revision(), revision);
        assert_eq!(app.engine.document().page().get(50, 40), Some(3));
        let m = mask(&app);
        assert!(m.protects(40, 40) && m.protects(49, 40) && m.protects(51, 40));
        assert!(!m.protects(41, 40) && !m.protects(52, 40));
        // The AnimBrush went on by its two stamps, as after any stroke.
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 2);
        assert!(app.anim_brush_in_use());
        // The right button frees in the shape of the cel it stamps (the third, a dot).
        let c = page_pos(&app, (50, 40));
        pass(
            &mut app,
            &ctx,
            0.4,
            vec![button(c, Secondary, true, NONE)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            0.42,
            vec![button(c, Secondary, false, NONE)],
            NONE,
        );
        let m = mask(&app);
        assert!(!m.protects(50, 40) && m.protects(49, 40) && m.protects(51, 40));
        // `u` takes back that mask stroke, not anything in the picture.
        pass(&mut app, &ctx, 0.5, key(egui::Key::U, NONE), NONE);
        assert!(mask(&app).protects(50, 40));
        assert_eq!(app.engine.document().revision(), revision);
    }

    #[test]
    fn an_animbrush_under_an_area_stencil_previews_and_goes_on_cel_by_cel() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pass(&mut app, &ctx, 0.1, vec![text("s")], NONE);
        app.set_new_anim_brush(dots(3));
        // An area stencil on this frame, away from the strokes (as Lock FG makes one).
        let mut bits = vec![false; 320 * 256];
        bits[10 * 320 + 10] = true;
        app.menu_data.palette_stencil.area = Some(crate::palette_stencil::Area {
            stencil: dpx_core::AreaStencil::from_mask(320, 256, bits).unwrap(),
            frame: 0,
            on: true,
        });
        let mut t = 0.2;
        for (i, x) in [40, 50, 60, 70].into_iter().enumerate() {
            let p = page_pos(&app, (x, 40));
            pass(&mut app, &ctx, t, vec![egui::Event::PointerMoved(p)], NONE);
            pass(
                &mut app,
                &ctx,
                t + 0.02,
                vec![button(p, Primary, true, NONE)],
                NONE,
            );
            let want = i as u8 % 3 + 1;
            assert_eq!(app.page().get(x, 40), Some(want), "previewed, click {i}");
            pass(
                &mut app,
                &ctx,
                t + 0.04,
                vec![button(p, Primary, false, NONE)],
                NONE,
            );
            let page = app.engine.document().page();
            assert_eq!(page.get(x, 40), Some(want), "painted, click {i}");
            t += 0.1;
        }
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 1);
        // A Line's rubber band shows while it is dragged.
        app.select_tool(Tool::Line, false);
        let (a, b) = (page_pos(&app, (100, 100)), page_pos(&app, (110, 100)));
        pass(&mut app, &ctx, t, vec![egui::Event::PointerMoved(a)], NONE);
        pass(
            &mut app,
            &ctx,
            t + 0.02,
            vec![button(a, Primary, true, NONE)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            t + 0.04,
            vec![egui::Event::PointerMoved(b)],
            NONE,
        );
        let band = (100..=110).filter(|&x| app.page().get(x, 100) != Some(0));
        assert_eq!(band.count(), 11, "the rubber band");
        pass(
            &mut app,
            &ctx,
            t + 0.06,
            vec![button(b, Primary, false, NONE)],
            NONE,
        );
        // Cel 2 first (the clock went on), then 3, 1, 2, ... along the eleven stamps.
        let page = app.engine.document().page();
        assert_eq!((page.get(100, 100), page.get(110, 100)), (Some(2), Some(3)));
    }

    #[test]
    fn perspective_clicks_with_an_animbrush_go_on_cel_by_cel() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        app.set_new_anim_brush(dots(3));
        let p = page_pos(&app, (100, 100));
        pass(
            &mut app,
            &ctx,
            0.1,
            vec![egui::Event::PointerMoved(p)],
            NONE,
        );
        pass(&mut app, &ctx, 0.12, key(egui::Key::Enter, NONE), NONE);
        assert!(app.perspective.active);
        let mut t = 0.2;
        for (i, x) in [100, 110, 120].into_iter().enumerate() {
            let p = page_pos(&app, (x, 100));
            pass(&mut app, &ctx, t, vec![egui::Event::PointerMoved(p)], NONE);
            pass(
                &mut app,
                &ctx,
                t + 0.02,
                vec![button(p, Primary, true, NONE)],
                NONE,
            );
            pass(
                &mut app,
                &ctx,
                t + 0.04,
                vec![button(p, Primary, false, NONE)],
                NONE,
            );
            let page = app.engine.document().page();
            assert_eq!(page.get(x, 100), Some(i as u8 + 1), "stamp {i}");
            t += 0.1;
        }
        assert!(app.perspective.active, "the cels change, the mode stays");
        assert!(app.anim_brush_in_use());
        // FillScreen (`-`) paints one cel and goes on by one too.
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 0);
        pass(
            &mut app,
            &ctx,
            t,
            with(key(egui::Key::Minus, NONE), text("-")),
            NONE,
        );
        assert_eq!(app.engine.document().page().get(0, 0), Some(1));
        assert_eq!(app.animbrush.brush.as_ref().unwrap().current(), 1);
    }

    #[test]
    fn alt_n_alt_b_and_alt_m_keep_stencil_paint_on() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        app.custom_brush = Some(solid(3, 3, 1));
        app.picked_brush = app.custom_brush.clone();
        pass(&mut app, &ctx, 0.1, key(egui::Key::S, CTRL), CTRL);
        assert!(app.stencil_painting());
        // Brush->Spare and Brush<->Spare change brushes only: the mask stays being painted.
        pass(
            &mut app,
            &ctx,
            0.2,
            with(key(egui::Key::N, ALT), text("~")),
            ALT,
        );
        assert!(app.tools.spare_brush.is_some());
        app.custom_brush = Some(solid(5, 5, 2));
        pass(
            &mut app,
            &ctx,
            0.3,
            with(key(egui::Key::B, ALT), text("∫")),
            ALT,
        );
        assert_eq!(app.custom_brush, Some(solid(3, 3, 1)), "swapped");
        assert!(app.stencil_painting());
        // The swapped brush paints the mask.
        let p = page_pos(&app, (40, 40));
        pass(
            &mut app,
            &ctx,
            0.4,
            vec![egui::Event::PointerMoved(p)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            0.42,
            vec![button(p, Primary, true, NONE)],
            NONE,
        );
        pass(
            &mut app,
            &ctx,
            0.44,
            vec![button(p, Primary, false, NONE)],
            NONE,
        );
        assert!(mask(&app).protects(41, 41));
        // Metamorph's requester opens over it, and Stencil Paint is still on after Cancel.
        pass(
            &mut app,
            &ctx,
            0.5,
            with(key(egui::Key::M, ALT), text("µ")),
            ALT,
        );
        assert!(app.ui.tool_requester.is_some());
        ui_pass(
            &mut app,
            &ctx,
            0.6,
            vec![key_event(egui::Key::Escape, true)],
        );
        assert!(app.ui.tool_requester.is_none());
        assert!(app.stencil_painting());
    }

    #[test]
    fn use_brush_palette_takes_a_loaded_animbrushs_own_palette() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // An AnimBrush file with its own 32 colours (not the picture's) and a colour range.
        let own = Palette::new(
            (0..32u8)
                .map(|i| dpx_core::Rgb8::new(i * 8, 255 - i * 8, 100))
                .collect(),
        )
        .unwrap();
        let cel = |c: u8| BrushImage {
            width: 2,
            height: 2,
            pixels: vec![c, 0, 0, c + 10],
            handle: (0, 0),
            transparent: Some(0),
        };
        let file = AnimBrush::new(vec![cel(1), cel(2), cel(3)]);
        let range = ColorRange {
            rate: 4000,
            flags: 1,
            low: 11,
            high: 13,
        };
        let opts = EncodeOptions {
            loop_frames: false,
            transparent: Some(0),
            grab: Some((0, 0)),
            ranges: vec![range],
            ..EncodeOptions::default()
        };
        let frames: Vec<IndexedImage> = file.cels.iter().map(BrushImage::to_image).collect();
        let bytes = iff_anim::encode(&frames, &own, &opts).unwrap();
        let dir = temp_dir("own-palette");
        let path = dir.join("sweep.anbr");
        std::fs::write(&path, bytes).unwrap();
        // A plain brush loaded before it (Brush ▸ Load..) must not get in the way.
        let plain = solid(2, 2, 7);
        app.menu_data.palette_stencil.loaded = Some(crate::palette_stencil::LoadedBrush {
            brush: plain.clone(),
            palette: Palette::dpaint_default_32(),
            ranges: Vec::new(),
            fitted: plain.clone(),
        });
        app.picked_brush = Some(plain);
        app.menu_file_accept(FileKind::LoadAnimBrush, &path, false);
        app.ui.note_requester = None;
        assert!(app.anim_brush_in_use());
        app.animbrush.brush.as_mut().unwrap().set_current(1);
        app.use_anim_brush();
        ui_pass(&mut app, &ctx, 0.05, vec![]);
        // Use Brush Palette (tutorial p.165): the file's palette and range, the file's
        // numbers in every cel, the AnimBrush still in use on the same cel.
        let t = menu_pick(&mut app, &ctx, 0.1, "palette.use_brush");
        ui_pass(&mut app, &ctx, t, vec![]);
        assert_eq!(app.engine.document().palette(), &own);
        assert_eq!(app.extras.ranges, vec![range]);
        assert!(app.anim_brush_in_use());
        let b = app.animbrush.brush.clone().unwrap();
        assert_eq!(b.cels, file.cels);
        assert_eq!(b.current(), 1);
        assert_eq!(app.custom_brush.as_ref(), Some(&file.cels[1]));
        // Once more changes nothing; Undo brings the picture palette back.
        let t = menu_pick(&mut app, &ctx, t + 0.1, "palette.use_brush");
        assert_eq!(app.animbrush.brush.as_ref().unwrap().cels, file.cels);
        assert_eq!(app.engine.document().palette(), &own);
        app.undo();
        ui_pass(&mut app, &ctx, t, vec![]);
        assert_eq!(
            app.engine.document().palette(),
            &Palette::dpaint_default_32()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
