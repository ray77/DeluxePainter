//! The layer panel (ADR-44, a DeluxePainter addition; DeluxePaint IV has no layers): a column
//! at the right with the title "Layers", one row per layer from the top layer down (an eye
//! box that shows or hides it, its name, a marker on the active layer) and the buttons New,
//! Copy, Del, Up, Dn and Merge, ghosted where the command would be refused. A double-click on
//! a name turns it into a string gadget (Return takes the name, Esc keeps the old one).
//!
//! Drawn in UI (LoRes) pixels with square corners, 1-pixel bevels and palette-derived pens,
//! like the Animation Control Panel. It sits either over the screen's right edge, left of the
//! toolbox ([`Layout::overlay`]), or in the work area right of the screen ([`Layout::docked`];
//! the app decides which and draws the docked strip itself). The panel only shows state and
//! reports what was hit; the app runs the layer commands.

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::ReqKey;
use crate::screen::UiState;
use crate::widgets::{TextField, button_face, centred, checker, draw_clipped, draw_field};

/// Panel geometry (UI pixels).
pub mod geom {
    /// Whole panel width, its dark left edge included.
    pub const PANEL_W: i32 = 80;
    /// The title row ("Layers"), drawn like the screen's title bar.
    pub const HEADER_H: i32 = 11;
    /// One layer row.
    pub const ROW_H: i32 = 10;
    /// Buttons and the pitch of their rows.
    pub const BUTTON_H: i32 = 11;
    pub const BUTTON_PITCH: i32 = 12;
    /// The button block at the bottom: a gap row and three rows of buttons.
    pub const BUTTONS_H: i32 = 1 + 3 * BUTTON_PITCH - 1;
    /// Active-layer marker, eye box and name, from the panel's left edge.
    pub const MARKER_X: i32 = 2;
    pub const EYE_X: i32 = 6;
    pub const EYE_W: i32 = 11;
    pub const EYE_H: i32 = 9;
    pub const NAME_X: i32 = 18;
    /// The scroll arrows in the title row (shown while rows do not fit).
    pub const ARROW_W: i32 = 9;
    /// Height of the screen title bar and of the Animation Control Panel, which the overlay
    /// keeps clear of.
    pub const TITLE_H: i32 = crate::screen::geom::TITLE_H;
    pub const ANIM_PANEL_H: i32 = crate::anim_panel::geom::PANEL_H;
}

/// A button of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerButton {
    /// A new transparent layer above the active one.
    New,
    /// A copy of the active layer above it.
    Copy,
    /// Deletes the active layer.
    Delete,
    /// Moves the active layer one up / down the stack.
    Up,
    Down,
    /// Merges the active layer into the one below it.
    Merge,
}

/// The buttons in drawing order (rows top to bottom, left to right).
pub const BUTTONS: [LayerButton; 6] = [
    LayerButton::New,
    LayerButton::Copy,
    LayerButton::Delete,
    LayerButton::Up,
    LayerButton::Down,
    LayerButton::Merge,
];

impl LayerButton {
    /// The label on the button.
    pub fn label(self) -> &'static str {
        match self {
            LayerButton::New => "New",
            LayerButton::Copy => "Copy",
            LayerButton::Delete => "Del",
            LayerButton::Up => "Up",
            LayerButton::Down => "Dn",
            LayerButton::Merge => "Merge",
        }
    }

    /// The name its tooltip shows (ADR-41).
    pub fn name(self) -> &'static str {
        match self {
            LayerButton::New => "New Layer",
            LayerButton::Copy => "Copy Layer",
            LayerButton::Delete => "Delete Layer",
            LayerButton::Up => "Move Layer Up",
            LayerButton::Down => "Move Layer Down",
            LayerButton::Merge => "Merge Down",
        }
    }

    fn index(self) -> usize {
        BUTTONS.iter().position(|&b| b == self).unwrap_or(0)
    }
}

/// What lies under a panel position. Layers are numbered as in the document: 0 is the bottom
/// layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerHit {
    /// The eye box of a layer: show or hide it.
    Eye(usize),
    /// The rest of a layer's row (marker and name): select it; a double-click renames it.
    Name(usize),
    /// The string gadget of the layer being renamed.
    Field,
    Button(LayerButton),
    /// The scroll arrows in the title row.
    ScrollUp,
    ScrollDown,
    /// The panel elsewhere (title, gaps, empty rows): nothing happens.
    Background,
}

