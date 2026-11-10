//! Effect ▸ Stencil ▸ Make.. (ADR-20): the Make Stencil requester. The palette is shown as a
//! grid of swatches; a click toggles whether a colour is protected (a drag paints the state
//! of the first swatch over the others). Make accepts, Clear unprotects everything, Invert
//! flips every colour. The result is the 256-entry table of the painting code.

use dpx_core::{IndexedImage, Palette, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::tool_requesters::ToolReqResponse;
use crate::widgets::{centred, draw_button, frame};

const COLS: usize = 16;
/// Swatch pitch: a 7×7 swatch and a 1-pixel gap.
const PITCH: i32 = 8;
const WIDTH: i32 = 240;
const BUTTON_W: i32 = 54;
const BUTTON_H: i32 = 13;
const GRID_Y: i32 = 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Make,
    Clear,
    Invert,
    Cancel,
}

impl Button {
    const ALL: [Button; 4] = [Button::Make, Button::Clear, Button::Invert, Button::Cancel];

    fn label(self) -> &'static str {
        match self {
            Button::Make => "Make",
            Button::Clear => "Clear",
            Button::Invert => "Invert",
            Button::Cancel => "Cancel",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StencilRequester {
    colors: usize,
    protected: Vec<bool>,
    /// The state a drag paints, while the button is held on the grid.
    paint: Option<bool>,
    held: Option<Button>,
    pointer: (i32, i32),
}

impl StencilRequester {
    /// `colors` palette entries; `current` is the stencil to start from (`None` = nothing
    /// protected).
    pub fn new(colors: usize, current: Option<&[bool]>) -> Self {
        let mut protected = vec![false; 256];
        if let Some(c) = current {
            for (p, &v) in protected.iter_mut().zip(c) {
                *p = v;
            }
        }
        Self {
            colors: colors.clamp(1, 256),
            protected,
            paint: None,
            held: None,
            pointer: (0, 0),
        }
    }

    fn rows(&self) -> i32 {
        self.colors.div_ceil(COLS) as i32
    }

    pub fn size(&self) -> (i32, i32) {
        (WIDTH, GRID_Y + self.rows() * PITCH + 12 + BUTTON_H + 8)
    }

    fn grid_x() -> i32 {
        (WIDTH - COLS as i32 * PITCH) / 2
    }

    /// The swatch cell of colour `c`, in requester coordinates.
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
        let i = Button::ALL.iter().position(|&x| x == b).unwrap_or(0) as i32;
        let y = self.size().1 - BUTTON_H - 5;
        Rect::new(6 + i * (BUTTON_W + 4), y, BUTTON_W, BUTTON_H)
    }

    fn button_at(&self, x: i32, y: i32) -> Option<Button> {
        Button::ALL
            .into_iter()
            .find(|&b| self.button_rect(b).contains(x, y))
    }

    /// How many colours are protected.
    pub fn count(&self) -> usize {
        self.protected[..self.colors].iter().filter(|&&p| p).count()
    }

    /// The stencil as made: 256 entries, `true` = protected.
    pub fn protected(&self) -> Vec<bool> {
        self.protected.clone()
    }

    /// Whether colour `c` is protected.
    pub fn is_protected(&self, c: usize) -> bool {
        self.protected.get(c).copied().unwrap_or(false)
    }

    fn click(&mut self, b: Button) -> ToolReqResponse {
        match b {
            Button::Make => return ToolReqResponse::Ok,
            Button::Cancel => return ToolReqResponse::Cancel,
            Button::Clear => self.protected[..self.colors].fill(false),
            Button::Invert => {
                for p in &mut self.protected[..self.colors] {
                    *p = !*p;
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
                if let Some(c) = self.cell_at(x, y) {
                    let state = !self.protected[c];
                    self.paint = Some(state);
                    self.protected[c] = state;
                }
                ToolReqResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                // Fast drags skip swatches: paint along the line from the last position.
                let (x0, y0) = self.pointer;
                self.pointer = (x, y);
                if let Some(state) = self.paint {
                    let steps = (x - x0).abs().max((y - y0).abs()).max(1);
                    for i in 1..=steps {
                        let (px, py) = (x0 + (x - x0) * i / steps, y0 + (y - y0) * i / steps);
                        if let Some(c) = self.cell_at(px, py) {
                            self.protected[c] = state;
                        }
                    }
                }
                ToolReqResponse::Continue
            }
            ReqEvent::Release => {
                self.paint = None;
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

    /// Renders the requester; the swatches come from `palette`.
    pub fn render(&self, pens: &Pens, palette: &Palette) -> IndexedImage {
        let (w, h) = self.size();
        let font = Font::builtin();
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, "Make Stencil", pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        for c in 0..self.colors {
            let r = self.cell_rect(c);
            let pen = if palette.get(c as u8).is_some() {
                c as u8
            } else {
                pens.dark
            };
            img.fill_rect(r.x, r.y, r.w, r.h, pen);
            if self.protected[c] {
                // A 3×3 mark: light block, dark centre, visible on any swatch colour.
                img.fill_rect(r.x + 2, r.y + 2, 3, 3, pens.shine);
                img.set(r.x + 3, r.y + 3, pens.dark);
                frame(&mut img, Rect::new(r.x + 1, r.y + 1, 5, 5), pens.dark);
            }
        }
        let info_y = GRID_Y + self.rows() * PITCH + 2;
        let text = format!("{} of {} colours protected", self.count(), self.colors);
        font.draw(&mut img, 6, info_y, &text, pens.dark);
        for b in Button::ALL {
            let pressed = self.held == Some(b)
                && self.button_rect(b).contains(self.pointer.0, self.pointer.1);
            draw_button(&mut img, self.button_rect(b), b.label(), pressed, pens);
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn click(r: &mut StencilRequester, x: i32, y: i32) -> ToolReqResponse {
        r.handle(ReqEvent::Press(x, y));
        r.handle(ReqEvent::Release)
    }

    fn click_cell(r: &mut StencilRequester, c: usize) {
        let cell = r.cell_rect(c);
        click(r, cell.x + 2, cell.y + 2);
    }

    #[test]
    fn click_toggles_a_colour_and_the_count_follows() {
        let mut r = StencilRequester::new(32, None);
        click_cell(&mut r, 5);
        click_cell(&mut r, 20);
        assert!(r.is_protected(5) && r.is_protected(20));
        assert_eq!(r.count(), 2);
        click_cell(&mut r, 5);
        assert!(!r.is_protected(5));
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn drag_paints_the_first_swatchs_new_state() {
        let mut r = StencilRequester::new(32, None);
        let a = r.cell_rect(2);
        r.handle(ReqEvent::Press(a.x + 1, a.y + 1));
        for c in 3..8 {
            let cell = r.cell_rect(c);
            r.handle(ReqEvent::Drag(cell.x + 1, cell.y + 1));
        }
        r.handle(ReqEvent::Release);
        assert_eq!(r.count(), 6);
        // Dragging from a protected swatch unprotects.
        let a = r.cell_rect(4);
        r.handle(ReqEvent::Press(a.x + 1, a.y + 1));
        let b = r.cell_rect(6);
        r.handle(ReqEvent::Drag(b.x + 1, b.y + 1));
        r.handle(ReqEvent::Release);
        assert!(r.is_protected(2) && r.is_protected(3) && r.is_protected(7));
        assert!(!r.is_protected(4) && !r.is_protected(5) && !r.is_protected(6));
    }

    #[test]
    fn clear_invert_make_cancel() {
        let mut r = StencilRequester::new(32, Some(&[true, false, true]));
        assert_eq!(r.count(), 2);
        let b = r.button_rect(Button::Invert);
        assert_eq!(click(&mut r, b.x + 2, b.y + 2), ToolReqResponse::Continue);
        assert_eq!(r.count(), 30, "only the 32 palette colours flip");
        assert!(!r.protected()[200], "colours beyond the palette stay off");
        let b = r.button_rect(Button::Clear);
        click(&mut r, b.x + 2, b.y + 2);
        assert_eq!(r.count(), 0);
        let b = r.button_rect(Button::Make);
        assert_eq!(click(&mut r, b.x + 2, b.y + 2), ToolReqResponse::Ok);
        let b = r.button_rect(Button::Cancel);
        assert_eq!(click(&mut r, b.x + 2, b.y + 2), ToolReqResponse::Cancel);
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Enter)), ToolReqResponse::Ok);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            ToolReqResponse::Cancel
        );
    }

    #[test]
    fn a_256_colour_palette_fits_the_smallest_screen() {
        let r = StencilRequester::new(256, None);
        let (w, h) = r.size();
        assert!(w <= 320 && h <= 200, "{w}x{h}");
        let last = r.cell_rect(255);
        assert!(last.y + last.h < h - BUTTON_H);
        let mut r = r;
        click_cell(&mut r, 255);
        assert!(r.is_protected(255));
    }

    #[test]
    fn render_shows_swatches_marks_and_count() {
        let p = pens();
        let pal = Palette::dpaint_default_32();
        let mut r = StencilRequester::new(32, None);
        let plain = r.render(&p, &pal);
        let c = r.cell_rect(9);
        assert_eq!(plain.get(c.x, c.y), Some(9), "swatch in its palette colour");
        click_cell(&mut r, 9);
        let marked = r.render(&p, &pal);
        assert_ne!(marked, plain);
        assert_eq!(marked.get(c.x + 3, c.y + 3), Some(p.dark));
        assert_eq!(marked.get(c.x + 2, c.y + 3), Some(p.shine));
    }
}
