//! Color ▸ Ranges..: the Range requester (DeluxePaint IV AGA, ADR-22).
//!
//! A modal requester on the UI layer, centred, drawn in LoRes UI pixels with palette-derived
//! pens like the Color Mixer. It edits a copy of the colour-cycling ranges (up to eight, as in
//! DeluxePaint): the palette strip marks the selected range's span, the buttons pick the
//! range, its speed, direction and whether it is active. The caller reads
//! [`RangeRequester::ranges`] on Ok and drops the requester on Cancel.
//!
//! A range here is exactly a CRNG chunk (`rate`, `flags`, `low`, `high`), so loaded ranges
//! round-trip untouched until the user edits them. The layout measurements are our own.

use dpx_core::{IndexedImage, Palette, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{button_face, centred, draw_button, frame};

/// Requester size in LoRes UI pixels; fits the smallest (320×200) UI screen.
pub const WIDTH: i32 = 288;
pub const HEIGHT: i32 = 122;
/// Ranges DeluxePaint IV offers.
pub const MAX_RANGES: usize = 8;
/// Most colours in one range.
pub const MAX_SPAN: u8 = 32;
/// One slider step in CRNG rate units (63 steps reach about 59 cycle steps per second).
pub const RATE_UNIT: u16 = 256;
/// Highest slider value.
pub const RATE_MAX: i32 = 63;
/// Rate given to a range whose span was just set.
const NEW_RATE: u16 = 16 * RATE_UNIT;

mod geom {
    pub const TITLE_Y: i32 = 2;
    pub const TITLE_LINE_Y: i32 = 12;
    pub const GRID_X: i32 = 16;
    pub const GRID_Y: i32 = 16;
    pub const GRID_W: i32 = 256;
    pub const GRID_H: i32 = 32;
    pub const ROW_A: i32 = 54;
    pub const SELECT_X: i32 = 64;
    pub const SELECT_W: i32 = 13;
    pub const SELECT_STEP: i32 = 14;
    pub const SPAN_X: i32 = 184;
    pub const ROW_B: i32 = 70;
    pub const SLIDER_X: i32 = 50;
    pub const SLIDER_W: i32 = 176;
    pub const SLIDER_H: i32 = 9;
    pub const KNOB_W: i32 = 10;
    pub const VALUE_RIGHT: i32 = 280;
    pub const ROW_C: i32 = 84;
    pub const INFO_Y: i32 = 102;
    pub const BUTTONS_Y: i32 = 104;
    pub const BUTTON_H: i32 = 13;
}

/// One colour range, as stored in a CRNG chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RangeData {
    /// 16384 = 60 steps per second.
    pub rate: u16,
    /// Bit 0 active, bit 1 reverse; other bits are kept as they are.
    pub flags: u16,
    pub low: u8,
    pub high: u8,
}

impl RangeData {
    /// An unused slot.
    pub const EMPTY: RangeData = RangeData {
        rate: 0,
        flags: 0,
        low: 0,
        high: 0,
    };

    /// A range needs at least two colours to cycle.
    pub fn defined(&self) -> bool {
        self.high > self.low
    }

    pub fn active(&self) -> bool {
        self.flags & 1 != 0
    }

