//! The editor state the tools work on (ADR-43). Tool logic never owns a document: it sends
//! jobs ([`crate::jobs`]) to whichever thread owns a [`Workspace`] — the headless engine
//! here, or the running GUI (`PainterApp`, which implements the trait on its UI thread).
//! A workspace only changes documents through its engine's commands, exactly like the mouse
//! and keyboard, and never blocks: file I/O and encoding happen on the server's threads.

use std::path::{Path, PathBuf};

use dpx_core::{Command, Document, Engine, IndexedImage, Outcome, Palette, Request};
use dpx_formats::project::ProjectExtras;

use crate::error::ToolError;

/// One of the two pages (Picture > Spare). The ids stay with a page when the pages are
/// swapped, so `select_document` is idempotent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    First,
    Second,
}

impl Slot {
    pub fn id(self) -> &'static str {
        match self {
            Slot::First => "page1",
            Slot::Second => "page2",
        }
    }

    pub fn parse(id: &str) -> Option<Slot> {
        match id {
            "page1" => Some(Slot::First),
            "page2" => Some(Slot::Second),
            _ => None,
        }
    }

    pub fn other(self) -> Slot {
        match self {
            Slot::First => Slot::Second,
            Slot::Second => Slot::First,
        }
    }
}

/// A custom brush made by `import_image` (target `brush`), in the picture's palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brush {
    pub image: IndexedImage,
    /// The colour that is not painted (transparent pixels of the imported picture).
    pub transparent: Option<u8>,
}

/// A yes/no question for the user (e.g. "replace this file?").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub title: String,
    pub text: String,
    pub yes: String,
    pub no: String,
}

/// Where a replacing picture came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PictureInfo {
    /// The file it was read from (open_picture), if any.
    pub path: Option<PathBuf>,
    /// A name to show for it (create_document's `name`, an imported file's name).
    pub name: Option<String>,
    /// Picture details a project keeps (ranges, grab point, transparent colour).
    pub extras: ProjectExtras,
    /// True if it equals a file on disk (just opened), so it is not "modified".
    pub saved: bool,
}

/// The editor state. Implemented by [`HeadlessWorkspace`] and by the GUI.
///
/// Rules for implementations:
/// - Every method runs on the owning thread and returns quickly.
/// - `execute`, `undo` and `redo` act on the shown page exactly as the GUI's own commands.
///   The GUI refuses changing calls with `EDITOR_BUSY` while a requester is open and holds
///   them while a mouse button is held on the picture ([`crate::jobs::JobKind`]); that
///   happens before a job runs, so implementations need not check it again.
/// - `swap_spare` swaps the pages *and* their slots, so `shown()` changes with it.
pub trait Workspace {
    /// `headless` or `gui`.
    fn kind(&self) -> &'static str;
    /// The shown page's engine (its document, undo state).
    fn engine(&self) -> &Engine;
    fn execute(&mut self, request: Request) -> Result<Outcome, ToolError>;
    fn undo(&mut self) -> Result<Outcome, ToolError>;
    fn redo(&mut self) -> Result<Outcome, ToolError>;
    /// Replaces the shown page with `doc` like Picture > Load (not undoable).
    fn replace_picture(&mut self, doc: Document, info: PictureInfo) -> Result<(), ToolError>;
    /// The slot of the shown page.
    fn shown(&self) -> Slot;
    /// The other page as it would be shown by a swap (same format and palette as the shown
    /// page, ADR-15); `None` until it is first used.
    fn spare(&self) -> Option<Document>;
    /// Makes `page` the other page's picture (one undo step on that page). It must have the
    /// shown page's size and use its palette.
    fn set_spare_page(&mut self, page: IndexedImage) -> Result<(), ToolError>;
    /// Picture > Spare > Swap.
    fn swap_spare(&mut self) -> Result<(), ToolError>;
    fn set_brush(&mut self, brush: Brush) -> Result<(), ToolError>;
    fn brush_size(&self) -> Option<(u32, u32)>;
    /// The shown page's name: its file name, or the name it was given.
    fn picture_name(&self) -> Option<String>;
    fn set_picture_name(&mut self, name: Option<String>);
    /// True if the shown page changed since it was last opened or saved.
    fn modified(&self) -> bool;
    /// The shown page was saved as a project at `path` when it was at `revision`. Only
    /// called while the shown page is still the document that was saved (same slot and
    /// [`dpx_core::Engine::id`]).
    fn mark_saved(&mut self, path: &Path, revision: u64);
    /// What a project of the shown page keeps besides the document.
    fn project_extras(&self) -> ProjectExtras {
        ProjectExtras::default()
    }
    /// The BG colour (the margin colour of imports).
    fn bg_color(&self) -> u8 {
        0
    }
    /// The image generator, its jobs and the Generate requester's choices (ADR-45), shared
    /// with the GUI's Picture > Generate..; the default (headless) has none.
    #[cfg(feature = "imagegen")]
    fn image_generation(&mut self) -> Result<crate::generation::GenHost, ToolError> {
        Err(crate::generation::unavailable())
    }
    /// Applies a converted generated picture as the Generate requester's Apply does (one
    /// undo step where it lands). Asking the user first is the caller's part.
    #[cfg(feature = "imagegen")]
    fn apply_generated(&mut self, apply: crate::generation::GenApply) -> Result<(), ToolError> {
        let _ = apply;
        Err(crate::generation::unavailable())
    }
    /// Asks the user; `answer` is called once with the reply (possibly later, from the
    /// owning thread): `Ok(true)` for `yes`, `Ok(false)` for `no`, or an error when the
    /// question could not be asked or was not answered (`EDITOR_BUSY`,
    /// [`crate::jobs::NO_ANSWER`], `APP_UNAVAILABLE`). A workspace without a user answers
    /// `Ok(false)` at once.
    fn ask(&mut self, question: Question, answer: Box<dyn FnOnce(Result<bool, ToolError>) + Send>);
    /// One line for the activity log ("what the client did").
    fn log(&mut self, entry: &str);
}

