//! The command engine: validates and applies [`Command`]s, keeps the revision counter and a
//! multi-level undo/redo history (ADR-2, Phase 1 implementation: see ADR-6).

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use crate::command::{Command, CommandError, ErrorCode, Footprint};
use crate::document::Document;
use crate::image::{IndexedImage, Rect};
use crate::palette::Palette;

/// A command plus execution options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub command: Command,
    /// Rejects the request with `REVISION_CONFLICT` unless the document is at this revision.
    pub expected_revision: Option<u64>,
    /// Validates and reports what would change, without changing anything.
    pub dry_run: bool,
}

impl From<Command> for Request {
    fn from(command: Command) -> Self {
        Self {
            command,
            expected_revision: None,
            dry_run: false,
        }
    }
}

/// Result of a successful execute, undo or redo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Document revision afterwards (unchanged for a dry run).
    pub revision: u64,
    /// Page pixels that (would) change, clipped to the page; `None` if no pixels change.
    pub affected: Option<Rect>,
    /// Whether the palette (would) change.
    pub palette_changed: bool,
    /// Whether page size or display mode (would) change; front-ends rebuild their screen.
    pub format_changed: bool,
    pub undo_available: bool,
    pub redo_available: bool,
}

/// What an undo step restores.
#[derive(Clone, Debug)]
enum Before {
    Pixels {
        rect: Rect,
        pixels: IndexedImage,
    },
    Palette(Palette),
    Document(Box<Document>),
    /// Arrange Palette: the order that arranges the palette and every frame back. Exact, and
    /// no copy of the frames is kept.
    Arranged(Vec<u8>),
}

impl Before {
    /// Bytes this snapshot keeps alive besides `doc`, the document after the command: pixel
    /// copies, and the frames of a document snapshot that `doc` no longer shares (each frame
    /// buffer counted once).
    fn bytes(&self, doc: &Document) -> u64 {
        match self {
            Before::Pixels { pixels, .. } => pixels.pixels().len() as u64,
            Before::Palette(p) => 3 * p.len() as u64,
            Before::Arranged(order) => order.len() as u64,
            Before::Document(snapshot) => {
                let mut seen: HashSet<*const IndexedImage> =
                    doc.others.iter().map(Arc::as_ptr).collect();
                let frames: u64 = snapshot
                    .others
                    .iter()
                    .filter(|f| seen.insert(Arc::as_ptr(f)))
                    .map(|f| f.pixels().len() as u64)
                    .sum();
                snapshot.page.pixels().len() as u64 + frames
            }
        }
    }
}

/// Bytes of a command's own data (pixels, points, brushes) that its undo step keeps.
fn command_bytes(command: &Command) -> u64 {
    let custom = |b: &Option<crate::BrushImage>| b.as_ref().map_or(0, |b| b.pixels.len() as u64);
    let points = |n: usize| 8 * n as u64;
    match command {
        Command::Stroke {
            brush,
            points: p,
            custom: c,
            ..
        } => points(brush.len() + p.len()) + custom(c),
        Command::Shape {
            shape,
            brush,
            custom: c,
            ..
        } => {
            let n = match shape {
                crate::Shape::Polygon { points, .. } | crate::Shape::Dots { points } => {
                    points.len()
                }
                _ => 0,
            };
            points(brush.len() + n) + custom(c)
        }
        Command::SetPage { pixels, .. } | Command::MergePage { pixels, .. } => pixels.len() as u64,
        Command::SetPalette { colors } => 3 * colors.len() as u64,
        Command::ArrangePalette { order } => order.len() as u64,
        // ADR-39: the mask (shared between commands, counted for each).
        Command::Stenciled { area, command } => {
            u64::from(area.width()) * u64::from(area.height()) + command_bytes(command)
        }
        _ => 0,
    }
}

#[derive(Clone, Debug)]
struct Step {
    command: Command,
    before: Before,
    /// The current frame when the command ran (ADR-28): undo and redo go back to it.
    frame: usize,
    /// Memory this step keeps (its snapshot and its command's data), for the byte budget.
    bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Engine {
    doc: Document,
    undo: VecDeque<Step>,
    /// Undone commands with their frame, most recent last; redo replays them (commands are
    /// deterministic).
    redo: Vec<(Command, usize)>,
    undo_limit: usize,
    /// Most bytes the undo steps may keep together; beyond it the oldest steps are dropped.
    undo_byte_limit: u64,
    /// Bytes the undo steps keep now (the sum of their `bytes`).
    undo_bytes: u64,
}

impl Engine {
    pub const DEFAULT_UNDO_LIMIT: usize = 64;
    /// Most memory the undo history keeps by default: twice the frame limit of ADR-28
    /// ([`crate::anim::MAX_FRAME_BYTES`]). Undo steps that copy whole animations (Page Size,
    /// Screen Format) would otherwise multiply that limit by the 64 steps.
    pub const DEFAULT_UNDO_BYTES: u64 = 2 * crate::anim::MAX_FRAME_BYTES;

    pub fn new(doc: Document) -> Self {
        Self::with_undo_limit(doc, Self::DEFAULT_UNDO_LIMIT)
    }

    /// `limit` = number of undo steps kept (at least 1); the oldest steps are dropped. The
    /// byte budget is [`Self::DEFAULT_UNDO_BYTES`].
    pub fn with_undo_limit(doc: Document, limit: usize) -> Self {
        Self::with_undo_budget(doc, limit, Self::DEFAULT_UNDO_BYTES)
    }

    /// `steps` = number of undo steps kept (at least 1), `bytes` = most memory they may keep
    /// together (snapshots and command data). The oldest steps are dropped first; the most
    /// recent step always stays, so the last command can be undone.
    pub fn with_undo_budget(doc: Document, steps: usize, bytes: u64) -> Self {
        Self {
            doc,
            undo: VecDeque::new(),
            redo: Vec::new(),
            undo_limit: steps.max(1),
            undo_byte_limit: bytes,
            undo_bytes: 0,
        }
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The command the next [`Self::redo`] replays, with the frame it runs on (after an undo:
    /// the command just undone).
    pub fn next_redo(&self) -> Option<(&Command, usize)> {
        self.redo.last().map(|(command, frame)| (command, *frame))
    }

    /// Validates and executes a command as one undo step. A new command clears the redo list.
    /// Settings (GotoFrame, SetFrameRate; ADR-28) are applied without an undo step and keep
    /// the redo list.
    pub fn execute(&mut self, request: impl Into<Request>) -> Result<Outcome, CommandError> {
        let request = request.into();
        if let Some(expected) = request.expected_revision
            && expected != self.doc.revision
        {
            return Err(CommandError::new(
                ErrorCode::RevisionConflict,
                format!(
                    "document is at revision {}, expected {expected}",
                    self.doc.revision
                ),
            ));
        }
        request.command.validate(&self.doc)?;
        let footprint = request.command.footprint(&self.doc);
        if request.dry_run {
            return Ok(self.outcome(footprint));
        }
        if footprint == Footprint::Setting {
            request.command.apply(&mut self.doc);
            self.doc.revision += 1;
            return Ok(self.outcome(footprint));
        }
        self.redo.clear();
        self.commit(request.command, footprint);
        Ok(self.outcome(footprint))
    }

    /// Reverts the most recent step. A pixel step of another frame makes that frame current
    /// first, so the change shows (ADR-28); the outcome then names the whole page.
    pub fn undo(&mut self) -> Result<Outcome, CommandError> {
        let step = self
            .undo
            .pop_back()
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToUndo, "nothing to undo"))?;
        self.undo_bytes -= step.bytes;
        let switched = step.frame != self.doc.current;
        let footprint = match step.before {
            Before::Pixels { rect, pixels } => {
                self.doc.go_to(step.frame);
                self.doc.page.blit(&pixels, rect.x, rect.y);
                Footprint::Pixels(rect)
            }
            Before::Palette(palette) => {
                self.doc.palette = palette;
                Footprint::Palette
            }
            Before::Document(doc) => {
                // The rate is a setting outside the undo history.
                let (revision, fps) = (self.doc.revision, self.doc.fps);
                self.doc = *doc;
                self.doc.revision = revision;
                self.doc.fps = fps;
                Footprint::Document
            }
            Before::Arranged(order) => {
                // Back on its frame, as a document snapshot would be.
                self.doc.go_to(step.frame);
                Command::ArrangePalette { order }.apply(&mut self.doc);
                Footprint::Document
            }
        };
        self.redo.push((step.command, step.frame));
        self.doc.revision += 1;
        Ok(self.outcome_on(footprint, switched))
    }