    pub fn reverse(&self) -> bool {
        self.flags & 2 != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeResponse {
    Continue,
    /// Close and forget the changes.
    Cancel,
    /// Close and keep [`RangeRequester::ranges`].
    Ok,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    /// Range number 0..MAX_RANGES (shown 1..).
    Select(usize),
    Direction,
    Active,
    Clear,
    Cancel,
    Ok,
}

impl Button {
    fn all() -> impl Iterator<Item = Button> {
        (0..MAX_RANGES).map(Button::Select).chain([
            Button::Direction,
            Button::Active,
            Button::Clear,
            Button::Cancel,
            Button::Ok,
        ])
    }

    fn rect(self) -> Rect {
        let at = |x, y, w| Rect::new(x, y, w, geom::BUTTON_H);
        match self {
            Button::Select(i) => at(
                geom::SELECT_X + i as i32 * geom::SELECT_STEP,
                geom::ROW_A - 2,
                geom::SELECT_W,
            ),
            Button::Direction => at(8, geom::ROW_C, 64),
            Button::Active => at(76, geom::ROW_C, 52),
            Button::Clear => at(132, geom::ROW_C, 44),
            Button::Cancel => at(186, geom::BUTTONS_Y, 52),
            Button::Ok => at(242, geom::BUTTONS_Y, 38),
        }
    }

    /// Needs a range with a span.
    fn needs_span(self) -> bool {
        matches!(self, Button::Direction | Button::Active | Button::Clear)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Button(Button),
    Knob {
        grab: i32,
    },
    Track,
    /// Pressed on a colour: the first colour of a span, which may be dragged to the last.
    Span {
        anchor: u8,
        moved: bool,
    },
}

/// State of the Range requester.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeRequester {
    colors: usize,
    /// The first [`MAX_RANGES`] ranges.
    slots: [RangeData; MAX_RANGES],
    /// Ranges beyond the eighth in a loaded file: kept, not editable.
    extra: Vec<RangeData>,
    selected: usize,
    /// The first colour was clicked; the next click on a colour sets the last one.
    awaiting_high: Option<u8>,
    held: Option<Held>,
    pointer: (i32, i32),
}

/// Slider value of a rate (nearest step, clamped).
pub fn rate_to_step(rate: u16) -> i32 {
    ((i32::from(rate) + i32::from(RATE_UNIT) / 2) / i32::from(RATE_UNIT)).min(RATE_MAX)
}

impl RangeRequester {
    /// Edits a copy of `ranges` (file order = range numbers 1..) over `palette`, opening on
    /// range `selected` (0-based; out of range = the first defined one).
    pub fn new(palette: &Palette, ranges: &[RangeData], selected: usize) -> Self {
        let mut slots = [RangeData::EMPTY; MAX_RANGES];
        for (s, r) in slots.iter_mut().zip(ranges) {
            *s = *r;
        }
        let extra = ranges.iter().skip(MAX_RANGES).copied().collect();
        let selected = if selected < MAX_RANGES {
            selected
        } else {
            slots.iter().position(RangeData::defined).unwrap_or(0)
        };
        Self {
            colors: palette.len(),
            slots,
            extra,
            selected,
            awaiting_high: None,
            held: None,
            pointer: (0, 0),
        }
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(screen_w: i32, screen_h: i32) -> (i32, i32) {
        (
            ((screen_w - WIDTH) / 2).max(0),
            ((screen_h - HEIGHT) / 2).max(0),
        )
    }

    /// The edited ranges: slots up to the last one in use, in range-number order (unused
    /// slots in between stay as empty entries so numbers do not shift), plus kept extras.
    pub fn ranges(&self) -> Vec<RangeData> {
        let used = self
            .slots
            .iter()
            .rposition(|r| *r != RangeData::EMPTY)
            .map_or(0, |i| i + 1);
        let mut v = self.slots[..used].to_vec();
        if !self.extra.is_empty() {
            v.resize(MAX_RANGES, RangeData::EMPTY);
            v.extend_from_slice(&self.extra);
        }
        v
    }

    /// The selected range number, 0-based.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected range.
    pub fn current(&self) -> RangeData {
        self.slots[self.selected]
    }

    /// Whether a span is waiting for its last colour.
    pub fn awaiting_high(&self) -> bool {
        self.awaiting_high.is_some()
    }

    /// Rect of range-number button `i` (0-based) in requester coordinates.
    pub fn select_rect(i: usize) -> Rect {
        Button::Select(i).rect()
    }

    /// Rect of the Forward/Reverse, Active and Clear buttons.
    pub fn direction_rect() -> Rect {
        Button::Direction.rect()
    }

    pub fn active_rect() -> Rect {
        Button::Active.rect()
    }

    pub fn clear_rect() -> Rect {
        Button::Clear.rect()
    }

    /// Track of the rate slider in requester coordinates.
    pub fn slider_rect() -> Rect {
        Rect::new(geom::SLIDER_X, geom::ROW_B, geom::SLIDER_W, geom::SLIDER_H)
    }

    /// Pixels the knob can travel.
    fn free() -> i32 {
        geom::SLIDER_W - 2 - geom::KNOB_W
    }

    fn knob_x(&self) -> i32 {
        Self::slider_rect().x + 1 + rate_to_step(self.current().rate) * Self::free() / RATE_MAX
    }

    /// (columns, rows, cell width, cell height) of the palette grid, as in the Mixer.
    fn grid(&self) -> (i32, i32, i32, i32) {
        let n = self.colors as i32;
        let cols = if n <= 16 {
            n
        } else if n <= 64 {
            16
        } else {
            32
        };
        let rows = (n + cols - 1) / cols;
        (cols, rows, geom::GRID_W / cols, geom::GRID_H / rows)
    }

    /// Cell of palette entry `i` in requester coordinates (without its dark separator line).
    pub fn cell_rect(&self, i: u8) -> Rect {
        let (cols, _, cw, ch) = self.grid();
        let i = i32::from(i);
        Rect::new(
            geom::GRID_X + (i % cols) * cw,
            geom::GRID_Y + (i / cols) * ch,
            cw,
            ch,
        )
    }

    fn cell_at(&self, x: i32, y: i32) -> Option<u8> {
        let (cols, rows, cw, ch) = self.grid();
        if x < geom::GRID_X || y < geom::GRID_Y {
            return None;
        }
        let (col, row) = ((x - geom::GRID_X) / cw, (y - geom::GRID_Y) / ch);
        let i = row * cols + col;
        (col < cols && row < rows && (i as usize) < self.colors).then_some(i as u8)
    }

    fn enabled(&self, b: Button) -> bool {
        !b.needs_span() || self.current().defined()
    }

    /// Sets the span of the selected range to `a..=b` (any order, at most [`MAX_SPAN`]
    /// colours). A range that was not defined before starts active at a medium speed.
    fn set_span(&mut self, a: u8, b: u8) {
        let (low, mut high) = (a.min(b), a.max(b));
        high = high.min(low.saturating_add(MAX_SPAN - 1));
        let r = &mut self.slots[self.selected];
        let fresh = !r.defined();
        r.low = low;
        r.high = high;
        if fresh && r.defined() {
            if r.rate == 0 {
                r.rate = NEW_RATE;
            }
            r.flags |= 1;
        }
    }

    fn set_step(&mut self, step: i32) {
        let step = step.clamp(0, RATE_MAX);
        // An untouched slider position keeps the file's exact rate.
        if step != rate_to_step(self.current().rate) {
            self.slots[self.selected].rate = step as u16 * RATE_UNIT;
        }
    }

    /// Moves the knob so its left edge is at `left`.
    fn knob_to(&mut self, left: i32) {
        let free = Self::free();
        let rel = (left - Self::slider_rect().x - 1).clamp(0, free);
        self.set_step((rel * RATE_MAX + free / 2) / free);
    }

    fn fire(&mut self, b: Button) -> RangeResponse {
        if !self.enabled(b) {
            return RangeResponse::Continue;
        }
        let r = &mut self.slots[self.selected];
        match b {
            Button::Select(i) => {
                self.selected = i;
                self.awaiting_high = None;
            }
            Button::Direction => r.flags ^= 2,
            Button::Active => r.flags ^= 1,
            Button::Clear => {
                *r = RangeData::EMPTY;
                self.awaiting_high = None;
            }
            Button::Cancel => return RangeResponse::Cancel,
            Button::Ok => return RangeResponse::Ok,
        }
        RangeResponse::Continue
    }

    fn press(&mut self, x: i32, y: i32) {
        self.pointer = (x, y);
        if let Some(b) = Button::all().find(|b| b.rect().contains(x, y)) {
            self.held = Some(Held::Button(b));
            return;
        }
        if let Some(c) = self.cell_at(x, y) {
            match self.awaiting_high.take() {
                Some(low) => self.set_span(low, c),
                // Click first, then last colour — or drag from first to last.
                None => {
                    self.held = Some(Held::Span {
                        anchor: c,
                        moved: false,
                    });
                }
            }
            return;
        }
        if self.current().defined() && Self::slider_rect().contains(x, y) {
            let kx = self.knob_x();
            if (kx..kx + geom::KNOB_W).contains(&x) {
                self.held = Some(Held::Knob { grab: x - kx });
            } else {
                let step = rate_to_step(self.current().rate) + if x < kx { -1 } else { 1 };
                self.set_step(step);
                self.held = Some(Held::Track);
            }
        }
    }

    fn pressed(&self, b: Button) -> bool {
        let held =
            self.held == Some(Held::Button(b)) && b.rect().contains(self.pointer.0, self.pointer.1);
        let on = match b {
            Button::Select(i) => i == self.selected,
            Button::Active => self.current().active() && self.current().defined(),
            _ => false,
        };
        held || on
    }

    /// Handles one event in requester coordinates (see [`RangeRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> RangeResponse {
        match event {
            ReqEvent::Press(x, y) => self.press(x, y),
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                match self.held {
                    Some(Held::Knob { grab }) => self.knob_to(x - grab),
                    Some(Held::Span { anchor, .. }) => {
                        if let Some(c) = self.cell_at(x, y)
                            && c != anchor
                        {
                            self.set_span(anchor, c);
                            self.held = Some(Held::Span {
                                anchor,
                                moved: true,
                            });
                        }
                    }
                    _ => {}
                }
            }
            ReqEvent::Release => match self.held.take() {
                Some(Held::Button(b)) if b.rect().contains(self.pointer.0, self.pointer.1) => {
                    return self.fire(b);
                }
                // Released without dragging: now wait for the last colour.
                Some(Held::Span {
                    anchor,
                    moved: false,
                }) => self.awaiting_high = Some(anchor),
                _ => {}
            },
            ReqEvent::Key(ReqKey::Enter) => return RangeResponse::Ok,
            ReqEvent::Key(ReqKey::Escape) => {
                if self.awaiting_high.take().is_none() {
                    return RangeResponse::Cancel;
                }
            }
            ReqEvent::Key(ReqKey::Char(c)) => match c {
                '1'..='8' => return self.fire(Button::Select(c as usize - '1' as usize)),
                'r' => return self.fire(Button::Direction),
                _ => {}
            },
            ReqEvent::Key(_) | ReqEvent::Scroll(_) => {}
        }
        RangeResponse::Continue
    }

    /// Draws the requester onto a UI screen at [`RangeRequester::origin`].
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = Self::origin(img.width() as i32, img.height() as i32);
        img.blit(&self.render(pens), x, y);
    }

