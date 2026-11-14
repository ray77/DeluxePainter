//! Color ▸ Palette ▸ Arrange (ADR-31): the Arrange Palette requester. The palette is a grid of
//! swatches; dragging a swatch to another place moves it there (the colours in between shift
//! by one) or, with the Ex button, exchanges the two. Ok hands back the new order; the app
//! applies it as one `ArrangePalette` command that renumbers the pixels too, so the picture
//! looks the same. Spread (which changes colours, not their order) is not part of it.

use dpx_core::remap::{identity, move_entry, swap_entries};
use dpx_core::{IndexedImage, Palette, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::tool_requesters::ToolReqResponse;
use crate::widgets::{centred, draw_button, frame};

const COLS: usize = 16;
/// Swatch pitch: a 7×7 swatch and a 1-pixel gap.
const PITCH: i32 = 8;
const WIDTH: i32 = 272;
const BUTTON_GAP: i32 = 3;
const BUTTON_H: i32 = 13;
const GRID_Y: i32 = 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Move,
    Exchange,
    Undo,
    Revert,
    Cancel,
    Ok,
}

impl Button {
    const ALL: [Button; 6] = [
        Button::Move,
        Button::Exchange,
        Button::Undo,
        Button::Revert,
        Button::Cancel,
        Button::Ok,
    ];

    /// Wide enough for the label in the 8-pixel font.
    fn width(self) -> i32 {
        match self {
            Button::Move | Button::Undo => 40,
            Button::Exchange => 24,
            Button::Revert | Button::Cancel => 56,
            Button::Ok => 30,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Button::Move => "Move",
            Button::Exchange => "Ex",
            Button::Undo => "Undo",
            Button::Revert => "Revert",
            Button::Cancel => "Cancel",
            Button::Ok => "Ok",
        }
    }
}

/// Ids of the controls in [`ArrangeRequester::view`].
pub mod ctl {
    use crate::dialog::ControlId;

    /// The swatch grid (one control).
    pub const GRID: ControlId = 0;
    pub const MOVE: ControlId = 1;
    pub const EXCHANGE: ControlId = 2;
    pub const UNDO: ControlId = 3;
    pub const REVERT: ControlId = 4;
    pub const CANCEL: ControlId = 5;
    pub const OK: ControlId = 6;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrangeRequester {
    colors: usize,
    /// `order[slot]` = the palette entry now shown in `slot`.
    order: Vec<u8>,
    /// Earlier orders, for Undo.
    history: Vec<Vec<u8>>,
    /// The Ex button is on: a drag exchanges instead of moving.
    exchange: bool,
    /// The slot a drag started on, and the slot under the pointer now.
    drag: Option<usize>,
    over: Option<usize>,
    held: Option<Button>,
    pointer: (i32, i32),
}

impl ArrangeRequester {
    /// A requester over a palette of `colors` entries, in their present order.
    pub fn new(colors: usize) -> Self {
        let colors = colors.clamp(1, 256);
        Self {
            colors,
            order: identity(colors),
            history: Vec::new(),
            exchange: false,
            drag: None,
            over: None,
            held: None,
            pointer: (0, 0),
        }
    }

    fn rows(&self) -> i32 {
        self.colors.div_ceil(COLS) as i32
    }

    /// The whole swatch grid: every cell with the gap that belongs to it.
    fn grid_rect(&self) -> Rect {
        Rect::new(
            Self::grid_x(),
            GRID_Y,
            COLS as i32 * PITCH,
            self.rows() * PITCH,
        )
    }

    fn held_on(&self, b: Button) -> bool {
        self.held == Some(b) && self.button_rect(b).contains(self.pointer.0, self.pointer.1)
    }