/// One layer as the panel shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerRow {
    pub name: String,
    pub visible: bool,
}

/// Where the panel's parts lie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub panel: Rect,
    /// The layer rows' area under the title row.
    pub list: Rect,
    /// How many rows fit.
    pub capacity: usize,
    /// Top of the button block (its gap row).
    pub buttons_y: i32,
}

impl Layout {
    /// The panel over a `screen_w`×`screen_h` UI screen: left of the toolbox when it is shown
    /// (at the right edge when it is hidden), below the title bar when it is shown, above the
    /// Animation Control Panel when that is shown.
    pub fn overlay(
        screen_w: i32,
        screen_h: i32,
        toolbox: bool,
        title: bool,
        anim_panel: bool,
    ) -> Self {
        let right = if toolbox {
            screen_w - crate::screen::geom::TOOLBOX_W
        } else {
            screen_w
        };
        let top = if title { geom::TITLE_H } else { 0 };
        let bottom = if anim_panel {
            screen_h - geom::ANIM_PANEL_H
        } else {
            screen_h
        };
        Self::at(right - geom::PANEL_W, top, bottom - top)
    }

    /// The panel in the work area right of the screen, `x` being the screen's width (UI
    /// pixels, in the coordinates of the screen); it is as high as the screen and its title
    /// row lines up with the title bar.
    pub fn docked(x: i32, screen_h: i32) -> Self {
        Self::at(x, 0, screen_h)
    }

    fn at(x: i32, y: i32, h: i32) -> Self {
        let panel = Rect::new(x, y, geom::PANEL_W, h.max(0));
        let list_y = y + geom::HEADER_H;
        let buttons_y = (y + h - geom::BUTTONS_H).max(list_y);
        let list = Rect::new(x + 1, list_y, geom::PANEL_W - 1, buttons_y - list_y);
        let capacity = ((list.h - 1).max(0) / geom::ROW_H) as usize;
        Self {
            panel,
            list,
            capacity,
            buttons_y,
        }
    }

    /// Where the panel of `ui` lies on a `screen_w`×`screen_h` UI screen; `None` while it is
    /// not shown (off, or hidden under a requester).
    pub fn of(ui: &UiState, screen_w: i32, screen_h: i32) -> Option<Self> {
        let panel = ui.layer_panel.as_ref().filter(|p| !p.hidden)?;
        Some(if panel.docked {
            Self::docked(screen_w, screen_h)
        } else {
            Self::overlay(
                screen_w,
                screen_h,
                ui.toolbox_visible,
                ui.title_visible,
                ui.anim_panel.is_some(),
            )
        })
    }

    /// Row `slot` of the list (0 = the top row shown).
    pub fn row(&self, slot: usize) -> Rect {
        Rect::new(
            self.list.x,
            self.list.y + 1 + slot as i32 * geom::ROW_H,
            self.list.w,
            geom::ROW_H,
        )
    }

    /// The eye box of row `slot`.
    pub fn eye(&self, slot: usize) -> Rect {
        let r = self.row(slot);
        Rect::new(self.panel.x + geom::EYE_X, r.y, geom::EYE_W, geom::EYE_H)
    }

    /// The name of row `slot` (its highlight and the rename gadget).
    pub fn name(&self, slot: usize) -> Rect {
        let r = self.row(slot);
        let x = self.panel.x + geom::NAME_X;
        Rect::new(x, r.y, self.panel.x + self.panel.w - x, geom::EYE_H)
    }

    /// The rectangle of `button`.
    pub fn button(&self, button: LayerButton) -> Rect {
        let x0 = self.panel.x + 1;
        let row_y = |row: i32| self.buttons_y + 1 + row * geom::BUTTON_PITCH;
        let (x, y, w) = match button {
            LayerButton::New => (x0, row_y(0), 39),
            LayerButton::Copy => (x0 + 40, row_y(0), 39),
            LayerButton::Delete => (x0, row_y(1), 26),
            LayerButton::Up => (x0 + 27, row_y(1), 25),
            LayerButton::Down => (x0 + 53, row_y(1), 26),
            LayerButton::Merge => (x0, row_y(2), 79),
        };
        Rect::new(x, y, w, geom::BUTTON_H)
    }