/// Width and height of `img`.
pub(crate) fn dims(img: &IndexedImage) -> (u32, u32) {
    (img.width(), img.height())
}

/// One page of the headless workspace.
#[derive(Clone, Debug)]
struct Page {
    engine: Engine,
    name: Option<String>,
    path: Option<PathBuf>,
    /// The revision that equals a file on disk.
    saved: Option<u64>,
}

impl Page {
    fn new(doc: Document, info: PictureInfo) -> Self {
        let saved = info.saved.then_some(doc.revision());
        Self {
            engine: Engine::new(doc),
            name: info.name,
            path: info.path,
            saved,
        }
    }

    fn blank_like(doc: &Document) -> Self {
        let blank = Document::from_picture(
            IndexedImage::new(doc.page().width(), doc.page().height(), 0),
            doc.palette().clone(),
            doc.target(),
        );
        Self::new(blank, PictureInfo::default())
    }
}

/// The in-process engine of `deluxepainter mcp` (stdio or `--http`): two pages (picture and
/// spare, with the GUI's rules), a brush and an activity log, no window.
#[derive(Debug)]
pub struct HeadlessWorkspace {
    shown: Page,
    other: Option<Page>,
    slot: Slot,
    brush: Option<Brush>,
    log: Vec<String>,
}

impl Default for HeadlessWorkspace {
    fn default() -> Self {
        Self::new(Document::new(320, 256, Palette::dpaint_default_32()))
    }
}

/// Most activity log lines kept.
const LOG_LINES: usize = 200;

impl HeadlessWorkspace {
    pub fn new(doc: Document) -> Self {
        Self {
            shown: Page::new(doc, PictureInfo::default()),
            other: None,
            slot: Slot::First,
            brush: None,
            log: Vec::new(),
        }
    }

    /// The activity log, oldest first.
    pub fn log_lines(&self) -> &[String] {
        &self.log
    }

    pub fn brush(&self) -> Option<&Brush> {
        self.brush.as_ref()
    }

    /// The other page made to match the shown one (the GUI's `take_spare`, ADR-15): another
    /// format gives a blank page; another palette of the same size is taken over (the undo
    /// history starts again); another palette size gives a blank page.
    fn matched_spare(&self) -> Page {
        let doc = self.shown.engine.document();
        match &self.other {
            Some(s) if s.engine.document().target() == doc.target() => {
                let spare = s.engine.document();
                if spare.palette() == doc.palette() {
                    s.clone()
                } else if spare.palette().len() == doc.palette().len()
                    && dims(spare.page()) == dims(doc.page())
                {
                    Page {
                        engine: Engine::new(spare.with_palette(doc.palette().clone())),
                        ..s.clone()
                    }
                } else {
                    Page::blank_like(doc)
                }
            }
            _ => Page::blank_like(doc),
        }
    }
}

