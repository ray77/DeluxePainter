//! Picture ▸ Load/Save file requester in the style of DeluxePaint IV AGA (manual figure 1.3).
//!
//! Unlike Screen Format it opens on the paint screen: it is drawn in LoRes UI pixels with the
//! pens taken from the picture palette and composed with the rest of the interface. The
//! requester only holds UI state. The app lists drawers ([`FileRequester::set_listing`]) and
//! reads or writes the files ([`FileResponse::Accept`]).
//!
//! Layout: a list of eight rows with a scroll bar, the Drawer, Pattern and File fields,
//! Disks/Assigns/Parent, "# of Frames" (Load; ghosted until animation exists) and the
//! Load/Save and Cancel buttons. Geometry is measured from the manual figure and still to be
//! calibrated against a real screen (AMIGA_GUI_FIDELITY.md).

use std::sync::OnceLock;

use dpx_core::{IndexedImage, Rect};

use crate::bitmap::{self, Bitmap};
use crate::font::Font;
use crate::pattern::Pattern;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{
    Inset, TextField, button_face, centred, checker, draw_button, draw_clipped, draw_field, frame,
};

/// Requester size in LoRes pixels.
pub const WIDTH: i32 = 264;
pub const HEIGHT: i32 = 196;
/// Two presses on the same file within this time are a double-click.
pub const DOUBLE_CLICK_MS: u64 = 500;
/// Longest text a field accepts (modern file names; AmigaDOS allowed 30 characters).
const MAX_FIELD_CHARS: usize = 255;

mod geom {
    pub const TITLE_Y: i32 = 2;
    pub const TITLE_LINE_Y: i32 = 12;
    pub const LIST_X: i32 = 1;
    pub const LIST_W: i32 = 248;
    pub const LIST_TOP: i32 = 13;
    pub const ROWS_Y: i32 = 14;
    pub const ROWS: i32 = 8;
    pub const ROW_H: i32 = 10;
    pub const LIST_BOTTOM: i32 = 96;
    pub const SCROLL_X: i32 = 250;
    pub const SCROLL_W: i32 = 13;
    pub const ARROW_H: i32 = 12;
    pub const LABEL_X: i32 = 10;
    pub const FIELD_X: i32 = 82;
    pub const FIELD_W: i32 = 178;
    pub const FIELD_H: i32 = 9;
    pub const DRAWER_Y: i32 = 101;
    pub const PATTERN_Y: i32 = 111;
    pub const FILE_Y: i32 = 121;
    pub const BUTTONS_Y: i32 = 137;
    pub const BUTTON_H: i32 = 13;
    pub const SMALL_W: i32 = 66;
    pub const DISKS_X: i32 = 10;
    pub const ASSIGNS_X: i32 = 98;
    pub const PARENT_X: i32 = 186;
    pub const SEPARATOR_Y: i32 = 156;
    pub const FRAMES_LABEL_X: i32 = 28;
    pub const FRAMES_Y: i32 = 160;
    pub const FRAMES_FIELD_X: i32 = 190;
    pub const FRAMES_FIELD_W: i32 = 56;
    pub const ACTION_Y: i32 = 177;
    pub const ACTION_W: i32 = 64;
    pub const ACTION_X: i32 = 10;
    pub const CANCEL_X: i32 = 196;
    pub const CONFIRM_X: i32 = 32;
    pub const CONFIRM_Y: i32 = 52;
    pub const CONFIRM_W: i32 = 200;
    pub const CONFIRM_H: i32 = 60;
}

/// What the requester is for; sets the title and the action button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    LoadPicture,
    SavePicture,
    /// Picture ▸ Delete..: the action button asks Yes/No before it answers (ADR-15).
    DeleteFile,
    LoadPalette,
    SavePalette,
    /// Brush ▸ Load.. / Save.. (ADR-19).
    LoadBrush,
    SaveBrush,
}

impl FileKind {
    pub fn title(self) -> &'static str {
        match self {
            FileKind::LoadPicture => "Load Picture",
            FileKind::SavePicture => "Save Picture",
            FileKind::DeleteFile => "Delete File",
            FileKind::LoadPalette => "Load Palette",
            FileKind::SavePalette => "Save Palette",
            FileKind::LoadBrush => "Load Brush",
            FileKind::SaveBrush => "Save Brush",
        }
    }

    fn action(self) -> &'static str {
        match self {
            FileKind::LoadPicture | FileKind::LoadPalette | FileKind::LoadBrush => "Load",
            FileKind::SavePicture | FileKind::SavePalette | FileKind::SaveBrush => "Save",
            FileKind::DeleteFile => "Delete",
        }
    }
}

/// Kind of a list entry that opens a drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKind {
    Dir,
    Volume,
    Assign,
}

impl PlaceKind {
    fn prefix(self) -> &'static str {
        match self {
            PlaceKind::Dir => "<DIR>",
            PlaceKind::Volume => "<VOL>",
            PlaceKind::Assign => "<ASN>",
        }
    }
}