    /// Renders the requester alone (WIDTH×HEIGHT). Palette cells use the palette indices
    /// themselves, so they show whatever palette the screen is shown in.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let mut img = IndexedImage::new(WIDTH as u32, HEIGHT as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, WIDTH, HEIGHT), pens.dark);
        centred(&mut img, 0, WIDTH, geom::TITLE_Y, "Range", pens.dark);
        img.hline(1, WIDTH - 2, geom::TITLE_LINE_Y, pens.dark);
        let ghost = pens.ghost.unwrap_or(pens.dark);
        let cur = self.current();

        // Palette grid with dark separator lines; a box marks each colour of the span (or
        // the lone first colour while the last one is awaited).
        let (cols, rows, cw, ch) = self.grid();
        img.fill_rect(
            geom::GRID_X,
            geom::GRID_Y,
            cols * cw + 1,
            rows * ch + 1,
            pens.dark,
        );
        for i in 0..self.colors {
            let n = i as u8;
            let r = self.cell_rect(n);
            img.fill_rect(r.x + 1, r.y + 1, r.w - 1, r.h - 1, n);
            let marked = match self.awaiting_high {
                Some(low) => n == low,
                None => cur.defined() && n >= cur.low && n <= cur.high,
            };
            if marked {
                let (mw, mh) = ((r.w - 3).min(6), (r.h - 3).min(6));
                let (mx, my) = (r.x + 1 + (r.w - 1 - mw) / 2, r.y + 1 + (r.h - 1 - mh) / 2);
                frame(&mut img, Rect::new(mx, my, mw, mh), pens.dark);
                if mw > 2 && mh > 2 {
                    img.fill_rect(mx + 1, my + 1, mw - 2, mh - 2, pens.shine);
                }
            }
        }