    /// The scroll arrows at the right end of the title row.
    pub fn scroll_up(&self) -> Rect {
        let p = self.panel;
        Rect::new(p.x + p.w - 2 * geom::ARROW_W - 2, p.y + 1, geom::ARROW_W, 9)
    }

    pub fn scroll_down(&self) -> Rect {
        let p = self.panel;
        Rect::new(p.x + p.w - geom::ARROW_W - 1, p.y + 1, geom::ARROW_W, 9)
    }
}

/// The name being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rename {
    layer: usize,
    field: TextField,
    /// What the gadget opened with: the name with only the characters it can show. While
    /// the text is still this, nothing was typed, and ending the gadget renames nothing (a
    /// name the core took with other characters keeps them).
    initial: String,
}

impl Rename {
    /// The name typed, if it is another one than the gadget opened with.
    fn typed(&self) -> Option<(usize, String)> {
        let text = self.field.text();
        (text != self.initial).then_some((self.layer, text))
    }
}

/// What a key did to the rename gadget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameStep {
    /// Still editing.
    Editing,
    /// Return: the layer and the name typed (the gadget closed).
    Done(usize, String),
    /// Esc, or Return on the name as it opened: the gadget closed, the name stays.
    Cancelled,
}

/// What the panel shows. The app refreshes the layers from the document before every redraw
/// ([`Self::update`]); the scroll position, the held button and the rename gadget belong to
/// the panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerPanel {
    /// The layers, bottom first (as the document numbers them).
    pub rows: Vec<LayerRow>,
    /// The active layer.
    pub active: usize,
    /// Which buttons work, in [`BUTTONS`] order; the others are ghosted.
    pub enabled: [bool; 6],
    /// The button held down, drawn pressed.
    pub held: Option<LayerButton>,
    /// Rows scrolled off the top of the list.
    pub scroll: usize,
    /// In the work area right of the screen (else over the screen's right edge).
    pub docked: bool,
    /// Not drawn and not hit: a requester, note or playback is open.
    pub hidden: bool,
    rename: Option<Rename>,
}

/// The eye (7×5) in the eye box of a visible layer.
const EYE: [&str; 5] = ["..###..", ".#...#.", "#..#..#", ".#...#.", "..###.."];
/// The active-layer marker (3×5).
const MARKER: [&str; 5] = ["#..", "##.", "###", "##.", "#.."];
/// The scroll arrows (5×3).
const ARROW_UP: [&str; 3] = ["..#..", ".###.", "#####"];
const ARROW_DOWN: [&str; 3] = ["#####", ".###.", "..#.."];

fn draw_rows(img: &mut IndexedImage, x: i32, y: i32, rows: &[&str], pen: u8) {
    for (dy, row) in rows.iter().enumerate() {
        for (dx, c) in row.chars().enumerate() {
            if c == '#' {
                img.set(x + dx as i32, y + dy as i32, pen);
            }
        }
    }
}

/// The characters a layer name can get in the panel: printable ASCII, the characters the
/// bitmap font draws (the core takes any name of at most 255 characters).
pub fn name_char(c: char) -> bool {
    (' '..='~').contains(&c)
}

impl LayerPanel {
    /// Refreshes the layers, the active one and the usable buttons from the document. While
    /// the active layer changes (or a layer comes or goes) the list scrolls so it shows;
    /// `capacity` is the number of rows that fit ([`Layout::capacity`]). A rename of a layer
    /// that no longer exists ends.
    pub fn update(
        &mut self,
        rows: Vec<LayerRow>,
        active: usize,
        enabled: [bool; 6],
        capacity: usize,
    ) {
        let changed = active != self.active || rows.len() != self.rows.len();
        self.rows = rows;
        self.active = active.min(self.rows.len().saturating_sub(1));
        self.enabled = enabled;
        if self
            .rename
            .as_ref()
            .is_some_and(|r| r.layer >= self.rows.len())
        {
            self.rename = None;
        }
        if changed {
            self.show_layer(self.active, capacity);
        }
        self.clamp_scroll(capacity);
    }

    /// Whether `button` works (else it is ghosted).
    pub fn enabled(&self, button: LayerButton) -> bool {
        self.enabled[button.index()]
    }