/// One row of the file list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    File(String),
    /// A directory, volume or assign: `label` is shown, clicking it lists `drawer`.
    Place {
        kind: PlaceKind,
        label: String,
        drawer: String,
    },
}

impl Entry {
    fn sort_key(&self) -> (u8, String) {
        match self {
            Entry::Place { label, .. } => (0, label.to_lowercase()),
            Entry::File(name) => (1, name.to_lowercase()),
        }
    }
}

/// A listing the app must provide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Browse {
    /// List this drawer (typed into the Drawer field or clicked in the list).
    Drawer(String),
    /// List the parent of [`FileRequester::drawer`].
    Parent,
    Disks,
    Assigns,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileResponse {
    Continue,
    /// The app lists the place and calls [`FileRequester::set_listing`] (or `set_message`).
    Browse(Browse),
    /// Load/Save chosen. `overwrite` is true once the user confirmed replacing a file (see
    /// [`FileRequester::ask_overwrite`]); for [`FileKind::DeleteFile`] it is always true (the
    /// deletion was confirmed).
    Accept {
        drawer: String,
        file: String,
        overwrite: bool,
    },
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Drawer,
    Pattern,
    File,
}

impl Field {
    const ALL: [Field; 3] = [Field::Drawer, Field::Pattern, Field::File];

    fn rect(self) -> Rect {
        let y = match self {
            Field::Drawer => geom::DRAWER_Y,
            Field::Pattern => geom::PATTERN_Y,
            Field::File => geom::FILE_Y,
        };
        Rect::new(geom::FIELD_X, y, geom::FIELD_W, geom::FIELD_H)
    }

    fn label(self) -> &'static str {
        match self {
            Field::Drawer => "Drawer",
            Field::Pattern => "Pattern",
            Field::File => "File",
        }
    }

    fn next(self) -> Field {
        match self {
            Field::Drawer => Field::Pattern,
            Field::Pattern => Field::File,
            Field::File => Field::Drawer,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Disks,
    Assigns,
    Parent,
    Action,
    Cancel,
    Up,
    Down,
    Yes,
    No,
}

impl Button {
    fn rect(self) -> Rect {
        let small = |x| Rect::new(x, geom::BUTTONS_Y, geom::SMALL_W, geom::BUTTON_H);
        let action = |x| Rect::new(x, geom::ACTION_Y, geom::ACTION_W, geom::BUTTON_H);
        let confirm = |dx| {
            Rect::new(
                geom::CONFIRM_X + dx,
                geom::CONFIRM_Y + 40,
                56,
                geom::BUTTON_H,
            )
        };
        match self {
            Button::Disks => small(geom::DISKS_X),
            Button::Assigns => small(geom::ASSIGNS_X),
            Button::Parent => small(geom::PARENT_X),
            Button::Action => action(geom::ACTION_X),
            Button::Cancel => action(geom::CANCEL_X),
            Button::Up => Rect::new(
                geom::SCROLL_X,
                geom::LIST_TOP,
                geom::SCROLL_W,
                geom::ARROW_H,
            ),
            Button::Down => Rect::new(
                geom::SCROLL_X,
                geom::LIST_BOTTOM - geom::ARROW_H,
                geom::SCROLL_W,
                geom::ARROW_H,
            ),
            Button::Yes => confirm(12),
            Button::No => confirm(geom::CONFIRM_W - 12 - 56),
        }
    }

    fn label(self, kind: FileKind) -> &'static str {
        match self {
            Button::Disks => "Disks",
            Button::Assigns => "Assigns",
            Button::Parent => "Parent",
            Button::Action => kind.action(),
            Button::Cancel => "Cancel",
            Button::Yes => "Yes",
            Button::No => "No",
            Button::Up | Button::Down => "",
        }
    }
}

/// Buttons of the main panel (not the overwrite confirmation).
const PANEL_BUTTONS: [Button; 5] = [
    Button::Disks,
    Button::Assigns,
    Button::Parent,
    Button::Action,
    Button::Cancel,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    /// Scroll knob held at this offset from its top.
    Knob(i32),
    Button(Button),
}

/// State of a Load/Save file requester.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRequester {
    kind: FileKind,
    /// The listed drawer ("" for the Disks and Assigns lists).
    listed: String,
    /// All entries of the listing, sorted: places first, then files.
    entries: Vec<Entry>,
    /// Indices into `entries` that pass the pattern (places always do).
    shown: Vec<usize>,
    top: usize,
    drawer: TextField,
    pattern: TextField,
    file: TextField,
    active: Option<Field>,
    drag: Option<Drag>,
    pointer: (i32, i32),
    /// Last press on a file row: (index into `shown`, time in ms).
    last_click: Option<(usize, u64)>,
    message: Option<String>,
    /// The overwrite confirmation is shown.
    confirm: bool,
}

impl FileRequester {
    /// Opens with an empty list and the File field active; the app lists the start drawer.
    pub fn new(kind: FileKind) -> Self {
        Self {
            kind,
            listed: String::new(),
            entries: Vec::new(),
            shown: Vec::new(),
            top: 0,
            drawer: TextField::new(MAX_FIELD_CHARS),
            pattern: TextField::new(MAX_FIELD_CHARS),
            file: TextField::new(MAX_FIELD_CHARS),
            active: Some(Field::File),
            drag: None,
            pointer: (0, 0),
            last_click: None,
            message: None,
            confirm: false,
        }
    }