        font.draw(&mut img, 8, geom::ROW_A, "Range", pens.dark);
        let span = if self.awaiting_high.is_some() {
            "Pick last".to_string()
        } else if cur.defined() {
            format!("Colors {}-{}", cur.low, cur.high)
        } else {
            "Empty".to_string()
        };
        font.draw(&mut img, geom::SPAN_X, geom::ROW_A, &span, pens.dark);

        // Rate slider (ghosted without a span).
        let t = Self::slider_rect();
        let on = cur.defined();
        let ink = if on { pens.dark } else { ghost };
        font.draw(&mut img, 8, t.y + 1, "Rate", ink);
        recessed(&mut img, t, pens);
        if on {
            let knob = Rect::new(self.knob_x(), t.y + 1, geom::KNOB_W, t.h - 2);
            let grabbed = matches!(self.held, Some(Held::Knob { .. }));
            button_face(&mut img, knob, grabbed, pens);
        }
        let text = rate_to_step(cur.rate).to_string();
        font.draw(
            &mut img,
            geom::VALUE_RIGHT - font.text_width(&text),
            t.y + 1,
            &text,
            ink,
        );

        let info = if self.awaiting_high.is_some() {
            "Click the last color of the range".to_string()
        } else if on {
            let sps = f64::from(cur.rate) * 60.0 / 16384.0;
            format!(
                "{} colors, {sps:.0}/s{}",
                usize::from(cur.high - cur.low) + 1,
                if cur.active() { "" } else { ", off" }
            )
        } else {
            "Click the first, then the last color".to_string()
        };
        font.draw(&mut img, 8, geom::INFO_Y, &info, pens.dark);