    /// The row slot (0 = top row shown) of layer `layer`, if it is shown.
    pub fn slot_of(&self, layer: usize, capacity: usize) -> Option<usize> {
        let from_top = self.rows.len().checked_sub(layer + 1)?;
        let slot = from_top.checked_sub(self.scroll)?;
        (slot < capacity).then_some(slot)
    }

    /// The layer in row `slot`, if there is one.
    pub fn layer_at(&self, slot: usize) -> Option<usize> {
        let from_top = self.scroll + slot;
        (from_top < self.rows.len()).then(|| self.rows.len() - 1 - from_top)
    }

    /// Whether rows are scrolled off the top / the bottom.
    pub fn more_above(&self) -> bool {
        self.scroll > 0
    }

    pub fn more_below(&self, capacity: usize) -> bool {
        self.scroll + capacity < self.rows.len()
    }

    /// Scrolls the list by `rows` rows (positive: further down the stack).
    pub fn scroll_by(&mut self, rows: i32, capacity: usize) {
        let to = self.scroll as i64 + i64::from(rows);
        self.scroll = to.max(0) as usize;
        self.clamp_scroll(capacity);
    }

    fn clamp_scroll(&mut self, capacity: usize) {
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(capacity));
    }

    /// Scrolls so layer `layer` shows.
    fn show_layer(&mut self, layer: usize, capacity: usize) {
        let Some(from_top) = self.rows.len().checked_sub(layer + 1) else {
            return;
        };
        if from_top < self.scroll {
            self.scroll = from_top;
        } else if capacity > 0 && from_top >= self.scroll + capacity {
            self.scroll = from_top + 1 - capacity;
        }
    }

    /// Opens the rename gadget on layer `layer` with its name, the cursor at the end.
    pub fn begin_rename(&mut self, layer: usize) {
        let Some(row) = self.rows.get(layer) else {
            return;
        };
        let mut field = TextField::new(dpx_core::layer::MAX_NAME_CHARS);
        let name: String = row.name.chars().filter(|&c| name_char(c)).collect();
        field.set(&name);
        self.rename = Some(Rename {
            layer,
            field,
            initial: name,
        });
    }

    /// The layer being renamed.
    pub fn renaming(&self) -> Option<usize> {
        self.rename.as_ref().map(|r| r.layer)
    }

    /// The name typed so far.
    pub fn rename_text(&self) -> Option<String> {
        self.rename.as_ref().map(|r| r.field.text())
    }

    /// A key for the rename gadget: Return ends it with the name (unless it is still the
    /// one the gadget opened with), Esc without; characters outside [`name_char`] are left
    /// out.
    pub fn rename_key(&mut self, key: ReqKey) -> RenameStep {
        let Some(r) = &mut self.rename else {
            return RenameStep::Cancelled;
        };
        match key {
            ReqKey::Enter => {
                let done = r
                    .typed()
                    .map_or(RenameStep::Cancelled, |(l, name)| RenameStep::Done(l, name));
                self.rename = None;
                done
            }
            ReqKey::Escape => {
                self.rename = None;
                RenameStep::Cancelled
            }
            ReqKey::Char(c) if !name_char(c) => RenameStep::Editing,
            key => {
                r.field.key(key);
                RenameStep::Editing
            }
        }
    }

    /// Ends the rename gadget, returning the layer and the name typed (as Return does):
    /// nothing while the text is still the one the gadget opened with.
    pub fn finish_rename(&mut self) -> Option<(usize, String)> {
        self.rename.take().and_then(|r| r.typed())
    }

    /// What lies at UI pixel (`x`, `y`), if it is on the panel.
    pub fn hit(&self, layout: &Layout, x: i32, y: i32) -> Option<LayerHit> {
        if self.hidden || !layout.panel.contains(x, y) {
            return None;
        }
        let cap = layout.capacity;
        if layout.scroll_up().contains(x, y) && (self.more_above() || self.more_below(cap)) {
            return Some(LayerHit::ScrollUp);
        }
        if layout.scroll_down().contains(x, y) && (self.more_above() || self.more_below(cap)) {
            return Some(LayerHit::ScrollDown);
        }
        if let Some(b) = BUTTONS.iter().find(|&&b| layout.button(b).contains(x, y)) {
            return Some(LayerHit::Button(*b));
        }
        if layout.list.contains(x, y) && y > layout.list.y {
            let slot = ((y - layout.list.y - 1) / geom::ROW_H) as usize;
            if slot < cap
                && let Some(layer) = self.layer_at(slot)
            {
                if self.renaming() == Some(layer) && layout.name(slot).contains(x, y) {
                    return Some(LayerHit::Field);
                }
                // The eye box and the column around it.
                let eye = layout.eye(slot);
                return Some(if (eye.x - 1..eye.x + eye.w + 1).contains(&x) {
                    LayerHit::Eye(layer)
                } else {
                    LayerHit::Name(layer)
                });
            }
        }
        Some(LayerHit::Background)
    }

    /// Draws the panel (nothing while it is hidden).
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens, layout: &Layout) {
        if self.hidden {
            return;
        }
        let p = layout.panel;
        let font = Font::builtin();
        img.fill_rect(p.x, p.y, p.w, p.h, pens.face);
        // The title row, like the screen's title bar.
        img.hline(p.x, p.x + p.w - 1, p.y, pens.shine);
        img.hline(p.x, p.x + p.w - 1, p.y + geom::HEADER_H - 1, pens.dark);
        font.draw(img, p.x + 4, p.y + 1, "Layers", pens.dark);
        let cap = layout.capacity;
        if self.more_above() || self.more_below(cap) {
            for (r, rows, on) in [
                (layout.scroll_up(), &ARROW_UP, self.more_above()),
                (layout.scroll_down(), &ARROW_DOWN, self.more_below(cap)),
            ] {
                let pen = if on {
                    pens.dark
                } else {
                    pens.ghost.unwrap_or(pens.shine)
                };
                draw_rows(img, r.x + 2, r.y + 3, rows, pen);
            }
        }
        // The rows, top layer first.
        for slot in 0..cap {
            let Some(layer) = self.layer_at(slot) else {
                break;
            };
            let row = &self.rows[layer];
            let r = layout.row(slot);
            let active = layer == self.active;
            if active {
                draw_rows(img, p.x + geom::MARKER_X, r.y + 2, &MARKER, pens.dark);
            }
            let eye = layout.eye(slot);
            crate::widgets::frame(img, eye, pens.dark);
            if row.visible {
                draw_rows(img, eye.x + 2, eye.y + 2, &EYE, pens.dark);
            }
            let name = layout.name(slot);
            if let Some(rename) = self.rename.as_ref().filter(|r| r.layer == layer) {
                draw_field(img, name, &rename.field, true, pens);
                continue;
            }
            let ink = if active {
                img.fill_rect(name.x, name.y, name.w, name.h, pens.select);
                pens.shine
            } else {
                pens.dark
            };
            draw_clipped(
                img,
                name.x + 1,
                name.y + 1,
                row.name.chars(),
                name.w - 2,
                ink,
            );
        }
        // The buttons.
        img.hline(p.x, p.x + p.w - 1, layout.buttons_y, pens.dark);
        for b in BUTTONS {
            let r = layout.button(b);
            let enabled = self.enabled(b);
            let ink = button_face(img, r, enabled && self.held == Some(b), pens);
            let ink = if enabled {
                ink
            } else {
                pens.ghost.unwrap_or(ink)
            };
            centred(img, r.x, r.w, r.y + (r.h - 8) / 2, b.label(), ink);
            if !enabled && pens.ghost.is_none() {
                checker(img, r, pens.face);
            }
        }
        // The dark left edge, from the top to the bottom (square corners).
        img.vline(p.x, p.y, p.y + p.h - 1, pens.dark);
    }

    /// The docked panel as its own picture: [`geom::PANEL_W`]×`screen_h` UI pixels, the
    /// strip right of the screen. While hidden the strip is the dark work area.
    pub fn render_strip(&self, pens: &Pens, screen_h: i32) -> IndexedImage {
        let mut img = IndexedImage::new(geom::PANEL_W as u32, screen_h.max(1) as u32, pens.dark);
        self.draw(&mut img, pens, &Layout::docked(0, screen_h));
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn rows(names: &[&str]) -> Vec<LayerRow> {
        names
            .iter()
            .map(|n| LayerRow {
                name: n.to_string(),
                visible: true,
            })
            .collect()
    }

    fn panel(names: &[&str], active: usize, capacity: usize) -> LayerPanel {
        let mut p = LayerPanel::default();
        p.update(rows(names), active, [true; 6], capacity);
        p
    }

    #[test]
    fn the_overlay_sits_left_of_the_toolbox_below_the_title_bar() {
        let l = Layout::overlay(320, 256, true, true, false);
        assert_eq!(l.panel, Rect::new(216, 11, 80, 245));
        assert_eq!(l.list, Rect::new(217, 22, 79, 198));
        assert_eq!(l.capacity, 19);
        assert_eq!(l.button(LayerButton::New), Rect::new(217, 221, 39, 11));
        assert_eq!(l.button(LayerButton::Copy), Rect::new(257, 221, 39, 11));
        assert_eq!(l.button(LayerButton::Down), Rect::new(270, 233, 26, 11));
        let merge = l.button(LayerButton::Merge);
        assert_eq!(merge, Rect::new(217, 245, 79, 11));
        assert_eq!(merge.y + merge.h, 256, "the last row is the screen's last");
        // Without the toolbox it is at the right edge; without the title bar at the top;
        // above the Animation Control Panel when that is shown.
        assert_eq!(
            Layout::overlay(320, 256, false, false, false).panel,
            Rect::new(240, 0, 80, 256)
        );
        let over_anim = Layout::overlay(320, 200, true, true, true);
        assert_eq!(over_anim.panel, Rect::new(216, 11, 80, 168));
        assert_eq!(over_anim.panel.y + over_anim.panel.h, 200 - 21);
        assert_eq!(over_anim.capacity, 12);
    }

    #[test]
    fn the_docked_panel_is_as_high_as_the_screen() {
        let l = Layout::docked(320, 256);
        assert_eq!(l.panel, Rect::new(320, 0, 80, 256));
        assert_eq!(
            l.list.y,
            geom::HEADER_H,
            "the title row lines up with the title bar"
        );
        assert_eq!(l.capacity, 20);
    }

    #[test]
    fn rows_list_the_top_layer_first_and_are_hit_where_they_are_drawn() {
        let l = Layout::overlay(320, 256, true, true, false);
        let p = panel(&["Background", "Hill", "Sky"], 1, l.capacity);
        // Top row: Sky (layer 2), then Hill, then Background.
        for (slot, layer) in [(0, 2), (1, 1), (2, 0)] {
            let eye = l.eye(slot);
            assert_eq!(p.hit(&l, eye.x + 3, eye.y + 3), Some(LayerHit::Eye(layer)));
            let name = l.name(slot);
            assert_eq!(
                p.hit(&l, name.x + 10, name.y + 3),
                Some(LayerHit::Name(layer))
            );
            // The marker column selects too.
            assert_eq!(
                p.hit(&l, l.panel.x + 3, name.y + 3),
                Some(LayerHit::Name(layer))
            );
        }
        let empty = l.row(5);
        assert_eq!(
            p.hit(&l, empty.x + 30, empty.y + 2),
            Some(LayerHit::Background)
        );
        for b in BUTTONS {
            let r = l.button(b);
            assert_eq!(
                p.hit(&l, r.x + r.w / 2, r.y + r.h / 2),
                Some(LayerHit::Button(b))
            );
        }
        assert_eq!(p.hit(&l, 100, 100), None, "the picture");
        assert_eq!(p.hit(&l, 300, 100), None, "the toolbox");
        assert_eq!(p.hit(&l, l.panel.x + 10, 15), Some(LayerHit::Background));
        let mut hidden = p.clone();
        hidden.hidden = true;
        assert_eq!(hidden.hit(&l, l.panel.x + 10, 30), None, "hidden: not hit");
    }

    #[test]
    fn the_active_layer_has_a_marker_and_hidden_layers_an_empty_eye_box() {
        let pens = pens();
        let l = Layout::overlay(320, 256, true, true, false);
        let mut p = panel(&["Background", "Hill", "Sky"], 1, l.capacity);
        p.rows[0].visible = false;
        let mut img = IndexedImage::new(320, 256, 0);
        p.draw(&mut img, &pens, &l);
        let marker = |img: &IndexedImage, slot: usize| {
            img.get(l.panel.x + geom::MARKER_X, l.row(slot).y + 4) == Some(pens.dark)
        };
        assert!(!marker(&img, 0) && marker(&img, 1) && !marker(&img, 2));
        let name = l.name(1);
        assert_eq!(
            img.get(name.x, name.y),
            Some(pens.select),
            "active name lit"
        );
        assert_eq!(img.get(l.name(0).x, l.name(0).y), Some(pens.face));
        let eye_centre = |slot: usize| {
            let e = l.eye(slot);
            img.get(e.x + 5, e.y + 4)
        };
        assert_eq!(eye_centre(0), Some(pens.dark), "Sky shows");
        assert_eq!(eye_centre(2), Some(pens.face), "Background is hidden");
        // Square corners: the left edge and the title row reach the panel's corners.
        assert_eq!(img.get(l.panel.x, l.panel.y), Some(pens.dark));
        assert_eq!(img.get(l.panel.x + 79, l.panel.y), Some(pens.shine));
        assert_eq!(img.get(l.panel.x, 255), Some(pens.dark));
        assert_eq!(img.get(l.panel.x - 1, 100), Some(0), "nothing left of it");
    }

    #[test]
    fn ghosted_buttons_are_grey_and_held_buttons_pressed() {
        let pens = pens();
        let l = Layout::overlay(320, 256, true, true, false);
        let mut p = panel(&["Background"], 0, l.capacity);
        let draw = |p: &LayerPanel| {
            let mut img = IndexedImage::new(320, 256, 0);
            p.draw(&mut img, &pens, &l);
            img
        };
        let all = draw(&p);
        p.enabled[LayerButton::Delete.index()] = false;
        let ghost = draw(&p);
        let del = l.button(LayerButton::Delete);
        let inked = |img: &IndexedImage, pen: u8| {
            (del.y..del.y + del.h)
                .flat_map(|y| (del.x + 1..del.x + del.w - 1).map(move |x| (x, y)))
                .filter(|&(x, y)| img.get(x, y) == Some(pen))
                .count()
        };
        let ghost_pen = pens.ghost.expect("the default palette has a grey");
        assert_eq!(inked(&all, ghost_pen), 0);
        assert!(inked(&ghost, ghost_pen) > 0, "grey label");
        p.held = Some(LayerButton::New);
        let held = draw(&p);
        let new = l.button(LayerButton::New);
        assert_eq!(held.get(new.x + 1, new.y + 1), Some(pens.select));
        assert_eq!(all.get(new.x + 1, new.y + 1), Some(pens.face));
    }

    #[test]
    fn the_list_scrolls_to_the_active_layer_and_with_the_arrows() {
        let l = Layout::overlay(320, 200, true, true, true);
        assert_eq!(l.capacity, 12);
        let names: Vec<String> = (0..20).map(|i| format!("L{i}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        // The top layer is active: the list starts at the top.
        let mut p = panel(&names, 19, l.capacity);
        assert_eq!(p.scroll, 0);
        assert!(!p.more_above() && p.more_below(l.capacity));
        assert_eq!(p.layer_at(0), Some(19));
        assert_eq!(p.slot_of(8, l.capacity), Some(11), "the last row shown");
        assert_eq!(p.slot_of(7, l.capacity), None);
        // Selecting the bottom layer scrolls it into view.
        p.update(rows(&names), 0, [true; 6], l.capacity);
        assert_eq!(p.scroll, 8);
        assert_eq!(p.slot_of(0, l.capacity), Some(11));
        assert!(p.more_above() && !p.more_below(l.capacity));
        // The arrows show and are hit; scrolling stops at the ends.
        let up = l.scroll_up();
        assert_eq!(p.hit(&l, up.x + 4, up.y + 4), Some(LayerHit::ScrollUp));
        let down = l.scroll_down();
        assert_eq!(
            p.hit(&l, down.x + 4, down.y + 4),
            Some(LayerHit::ScrollDown)
        );
        p.scroll_by(-3, l.capacity);
        assert_eq!(p.scroll, 5);
        p.scroll_by(-30, l.capacity);
        assert_eq!(p.scroll, 0);
        p.scroll_by(30, l.capacity);
        assert_eq!(p.scroll, 8);
        // The same active layer again leaves the scroll position alone.
        p.scroll_by(-2, l.capacity);
        p.update(rows(&names), 0, [true; 6], l.capacity);
        assert_eq!(p.scroll, 6);
        // Few layers: no arrows, their place is the title row.
        let few = panel(&["a", "b"], 0, l.capacity);
        assert_eq!(few.hit(&l, up.x + 4, up.y + 4), Some(LayerHit::Background));
    }

    #[test]
    fn rename_takes_printable_characters_return_and_esc() {
        let l = Layout::overlay(320, 256, true, true, false);
        let mut p = panel(&["Background", "Hill"], 1, l.capacity);
        p.begin_rename(1);
        assert_eq!(p.renaming(), Some(1));
        let name = l.name(0);
        assert_eq!(p.hit(&l, name.x + 5, name.y + 2), Some(LayerHit::Field));
        let eye = l.eye(0);
        assert_eq!(p.hit(&l, eye.x + 2, eye.y + 2), Some(LayerHit::Eye(1)));
        for _ in 0..4 {
            assert_eq!(p.rename_key(ReqKey::Backspace), RenameStep::Editing);
        }
        for c in "Sun\u{e9}!".chars() {
            p.rename_key(ReqKey::Char(c));
        }
        assert_eq!(p.rename_text().as_deref(), Some("Sun!"), "no e-acute");
        assert_eq!(
            p.rename_key(ReqKey::Enter),
            RenameStep::Done(1, "Sun!".into())
        );
        assert_eq!(p.renaming(), None);
        p.begin_rename(0);
        p.rename_key(ReqKey::Char('x'));
        assert_eq!(p.rename_key(ReqKey::Escape), RenameStep::Cancelled);
        assert_eq!(p.renaming(), None);
        p.begin_rename(0);
        p.rename_key(ReqKey::Char('2'));
        assert_eq!(p.finish_rename(), Some((0, "Background2".into())));
        // The gadget shows the text, the row's name does not.
        let pens = pens();
        p.begin_rename(1);
        let mut img = IndexedImage::new(320, 256, 0);
        p.draw(&mut img, &pens, &l);
        assert_eq!(img.get(name.x, name.y + 4), Some(pens.dark), "dark gadget");
        // A rename of a layer that went ends.
        p.update(rows(&["Background"]), 0, [true; 6], l.capacity);
        assert_eq!(p.renaming(), None);
    }

    #[test]
    fn an_untouched_name_is_no_rename() {
        let l = Layout::overlay(320, 256, true, true, false);
        // The core takes any name; the gadget shows only the characters the font has.
        let mut p = panel(&["Background", "Caf\u{e9} \u{2601}"], 1, l.capacity);
        p.begin_rename(1);
        assert_eq!(p.rename_text().as_deref(), Some("Caf "));
        // Return or a click elsewhere without typing: no new name, so "Caf" never replaces it.
        assert_eq!(p.rename_key(ReqKey::Enter), RenameStep::Cancelled);
        assert_eq!(p.renaming(), None);
        p.begin_rename(1);
        assert_eq!(p.finish_rename(), None);
        // Typed and taken back again: still the name it opened with.
        p.begin_rename(1);
        p.rename_key(ReqKey::Char('x'));
        p.rename_key(ReqKey::Backspace);
        assert_eq!(p.rename_key(ReqKey::Enter), RenameStep::Cancelled);
        // A real change is one, also for a plain name; so is a cleared field (the app keeps
        // the old name for an empty one).
        p.begin_rename(0);
        assert_eq!(p.rename_key(ReqKey::Enter), RenameStep::Cancelled);
        p.begin_rename(1);
        p.rename_key(ReqKey::Char('!'));
        assert_eq!(p.finish_rename(), Some((1, "Caf !".into())));
        p.begin_rename(0);
        for _ in 0..10 {
            p.rename_key(ReqKey::Backspace);
        }
        assert_eq!(
            p.rename_key(ReqKey::Enter),
            RenameStep::Done(0, String::new())
        );
    }

    #[test]
    fn the_strip_is_the_docked_panel_or_the_dark_work_area() {
        let pens = pens();
        let mut p = panel(&["Background", "Sky"], 1, 20);
        p.docked = true;
        let strip = p.render_strip(&pens, 256);
        assert_eq!((strip.width(), strip.height()), (80, 256));
        assert_eq!(strip.get(0, 0), Some(pens.dark), "the left edge");
        assert_eq!(strip.get(10, 5), Some(pens.face), "the title row");
        p.hidden = true;
        let hidden = p.render_strip(&pens, 256);
        assert!(hidden.pixels().iter().all(|&c| c == pens.dark));
    }
}