    pub fn kind(&self) -> FileKind {
        self.kind
    }

    /// The listed drawer ("" while Disks or Assigns are listed).
    pub fn drawer(&self) -> &str {
        &self.listed
    }

    pub fn file(&self) -> String {
        self.file.text()
    }

    pub fn set_file(&mut self, name: &str) {
        self.file.set(name);
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(screen_w: i32, screen_h: i32) -> (i32, i32) {
        (
            ((screen_w - WIDTH) / 2).max(0),
            ((screen_h - HEIGHT) / 2).max(0),
        )
    }

    /// Shows a listing; `drawer` goes into the Drawer field.
    pub fn set_listing(&mut self, drawer: &str, mut entries: Vec<Entry>) {
        entries.sort_by_cached_key(Entry::sort_key);
        self.listed = drawer.to_string();
        self.drawer.set(drawer);
        self.entries = entries;
        self.top = 0;
        self.last_click = None;
        self.message = None;
        self.refilter();
    }

    /// Shows a short message (e.g. why loading failed); the requester stays open.
    pub fn set_message(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
        self.confirm = false;
    }

    /// Asks whether the existing file may be replaced; "Yes" answers with
    /// [`FileResponse::Accept`] and `overwrite: true`.
    pub fn ask_overwrite(&mut self) {
        self.confirm = true;
        self.drag = None;
    }

    fn refilter(&mut self) {
        let pattern = Pattern::parse(&self.pattern.text());
        if let Err(e) = &pattern {
            self.message = Some(e.to_string());
        }
        self.shown = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match (e, &pattern) {
                (Entry::File(name), Ok(p)) => p.matches(name),
                _ => true,
            })
            .map(|(i, _)| i)
            .collect();
        self.top = self.top.min(self.max_top());
    }

    fn max_top(&self) -> usize {
        self.shown.len().saturating_sub(geom::ROWS as usize)
    }

    fn scroll_by(&mut self, rows: i32) {
        self.top = (self.top as i32 + rows).clamp(0, self.max_top() as i32) as usize;
    }

    fn accept(&mut self, overwrite: bool) -> FileResponse {
        let file = self.file.text();
        if file.trim().is_empty() {
            self.message = Some("Enter a file name".into());
            return FileResponse::Continue;
        }
        if self.kind == FileKind::DeleteFile && !overwrite {
            self.ask_overwrite();
            return FileResponse::Continue;
        }
        self.confirm = false;
        FileResponse::Accept {
            drawer: self.drawer.text(),
            file,
            overwrite,
        }
    }

    /// Leaves the active field; a changed pattern takes effect.
    fn deactivate(&mut self) {
        if self.active.take() == Some(Field::Pattern) {
            self.message = None;
            self.refilter();
        }
    }

    fn activate(&mut self, field: Field, click_x: Option<i32>) {
        if self.active != Some(field) {
            self.deactivate();
        }
        self.active = Some(field);
        let f = self.field_mut(field);
        if let Some(x) = click_x {
            // Cursor on the clicked character (or at the end).
            let font = Font::builtin();
            let start = f.first_visible(geom::FIELD_W - 4);
            let mut cx = field.rect().x + 2;
            let mut pos = f.chars.len();
            for (i, &c) in f.chars.iter().enumerate().skip(start) {
                let adv = font.char_advance(c);
                if x < cx + adv {
                    pos = i;
                    break;
                }
                cx += adv;
            }
            f.cursor = pos;
        }
    }

    fn field_mut(&mut self, field: Field) -> &mut TextField {
        match field {
            Field::Drawer => &mut self.drawer,
            Field::Pattern => &mut self.pattern,
            Field::File => &mut self.file,
        }
    }

    fn field(&self, field: Field) -> &TextField {
        match field {
            Field::Drawer => &self.drawer,
            Field::Pattern => &self.pattern,
            Field::File => &self.file,
        }
    }