impl Workspace for HeadlessWorkspace {
    fn kind(&self) -> &'static str {
        "headless"
    }

    fn engine(&self) -> &Engine {
        &self.shown.engine
    }

    fn execute(&mut self, request: Request) -> Result<Outcome, ToolError> {
        Ok(self.shown.engine.execute(request)?)
    }

    fn undo(&mut self) -> Result<Outcome, ToolError> {
        Ok(self.shown.engine.undo()?)
    }

    fn redo(&mut self) -> Result<Outcome, ToolError> {
        Ok(self.shown.engine.redo()?)
    }

    fn replace_picture(&mut self, doc: Document, info: PictureInfo) -> Result<(), ToolError> {
        self.shown = Page::new(doc, info);
        Ok(())
    }

    fn shown(&self) -> Slot {
        self.slot
    }

    fn spare(&self) -> Option<Document> {
        self.other.as_ref()?;
        Some(self.matched_spare().engine.document().clone())
    }

    fn set_spare_page(&mut self, page: IndexedImage) -> Result<(), ToolError> {
        let mut spare = self.matched_spare();
        let (width, height) = dims(&page);
        spare.engine.execute(Command::SetPage {
            width,
            height,
            pixels: page.pixels().to_vec(),
        })?;
        self.other = Some(spare);
        Ok(())
    }

    fn swap_spare(&mut self) -> Result<(), ToolError> {
        let spare = self.matched_spare();
        let shown = std::mem::replace(&mut self.shown, spare);
        self.other = Some(shown);
        self.slot = self.slot.other();
        Ok(())
    }

    fn set_brush(&mut self, brush: Brush) -> Result<(), ToolError> {
        self.brush = Some(brush);
        Ok(())
    }

    fn brush_size(&self) -> Option<(u32, u32)> {
        self.brush.as_ref().map(|b| dims(&b.image))
    }

    fn picture_name(&self) -> Option<String> {
        self.shown.name.clone().or_else(|| {
            self.shown
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
        })
    }

    fn set_picture_name(&mut self, name: Option<String>) {
        self.shown.name = name;
    }

    fn modified(&self) -> bool {
        self.shown.saved != Some(self.shown.engine.document().revision())
    }

    fn mark_saved(&mut self, path: &Path, revision: u64) {
        self.shown.saved = Some(revision);
        self.shown.path = Some(path.to_path_buf());
        self.shown.name = None;
    }

    fn ask(
        &mut self,
        _question: Question,
        answer: Box<dyn FnOnce(Result<bool, ToolError>) + Send>,
    ) {
        // Nobody to ask.
        answer(Ok(false));
    }

    fn log(&mut self, entry: &str) {
        if self.log.len() == LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(entry.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Shape;

    fn dot(ws: &mut HeadlessWorkspace, x: i32, color: u8) {
        ws.execute(
            Command::Shape {
                shape: Shape::Dots {
                    points: vec![(x, 0)],
                },
                brush: vec![(0, 0)],
                color,
                filled: false,
                symmetry: None,
                custom: None,
                mode: Default::default(),
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            }
            .into(),
        )
        .unwrap();
    }

    #[test]
    fn swap_keeps_slots_and_pages() {
        let mut ws = HeadlessWorkspace::default();
        assert_eq!(ws.spare(), None);
        dot(&mut ws, 0, 5);
        ws.swap_spare().unwrap();
        assert_eq!(ws.shown(), Slot::Second);
        assert_eq!(
            ws.engine().document().page().get(0, 0),
            Some(0),
            "blank spare"
        );
        dot(&mut ws, 1, 7);
        ws.swap_spare().unwrap();
        assert_eq!(ws.shown(), Slot::First);
        assert_eq!(ws.engine().document().page().get(0, 0), Some(5));
        assert_eq!(ws.spare().unwrap().page().get(1, 0), Some(7));
    }

    #[test]
    fn modified_follows_saves_and_opens() {
        let mut ws = HeadlessWorkspace::default();
        assert!(ws.modified(), "never saved");
        let rev = ws.engine().document().revision();
        ws.mark_saved(Path::new("/tmp/a.dpxproj"), rev);
        assert!(!ws.modified());
        assert_eq!(ws.picture_name().as_deref(), Some("a.dpxproj"));
        dot(&mut ws, 0, 1);
        assert!(ws.modified());
        let doc = ws.engine().document().clone();
        ws.replace_picture(
            doc,
            PictureInfo {
                saved: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!ws.modified());
    }
}
