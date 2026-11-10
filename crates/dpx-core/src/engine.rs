//! The command engine: validates and applies [`Command`]s, keeps the revision counter and a
//! multi-level undo/redo history (ADR-2, Phase 1 implementation: see ADR-6).

use std::collections::VecDeque;

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
    Pixels { rect: Rect, pixels: IndexedImage },
    Palette(Palette),
    Document(Box<Document>),
}

#[derive(Clone, Debug)]
struct Step {
    command: Command,
    before: Before,
}

#[derive(Clone, Debug)]
pub struct Engine {
    doc: Document,
    undo: VecDeque<Step>,
    /// Undone commands, most recent last; redo replays them (commands are deterministic).
    redo: Vec<Command>,
    undo_limit: usize,
}

impl Engine {
    pub const DEFAULT_UNDO_LIMIT: usize = 64;

    pub fn new(doc: Document) -> Self {
        Self::with_undo_limit(doc, Self::DEFAULT_UNDO_LIMIT)
    }

    /// `limit` = number of undo steps kept (at least 1); the oldest steps are dropped.
    pub fn with_undo_limit(doc: Document, limit: usize) -> Self {
        Self {
            doc,
            undo: VecDeque::new(),
            redo: Vec::new(),
            undo_limit: limit.max(1),
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

    /// Validates and executes a command as one undo step. A new command clears the redo list.
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
        self.redo.clear();
        Ok(self.commit(request.command, footprint))
    }

    /// Reverts the most recent step.
    pub fn undo(&mut self) -> Result<Outcome, CommandError> {
        let step = self
            .undo
            .pop_back()
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToUndo, "nothing to undo"))?;
        let footprint = match step.before {
            Before::Pixels { rect, pixels } => {
                self.doc.page.blit(&pixels, rect.x, rect.y);
                Footprint::Pixels(rect)
            }
            Before::Palette(palette) => {
                self.doc.palette = palette;
                Footprint::Palette
            }
            Before::Document(doc) => {
                let revision = self.doc.revision;
                self.doc = *doc;
                self.doc.revision = revision;
                Footprint::Document
            }
        };
        self.redo.push(step.command);
        self.doc.revision += 1;
        Ok(self.outcome(footprint))
    }

    /// Re-applies the most recently undone command.
    pub fn redo(&mut self) -> Result<Outcome, CommandError> {
        let command = self
            .redo
            .pop()
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToRedo, "nothing to redo"))?;
        // The document is back in the state the command was first validated against.
        command.validate(&self.doc)?;
        let footprint = command.footprint(&self.doc);
        Ok(self.commit(command, footprint))
    }

    fn commit(&mut self, command: Command, footprint: Footprint) -> Outcome {
        let before = match footprint {
            Footprint::Pixels(rect) => Before::Pixels {
                rect,
                pixels: self.doc.page.crop(rect),
            },
            Footprint::Palette => Before::Palette(self.doc.palette.clone()),
            Footprint::Document => Before::Document(Box::new(self.doc.clone())),
        };
        command.apply(&mut self.doc);
        if self.undo.len() == self.undo_limit {
            self.undo.pop_front();
        }
        self.undo.push_back(Step { command, before });
        self.doc.revision += 1;
        self.outcome(footprint)
    }

    fn outcome(&self, footprint: Footprint) -> Outcome {
        let (affected, palette_changed, format_changed) = match footprint {
            Footprint::Pixels(r) => ((!r.is_empty()).then_some(r), false, false),
            Footprint::Palette => (None, true, false),
            Footprint::Document => (Some(self.doc.page.bounds()), true, true),
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
    use crate::command::Axis;
    use crate::raster;
    use crate::{IndexedImage, Rgb8};

    fn engine() -> Engine {
        Engine::new(Document::new(16, 8, Palette::dpaint_default_32()))
    }

    fn stroke(color: u8, points: &[(i32, i32)]) -> Command {
        Command::Stroke {
            brush: vec![(0, 0), (1, 0), (0, 1)],
            color,
            points: points.to_vec(),
            continuous: true,
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
        let cases = [
            (stroke(32, &[(0, 0)]), ErrorCode::PaletteRangeInvalid),
            (stroke(1, &[]), ErrorCode::InvalidArgument),
            (stroke(1, &[(MAX_POINT, 0)]), ErrorCode::InvalidArgument),
            (
                Command::Stroke {
                    brush: vec![],
                    color: 1,
                    points: vec![(0, 0)],
                    continuous: false,
                },
                ErrorCode::InvalidArgument,
            ),
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
}