    /// The requester as the Modern skin shows it (ADR-47); the swatches' colours come from
    /// `palette`. Move and Ex stand in a line under the grid (they are the mode of a drag),
    /// Undo, Revert, Cancel and Ok at the bottom.
    pub fn view(&self, palette: &Palette) -> crate::dialog::Dialog {
        use crate::dialog::{
            Button as ViewButton, Control, Dialog, Item, Kind, Mark, Part, Push, Role, Size,
            Swatch, Swatches, Tone,
        };
        let cells = self
            .order
            .iter()
            .enumerate()
            .take(self.colors)
            .map(|(slot, &entry)| Swatch {
                rgb: crate::stencil_requester::swatch_rgb(palette, entry),
                mark: if self.drag == Some(slot) {
                    Mark::Picked
                } else if self.drag.is_some() && self.over == Some(slot) {
                    Mark::Target
                } else {
                    Mark::None
                },
            })
            .collect();
        let mode = |b: Button, id, on: bool| {
            Part::Button(Push {
                control: Control::new(id, self.button_rect(b)),
                label: b.label().into(),
                on,
                pressed: self.held_on(b),
                enabled: true,
            })
        };
        let button = |b: Button, id, role| {
            ViewButton::new(
                Control::new(id, self.button_rect(b)),
                b.label(),
                role,
                self.held_on(b),
            )
        };
        Dialog {
            title: "Arrange Palette".into(),
            kind: Kind::Plain,
            size: Size::Medium,
            columns: vec![vec![
                Item::Swatches(Swatches {
                    control: Control::new(ctl::GRID, self.grid_rect()),
                    cols: COLS,
                    cells,
                    strip: false,
                }),
                Item::Line {
                    label: "Drag".into(),
                    key: None,
                    parts: vec![
                        mode(Button::Move, ctl::MOVE, !self.exchange),
                        mode(Button::Exchange, ctl::EXCHANGE, self.exchange),
                        Part::Text(
                            format!("{} of {} colours moved", self.moved(), self.colors),
                            Tone::Dim,
                        ),
                    ],
                },
            ]],
            buttons: vec![
                button(Button::Undo, ctl::UNDO, Role::Plain),
                button(Button::Revert, ctl::REVERT, Role::Plain),
                button(Button::Cancel, ctl::CANCEL, Role::Cancel),
                button(Button::Ok, ctl::OK, Role::Default),
            ],
        }
    }

    pub fn size(&self) -> (i32, i32) {
        (WIDTH, GRID_Y + self.rows() * PITCH + 12 + BUTTON_H + 8)
    }

    fn grid_x() -> i32 {
        (WIDTH - COLS as i32 * PITCH) / 2
    }

    /// The swatch cell of slot `c`, in requester coordinates.
    pub fn cell_rect(&self, c: usize) -> Rect {
        let (col, row) = ((c % COLS) as i32, (c / COLS) as i32);
        Rect::new(
            Self::grid_x() + col * PITCH,
            GRID_Y + row * PITCH,
            PITCH - 1,
            PITCH - 1,
        )
    }

    fn cell_at(&self, x: i32, y: i32) -> Option<usize> {
        let (gx, gy) = (x - Self::grid_x(), y - GRID_Y);
        if gx < 0 || gy < 0 {
            return None;
        }
        let (col, row) = ((gx / PITCH) as usize, (gy / PITCH) as usize);
        let c = row * COLS + col;
        (col < COLS && c < self.colors).then_some(c)
    }

    fn button_rect(&self, b: Button) -> Rect {
        let mut x = 6;
        for other in Button::ALL {
            if other == b {
                break;
            }
            x += other.width() + BUTTON_GAP;
        }
        let y = self.size().1 - BUTTON_H - 5;
        Rect::new(x, y, b.width(), BUTTON_H)
    }

    fn button_at(&self, x: i32, y: i32) -> Option<Button> {
        Button::ALL
            .into_iter()
            .find(|&b| self.button_rect(b).contains(x, y))
    }

    /// The arrangement made: `order[new] = old` for every palette entry.
    pub fn order(&self) -> &[u8] {
        &self.order
    }

    /// How many slots hold another entry than they started with.
    pub fn moved(&self) -> usize {
        self.order
            .iter()
            .enumerate()
            .filter(|&(i, &o)| usize::from(o) != i)
            .count()
    }