        for b in Button::all() {
            let label = match b {
                Button::Select(i) => (i + 1).to_string(),
                Button::Direction => if cur.reverse() { "Reverse" } else { "Forward" }.to_string(),
                Button::Active => "Active".to_string(),
                Button::Clear => "Clear".to_string(),
                Button::Cancel => "Cancel".to_string(),
                Button::Ok => "Ok".to_string(),
            };
            if self.enabled(b) {
                draw_button(&mut img, b.rect(), &label, self.pressed(b), pens);
            } else {
                let r = b.rect();
                button_face(&mut img, r, false, pens);
                centred(&mut img, r.x, r.w, r.y + (r.h - 8) / 2, &label, ghost);
            }
        }
        img
    }
}

/// Recessed bevel: dark top/left, shine bottom/right, face inside.
fn recessed(img: &mut IndexedImage, r: Rect, pens: &Pens) {
    img.fill_rect(r.x, r.y, r.w, r.h, pens.face);
    let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
    img.hline(r.x, x1, r.y, pens.dark);
    img.vline(r.x, r.y, y1, pens.dark);
    img.hline(r.x, x1, y1, pens.shine);
    img.vline(x1, r.y, y1, pens.shine);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        Palette::dpaint_default_32()
    }

    fn click(r: &mut RangeRequester, rect: Rect) -> RangeResponse {
        r.handle(ReqEvent::Press(rect.x + rect.w / 2, rect.y + rect.h / 2));
        r.handle(ReqEvent::Release)
    }

    fn cell(r: &mut RangeRequester, i: u8) {
        let c = r.cell_rect(i);
        click(r, c);
    }

    fn empty() -> RangeRequester {
        RangeRequester::new(&pal(), &[], 0)
    }

    #[test]
    fn selecting_ranges_by_button_and_key() {
        let mut r = empty();
        assert_eq!(r.selected(), 0);
        click(&mut r, RangeRequester::select_rect(4));
        assert_eq!(r.selected(), 4);
        r.handle(ReqEvent::Key(ReqKey::Char('8')));
        assert_eq!(r.selected(), 7);
        r.handle(ReqEvent::Key(ReqKey::Char('1')));
        assert_eq!(r.selected(), 0);
        // Opens on the first defined range when no number is asked for.
        let data = [
            RangeData::EMPTY,
            RangeData {
                rate: 256,
                flags: 1,
                low: 2,
                high: 5,
            },
        ];
        assert_eq!(RangeRequester::new(&pal(), &data, 99).selected(), 1);
    }

    #[test]
    fn click_low_then_high_sets_the_span() {
        let mut r = empty();
        cell(&mut r, 20);
        assert!(r.awaiting_high());
        assert!(
            !r.current().defined(),
            "nothing changes until the 2nd click"
        );
        cell(&mut r, 27);
        assert!(!r.awaiting_high());
        let c = r.current();
        assert_eq!((c.low, c.high), (20, 27));
        // A new range starts active at a medium speed.
        assert!(c.active() && c.rate == 16 * RATE_UNIT && !c.reverse());
        // High before low is the same span.
        click(&mut r, RangeRequester::select_rect(1));
        cell(&mut r, 15);
        cell(&mut r, 8);
        assert_eq!((r.current().low, r.current().high), (8, 15));
        // Range 1 is untouched.
        click(&mut r, RangeRequester::select_rect(0));
        assert_eq!((r.current().low, r.current().high), (20, 27));
    }

    #[test]
    fn dragging_across_colours_sets_the_span() {
        let mut r = empty();
        let (a, b) = (r.cell_rect(2), r.cell_rect(6));
        r.handle(ReqEvent::Press(a.x + 2, a.y + 2));
        r.handle(ReqEvent::Drag(b.x + 2, b.y + 2));
        r.handle(ReqEvent::Release);
        assert!(!r.awaiting_high());
        assert_eq!((r.current().low, r.current().high), (2, 6));
    }

    #[test]
    fn span_is_limited_to_32_colours() {
        let big = Palette::new(vec![dpx_core::Rgb8::BLACK; 64]).unwrap();
        let mut r = RangeRequester::new(&big, &[], 0);
        cell(&mut r, 3);
        cell(&mut r, 60);
        assert_eq!((r.current().low, r.current().high), (3, 34));
    }

    #[test]
    fn escape_withdraws_a_half_set_span_then_cancels() {
        let mut r = empty();
        cell(&mut r, 4);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            RangeResponse::Continue
        );
        assert!(!r.awaiting_high());
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            RangeResponse::Cancel
        );
    }

    #[test]
    fn rate_direction_active_and_clear() {
        let mut r = empty();
        // Ghosted without a span: nothing happens.
        click(&mut r, RangeRequester::direction_rect());
        click(&mut r, RangeRequester::active_rect());
        let t = RangeRequester::slider_rect();
        r.handle(ReqEvent::Press(t.x + 100, t.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.current(), RangeData::EMPTY);

        cell(&mut r, 8);
        cell(&mut r, 15);
        // Drag the knob to the far right: top speed.
        let kx = r.knob_x();
        r.handle(ReqEvent::Press(kx + 2, t.y + 4));
        r.handle(ReqEvent::Drag(t.x + t.w + 30, t.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.current().rate, 63 * RATE_UNIT);
        // A click beside the knob moves one step.
        r.handle(ReqEvent::Press(t.x + 2, t.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.current().rate, 62 * RATE_UNIT);
        // Direction and active toggle; other flag bits survive.
        r.slots[0].flags |= 0x100;
        click(&mut r, RangeRequester::direction_rect());
        assert!(r.current().reverse());
        click(&mut r, RangeRequester::direction_rect());
        assert!(!r.current().reverse());
        click(&mut r, RangeRequester::active_rect());
        assert!(!r.current().active());
        assert_eq!(r.current().flags & 0x100, 0x100);
        r.handle(ReqEvent::Key(ReqKey::Char('r')));
        assert!(r.current().reverse());
        // Clear empties the range.
        click(&mut r, RangeRequester::clear_rect());
        assert_eq!(r.current(), RangeData::EMPTY);
    }

    #[test]
    fn untouched_rates_keep_their_exact_value() {
        let data = [RangeData {
            rate: 1234,
            flags: 3,
            low: 1,
            high: 4,
        }];
        let mut r = RangeRequester::new(&pal(), &data, 0);
        // Pressing the knob without moving it does not round the rate.
        let t = RangeRequester::slider_rect();
        let kx = r.knob_x();
        r.handle(ReqEvent::Press(kx + 2, t.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.ranges(), data);
    }

    #[test]
    fn ok_and_cancel_by_button_and_key() {
        let mut r = empty();
        let ok = Button::Ok.rect();
        let cancel = Button::Cancel.rect();
        assert_eq!(click(&mut r, ok), RangeResponse::Ok);
        assert_eq!(click(&mut r, cancel), RangeResponse::Cancel);
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Enter)), RangeResponse::Ok);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            RangeResponse::Cancel
        );
    }

    #[test]
    fn ranges_keep_numbers_and_trim_trailing_empties() {
        let mut r = empty();
        assert!(r.ranges().is_empty());
        click(&mut r, RangeRequester::select_rect(2));
        cell(&mut r, 1);
        cell(&mut r, 3);
        let v = r.ranges();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], RangeData::EMPTY);
        assert_eq!((v[2].low, v[2].high), (1, 3));
        // Ranges beyond the eighth are kept as they are.
        let many: Vec<RangeData> = (0..10)
            .map(|i| RangeData {
                rate: 256,
                flags: 1,
                low: i,
                high: i + 2,
            })
            .collect();
        assert_eq!(RangeRequester::new(&pal(), &many, 0).ranges(), many);
    }

    #[test]
    fn renders_and_marks_the_span() {
        let pens = Pens::from_palette(&pal());
        let data = [RangeData {
            rate: 4096,
            flags: 1,
            low: 20,
            high: 27,
        }];
        let r = RangeRequester::new(&pal(), &data, 0);
        let img = r.render(&pens);
        assert_eq!((img.width() as i32, img.height() as i32), (WIDTH, HEIGHT));
        // The cell centre of a marked colour is the shine pen, an unmarked one its own index.
        let m = r.cell_rect(21);
        let u = r.cell_rect(12);
        let at = |c: Rect, dx, dy| img.get(c.x + dx, c.y + dy);
        assert_eq!(
            at(m, 1 + (m.w - 1) / 2, 1 + (m.h - 1) / 2),
            Some(pens.shine)
        );
        assert_eq!(at(u, 1 + (u.w - 1) / 2, 1 + (u.h - 1) / 2), Some(12));
    }

    #[test]
    fn layout_stays_inside_the_box() {
        for b in Button::all() {
            let r = b.rect();
            assert!(r.x >= 1 && r.x + r.w < WIDTH && r.y + r.h < HEIGHT, "{b:?}");
        }
        let s = RangeRequester::slider_rect();
        assert!(s.x + s.w <= geom::VALUE_RIGHT - 12);
    }
}