    /// Handles one event in requester coordinates (see [`FileRequester::origin`]); `now_ms` is
    /// a monotonic time used to detect double-clicks.
    pub fn handle(&mut self, event: ReqEvent, now_ms: u64) -> FileResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.press(x, y, now_ms)
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                if let Some(Drag::Knob(offset)) = self.drag {
                    self.knob_to(y - offset);
                }
                FileResponse::Continue
            }
            ReqEvent::Release => match self.drag.take() {
                Some(Drag::Button(b)) if b.rect().contains(self.pointer.0, self.pointer.1) => {
                    self.button(b)
                }
                _ => FileResponse::Continue,
            },
            ReqEvent::Scroll(rows) => {
                if !self.confirm {
                    self.scroll_by(rows);
                }
                FileResponse::Continue
            }
            ReqEvent::Key(key) => self.key(key),
        }
    }

    fn press(&mut self, x: i32, y: i32, now_ms: u64) -> FileResponse {
        if self.confirm {
            for b in [Button::Yes, Button::No] {
                if b.rect().contains(x, y) {
                    self.drag = Some(Drag::Button(b));
                }
            }
            return FileResponse::Continue;
        }
        if let Some(&f) = Field::ALL.iter().find(|f| f.rect().contains(x, y)) {
            self.activate(f, Some(x));
            return FileResponse::Continue;
        }
        self.deactivate();
        let rows = Rect::new(
            geom::LIST_X,
            geom::ROWS_Y,
            geom::LIST_W,
            geom::ROWS * geom::ROW_H,
        );
        let track = track_rect();
        if rows.contains(x, y) {
            let i = self.top + ((y - geom::ROWS_Y) / geom::ROW_H) as usize;
            return self.click_entry(i, now_ms);
        }
        if track.contains(x, y) {
            let (ky, kh) = self.knob();
            if y < ky {
                self.scroll_by(-geom::ROWS);
            } else if y >= ky + kh {
                self.scroll_by(geom::ROWS);
            } else {
                self.drag = Some(Drag::Knob(y - ky));
            }
            return FileResponse::Continue;
        }
        for b in [Button::Up, Button::Down] {
            if b.rect().contains(x, y) {
                self.scroll_by(if b == Button::Up { -1 } else { 1 });
                self.drag = Some(Drag::Button(b));
            }
        }
        for b in PANEL_BUTTONS {
            if b.rect().contains(x, y) {
                self.drag = Some(Drag::Button(b));
            }
        }
        FileResponse::Continue
    }

    fn click_entry(&mut self, i: usize, now_ms: u64) -> FileResponse {
        let Some(entry) = self.shown.get(i).map(|&e| self.entries[e].clone()) else {
            return FileResponse::Continue;
        };
        match entry {
            Entry::Place { drawer, .. } => FileResponse::Browse(Browse::Drawer(drawer)),
            Entry::File(name) => {
                let double = matches!(self.last_click,
                    Some((last, t)) if last == i && now_ms.saturating_sub(t) <= DOUBLE_CLICK_MS);
                self.file.set(&name);
                self.message = None;
                if double {
                    self.last_click = None;
                    self.accept(false)
                } else {
                    self.last_click = Some((i, now_ms));
                    FileResponse::Continue
                }
            }
        }
    }

    fn button(&mut self, b: Button) -> FileResponse {
        match b {
            Button::Disks => FileResponse::Browse(Browse::Disks),
            Button::Assigns => FileResponse::Browse(Browse::Assigns),
            Button::Parent => FileResponse::Browse(Browse::Parent),
            Button::Action => self.accept(false),
            Button::Cancel => FileResponse::Cancel,
            Button::Yes => self.accept(true),
            Button::No => {
                self.confirm = false;
                FileResponse::Continue
            }
            Button::Up | Button::Down => FileResponse::Continue,
        }
    }

    fn key(&mut self, key: ReqKey) -> FileResponse {
        if self.confirm {
            return match key {
                ReqKey::Char('y' | 'Y') => self.accept(true),
                ReqKey::Char('n' | 'N') | ReqKey::Escape => self.button(Button::No),
                _ => FileResponse::Continue,
            };
        }
        match (key, self.active) {
            (ReqKey::Escape, _) => FileResponse::Cancel,
            (ReqKey::Up, _) => {
                self.scroll_by(-1);
                FileResponse::Continue
            }
            (ReqKey::Down, _) => {
                self.scroll_by(1);
                FileResponse::Continue
            }
            (ReqKey::Tab, Some(f)) => {
                self.activate(f.next(), None);
                FileResponse::Continue
            }
            (ReqKey::Enter, Some(Field::Drawer)) => {
                self.deactivate();
                FileResponse::Browse(Browse::Drawer(self.drawer.text()))
            }
            (ReqKey::Enter, Some(Field::Pattern)) => {
                self.deactivate();
                FileResponse::Continue
            }
            (ReqKey::Enter, _) => self.accept(false),
            (k, Some(f)) => {
                self.field_mut(f).key(k);
                FileResponse::Continue
            }
            (_, None) => FileResponse::Continue,
        }
    }

    /// Scroll knob top and height inside the track.
    fn knob(&self) -> (i32, i32) {
        let t = track_rect().inset(1);
        let total = self.shown.len().max(1) as i32;
        let rows = geom::ROWS.min(total);
        let h = (t.h * rows / total).max(4);
        let range = (total - rows).max(1);
        (t.y + (t.h - h) * self.top as i32 / range, h)
    }

    fn knob_to(&mut self, top: i32) {
        let t = track_rect().inset(1);
        let (_, h) = self.knob();
        let range = self.max_top() as i32;
        let free = (t.h - h).max(1);
        let pos = ((top - t.y) * range + free / 2) / free;
        self.top = pos.clamp(0, range) as usize;
    }

    fn pressed(&self, b: Button) -> bool {
        self.drag == Some(Drag::Button(b)) && b.rect().contains(self.pointer.0, self.pointer.1)
    }

    /// Draws the requester onto a UI screen at [`FileRequester::origin`].
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = Self::origin(img.width() as i32, img.height() as i32);
        img.blit(&self.render(pens), x, y);
    }

    /// Renders the requester alone (WIDTH×HEIGHT).
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let mut img = IndexedImage::new(WIDTH as u32, HEIGHT as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, WIDTH, HEIGHT), pens.dark);
        centred(
            &mut img,
            0,
            WIDTH,
            geom::TITLE_Y,
            self.kind.title(),
            pens.dark,
        );
        img.hline(1, WIDTH - 2, geom::TITLE_LINE_Y, pens.dark);

        // File list: places in the select pen, the chosen file highlighted.
        let chosen = self.file.text();
        for row in 0..geom::ROWS {
            let Some(&e) = self.shown.get(self.top + row as usize) else {
                break;
            };
            let y = geom::ROWS_Y + row * geom::ROW_H;
            let (text, ink) = match &self.entries[e] {
                Entry::File(name) if *name == chosen => {
                    img.fill_rect(geom::LIST_X, y, geom::LIST_W, geom::ROW_H, pens.select);
                    (name.clone(), pens.shine)
                }
                Entry::File(name) => (name.clone(), pens.dark),
                Entry::Place { kind, label, .. } => {
                    (format!("{} {label}", kind.prefix()), pens.select)
                }
            };
            draw_clipped(
                &mut img,
                geom::LIST_X + 2,
                y + 1,
                text.chars(),
                geom::LIST_W - 4,
                ink,
            );
        }
        img.vline(
            geom::SCROLL_X - 1,
            geom::TITLE_LINE_Y,
            geom::LIST_BOTTOM,
            pens.dark,
        );

        // Scroll bar: arrows and a track with a dark knob.
        let g = glyphs();
        for (b, glyph) in [(Button::Up, &g.arrow_up), (Button::Down, &g.arrow_down)] {
            let r = b.rect();
            let ink = button_face(&mut img, r, self.pressed(b), pens);
            draw_bitmap(
                &mut img,
                glyph,
                r.x + (r.w - glyph.width as i32) / 2,
                r.y + (r.h - glyph.height as i32) / 2,
                ink,
            );
        }
        let t = track_rect();
        img.fill_rect(t.x, t.y, t.w, t.h, pens.shine);
        frame(&mut img, t, pens.dark);
        let (ky, kh) = self.knob();
        img.fill_rect(t.x + 1, ky, t.w - 2, kh, pens.dark);
        img.hline(1, WIDTH - 2, geom::LIST_BOTTOM, pens.dark);

        // Drawer, Pattern, File.
        for f in Field::ALL {
            let r = f.rect();
            font.draw(&mut img, geom::LABEL_X, r.y + 1, f.label(), pens.dark);
            draw_field(&mut img, r, self.field(f), self.active == Some(f), pens);
        }

        for b in PANEL_BUTTONS {
            draw_button(
                &mut img,
                b.rect(),
                b.label(self.kind),
                self.pressed(b),
                pens,
            );
        }
        img.hline(1, WIDTH - 2, geom::SEPARATOR_Y, pens.dark);

        // "# of Frames" exists for Load only; ghosted until animation frames exist.
        if self.kind == FileKind::LoadPicture {
            let r = Rect::new(
                geom::FRAMES_FIELD_X,
                geom::FRAMES_Y,
                geom::FRAMES_FIELD_W,
                geom::FIELD_H,
            );
            match pens.ghost {
                // Grey label and field (owner decision: ghosted options are grey).
                Some(g) => {
                    font.draw(
                        &mut img,
                        geom::FRAMES_LABEL_X,
                        geom::FRAMES_Y + 1,
                        "# of Frames:",
                        g,
                    );
                    img.fill_rect(r.x, r.y, r.w, r.h, g);
                    font.draw(&mut img, r.x + 2, r.y + 1, "1", pens.face);
                }
                // Two-colour palettes have no grey: thin the dark label with dots.
                None => {
                    font.draw(
                        &mut img,
                        geom::FRAMES_LABEL_X,
                        geom::FRAMES_Y + 1,
                        "# of Frames:",
                        pens.dark,
                    );
                    img.fill_rect(r.x, r.y, r.w, r.h, pens.dark);
                    font.draw(&mut img, r.x + 2, r.y + 1, "1", pens.shine);
                    checker(
                        &mut img,
                        Rect::new(
                            geom::FRAMES_LABEL_X - 2,
                            geom::FRAMES_Y - 1,
                            r.x + r.w + 2 - geom::FRAMES_LABEL_X,
                            r.h + 2,
                        ),
                        pens.face,
                    );
                }
            }
        }
        if let Some(msg) = &self.message {
            let r = Rect::new(2, geom::SEPARATOR_Y + 2, WIDTH - 4, 15);
            img.fill_rect(r.x, r.y, r.w, r.h, pens.select);
            draw_clipped(&mut img, r.x + 4, r.y + 4, msg.chars(), r.w - 8, pens.shine);
        }

        if self.confirm {
            self.draw_confirm(&mut img, pens);
        }
        img
    }

    fn draw_confirm(&self, img: &mut IndexedImage, pens: &Pens) {
        let r = Rect::new(
            geom::CONFIRM_X,
            geom::CONFIRM_Y,
            geom::CONFIRM_W,
            geom::CONFIRM_H,
        );
        img.fill_rect(r.x, r.y, r.w, r.h, pens.face);
        frame(img, r, pens.dark);
        frame(img, r.inset(2), pens.dark);
        let name = format!("\"{}\"", self.file.text());
        let font = Font::builtin();
        let w = font.text_width(&name).min(r.w - 12);
        draw_clipped(
            img,
            r.x + (r.w - w) / 2,
            r.y + 8,
            name.chars(),
            r.w - 12,
            pens.dark,
        );
        let question = if self.kind == FileKind::DeleteFile {
            "Delete this file?"
        } else {
            "exists. Replace it?"
        };
        centred(img, r.x, r.w, r.y + 20, question, pens.dark);
        for b in [Button::Yes, Button::No] {
            draw_button(img, b.rect(), b.label(self.kind), self.pressed(b), pens);
        }
    }
}