    /// Re-applies the most recently undone command, on the frame it first ran on (the
    /// outcome names the whole page when that frame was not current).
    pub fn redo(&mut self) -> Result<Outcome, CommandError> {
        let (command, frame) = self
            .redo
            .pop()
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToRedo, "nothing to redo"))?;
        // The document is back in the state the command was first validated against (only
        // the current frame may differ: settings keep the redo list).
        let switched = frame != self.doc.current;
        self.doc.go_to(frame);
        command.validate(&self.doc)?;
        let footprint = command.footprint(&self.doc);
        self.commit(command, footprint);
        Ok(self.outcome_on(footprint, switched))
    }

    /// Applies a validated command as a new undo step.
    fn commit(&mut self, command: Command, footprint: Footprint) {
        let before = match (&command, footprint) {
            (Command::ArrangePalette { order }, _) => {
                Before::Arranged(crate::remap::inverse(order)[..order.len()].to_vec())
            }
            (_, Footprint::Pixels(rect)) => Before::Pixels {
                rect,
                pixels: self.doc.page.crop(rect),
            },
            (_, Footprint::Palette) => Before::Palette(self.doc.palette.clone()),
            // Other frames are shared, not copied (Arc).
            (_, Footprint::Document | Footprint::Frames | Footprint::Setting) => {
                Before::Document(Box::new(self.doc.clone()))
            }
        };
        let frame = self.doc.current;
        command.apply(&mut self.doc);
        let bytes = before.bytes(&self.doc) + command_bytes(&command);
        self.undo.push_back(Step {
            command,
            before,
            frame,
            bytes,
        });
        self.undo_bytes += bytes;
        // The oldest steps go beyond the step limit or the byte budget; the newest stays.
        while self.undo.len() > self.undo_limit
            || (self.undo_bytes > self.undo_byte_limit && self.undo.len() > 1)
        {
            if let Some(old) = self.undo.pop_front() {
                self.undo_bytes -= old.bytes;
            }
        }
        self.doc.revision += 1;
    }

    /// [`Self::outcome`] after undo or redo; `switched`: another frame became current, so the
    /// whole page shows something else.
    fn outcome_on(&self, footprint: Footprint, switched: bool) -> Outcome {
        let mut out = self.outcome(footprint);
        if switched {
            out.affected = Some(self.doc.page.bounds());
        }
        out
    }

    fn outcome(&self, footprint: Footprint) -> Outcome {
        let (affected, palette_changed, format_changed) = match footprint {
            Footprint::Pixels(r) => ((!r.is_empty()).then_some(r), false, false),
            Footprint::Palette => (None, true, false),
            Footprint::Document => (Some(self.doc.page.bounds()), true, true),
            Footprint::Frames | Footprint::Setting => (Some(self.doc.page.bounds()), false, false),
        };
        Outcome {
            revision: self.doc.revision,
            affected,
            palette_changed,
            format_changed,
            undo_available: self.can_undo(),
            redo_available: self.can_redo(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrange_palette_renumbers_every_frame() {
        let pal = Palette::dpaint_default_32();
        let frame = |c: u8| IndexedImage::new(2, 1, c);
        let doc = Document::from_frames(
            vec![frame(1), frame(2)],
            0,
            30,
            pal.clone(),
            crate::Target::default(),
        )
        .unwrap();
        let mut e = Engine::new(doc);
        // Swap entries 1 and 2: both frames keep their look.
        let mut order: Vec<u8> = (0..32).collect();
        order.swap(1, 2);
        e.execute(Command::ArrangePalette { order }).unwrap();
        let d = e.document();
        assert_eq!(d.palette().get(1), pal.get(2));
        assert_eq!(d.frame(0).unwrap().get(0, 0), Some(2));
        assert_eq!(d.frame(1).unwrap().get(0, 0), Some(1));
        e.undo().unwrap();
        assert_eq!(e.document().frame(1).unwrap().get(0, 0), Some(2));
    }
    use crate::command::Axis;
    use crate::raster;
    use crate::{BrushImage, DrawMode, IndexedImage, Rgb8};

    fn engine() -> Engine {
        Engine::new(Document::new(16, 8, Palette::dpaint_default_32()))
    }

    fn stroke(color: u8, points: &[(i32, i32)]) -> Command {
        Command::Stroke {
            brush: vec![(0, 0), (1, 0), (0, 1)],
            color,
            points: points.to_vec(),
            continuous: true,
            custom: None,
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    #[test]
    fn execute_undo_redo_round_trip() {
        let mut e = engine();
        let blank = e.document().page().clone();
        let out = e.execute(stroke(2, &[(1, 1), (6, 3)])).unwrap();
        assert_eq!(out.revision, 1);
        assert!(out.undo_available && !out.redo_available);
        let painted = e.document().page().clone();
        assert_ne!(painted, blank);
        let r = out.affected.unwrap();
        for y in 0..8 {
            for x in 0..16 {
                if painted.get(x, y) != blank.get(x, y) {
                    assert!(r.contains(x, y), "change at ({x},{y}) outside {r:?}");
                }
            }
        }

        let out = e.undo().unwrap();
        assert_eq!(out.revision, 2);
        assert!(!out.undo_available && out.redo_available);
        assert_eq!(e.document().page(), &blank);

        e.redo().unwrap();
        assert_eq!(e.document().page(), &painted);
        assert_eq!(e.document().revision(), 3);
        assert_eq!(e.undo().unwrap().affected, Some(r));
    }

    #[test]
    fn engine_stroke_matches_raster_preview() {
        // The GUI previews strokes with `raster::stroke` on a copy and commits on release.
        let mut e = engine();
        let points = [(-2, 3), (4, 0), (15, 7), (9, 9)];
        let mut preview = e.document().page().clone();
        for pair in points.windows(2) {
            raster::stroke(&mut preview, &[(0, 0), (1, 0), (0, 1)], 5, pair, true);
        }
        e.execute(stroke(5, &points)).unwrap();
        assert_eq!(e.document().page(), &preview);
    }

    #[test]
    fn new_command_clears_redo() {
        let mut e = engine();
        e.execute(Command::Clear { color: 1 }).unwrap();
        e.undo().unwrap();
        assert!(e.can_redo());
        e.execute(Command::Clear { color: 2 }).unwrap();
        assert!(!e.can_redo());
        assert_eq!(e.redo().unwrap_err().code, ErrorCode::NothingToRedo);
    }

    #[test]
    fn next_redo_names_the_command_just_undone_and_its_frame() {
        let mut e = engine();
        assert_eq!(e.next_redo(), None);
        e.execute(Command::SetFrameCount { count: 3 }).unwrap();
        e.execute(Command::GotoFrame { frame: 2 }).unwrap();
        e.execute(Command::Clear { color: 4 }).unwrap();
        e.undo().unwrap();
        assert_eq!(e.next_redo(), Some((&Command::Clear { color: 4 }, 2)));
        e.undo().unwrap();
        assert_eq!(
            e.next_redo(),
            Some((&Command::SetFrameCount { count: 3 }, 0))
        );
        e.redo().unwrap();
        assert_eq!(e.next_redo(), Some((&Command::Clear { color: 4 }, 2)));
    }

    #[test]
    fn multi_level_undo_and_limit() {
        let mut e = Engine::with_undo_limit(Document::new(4, 4, Palette::dpaint_default_32()), 3);
        for c in 1..=5 {
            e.execute(Command::Clear { color: c }).unwrap();
        }
        assert_eq!(e.document().page().get(0, 0), Some(5));
        for expected in [4, 3, 2] {
            e.undo().unwrap();
            assert_eq!(e.document().page().get(0, 0), Some(expected));
        }
        assert_eq!(e.undo().unwrap_err().code, ErrorCode::NothingToUndo);
        for expected in [3, 4, 5] {
            e.redo().unwrap();
            assert_eq!(e.document().page().get(0, 0), Some(expected));
        }
    }

    #[test]
    fn whole_page_commands_and_palette_undo() {
        let mut e = engine();
        e.execute(stroke(3, &[(0, 0)])).unwrap();
        e.execute(Command::Flip {
            axis: Axis::Horizontal,
        })
        .unwrap();
        assert_eq!(e.document().page().get(15, 0), Some(3));
        e.execute(Command::Flip {
            axis: Axis::Vertical,
        })
        .unwrap();
        assert_eq!(e.document().page().get(15, 7), Some(3));
        e.execute(Command::ReplaceColor { from: 0, to: 1 }).unwrap();
        assert_eq!(e.document().page().get(0, 0), Some(1));
        e.execute(Command::SwapColors { a: 1, b: 3 }).unwrap();
        assert_eq!(e.document().page().get(15, 7), Some(1));
        assert_eq!(e.document().page().get(0, 0), Some(3));

        let original = e.document().palette().clone();
        let grey = vec![Rgb8::new(9, 9, 9); 32];
        let out = e
            .execute(Command::SetPalette {
                colors: grey.clone(),
            })
            .unwrap();
        assert!(out.palette_changed && out.affected.is_none());
        assert_eq!(e.document().palette().colors(), &grey[..]);
        e.undo().unwrap();
        assert_eq!(e.document().palette(), &original);

        for _ in 0..5 {
            e.undo().unwrap();
        }
        assert_eq!(e.document().page(), &IndexedImage::new(16, 8, 0));
    }

    #[test]
    fn invalid_commands_change_nothing() {
        let mut e = engine();
        let before = e.document().clone();
        let mut no_brush = stroke(1, &[(0, 0)]);
        if let Command::Stroke { brush, .. } = &mut no_brush {
            brush.clear();
        }
        let cases = [
            (stroke(32, &[(0, 0)]), ErrorCode::PaletteRangeInvalid),
            (stroke(1, &[]), ErrorCode::InvalidArgument),
            (stroke(1, &[(MAX_POINT, 0)]), ErrorCode::InvalidArgument),
            (no_brush, ErrorCode::InvalidArgument),
            (
                Command::ReplaceColor { from: 0, to: 200 },
                ErrorCode::PaletteRangeInvalid,
            ),
            (
                Command::SetPalette {
                    colors: vec![Rgb8::BLACK; 16],
                },
                ErrorCode::InvalidArgument,
            ),
        ];
        for (cmd, code) in cases {
            assert_eq!(e.execute(cmd.clone()).unwrap_err().code, code, "{cmd:?}");
        }
        assert_eq!(e.document(), &before);
        assert!(!e.can_undo());
    }

    const MAX_POINT: i32 = crate::command::MAX_COORD + 1;

    #[test]
    fn set_format_changes_mode_page_and_palette_and_undoes() {
        use crate::{DisplayMode, Resolution, Video};
        let mut e = engine();
        e.execute(stroke(31, &[(2, 2)])).unwrap();
        let before = e.document().clone();
        let hires = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        let out = e
            .execute(Command::SetFormat {
                mode: hires,
                colors: 64,
                width: 640,
                height: 256,
                retain: true,
                screen: None,
            })
            .unwrap();
        assert!(out.format_changed && out.palette_changed);
        assert_eq!(out.affected, Some(Rect::new(0, 0, 640, 256)));
        let d = e.document();
        assert_eq!(d.target().mode, hires);
        assert_eq!((d.page().width(), d.page().height()), (640, 256));
        assert_eq!(d.page().get(2, 2), Some(31), "picture retained");
        assert_eq!(d.palette().len(), 64);
        assert_eq!(d.palette().get(31), before.palette().get(31));
        assert_eq!(d.palette().get(63), Some(Rgb8::WHITE));

        // Fewer colours than the picture uses: refused, nothing changes.
        let err = e
            .execute(Command::SetFormat {
                mode: hires,
                colors: 16,
                width: 640,
                height: 256,
                retain: true,
                screen: None,
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::PaletteRangeInvalid);
        // A page smaller than the screen is refused.
        let err = e
            .execute(Command::SetFormat {
                mode: hires,
                colors: 16,
                width: 100,
                height: 50,
                retain: false,
                screen: None,
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        // Without retain the page is cleared, so 16 colours are fine.
        e.execute(Command::SetFormat {
            mode: hires,
            colors: 16,
            width: 640,
            height: 300,
            retain: false,
            screen: None,
        })
        .unwrap();
        assert_eq!(e.document().page(), &IndexedImage::new(640, 300, 0));

        e.undo().unwrap();
        e.undo().unwrap();
        let restored = e.document();
        assert_eq!(restored.page(), before.page());
        assert_eq!(restored.palette(), before.palette());
        assert_eq!(restored.target(), before.target());
        // stroke, two formats, two undos (the refused command does not count)
        assert_eq!(restored.revision(), 5);
    }

    #[test]
    fn set_format_respects_chipset_limits() {
        use crate::{Chipset, DisplayMode, Resolution, Target, Video};
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES);
        let mut e = Engine::new(Document::from_picture(
            IndexedImage::new(8, 8, 0),
            Palette::dpaint_default_32(),
            ocs,
        ));
        let fmt = |mode: DisplayMode, colors| Command::SetFormat {
            mode,
            colors,
            width: mode.size().0,
            height: mode.size().1,
            retain: false,
            screen: None,
        };
        let hires = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        let shires = DisplayMode::new(Video::Pal, Resolution::SuperHi, false);
        assert_eq!(
            e.execute(fmt(hires, 32)).unwrap_err().code,
            ErrorCode::PaletteRangeInvalid
        );
        assert!(e.execute(fmt(hires, 16)).is_ok());
        assert_eq!(
            e.execute(fmt(shires, 4)).unwrap_err().code,
            ErrorCode::UnsupportedMode
        );
        assert_eq!(
            e.execute(fmt(DisplayMode::PAL_LORES, 24)).unwrap_err().code,
            ErrorCode::PaletteRangeInvalid,
            "not a power of two"
        );
    }

    #[test]
    fn revision_check_and_dry_run() {
        let mut e = engine();
        let dry = e
            .execute(Request {
                command: Command::Clear { color: 4 },
                expected_revision: Some(0),
                dry_run: true,
            })
            .unwrap();
        assert_eq!(dry.revision, 0);
        assert_eq!(dry.affected, Some(Rect::new(0, 0, 16, 8)));
        assert_eq!(e.document().page().get(0, 0), Some(0));
        assert!(!e.can_undo());

        e.execute(Command::Clear { color: 4 }).unwrap();
        let stale = Request {
            command: Command::Clear { color: 5 },
            expected_revision: Some(0),
            dry_run: false,
        };
        assert_eq!(
            e.execute(stale).unwrap_err().code,
            ErrorCode::RevisionConflict
        );
        assert_eq!(e.document().page().get(0, 0), Some(4));
    }

    #[test]
    fn stroke_outside_the_page_is_a_harmless_step() {
        let mut e = engine();
        let out = e.execute(stroke(1, &[(-50, -50)])).unwrap();
        assert_eq!(out.affected, None);
        e.undo().unwrap();
        assert_eq!(e.document().page(), &IndexedImage::new(16, 8, 0));
    }

    #[test]
    fn set_page_size_keeps_the_picture_fills_with_bg_and_undoes() {
        use crate::{DisplayMode, Target};
        let mut page = IndexedImage::new(320, 256, 0);
        page.set(319, 255, 7);
        let mut e = Engine::new(Document::from_picture(
            page.clone(),
            Palette::dpaint_default_32(),
            Target::default(),
        ));
        let size = |width, height| Command::SetPageSize {
            width,
            height,
            fill: 3,
        };
        e.execute(size(640, 400)).unwrap();
        let d = e.document();
        assert_eq!((d.page().width(), d.page().height()), (640, 400));
        assert_eq!(d.page().get(319, 255), Some(7));
        assert_eq!(d.page().get(320, 0), Some(3), "new area in the fill colour");
        assert_eq!(d.page().get(0, 256), Some(3));
        assert_eq!(d.target().mode, DisplayMode::PAL_LORES);
        // Limits: never smaller than the screen, at most 1008x1024; fill must be a colour.
        for bad in [
            size(319, 256),
            size(320, 255),
            size(1009, 300),
            size(400, 1025),
        ] {
            assert_eq!(e.execute(bad).unwrap_err().code, ErrorCode::InvalidArgument);
        }
        let err = e
            .execute(Command::SetPageSize {
                width: 320,
                height: 256,
                fill: 200,
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::PaletteRangeInvalid);
        e.execute(size(1008, 1024)).unwrap();
        // Shrinking crops.
        e.execute(size(320, 256)).unwrap();
        assert_eq!(e.document().page().get(319, 255), Some(7));
        e.undo().unwrap();
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(e.document().page(), &page);
    }

    #[test]
    fn set_format_with_an_overscan_screen() {
        use crate::{DisplayMode, Target};
        let mut e = Engine::new(Document::from_picture(
            IndexedImage::new(320, 256, 0),
            Palette::dpaint_default_32(),
            Target::default(),
        ));
        let fmt = |screen, width, height| Command::SetFormat {
            mode: DisplayMode::PAL_LORES,
            colors: 32,
            width,
            height,
            retain: true,
            screen,
        };
        e.execute(fmt(Some((368, 283)), 368, 283)).unwrap();
        assert_eq!(e.document().target().screen, (368, 283));
        assert_eq!(e.document().page().width(), 368);
        // The page must cover the screen; the screen must fit the mode.
        for bad in [
            fmt(Some((368, 283)), 320, 256),
            fmt(Some((400, 256)), 400, 256),
            fmt(Some((320, 199)), 320, 256),
        ] {
            assert_eq!(e.execute(bad).unwrap_err().code, ErrorCode::InvalidArgument);
        }
        // Page Size now starts at the screen size.
        let err = e
            .execute(Command::SetPageSize {
                width: 320,
                height: 256,
                fill: 0,
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        e.undo().unwrap();
        assert_eq!(e.document().target(), Target::default());
        // JSON: the screen field is optional.
        let json = r#"{"op":"set_format","mode":{"video":"pal","resolution":"lo","interlace":false},"colors":32,"width":320,"height":256,"retain":true}"#;
        let cmd: Command = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, Command::SetFormat { screen: None, .. }));
    }

    fn shape(shape: crate::Shape, filled: bool, symmetry: Option<crate::Symmetry>) -> Command {
        Command::Shape {
            shape,
            brush: vec![(0, 0), (1, 0)],
            color: 4,
            filled,
            symmetry,
            custom: None,
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    /// A Shape without a built-in brush (`brush` empty), else like [`shape`].
    fn bare_shape(shape: crate::Shape, color: u8, filled: bool) -> Command {
        let mut cmd = self::shape(shape, filled, None);
        if let Command::Shape {
            brush, color: c, ..
        } = &mut cmd
        {
            brush.clear();
            *c = color;
        }
        cmd
    }

    #[test]
    fn shapes_match_their_preview_and_undo_in_one_step() {
        use crate::{Shape, Symmetry};
        let sym = Some(Symmetry {
            center: (8, 4),
            order: 6,
            aspect: (1, 1),
            mirror: false,
        });
        let cases = [
            shape(
                Shape::Line {
                    from: (-3, 1),
                    to: (12, 6),
                },
                false,
                None,
            ),
            shape(
                Shape::Curve {
                    from: (0, 7),
                    to: (15, 7),
                    through: (7, 0),
                },
                false,
                sym,
            ),
            shape(
                Shape::Rect {
                    from: (2, 1),
                    to: (9, 6),
                },
                true,
                sym,
            ),
            shape(
                Shape::Ellipse {
                    center: (8, 4),
                    radius: (6, 3),
                },
                false,
                None,
            ),
            shape(
                Shape::Polygon {
                    points: vec![(1, 1), (14, 2), (6, 7)],
                    closed: false,
                },
                true,
                None,
            ),
            shape(
                Shape::Dots {
                    points: vec![(3, 3), (20, 20), (9, 2)],
                },
                false,
                sym,
            ),
            Command::FloodFill {
                x: 0,
                y: 0,
                color: 6,
                fill: crate::gradient::FillType::Solid,
            },
        ];
        for cmd in cases {
            let mut e = engine();
            e.execute(stroke(2, &[(0, 3), (15, 3)])).unwrap();
            let before = e.document().page().clone();
            let mut preview = before.clone();
            assert!(cmd.paint(&mut preview, &before, e.document().palette()));
            let out = e.execute(cmd.clone()).unwrap();
            assert_eq!(e.document().page(), &preview, "{cmd:?}");
            assert_ne!(&preview, &before, "{cmd:?} changes something");
            let r = out.affected.unwrap();
            for y in 0..8 {
                for x in 0..16 {
                    if preview.get(x, y) != before.get(x, y) {
                        assert!(r.contains(x, y), "{cmd:?}: ({x},{y}) outside {r:?}");
                    }
                }
            }
            e.undo().unwrap();
            assert_eq!(e.document().page(), &before, "{cmd:?} undone");
            e.redo().unwrap();
            assert_eq!(e.document().page(), &preview, "{cmd:?} redone");
        }
    }

    #[test]
    fn flood_fill_only_snapshots_the_filled_region() {
        let mut e = engine();
        e.execute(stroke(2, &[(5, 0), (5, 7)])).unwrap();
        let out = e
            .execute(Command::FloodFill {
                x: 0,
                y: 0,
                color: 3,
                fill: crate::gradient::FillType::Solid,
            })
            .unwrap();
        assert_eq!(out.affected, Some(Rect::new(0, 0, 5, 8)));
        assert_eq!(e.document().page().get(7, 0), Some(0), "wall stops it");
        // Filling with the colour already there changes nothing but is a harmless step.
        let out = e
            .execute(Command::FloodFill {
                x: 0,
                y: 0,
                color: 3,
                fill: crate::gradient::FillType::Solid,
            })
            .unwrap();
        assert_eq!(out.affected, None);
    }

    #[test]
    fn invalid_shapes_change_nothing() {
        use crate::shapes::{MAX_SHAPE_COORD, MAX_SYMMETRY_ORDER};
        use crate::{Shape, Symmetry};
        let mut e = engine();
        let before = e.document().clone();
        let line = Shape::Line {
            from: (0, 0),
            to: (3, 3),
        };
        let cases = [
            (
                bare_shape(line.clone(), 1, false),
                ErrorCode::InvalidArgument,
            ),
            (
                bare_shape(line.clone(), 99, true),
                ErrorCode::PaletteRangeInvalid,
            ),
            (
                shape(
                    Shape::Line {
                        from: (0, 0),
                        to: (MAX_SHAPE_COORD + 1, 0),
                    },
                    false,
                    None,
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                shape(
                    Shape::Ellipse {
                        center: (0, 0),
                        radius: (-1, 2),
                    },
                    true,
                    None,
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                shape(
                    Shape::Polygon {
                        points: vec![],
                        closed: true,
                    },
                    true,
                    None,
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                shape(
                    line.clone(),
                    false,
                    Some(Symmetry {
                        center: (0, 0),
                        order: MAX_SYMMETRY_ORDER + 1,
                        aspect: (1, 1),
                        mirror: false,
                    }),
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                shape(
                    line,
                    false,
                    Some(Symmetry {
                        center: (0, 0),
                        order: 6,
                        aspect: (0, 1),
                        mirror: false,
                    }),
                ),
                ErrorCode::InvalidArgument,
            ),
            (
                Command::FloodFill {
                    x: 0,
                    y: 0,
                    color: 32,
                    fill: crate::gradient::FillType::Solid,
                },
                ErrorCode::PaletteRangeInvalid,
            ),
        ];
        for (cmd, code) in cases {
            assert_eq!(e.execute(cmd.clone()).unwrap_err().code, code, "{cmd:?}");
        }
        // A filled shape needs no brush.
        e.execute(bare_shape(
            Shape::Rect {
                from: (0, 0),
                to: (2, 2),
            },
            1,
            true,
        ))
        .unwrap();
        e.undo().unwrap();
        assert_eq!(e.document().page(), before.page());
    }

    /// Colour of pixel (2, 2) in every frame.
    fn frame_pixels(e: &Engine) -> Vec<u8> {
        e.document()
            .frames()
            .map(|f| f.get(2, 2).unwrap())
            .collect()
    }

    #[test]
    fn page_commands_act_on_the_current_frame_and_undo_goes_back_to_it() {
        let mut e = engine();
        e.execute(Command::Clear { color: 1 }).unwrap();
        e.execute(Command::SetFrameCount { count: 3 }).unwrap();
        assert_eq!(e.document().frame_count(), 3);
        assert_eq!(
            frame_pixels(&e),
            [1, 1, 1],
            "new frames copy the current one"
        );
        // Going to a frame is no undo step.
        let undo_before = e.can_undo();
        let out = e.execute(Command::GotoFrame { frame: 1 }).unwrap();
        assert_eq!(out.affected, Some(Rect::new(0, 0, 16, 8)));
        assert_eq!(e.can_undo(), undo_before);
        e.execute(stroke(5, &[(2, 2)])).unwrap();
        assert_eq!(frame_pixels(&e), [1, 5, 1]);
        // Undo from another frame goes back to frame 2 and restores it there.
        e.execute(Command::GotoFrame { frame: 2 }).unwrap();
        e.undo().unwrap();
        assert_eq!(e.document().current_frame(), 1);
        assert_eq!(frame_pixels(&e), [1, 1, 1]);
        // Going elsewhere keeps the redo list; redo paints frame 2 again.
        e.execute(Command::GotoFrame { frame: 0 }).unwrap();
        assert!(e.can_redo());
        e.redo().unwrap();
        assert_eq!(e.document().current_frame(), 1);
        assert_eq!(frame_pixels(&e), [1, 5, 1]);
        // Frame commands undo as one step, back to the frame they ran on.
        e.execute(Command::AddFrames { count: 2 }).unwrap();
        assert_eq!(frame_pixels(&e), [1, 5, 5, 5, 1]);
        assert_eq!(e.document().current_frame(), 3);
        e.execute(Command::SetFrameRate { fps: 12 }).unwrap();
        e.undo().unwrap();
        assert_eq!(frame_pixels(&e), [1, 5, 1]);
        assert_eq!(e.document().current_frame(), 1);
        assert_eq!(e.document().fps(), 12, "the rate is a setting, not undone");
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(e.document().frame_count(), 1);
        e.undo().unwrap();
        assert_eq!(e.document().page(), &IndexedImage::new(16, 8, 0));
        // A dry run goes nowhere.
        e.redo().unwrap();
        e.redo().unwrap();
        let dry = Request {
            command: Command::GotoFrame { frame: 2 },
            expected_revision: None,
            dry_run: true,
        };
        e.execute(dry).unwrap();
        assert_eq!(e.document().current_frame(), 0);
        assert_eq!(
            e.execute(Command::GotoFrame { frame: 3 }).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn page_size_and_format_change_every_frame() {
        use crate::{DisplayMode, Target};
        let mut e = Engine::new(Document::from_picture(
            IndexedImage::new(320, 256, 2),
            Palette::dpaint_default_32(),
            Target::default(),
        ));
        e.execute(Command::SetFrameCount { count: 3 }).unwrap();
        e.execute(Command::GotoFrame { frame: 2 }).unwrap();
        e.execute(Command::Clear { color: 31 }).unwrap();
        e.execute(Command::SetPageSize {
            width: 400,
            height: 300,
            fill: 4,
        })
        .unwrap();
        for f in e.document().frames() {
            assert_eq!((f.width(), f.height()), (400, 300));
            assert_eq!(f.get(399, 299), Some(4));
        }
        // Colours of every frame count when the palette shrinks.
        e.execute(Command::GotoFrame { frame: 0 }).unwrap();
        let fewer = Command::SetFormat {
            mode: DisplayMode::PAL_LORES,
            colors: 16,
            width: 400,
            height: 300,
            retain: true,
            screen: None,
        };
        assert_eq!(
            e.execute(fewer).unwrap_err().code,
            ErrorCode::PaletteRangeInvalid,
            "frame 3 uses colour 31"
        );
        // Another page size by SetPage would split the frames: refused.
        let other = Command::SetPage {
            width: 320,
            height: 256,
            pixels: vec![0; 320 * 256],
        };
        assert_eq!(
            e.execute(other).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        // Too many frames for the memory limit: a big page with many frames is refused.
        e.execute(Command::SetFrameCount { count: 200 }).unwrap();
        let big = Command::SetPageSize {
            width: 1008,
            height: 1024,
            fill: 0,
        };
        assert_eq!(e.execute(big).unwrap_err().code, ErrorCode::InvalidArgument);
        e.undo().unwrap();
        e.undo().unwrap();
        assert!(e.document().frames().all(|f| f.width() == 320));
        assert_eq!(e.document().frame_count(), 3);
        // JSON names of the frame commands.
        let json = serde_json::to_string(&Command::GotoFrame { frame: 2 }).unwrap();
        assert_eq!(json, r#"{"op":"goto_frame","frame":2}"#);
        let cmd: Command =
            serde_json::from_str(r#"{"op":"delete_frames","first":0,"last":1}"#).unwrap();
        assert_eq!(
            cmd,
            Command::DeleteFrames {
                first: 0,
                last: 1,
                fill: 0
            }
        );
    }

    /// A 3×2 brush with colour 0 transparent: `.5.` over `66.`.
    fn small_brush() -> BrushImage {
        BrushImage::from_image(
            &IndexedImage::from_pixels(3, 2, vec![0, 5, 0, 6, 6, 0]).unwrap(),
            Some(0),
        )
    }

    /// A stroke of `points` with the custom brush only (no built-in brush).
    fn custom_stroke(brush: BrushImage, mode: DrawMode, points: &[(i32, i32)]) -> Command {
        let mut cmd = stroke(3, points);
        if let Command::Stroke {
            brush: offsets,
            custom,
            mode: m,
            ..
        } = &mut cmd
        {
            offsets.clear();
            *custom = Some(brush);
            *m = mode;
        }
        cmd
    }

    #[test]
    fn a_custom_brush_stroke_is_one_undo_step_and_replays() {
        let mut e = engine();
        e.execute(Command::Clear { color: 9 }).unwrap();
        let before = e.document().page().clone();
        let cmd = custom_stroke(small_brush(), DrawMode::Matte, &[(2, 2), (10, 5)]);
        let mut preview = before.clone();
        cmd.paint(&mut preview, &before, e.document().palette());
        let out = e.execute(cmd).unwrap();
        assert_eq!(e.document().page(), &preview, "preview = result");
        // The handle (1, 1) sits on each point: brush pixel (1, 0) = 5 lands one row above.
        assert_eq!(e.document().page().get(2, 1), Some(5));
        assert_eq!(e.document().page().get(1, 1), Some(9), "transparent pixel");
        // The undo footprint covers the custom brush around every point.
        let r = out.affected.unwrap();
        for y in 0..8 {
            for x in 0..16 {
                if preview.get(x, y) != before.get(x, y) {
                    assert!(r.contains(x, y), "({x},{y}) outside {r:?}");
                }
            }
        }
        e.undo().unwrap();
        assert_eq!(e.document().page(), &before);
        e.redo().unwrap();
        assert_eq!(e.document().page(), &preview);
    }

    #[test]
    fn paint_fields_are_validated() {
        let mut e = engine();
        let ok = custom_stroke(small_brush(), DrawMode::Replace, &[(3, 3)]);
        e.execute(ok.clone()).unwrap();
        let with = |f: &dyn Fn(&mut BrushImage)| {
            let mut b = small_brush();
            f(&mut b);
            custom_stroke(b, DrawMode::Matte, &[(3, 3)])
        };
        let mut long_stencil = ok.clone();
        let mut bad_range = ok.clone();
        let mut outside_range = ok.clone();
        if let (
            Command::Stroke { stencil, .. },
            Command::Stroke { range, .. },
            Command::Stroke { range: outside, .. },
        ) = (&mut long_stencil, &mut bad_range, &mut outside_range)
        {
            *stencil = Some(vec![false; 255]);
            *range = Some((4, 2));
            *outside = Some((20, 40));
        }
        let cases = [
            (
                with(&|b| {
                    b.pixels.pop();
                }),
                ErrorCode::InvalidArgument,
            ),
            (with(&|b| b.pixels[1] = 32), ErrorCode::PaletteRangeInvalid),
            (
                with(&|b| b.transparent = Some(40)),
                ErrorCode::PaletteRangeInvalid,
            ),
            (
                with(&|b| {
                    // One limit for every brush (shared with brush_ops).
                    b.width = crate::command::MAX_BRUSH_SIDE + 1;
                    b.pixels = vec![1; b.width as usize * 2];
                }),
                ErrorCode::InvalidArgument,
            ),
            (with(&|b| b.height = 0), ErrorCode::InvalidArgument),
            (long_stencil, ErrorCode::InvalidArgument),
            (bad_range, ErrorCode::InvalidArgument),
            (outside_range, ErrorCode::PaletteRangeInvalid),
        ];
        for (cmd, code) in cases {
            assert_eq!(e.execute(cmd.clone()).unwrap_err().code, code, "{cmd:?}");
        }
        assert_eq!(e.document().revision(), 1, "nothing else ran");
    }

    /// A one-row document of `pixels` (PAL LoRes target, default palette).
    fn row_engine(pixels: Vec<u8>) -> Engine {
        let w = pixels.len() as u32;
        Engine::new(Document::from_picture(
            IndexedImage::from_pixels(w, 1, pixels).unwrap(),
            Palette::dpaint_default_32(),
            crate::Target::default(),
        ))
    }

    #[test]
    fn smear_is_mirror_symmetric() {
        // Colours 2, 3, ..., 22 in one row; a 5x1 opaque brush with the handle in the middle.
        let row: Vec<u8> = (2..=22).collect();
        let brush = BrushImage {
            width: 5,
            height: 1,
            pixels: vec![1; 5],
            handle: (2, 0),
            transparent: None,
        };
        let smear = |pixels: Vec<u8>, from, to| {
            let mut e = row_engine(pixels);
            e.execute(custom_stroke(brush.clone(), DrawMode::Smear, &[from, to]))
                .unwrap();
            e.document().page().pixels().to_vec()
        };
        // One step to the right moves the colours under the brush one pixel to the right.
        let right = smear(row.clone(), (10, 0), (11, 0));
        let mut expect: Vec<u8> = (2..=10).collect();
        expect.extend([10, 11, 12, 13, 14]);
        expect.extend(16..=22);
        assert_eq!(right, expect);
        // The mirrored picture smeared to the left, mirrored back, is the same.
        let mut left = smear(row.iter().rev().copied().collect(), (10, 0), (9, 0));
        left.reverse();
        assert_eq!(left, right, "the result depends on the direction");
        // A built-in brush gives the same, whatever the order of its offsets.
        let offsets: Vec<(i32, i32)> = (-2..=2).map(|x| (x, 0)).collect();
        for order in [offsets.clone(), offsets.iter().rev().copied().collect()] {
            let mut e = row_engine(row.clone());
            let mut cmd = stroke(3, &[(10, 0), (11, 0)]);
            if let Command::Stroke { brush, mode, .. } = &mut cmd {
                *brush = order.clone();
                *mode = DrawMode::Smear;
            }
            e.execute(cmd).unwrap();
            assert_eq!(e.document().page().pixels(), &right[..], "{order:?}");
        }
    }

    #[test]
    fn undo_and_redo_on_another_frame_report_the_whole_page() {
        let mut e = Engine::new(Document::new(8, 8, Palette::dpaint_default_32()));
        e.execute(Command::SetFrameCount { count: 2 }).unwrap();
        e.execute(Command::GotoFrame { frame: 1 }).unwrap();
        let painted = e.execute(stroke(5, &[(0, 0)])).unwrap();
        assert_eq!(painted.affected, Some(Rect::new(0, 0, 2, 2)));
        e.execute(Command::GotoFrame { frame: 0 }).unwrap();
        let whole = Some(Rect::new(0, 0, 8, 8));
        let out = e.undo().unwrap();
        assert_eq!(e.document().current_frame(), 1);
        assert_eq!(out.affected, whole, "undo switched frames");
        e.execute(Command::GotoFrame { frame: 0 }).unwrap();
        let out = e.redo().unwrap();
        assert_eq!(e.document().current_frame(), 1);
        assert_eq!(out.affected, whole, "redo switched frames");
        // On the step's own frame only its rectangle changes.
        assert_eq!(e.undo().unwrap().affected, painted.affected);
        assert_eq!(e.redo().unwrap().affected, painted.affected);
    }

    #[test]
    fn brush_transforms_always_give_a_paintable_brush() {
        use crate::brush_ops;
        // A 600x4 brush, as picked from a 640-wide HiRes screen.
        let wide = BrushImage {
            width: 600,
            height: 4,
            pixels: vec![5; 2400],
            handle: (300, 2),
            transparent: Some(0),
        };
        let doubled = |b: &BrushImage| brush_ops::double(b, true, false);
        let results = [
            brush_ops::stretch(&wide, 100_000, 4),
            brush_ops::stretch(&wide, 4, 100_000),
            doubled(&wide),
            doubled(&doubled(&doubled(&doubled(&wide)))),
            brush_ops::shear(&wide, i32::MAX / 2, 32),
            brush_ops::shear(&rotate_quarter(&wide), -100_000, 32),
            brush_ops::bend(&wide, false, 100_000, 1, 32),
            brush_ops::bend(&rotate_quarter(&wide), true, -100_000, 1, 32),
            brush_ops::rotate_any(&wide, 45.0, 32),
            brush_ops::outline(&brush_ops::stretch(&wide, 100_000, 3), 7),
        ];
        let mut e = engine();
        for b in results {
            let size = (b.width, b.height);
            e.execute(custom_stroke(b, DrawMode::Matte, &[(3, 3)]))
                .unwrap_or_else(|err| panic!("{size:?}: {err}"));
        }
        // Rotating keeps every pixel (no silent crop).
        let long = brush_ops::stretch(&wide, 3000, 4);
        let turned = brush_ops::rotate_any(&long, 90.0, 32);
        assert_eq!((turned.width, turned.height), (4, 3000));
    }

    /// Pixel bytes the undo history keeps alive beyond the document itself: snapshot pages,
    /// rectangles and every frame buffer not shared with the document (counted once).
    fn held_bytes(e: &Engine) -> u64 {
        let live: Vec<*const IndexedImage> = e.doc.others.iter().map(Arc::as_ptr).collect();
        let mut seen = Vec::new();
        let mut bytes = 0u64;
        for step in &e.undo {
            match &step.before {
                Before::Pixels { pixels, .. } => bytes += pixels.pixels().len() as u64,
                Before::Document(d) => {
                    bytes += d.page.pixels().len() as u64;
                    for f in &d.others {
                        let p = Arc::as_ptr(f);
                        if !live.contains(&p) && !seen.contains(&p) {
                            seen.push(p);
                            bytes += f.pixels().len() as u64;
                        }
                    }
                }
                _ => {}
            }
        }
        bytes
    }

    #[test]
    fn arrange_palette_undo_keeps_no_frame_copies() {
        let mut e = Engine::new(Document::new(64, 64, Palette::dpaint_default_32()));
        e.execute(Command::SetFrameCount { count: 200 }).unwrap();
        let animation = 200 * 64 * 64;
        let shared = held_bytes(&e);
        assert!(shared < animation / 10, "frames are shared: {shared}");
        let mut order: Vec<u8> = (0..32).collect();
        order.swap(0, 1);
        for _ in 0..10 {
            e.execute(Command::ArrangePalette {
                order: order.clone(),
            })
            .unwrap();
        }
        let held = held_bytes(&e);
        assert!(
            held < animation,
            "10 arranges keep {held} bytes of an animation of {animation}"
        );
        // Every arrange still undoes exactly.
        for _ in 0..10 {
            e.undo().unwrap();
        }
        assert!(e.document().frames().all(|f| f.get(0, 0) == Some(0)));
        assert_eq!(e.document().palette(), &Palette::dpaint_default_32());
    }

    #[test]
    fn the_undo_history_keeps_a_byte_budget() {
        assert_eq!(
            Engine::DEFAULT_UNDO_BYTES,
            2 * crate::anim::MAX_FRAME_BYTES,
            "twice the frame limit (ADR-28)"
        );
        let page = || IndexedImage::new(320, 256, 0);
        let doc = Document::from_picture(page(), Palette::dpaint_default_32(), Default::default());
        // Room for about three whole-animation copies of 10 frames.
        let animation = 10 * 320 * 256;
        let mut e = Engine::with_undo_budget(doc, 64, 3 * animation + animation / 2);
        e.execute(Command::SetFrameCount { count: 10 }).unwrap();
        let shared = e.undo_bytes;
        assert!(shared <= 320 * 256, "new frames are shared: {shared}");
        // Each Page Size gives every frame a new buffer; its step keeps the old ones.
        for i in 0..10u32 {
            e.execute(Command::SetPageSize {
                width: 320,
                height: 256 + i % 2,
                fill: 0,
            })
            .unwrap();
            assert_eq!(e.undo_bytes, e.undo.iter().map(|s| s.bytes).sum::<u64>());
            assert!(e.undo_bytes <= e.undo_byte_limit, "{}", e.undo_bytes);
            assert!(
                held_bytes(&e) <= e.undo_bytes,
                "the count covers what is held"
            );
        }
        assert_eq!(e.undo.len(), 3, "only the newest steps stay");
        for _ in 0..3 {
            e.undo().unwrap();
        }
        assert_eq!(e.undo().unwrap_err().code, ErrorCode::NothingToUndo);
        assert_eq!(e.undo_bytes, 0);
        // The newest step stays even when it alone exceeds the budget.
        let mut tiny =
            Engine::with_undo_budget(Document::new(8, 8, Palette::dpaint_default_32()), 64, 1);
        tiny.execute(Command::Clear { color: 1 }).unwrap();
        tiny.execute(Command::Clear { color: 2 }).unwrap();
        assert_eq!(tiny.undo.len(), 1);
        tiny.undo().unwrap();
        assert_eq!(tiny.document().page().get(0, 0), Some(1));
    }

    #[test]
    fn check_paint_is_the_engine_check_for_page_pixel_commands() {
        let e = engine();
        let (page, pal) = (e.document().page(), e.document().palette());
        assert!(stroke(3, &[(1, 1)]).check_paint(page, pal).is_ok());
        assert_eq!(
            stroke(40, &[(1, 1)])
                .check_paint(page, pal)
                .unwrap_err()
                .code,
            ErrorCode::PaletteRangeInvalid
        );
        assert_eq!(
            Command::Clear { color: 1 }
                .check_paint(page, pal)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    fn rotate_quarter(b: &BrushImage) -> BrushImage {
        crate::brush_ops::rotate_90(b)
    }

    #[test]
    fn work_far_off_the_page_is_free_and_huge_work_is_refused() {
        let dry = |e: &mut Engine, command: Command| {
            e.execute(Request {
                command,
                expected_revision: None,
                dry_run: true,
            })
        };
        // 100 000 points zigzagging far beyond the page (the review's case): every segment
        // is clipped to the few pixels near the page, so it is cheap and allowed.
        let far: Vec<(i32, i32)> = (0..100_000)
            .map(|i| {
                if i % 2 == 0 {
                    (-1_000_000, -1_000_000)
                } else {
                    (1_000_000, 1_000_000)
                }
            })
            .collect();
        let zigzag = stroke(1, &far);
        let work = zigzag.paint_work(16, 8, 32);
        assert!(work < 5_000_000, "{work}");
        let mut e = engine();
        e.execute(zigzag).unwrap();
        assert_eq!(
            e.document().page().get(5, 5),
            Some(1),
            "the diagonal is painted"
        );
        // Never near the page: only the points count.
        let mut beside = far.clone();
        for p in beside.iter_mut().skip(1).step_by(2) {
            p.1 = -999_000;
        }
        assert_eq!(stroke(1, &beside).paint_work(16, 8, 32), 100_000);
        // A page-sized custom brush stamped 100 000 times on a big page: refused.
        let mut e = Engine::new(Document::new(1008, 1024, Palette::dpaint_default_32()));
        let big = BrushImage {
            width: 1008,
            height: 1024,
            pixels: vec![1; 1008 * 1024],
            handle: (0, 0),
            transparent: None,
        };
        let mut cmd = custom_stroke(big, DrawMode::Matte, &[(0, 0); 100_000]);
        if let Command::Stroke { continuous, .. } = &mut cmd {
            *continuous = false;
        }
        assert_eq!(
            dry(&mut e, cmd).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        // A filled polygon whose 100 000 edges each cross every row of a tall page: refused.
        let mut e = Engine::new(Document::new(320, 4096, Palette::dpaint_default_32()));
        let zigzag: Vec<(i32, i32)> = (0..100_000)
            .map(|i| (i % 320, if i % 2 == 0 { -16_384 } else { 16_384 }))
            .collect();
        let poly = shape(
            crate::Shape::Polygon {
                points: zigzag,
                closed: true,
            },
            true,
            None,
        );
        assert_eq!(
            dry(&mut e, poly).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    /// Commands just inside [`crate::command::MAX_PAINT_WORK`] and how long they take. Run in
    /// a release build: `cargo test --release -p dpx-core -- --ignored --nocapture timing`.
    #[test]
    #[ignore = "timing, release build only"]
    fn timing_of_commands_at_the_work_budget() {
        use crate::command::MAX_PAINT_WORK;
        use std::time::Instant;
        let page = || Engine::new(Document::new(1008, 1024, Palette::dpaint_default_32()));
        let square: Vec<(i32, i32)> = (0..100 * 100)
            .map(|i| (i % 100 - 50, i / 100 - 50))
            .collect();
        // Back and forth across the page diagonal until the budget is nearly used.
        let zigzag = |per_stamp: u64| -> Vec<(i32, i32)> {
            let segments = (MAX_PAINT_WORK / per_stamp / 1025).max(1) as usize;
            (0..=segments)
                .map(|i| if i % 2 == 0 { (0, 0) } else { (1007, 1023) })
                .collect()
        };
        let run = |name: &str, cmd: Command| {
            let mut e = page();
            let work = cmd.paint_work(1008, 1024, 32);
            let t = Instant::now();
            let r = e.execute(cmd);
            println!(
                "{name}: work {work} ({:.2} of budget), {:?}, ok={}",
                work as f64 / MAX_PAINT_WORK as f64,
                t.elapsed(),
                r.is_ok()
            );
        };
        for mode in [DrawMode::Matte, DrawMode::Smear, DrawMode::Blend] {
            let weight = crate::paint::pixel_work(mode, &Default::default(), 32);
            let mut cmd = stroke(1, &zigzag(weight * square.len() as u64));
            if let Command::Stroke { brush, mode: m, .. } = &mut cmd {
                *brush = square.clone();
                *m = mode;
            }
            run(&format!("built-in 100x100 {mode:?}"), cmd);
            let custom = BrushImage {
                width: 100,
                height: 100,
                pixels: vec![1; 100 * 100],
                handle: (50, 50),
                transparent: None,
            };
            run(
                &format!("custom 100x100 {mode:?}"),
                custom_stroke(custom, mode, &zigzag(weight * 10_000)),
            );
        }
        let edges: Vec<(i32, i32)> = (0..100_000)
            .map(|i| (i % 1008, if i % 2 == 0 { -16_384 } else { 16_384 }))
            .collect();
        let n = (MAX_PAINT_WORK / crate::command::POLYGON_CROSSING_WORK / 1024 * 9 / 10) as usize;
        run(
            "filled polygon",
            shape(
                crate::Shape::Polygon {
                    points: edges[..n.min(edges.len())].to_vec(),
                    closed: true,
                },
                true,
                None,
            ),
        );
        let far: Vec<(i32, i32)> = (0..100_000)
            .map(|i| {
                if i % 2 == 0 {
                    (-1_000_000, -1_000_000)
                } else {
                    (1_000_000, 1_000_000)
                }
            })
            .collect();
        run("100 000 points far off the page", stroke(1, &far));
    }
}