    /// Whether the Ex button is on.
    pub fn exchanging(&self) -> bool {
        self.exchange
    }

    fn apply_drag(&mut self, from: usize, to: usize) {
        if from == to {
            return;
        }
        self.history.push(self.order.clone());
        if self.exchange {
            swap_entries(&mut self.order, from, to);
        } else {
            move_entry(&mut self.order, from, to);
        }
    }

    fn click(&mut self, b: Button) -> ToolReqResponse {
        match b {
            Button::Ok => return ToolReqResponse::Ok,
            Button::Cancel => return ToolReqResponse::Cancel,
            Button::Move => self.exchange = false,
            Button::Exchange => self.exchange = true,
            Button::Undo => {
                if let Some(o) = self.history.pop() {
                    self.order = o;
                }
            }
            Button::Revert => {
                if self.moved() > 0 {
                    self.history.push(self.order.clone());
                    self.order = identity(self.colors);
                }
            }
        }
        ToolReqResponse::Continue
    }

    pub fn handle(&mut self, event: ReqEvent) -> ToolReqResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.button_at(x, y);
                self.drag = self.cell_at(x, y);
                self.over = self.drag;
                ToolReqResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                if self.drag.is_some() {
                    self.over = self.cell_at(x, y);
                }
                ToolReqResponse::Continue
            }
            ReqEvent::Release => {
                let (drag, over) = (self.drag.take(), self.over.take());
                if let (Some(from), Some(to)) = (drag, over) {
                    self.apply_drag(from, to);
                }
                match self.held.take() {
                    Some(b) if self.button_rect(b).contains(self.pointer.0, self.pointer.1) => {
                        self.click(b)
                    }
                    _ => ToolReqResponse::Continue,
                }
            }
            ReqEvent::Scroll(_) => ToolReqResponse::Continue,
            ReqEvent::Key(ReqKey::Enter) => ToolReqResponse::Ok,
            ReqEvent::Key(ReqKey::Escape) => ToolReqResponse::Cancel,
            ReqEvent::Key(_) => ToolReqResponse::Continue,
        }
    }

    /// Renders the requester; swatch `slot` shows palette entry `order[slot]`.
    pub fn render(&self, pens: &Pens, palette: &Palette) -> IndexedImage {
        let (w, h) = self.size();
        let font = Font::builtin();
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, "Arrange Palette", pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        for (slot, &entry) in self.order.iter().enumerate().take(self.colors) {
            let r = self.cell_rect(slot);
            let pen = if palette.get(entry).is_some() {
                entry
            } else {
                pens.dark
            };
            img.fill_rect(r.x, r.y, r.w, r.h, pen);
            if self.drag == Some(slot) {
                // The colour being dragged: a 3×3 mark, visible on any swatch colour.
                img.fill_rect(r.x + 2, r.y + 2, 3, 3, pens.shine);
                img.set(r.x + 3, r.y + 3, pens.dark);
                frame(&mut img, Rect::new(r.x + 1, r.y + 1, 5, 5), pens.dark);
            }
        }
        if let (Some(from), Some(to)) = (self.drag, self.over)
            && from != to
        {
            // Where it would land.
            let r = self.cell_rect(to);
            frame(&mut img, Rect::new(r.x, r.y, r.w, r.h), pens.shine);
            frame(
                &mut img,
                Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2),
                pens.dark,
            );
        }
        let info_y = GRID_Y + self.rows() * PITCH + 2;
        let text = format!(
            "{} - {} of {} colours moved",
            if self.exchange { "Ex" } else { "Move" },
            self.moved(),
            self.colors
        );
        font.draw(&mut img, 6, info_y, &text, pens.dark);
        for b in Button::ALL {
            let held = self.held == Some(b)
                && self.button_rect(b).contains(self.pointer.0, self.pointer.1);
            let on =
                (b == Button::Move && !self.exchange) || (b == Button::Exchange && self.exchange);
            draw_button(&mut img, self.button_rect(b), b.label(), held || on, pens);
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The view (ADR-47): the grid is one control over all cells; the swatch a drag started
    /// on and the one it would land on are marked; Move and Ex show the mode.
    #[test]
    fn the_view_marks_the_drag_and_shows_the_mode() {
        use crate::dialog::{Item, Mark, Part};
        let pal = Palette::dpaint_default_32();
        let mut r = ArrangeRequester::new(32);
        let v = r.view(&pal);
        let ids: Vec<u16> = v.controls().iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                ctl::GRID,
                ctl::MOVE,
                ctl::EXCHANGE,
                ctl::UNDO,
                ctl::REVERT,
                ctl::CANCEL,
                ctl::OK
            ]
        );
        let grid = v.control(ctl::GRID).unwrap().area;
        for c in 0..32usize {
            let (col, row) = ((c % 16) as i32, (c / 16) as i32);
            let (x, y) = (
                grid.x + (2 * col + 1) * grid.w / 32,
                grid.y + (2 * row + 1) * grid.h / 4,
            );
            assert_eq!(r.cell_at(x, y), Some(c));
        }
        for (id, button) in [
            (ctl::MOVE, Button::Move),
            (ctl::EXCHANGE, Button::Exchange),
            (ctl::UNDO, Button::Undo),
            (ctl::REVERT, Button::Revert),
            (ctl::CANCEL, Button::Cancel),
            (ctl::OK, Button::Ok),
        ] {
            let a = v.control(id).unwrap().area;
            assert_eq!(r.button_at(a.x + a.w / 2, a.y + a.h / 2), Some(button));
        }
        let marks = |v: &crate::dialog::Dialog| -> Vec<(usize, Mark)> {
            match &v.columns[0][0] {
                Item::Swatches(g) => g
                    .cells
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.mark != Mark::None)
                    .map(|(i, c)| (i, c.mark))
                    .collect(),
                i => panic!("the grid: {i:?}"),
            }
        };
        let modes = |v: &crate::dialog::Dialog| -> (bool, bool, String) {
            match &v.columns[0][1] {
                Item::Line { parts, .. } => match &parts[..] {
                    [Part::Button(a), Part::Button(b), Part::Text(t, _)] => (a.on, b.on, t.clone()),
                    p => panic!("{p:?}"),
                },
                i => panic!("{i:?}"),
            }
        };
        assert!(marks(&v).is_empty());
        assert_eq!(modes(&v), (true, false, "0 of 32 colours moved".into()));
        let (from, to) = (r.cell_rect(9), r.cell_rect(2));
        r.handle(ReqEvent::Press(from.x + 3, from.y + 3));
        assert_eq!(marks(&r.view(&pal)), [(9, Mark::Picked)]);
        r.handle(ReqEvent::Drag(to.x + 3, to.y + 3));
        assert_eq!(marks(&r.view(&pal)), [(2, Mark::Target), (9, Mark::Picked)]);
        r.handle(ReqEvent::Release);
        let v = r.view(&pal);
        assert!(marks(&v).is_empty());
        assert_eq!(modes(&v).2, "8 of 32 colours moved");
        // Slot 2 now shows the colour that was entry 9.
        let Item::Swatches(g) = &v.columns[0][0] else {
            panic!()
        };
        let c = pal.get(9).unwrap();
        assert_eq!(g.cells[2].rgb, [c.r, c.g, c.b]);
        let ex = r.button_rect(Button::Exchange);
        r.handle(ReqEvent::Press(ex.x + 1, ex.y + 1));
        r.handle(ReqEvent::Release);
        let m = modes(&r.view(&pal));
        assert_eq!((m.0, m.1), (false, true));
    }

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn press_release(r: &mut ArrangeRequester, x: i32, y: i32) -> ToolReqResponse {
        r.handle(ReqEvent::Press(x, y));
        r.handle(ReqEvent::Release)
    }

    fn click_button(r: &mut ArrangeRequester, b: Button) -> ToolReqResponse {
        let rect = r.button_rect(b);
        press_release(r, rect.x + 2, rect.y + 2)
    }

    fn drag(r: &mut ArrangeRequester, from: usize, to: usize) {
        let (a, b) = (r.cell_rect(from), r.cell_rect(to));
        r.handle(ReqEvent::Press(a.x + 1, a.y + 1));
        r.handle(ReqEvent::Drag(b.x + 1, b.y + 1));
        r.handle(ReqEvent::Release);
    }

    #[test]
    fn dragging_a_swatch_moves_it_and_shifts_the_others() {
        let mut r = ArrangeRequester::new(32);
        assert_eq!(r.moved(), 0);
        drag(&mut r, 2, 5);
        assert_eq!(&r.order()[..7], &[0, 1, 3, 4, 5, 2, 6]);
        assert_eq!(r.moved(), 4);
        drag(&mut r, 5, 0);
        assert_eq!(&r.order()[..7], &[2, 0, 1, 3, 4, 5, 6]);
        assert!(dpx_core::remap::is_permutation(r.order(), 32));
    }

    #[test]
    fn ex_exchanges_two_swatches() {
        let mut r = ArrangeRequester::new(32);
        click_button(&mut r, Button::Exchange);
        assert!(r.exchanging());
        drag(&mut r, 1, 9);
        assert_eq!(r.order()[1], 9);
        assert_eq!(r.order()[9], 1);
        assert_eq!(r.moved(), 2);
        click_button(&mut r, Button::Move);
        assert!(!r.exchanging());
    }

    #[test]
    fn dropping_on_the_same_or_no_swatch_changes_nothing() {
        let mut r = ArrangeRequester::new(32);
        drag(&mut r, 4, 4);
        let a = r.cell_rect(4);
        r.handle(ReqEvent::Press(a.x + 1, a.y + 1));
        r.handle(ReqEvent::Drag(2, 2)); // the title area
        r.handle(ReqEvent::Release);
        assert_eq!(r.moved(), 0);
        assert!(r.history.is_empty());
    }

    #[test]
    fn undo_and_revert() {
        let mut r = ArrangeRequester::new(32);
        drag(&mut r, 0, 3);
        drag(&mut r, 10, 12);
        click_button(&mut r, Button::Undo);
        assert_eq!(&r.order()[..4], &[1, 2, 3, 0]);
        assert_eq!(r.order()[10], 10);
        click_button(&mut r, Button::Revert);
        assert_eq!(r.moved(), 0);
        // Revert is itself undoable.
        click_button(&mut r, Button::Undo);
        assert_eq!(r.moved(), 4);
    }

    #[test]
    fn ok_cancel_and_keys() {
        let mut r = ArrangeRequester::new(32);
        assert_eq!(click_button(&mut r, Button::Ok), ToolReqResponse::Ok);
        assert_eq!(
            click_button(&mut r, Button::Cancel),
            ToolReqResponse::Cancel
        );
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Enter)), ToolReqResponse::Ok);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            ToolReqResponse::Cancel
        );
    }

    #[test]
    fn a_256_colour_palette_fits_the_smallest_screen() {
        let mut r = ArrangeRequester::new(256);
        let (w, h) = r.size();
        assert!(w <= 320 && h <= 200, "{w}x{h}");
        let last = r.cell_rect(255);
        assert!(last.y + last.h < h - BUTTON_H);
        drag(&mut r, 255, 0);
        assert_eq!(r.order()[0], 255);
        assert_eq!(r.order()[255], 254);
        let ok = r.button_rect(Button::Ok);
        assert!(ok.x + ok.w <= w);
    }

    #[test]
    fn render_shows_swatches_in_the_new_order() {
        let p = pens();
        let pal = Palette::dpaint_default_32();
        let mut r = ArrangeRequester::new(32);
        let c = r.cell_rect(0);
        assert_eq!(r.render(&p, &pal).get(c.x, c.y), Some(0));
        drag(&mut r, 9, 0);
        let img = r.render(&p, &pal);
        assert_eq!(img.get(c.x, c.y), Some(9), "slot 0 now shows entry 9");
        let c1 = r.cell_rect(1);
        assert_eq!(img.get(c1.x, c1.y), Some(0));
    }
}