fn track_rect() -> Rect {
    Rect::new(
        geom::SCROLL_X,
        geom::LIST_TOP + geom::ARROW_H,
        geom::SCROLL_W,
        geom::LIST_BOTTOM - geom::LIST_TOP - 2 * geom::ARROW_H,
    )
}

struct Glyphs {
    arrow_up: Bitmap,
    arrow_down: Bitmap,
}

fn glyphs() -> &'static Glyphs {
    static G: OnceLock<Glyphs> = OnceLock::new();
    G.get_or_init(|| {
        let parsed = bitmap::parse(include_str!("../../../assets/ui/file-requester-glyphs.txt"))
            .expect("file requester glyphs parse");
        let get = |n: &str| {
            parsed
                .iter()
                .find(|(name, _)| name == n)
                .map(|(_, b)| b.clone())
                .unwrap_or_else(|| panic!("missing glyph {n}"))
        };
        Glyphs {
            arrow_up: get("arrow_up"),
            arrow_down: get("arrow_down"),
        }
    })
}

fn draw_bitmap(img: &mut IndexedImage, bmp: &Bitmap, x: i32, y: i32, pen: u8) {
    for by in 0..bmp.height {
        for bx in 0..bmp.width {
            if bmp.get(bx, by) {
                img.set(x + bx as i32, y + by as i32, pen);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

    fn place(kind: PlaceKind, label: &str) -> Entry {
        Entry::Place {
            kind,
            label: label.into(),
            drawer: format!("/x/{label}"),
        }
    }

    fn listing() -> Vec<Entry> {
        let mut v = vec![
            Entry::File("zebra.iff".into()),
            Entry::File("Venus.LoRes".into()),
            Entry::File("notes.txt".into()),
            place(PlaceKind::Dir, "pictures"),
            Entry::File("alpha.IFF".into()),
            place(PlaceKind::Dir, "Brushes"),
        ];
        v.extend((0..10).map(|i| Entry::File(format!("pic{i:02}.iff"))));
        v
    }

    fn req(kind: FileKind) -> FileRequester {
        let mut r = FileRequester::new(kind);
        r.set_listing("/x", listing());
        r
    }

    fn row_y(row: i32) -> i32 {
        geom::ROWS_Y + row * geom::ROW_H + 4
    }

    fn label(r: &FileRequester, shown: usize) -> String {
        match &r.entries[r.shown[shown]] {
            Entry::File(n) => n.clone(),
            Entry::Place { label, .. } => label.clone(),
        }
    }

    fn click(r: &mut FileRequester, b: Button) -> FileResponse {
        let rect = b.rect();
        r.handle(ReqEvent::Press(rect.x + 3, rect.y + 3), 0);
        r.handle(ReqEvent::Release, 0)
    }

    fn type_text(r: &mut FileRequester, s: &str) {
        for c in s.chars() {
            r.handle(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
    }

    #[test]
    fn places_come_first_and_the_pattern_filters_files() {
        let mut r = req(FileKind::LoadPicture);
        assert_eq!(label(&r, 0), "Brushes");
        assert_eq!(label(&r, 1), "pictures");
        assert_eq!(label(&r, 2), "alpha.IFF");
        assert_eq!(r.shown.len(), 16);
        // Type a pattern; it applies on Return.
        let p = Field::Pattern.rect();
        r.handle(ReqEvent::Press(p.x + 5, p.y + 3), 0);
        type_text(&mut r, "#?.iff");
        assert_eq!(r.shown.len(), 16, "not applied while typing");
        r.handle(ReqEvent::Key(ReqKey::Enter), 0);
        assert_eq!(r.shown.len(), 2 + 12, "places + .iff/.IFF files");
        // A broken pattern shows everything and says why.
        r.handle(ReqEvent::Press(p.x + 5, p.y + 3), 0);
        type_text(&mut r, "(");
        r.handle(ReqEvent::Key(ReqKey::Enter), 0);
        assert_eq!(r.shown.len(), 16);
        assert!(r.message.as_deref().unwrap().contains("pattern"));
    }

    #[test]
    fn clicking_files_and_places() {
        let mut r = req(FileKind::LoadPicture);
        // Place row: lists that drawer.
        assert_eq!(
            r.handle(ReqEvent::Press(20, row_y(1)), 0),
            FileResponse::Browse(Browse::Drawer("/x/pictures".into()))
        );
        // File row: the name goes into the File field; a quick second click loads it.
        assert_eq!(
            r.handle(ReqEvent::Press(20, row_y(2)), 1000),
            FileResponse::Continue
        );
        assert_eq!(r.file(), "alpha.IFF");
        r.handle(ReqEvent::Release, 1000);
        assert_eq!(
            r.handle(ReqEvent::Press(20, row_y(2)), 1400),
            FileResponse::Accept {
                drawer: "/x".into(),
                file: "alpha.IFF".into(),
                overwrite: false,
            }
        );
        // Too slow, or another row, is no double-click.
        r.handle(ReqEvent::Press(20, row_y(3)), 5000);
        assert_eq!(
            r.handle(ReqEvent::Press(20, row_y(3)), 5000 + DOUBLE_CLICK_MS + 1),
            FileResponse::Continue
        );
        assert_eq!(
            r.handle(ReqEvent::Press(20, row_y(4)), 5000 + DOUBLE_CLICK_MS + 2),
            FileResponse::Continue
        );
        // Rows past the end do nothing.
        r.set_listing("/y", vec![Entry::File("a".into())]);
        assert_eq!(
            r.handle(ReqEvent::Press(20, row_y(5)), 0),
            FileResponse::Continue
        );
    }

    #[test]
    fn editing_fields() {
        let mut r = FileRequester::new(FileKind::SavePicture);
        assert_eq!(r.active, Some(Field::File), "File is active on open");
        type_text(&mut r, "venus");
        r.handle(ReqEvent::Key(ReqKey::Backspace), 0);
        r.handle(ReqEvent::Key(ReqKey::Home), 0);
        r.handle(ReqEvent::Key(ReqKey::Delete), 0);
        type_text(&mut r, "V");
        r.handle(ReqEvent::Key(ReqKey::End), 0);
        type_text(&mut r, "s.iff");
        r.handle(ReqEvent::Key(ReqKey::Left), 0);
        r.handle(ReqEvent::Key(ReqKey::Right), 0);
        assert_eq!(r.file(), "Venus.iff");
        // Control characters are not inserted; the length is capped.
        type_text(&mut r, "\u{7}");
        assert_eq!(r.file(), "Venus.iff");
        r.set_file(&"x".repeat(400));
        assert_eq!(r.file().len(), MAX_FIELD_CHARS);
        r.set_file("Venus.iff");
        // Tab cycles to the Drawer field; Return there lists the typed drawer.
        r.handle(ReqEvent::Key(ReqKey::Tab), 0);
        assert_eq!(r.active, Some(Field::Drawer));
        type_text(&mut r, "/pics");
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter), 0),
            FileResponse::Browse(Browse::Drawer("/pics".into()))
        );
        assert_eq!(r.active, None);
        // With no field active, Return chooses the action button.
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter), 0),
            FileResponse::Accept {
                drawer: "/pics".into(),
                file: "Venus.iff".into(),
                overwrite: false,
            }
        );
        // Clicking a field puts the cursor under the pointer.
        let f = Field::File.rect();
        r.handle(ReqEvent::Press(f.x + 2, f.y + 3), 0);
        assert_eq!((r.active, r.file.cursor), (Some(Field::File), 0));
        r.handle(ReqEvent::Press(f.x + f.w - 3, f.y + 3), 0);
        assert_eq!(r.file.cursor, r.file.chars.len());
    }

    #[test]
    fn empty_file_name_is_refused_with_a_message() {
        let mut r = req(FileKind::LoadPicture);
        assert_eq!(click(&mut r, Button::Action), FileResponse::Continue);
        assert_eq!(r.message.as_deref(), Some("Enter a file name"));
    }

    #[test]
    fn buttons_act_on_release_inside() {
        let mut r = req(FileKind::LoadPicture);
        assert_eq!(
            click(&mut r, Button::Disks),
            FileResponse::Browse(Browse::Disks)
        );
        assert_eq!(
            click(&mut r, Button::Assigns),
            FileResponse::Browse(Browse::Assigns)
        );
        assert_eq!(
            click(&mut r, Button::Parent),
            FileResponse::Browse(Browse::Parent)
        );
        let c = Button::Cancel.rect();
        r.handle(ReqEvent::Press(c.x + 3, c.y + 3), 0);
        r.handle(ReqEvent::Drag(c.x - 30, c.y + 3), 0);
        assert_eq!(r.handle(ReqEvent::Release, 0), FileResponse::Continue);
        assert_eq!(click(&mut r, Button::Cancel), FileResponse::Cancel);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape), 0),
            FileResponse::Cancel
        );
    }

    #[test]
    fn overwrite_needs_confirmation() {
        let mut r = req(FileKind::SavePicture);
        r.set_file("zebra.iff");
        r.ask_overwrite();
        // The panel below is blocked; No goes back.
        assert_eq!(click(&mut r, Button::Cancel), FileResponse::Continue);
        assert_eq!(click(&mut r, Button::No), FileResponse::Continue);
        assert!(!r.confirm);
        r.ask_overwrite();
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter), 0),
            FileResponse::Continue,
            "Return does not confirm"
        );
        assert_eq!(
            click(&mut r, Button::Yes),
            FileResponse::Accept {
                drawer: "/x".into(),
                file: "zebra.iff".into(),
                overwrite: true,
            }
        );
        r.ask_overwrite();
        assert!(matches!(
            r.handle(ReqEvent::Key(ReqKey::Char('y')), 0),
            FileResponse::Accept {
                overwrite: true,
                ..
            }
        ));
        r.ask_overwrite();
        r.handle(ReqEvent::Key(ReqKey::Escape), 0);
        assert!(!r.confirm, "Escape answers No");
    }

    #[test]
    fn delete_asks_yes_or_no_first() {
        let mut r = req(FileKind::DeleteFile);
        assert_eq!(r.kind().title(), "Delete File");
        r.set_file("zebra.iff");
        // The Delete button only asks; No goes back, Yes answers with the file.
        assert_eq!(click(&mut r, Button::Action), FileResponse::Continue);
        assert!(r.confirm);
        assert_eq!(click(&mut r, Button::No), FileResponse::Continue);
        assert!(!r.confirm);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter), 0),
            FileResponse::Continue
        );
        assert!(r.confirm, "Return asks too");
        assert_eq!(
            click(&mut r, Button::Yes),
            FileResponse::Accept {
                drawer: "/x".into(),
                file: "zebra.iff".into(),
                overwrite: true,
            }
        );
        // A double-click on a file asks as well.
        r.handle(ReqEvent::Press(20, row_y(2)), 0);
        assert_eq!(
            r.handle(ReqEvent::Press(20, row_y(2)), 100),
            FileResponse::Continue
        );
        assert!(r.confirm);
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let img = r.render(&pens);
        assert_eq!((img.width(), img.height()), (WIDTH as u32, HEIGHT as u32));
        // Palette requesters have their own titles and buttons.
        assert_eq!(FileKind::LoadPalette.title(), "Load Palette");
        assert_eq!(FileKind::SavePalette.action(), "Save");
        assert_eq!(FileKind::DeleteFile.action(), "Delete");
    }

    #[test]
    fn scrolling() {
        let mut r = req(FileKind::LoadPicture);
        let max = r.shown.len() - geom::ROWS as usize;
        click(&mut r, Button::Down);
        assert_eq!(r.top, 1);
        click(&mut r, Button::Up);
        click(&mut r, Button::Up);
        assert_eq!(r.top, 0);
        r.handle(ReqEvent::Scroll(100), 0);
        assert_eq!(r.top, max);
        r.handle(ReqEvent::Key(ReqKey::Up), 0);
        assert_eq!(r.top, max - 1);
        // Paging in the track and dragging the knob.
        let t = track_rect();
        r.handle(ReqEvent::Press(t.x + 3, t.y + 2), 0);
        assert_eq!(r.top, 0);
        let (ky, _) = r.knob();
        r.handle(ReqEvent::Press(t.x + 3, ky + 1), 0);
        r.handle(ReqEvent::Drag(t.x + 3, t.y + t.h + 20), 0);
        r.handle(ReqEvent::Release, 0);
        assert_eq!(r.top, max);
    }

    #[test]
    fn centred_on_pal_and_ntsc_screens() {
        assert_eq!(FileRequester::origin(320, 256), (28, 30));
        assert_eq!(FileRequester::origin(320, 200), (28, 2));
    }

    #[test]
    fn renders_title_places_fields_and_cursor() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let mut r = req(FileKind::LoadPicture);
        r.set_file("alpha.IFF");
        let img = r.render(&pens);
        assert_eq!((img.width(), img.height()), (WIDTH as u32, HEIGHT as u32));
        let row_has = |y0: i32, x0: i32, x1: i32, pen: u8| {
            (y0..y0 + 8).any(|y| (x0..x1).any(|x| img.get(x, y) == Some(pen)))
        };
        // Title in the dark pen, places in the select pen.
        assert!(row_has(geom::TITLE_Y, 90, 170, pens.dark));
        assert!(row_has(geom::ROWS_Y + 1, 3, 60, pens.select));
        // The chosen file row is highlighted.
        assert_eq!(
            img.get(geom::LIST_X, geom::ROWS_Y + 2 * geom::ROW_H),
            Some(pens.select)
        );
        // The active File field shows the block cursor after the text.
        let f = Field::File.rect();
        assert!(row_has(f.y, f.x + 2, f.x + f.w, pens.shine));
        // The Save requester has no "# of Frames" row.
        let save = req(FileKind::SavePicture).render(&pens);
        assert!(
            !(geom::FRAMES_Y..geom::FRAMES_Y + 9)
                .any(|y| (20..250).any(|x| save.get(x, y) == Some(pens.dark)))
        );
    }
}
