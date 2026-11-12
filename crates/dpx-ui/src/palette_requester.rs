//! Color ▸ Palette ▸ Mixer On/Off (`p`): the Color Mixer requester (DeluxePaint IV AGA).
//!
//! A modal requester on the UI layer, centred, drawn in LoRes UI pixels with palette-derived
//! pens like the other requesters. It edits a copy of the palette: while it is open the caller
//! shows [`PaletteRequester::palette`] (so every change is visible on the whole screen) and on
//! Ok commits [`PaletteRequester::colors`] as one command. Cancel simply drops the copy.
//! The controls follow DeluxePaint IV (palette grid, sample box, RGB/HSV sliders, SPREAD, COPY,
//! EX, UNDO); the layout measurements are our own (ADR-16).

use dpx_core::{Chipset, IndexedImage, Palette, Rect, Rgb8};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{button_face, centred, draw_button, frame};

/// Requester size in LoRes UI pixels; fits the smallest (320×200) UI screen.
pub const WIDTH: i32 = 288;
pub const HEIGHT: i32 = 122;
/// Most undo steps kept inside the requester.
const MAX_UNDO: usize = 64;

mod geom {
    pub const TITLE_Y: i32 = 2;
    pub const TITLE_LINE_Y: i32 = 12;
    pub const GRID_X: i32 = 16;
    pub const GRID_Y: i32 = 16;
    pub const GRID_W: i32 = 256;
    pub const GRID_H: i32 = 32;
    pub const SAMPLE_X: i32 = 8;
    pub const SAMPLE_Y: i32 = 53;
    pub const SAMPLE_W: i32 = 40;
    pub const SAMPLE_H: i32 = 30;
    pub const SYSTEM_Y: i32 = 86;
    pub const LABEL_X: i32 = 56;
    pub const SLIDER_X: i32 = 66;
    pub const SLIDER_Y: i32 = 53;
    pub const SLIDER_W: i32 = 176;
    pub const SLIDER_H: i32 = 9;
    pub const SLIDER_STEP: i32 = 12;
    pub const KNOB_W: i32 = 10;
    pub const VALUE_RIGHT: i32 = 280;
    pub const INFO_Y: i32 = 90;
    pub const BUTTONS_Y: i32 = 104;
    pub const BUTTON_H: i32 = 13;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteResponse {
    Continue,
    /// Close and forget the changes.
    Cancel,
    /// Close and keep [`PaletteRequester::colors`].
    Ok,
}

/// An operation waiting for its second colour ("TO" in DeluxePaint).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Spread,
    Copy,
    Ex,
}

/// Colour system of the three sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum System {
    Rgb,
    Hsv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    System,
    Spread,
    Copy,
    Ex,
    Undo,
    Cancel,
    Ok,
}

impl Button {
    const ALL: [Button; 7] = [
        Button::System,
        Button::Spread,
        Button::Copy,
        Button::Ex,
        Button::Undo,
        Button::Cancel,
        Button::Ok,
    ];

    fn rect(self) -> Rect {
        let at = |x, w| Rect::new(x, geom::BUTTONS_Y, w, geom::BUTTON_H);
        match self {
            Button::System => Rect::new(
                geom::SAMPLE_X,
                geom::SYSTEM_Y,
                geom::SAMPLE_W,
                geom::BUTTON_H,
            ),
            Button::Spread => at(8, 54),
            Button::Copy => at(65, 40),
            Button::Ex => at(108, 26),
            Button::Undo => at(137, 40),
            Button::Cancel => at(186, 52),
            Button::Ok => at(242, 38),
        }
    }

    fn op(self) -> Option<Op> {
        match self {
            Button::Spread => Some(Op::Spread),
            Button::Copy => Some(Op::Copy),
            Button::Ex => Some(Op::Ex),
            _ => None,
        }
    }
}

/// What the left button holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Button(Button),
    /// A slider knob, grabbed `grab` pixels right of its left edge.
    Knob {
        channel: usize,
        grab: i32,
    },
    /// A slider track (clicked beside the knob: one step).
    Track,
}

/// State of the Color Mixer requester.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRequester {
    colors: Vec<Rgb8>,
    /// Highest slider level of an RGB channel: 15 on OCS (4 bits), 255 on AGA (8 bits).
    max_level: u8,
    selected: u8,
    system: System,
    /// HSV slider values of the selected colour (hue 0–360, saturation and value 0–100),
    /// kept while dragging so hue survives a grey or black colour.
    hsv: [u16; 3],
    pending: Option<Op>,
    undo: Vec<Vec<Rgb8>>,
    held: Option<Held>,
    pointer: (i32, i32),
}

/// A channel value (0–255) as a slider level.
fn to_level(v: u8, max_level: u8) -> i32 {
    if max_level == 15 {
        (i32::from(v) + 8) / 17
    } else {
        i32::from(v)
    }
}

/// A slider level as a stored channel value: OCS levels are stored as `level * 17`.
fn from_level(l: i32, max_level: u8) -> u8 {
    let l = l.clamp(0, i32::from(max_level)) as u8;
    if max_level == 15 { l * 17 } else { l }
}

/// A 0.0–1.0 channel quantised to the chipset's depth.
fn quantise(f: f32, max_level: u8) -> u8 {
    from_level(
        (f.clamp(0.0, 1.0) * f32::from(max_level)).round() as i32,
        max_level,
    )
}

fn channel(c: Rgb8, ch: usize) -> u8 {
    [c.r, c.g, c.b][ch]
}

fn set_channel(c: &mut Rgb8, ch: usize, v: u8) {
    match ch {
        0 => c.r = v,
        1 => c.g = v,
        _ => c.b = v,
    }
}

/// Hue in degrees (0–359), saturation and value in percent.
pub fn rgb_to_hsv(c: Rgb8) -> [u16; 3] {
    let (r, g, b) = (
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
    );
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max > 0.0 { d / max } else { 0.0 };
    [
        (h.round() as u16) % 360,
        (s * 100.0).round() as u16,
        (max * 100.0).round() as u16,
    ]
}

/// Hue in degrees, saturation and value in percent → channels 0.0–1.0.
fn hsv_to_rgb([h, s, v]: [u16; 3]) -> [f32; 3] {
    let h = f32::from(h % 360) / 60.0;
    let (s, v) = (f32::from(s.min(100)) / 100.0, f32::from(v.min(100)) / 100.0);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

impl PaletteRequester {
    /// Edits a copy of `palette` with `selected` (usually the FG colour) chosen. OCS
    /// documents get 4-bit sliders (0–15), AGA documents 8-bit sliders (0–255).
    pub fn new(palette: &Palette, selected: u8, chipset: Chipset) -> Self {
        let colors = palette.colors().to_vec();
        let last = (colors.len() - 1).min(255) as u8;
        let mut r = Self {
            colors,
            max_level: match chipset {
                Chipset::Ocs => 15,
                Chipset::Aga => 255,
            },
            selected: selected.min(last),
            system: System::Rgb,
            hsv: [0; 3],
            pending: None,
            undo: Vec::new(),
            held: None,
            pointer: (0, 0),
        };
        r.sync_hsv();
        r
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(screen_w: i32, screen_h: i32) -> (i32, i32) {
        (
            ((screen_w - WIDTH) / 2).max(0),
            ((screen_h - HEIGHT) / 2).max(0),
        )
    }

    /// The edited colours.
    pub fn colors(&self) -> &[Rgb8] {
        &self.colors
    }

    /// The edited palette, to show the screen in while the requester is open.
    pub fn palette(&self) -> Palette {
        Palette::new(self.colors.clone()).expect("same size as the opened palette")
    }

    /// The selected colour (shown in the sample box).
    pub fn selected(&self) -> u8 {
        self.selected
    }

    /// Track of slider `channel` (0–2: R, G, B or H, S, V) in requester coordinates.
    pub fn slider_rect(channel: usize) -> Rect {
        Rect::new(
            geom::SLIDER_X,
            geom::SLIDER_Y + channel as i32 * geom::SLIDER_STEP,
            geom::SLIDER_W,
            geom::SLIDER_H,
        )
    }

    /// (columns, rows, cell width, cell height) of the palette grid: one row for up to 16
    /// colours, 16 columns up to 64 colours, 32 columns (like DeluxePaint's Color Set rows)
    /// above.
    fn grid(&self) -> (i32, i32, i32, i32) {
        let n = self.colors.len() as i32;
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
        (col < cols && row < rows && (i as usize) < self.colors.len()).then_some(i as u8)
    }

    fn current(&self) -> Rgb8 {
        self.colors[usize::from(self.selected)]
    }

    fn sync_hsv(&mut self) {
        self.hsv = rgb_to_hsv(self.current());
    }

    fn max(&self, ch: usize) -> i32 {
        match self.system {
            System::Rgb => i32::from(self.max_level),
            System::Hsv => [360, 100, 100][ch],
        }
    }

    fn value(&self, ch: usize) -> i32 {
        match self.system {
            System::Rgb => to_level(channel(self.current(), ch), self.max_level),
            System::Hsv => i32::from(self.hsv[ch]),
        }
    }

    /// `c` as an edit writes it: on OCS the 12-bit colour the sliders show. An older
    /// project's palette may hold OCS colours that are not 12-bit (ADR-46, wave-8
    /// integration); an entry the user edits becomes 12-bit, the others stay as they are.
    fn written(&self, c: Rgb8) -> Rgb8 {
        if self.max_level == 15 {
            Rgb8::from_rgb4(c.to_rgb4())
        } else {
            c
        }
    }

    fn set_value(&mut self, ch: usize, v: i32) {
        let v = v.clamp(0, self.max(ch));
        let i = usize::from(self.selected);
        match self.system {
            System::Rgb => {
                let mut c = self.written(self.colors[i]);
                set_channel(&mut c, ch, from_level(v, self.max_level));
                self.colors[i] = c;
            }
            System::Hsv => {
                self.hsv[ch] = v as u16;
                let [r, g, b] = hsv_to_rgb(self.hsv);
                let m = self.max_level;
                self.colors[i] = Rgb8::new(quantise(r, m), quantise(g, m), quantise(b, m));
            }
        }
    }

    /// Pixels the knob can travel.
    fn free() -> i32 {
        geom::SLIDER_W - 2 - geom::KNOB_W
    }

    fn knob_x(&self, ch: usize) -> i32 {
        Self::slider_rect(ch).x + 1 + self.value(ch) * Self::free() / self.max(ch)
    }

    /// Moves knob `ch` so its left edge is at `left`.
    fn knob_to(&mut self, ch: usize, left: i32) {
        let free = Self::free();
        let rel = (left - Self::slider_rect(ch).x - 1).clamp(0, free);
        self.set_value(ch, (rel * self.max(ch) + free / 2) / free);
    }

    /// Remembers the colours for Undo before an edit.
    fn checkpoint(&mut self) {
        if self.undo.len() == MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(self.colors.clone());
    }

    /// Drops the last checkpoint if the edit changed nothing.
    fn end_edit(&mut self) {
        if self.undo.last() == Some(&self.colors) {
            self.undo.pop();
        }
    }

    /// Spread: an even gradient between entries `a` and `b` (both kept), in slider levels.
    fn spread(&mut self, a: usize, b: usize) {
        let (lo, hi) = (a.min(b), a.max(b));
        let n = (hi - lo) as i32;
        let (ca, cb, m) = (self.colors[lo], self.colors[hi], self.max_level);
        for k in 1..n {
            let mix = |x: u8, y: u8| {
                let (x, y) = (to_level(x, m), to_level(y, m));
                from_level((x * (n - k) + y * k + n / 2) / n, m)
            };
            let c = Rgb8::new(mix(ca.r, cb.r), mix(ca.g, cb.g), mix(ca.b, cb.b));
            self.colors[lo + k as usize] = c;
        }
    }

    /// A palette cell was clicked: selects it, or completes a waiting Spread/Copy/Ex.
    fn pick(&mut self, i: u8) {
        if let Some(op) = self.pending.take() {
            let (a, b) = (usize::from(self.selected), usize::from(i));
            self.checkpoint();
            match op {
                Op::Spread => self.spread(a, b),
                Op::Copy => self.colors[b] = self.written(self.colors[a]),
                Op::Ex => {
                    self.colors.swap(a, b);
                    // Both entries change: as every edit, 12-bit on OCS.
                    if a != b {
                        self.colors[a] = self.written(self.colors[a]);
                        self.colors[b] = self.written(self.colors[b]);
                    }
                }
            }
            self.end_edit();
        }
        self.selected = i;
        self.sync_hsv();
    }

    fn fire(&mut self, b: Button) -> PaletteResponse {
        if let Some(op) = b.op() {
            self.pending = (self.pending != Some(op)).then_some(op);
            return PaletteResponse::Continue;
        }
        match b {
            Button::Undo => {
                self.pending = None;
                if let Some(colors) = self.undo.pop() {
                    self.colors = colors;
                    self.sync_hsv();
                }
            }
            Button::System => {
                self.system = match self.system {
                    System::Rgb => System::Hsv,
                    System::Hsv => System::Rgb,
                };
                self.sync_hsv();
            }
            Button::Cancel => return PaletteResponse::Cancel,
            Button::Ok => return PaletteResponse::Ok,
            Button::Spread | Button::Copy | Button::Ex => {}
        }
        PaletteResponse::Continue
    }

    fn press(&mut self, x: i32, y: i32) {
        self.pointer = (x, y);
        if let Some(b) = Button::ALL.into_iter().find(|b| b.rect().contains(x, y)) {
            self.held = Some(Held::Button(b));
            return;
        }
        if let Some(i) = self.cell_at(x, y) {
            self.pick(i);
            return;
        }
        if let Some(ch) = (0..3).find(|&ch| Self::slider_rect(ch).contains(x, y)) {
            self.checkpoint();
            let kx = self.knob_x(ch);
            if (kx..kx + geom::KNOB_W).contains(&x) {
                self.held = Some(Held::Knob {
                    channel: ch,
                    grab: x - kx,
                });
            } else {
                let step = if x < kx { -1 } else { 1 };
                self.set_value(ch, self.value(ch) + step);
                self.held = Some(Held::Track);
            }
        }
    }

    fn pressed(&self, b: Button) -> bool {
        let held =
            self.held == Some(Held::Button(b)) && b.rect().contains(self.pointer.0, self.pointer.1);
        held || (b.op().is_some() && b.op() == self.pending)
    }

    /// Handles one event in requester coordinates (see [`PaletteRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> PaletteResponse {
        match event {
            ReqEvent::Press(x, y) => self.press(x, y),
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                if let Some(Held::Knob { channel, grab }) = self.held {
                    self.knob_to(channel, x - grab);
                }
            }
            ReqEvent::Release => match self.held.take() {
                Some(Held::Button(b)) if b.rect().contains(self.pointer.0, self.pointer.1) => {
                    return self.fire(b);
                }
                Some(Held::Knob { .. } | Held::Track) => self.end_edit(),
                _ => {}
            },
            // `p` again = Mixer Off, which keeps the changes in DeluxePaint.
            ReqEvent::Key(ReqKey::Enter | ReqKey::Char('p')) => return PaletteResponse::Ok,
            ReqEvent::Key(ReqKey::Escape) => {
                if self.pending.take().is_none() {
                    return PaletteResponse::Cancel;
                }
            }
            ReqEvent::Key(ReqKey::Char('u')) => return self.fire(Button::Undo),
            ReqEvent::Key(_) | ReqEvent::Scroll(_) => {}
        }
        PaletteResponse::Continue
    }

    /// Draws the requester onto a UI screen at [`PaletteRequester::origin`].
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = Self::origin(img.width() as i32, img.height() as i32);
        img.blit(&self.render(pens), x, y);
    }

    /// Renders the requester alone (WIDTH×HEIGHT). Palette cells and the sample box use the
    /// palette indices themselves, so they show whatever palette the screen is shown in.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let mut img = IndexedImage::new(WIDTH as u32, HEIGHT as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, WIDTH, HEIGHT), pens.dark);
        centred(&mut img, 0, WIDTH, geom::TITLE_Y, "Color Mixer", pens.dark);
        img.hline(1, WIDTH - 2, geom::TITLE_LINE_Y, pens.dark);

        // Palette grid: dark separator lines, the selected cell ringed shine/dark.
        let (cols, rows, cw, ch) = self.grid();
        img.fill_rect(
            geom::GRID_X,
            geom::GRID_Y,
            cols * cw + 1,
            rows * ch + 1,
            pens.dark,
        );
        for i in 0..self.colors.len() {
            let r = self.cell_rect(i as u8);
            img.fill_rect(r.x + 1, r.y + 1, r.w - 1, r.h - 1, i as u8);
        }
        let s = self.cell_rect(self.selected);
        frame(&mut img, Rect::new(s.x, s.y, s.w + 1, s.h + 1), pens.shine);
        if s.w > 3 && s.h > 3 {
            frame(
                &mut img,
                Rect::new(s.x + 1, s.y + 1, s.w - 1, s.h - 1),
                pens.dark,
            );
        }

        // Sample box.
        let b = Rect::new(
            geom::SAMPLE_X,
            geom::SAMPLE_Y,
            geom::SAMPLE_W,
            geom::SAMPLE_H,
        );
        recessed(&mut img, b, pens);
        img.fill_rect(b.x + 1, b.y + 1, b.w - 2, b.h - 2, self.selected);

        // Sliders.
        let labels = match self.system {
            System::Rgb => ["R", "G", "B"],
            System::Hsv => ["H", "S", "V"],
        };
        for (ch, label) in labels.iter().enumerate() {
            let t = Self::slider_rect(ch);
            font.draw(&mut img, geom::LABEL_X, t.y + 1, label, pens.dark);
            recessed(&mut img, t, pens);
            let knob = Rect::new(self.knob_x(ch), t.y + 1, geom::KNOB_W, t.h - 2);
            let grabbed = matches!(self.held, Some(Held::Knob { channel, .. }) if channel == ch);
            button_face(&mut img, knob, grabbed, pens);
            let text = self.value(ch).to_string();
            font.draw(
                &mut img,
                geom::VALUE_RIGHT - font.text_width(&text),
                t.y + 1,
                &text,
                pens.dark,
            );
        }
        let info = match self.pending {
            None => format!("Color {}", self.selected),
            Some(Op::Spread) => format!("Spread {} to?", self.selected),
            Some(Op::Copy) => format!("Copy {} to?", self.selected),
            Some(Op::Ex) => format!("Exchange {} with?", self.selected),
        };
        font.draw(&mut img, geom::LABEL_X, geom::INFO_Y, &info, pens.dark);

        for b in Button::ALL {
            let label = match b {
                Button::System => match self.system {
                    System::Rgb => "RGB",
                    System::Hsv => "HSV",
                },
                Button::Spread => "Spread",
                Button::Copy => "Copy",
                Button::Ex => "Ex",
                Button::Undo => "Undo",
                Button::Cancel => "Cancel",
                Button::Ok => "Ok",
            };
            draw_button(&mut img, b.rect(), label, self.pressed(b), pens);
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
    use crate::screen::UiState;

    fn aga() -> PaletteRequester {
        PaletteRequester::new(&Palette::dpaint_default_32(), 1, Chipset::Aga)
    }

    fn click(r: &mut PaletteRequester, rect: Rect) -> PaletteResponse {
        r.handle(ReqEvent::Press(rect.x + rect.w / 2, rect.y + rect.h / 2));
        r.handle(ReqEvent::Release)
    }

    fn cell(r: &mut PaletteRequester, i: u8) {
        let c = r.cell_rect(i);
        click(r, c);
    }

    fn button(r: &mut PaletteRequester, b: Button) -> PaletteResponse {
        click(r, b.rect())
    }

    /// Drags knob `ch` from wherever it is so its left edge goes to `left`.
    fn drag_knob(r: &mut PaletteRequester, ch: usize, left: i32) {
        let t = PaletteRequester::slider_rect(ch);
        let kx = r.knob_x(ch);
        r.handle(ReqEvent::Press(kx + 2, t.y + 4));
        r.handle(ReqEvent::Drag(left + 2, t.y + 4));
        r.handle(ReqEvent::Release);
    }

    #[test]
    fn clicking_a_cell_selects_it() {
        let mut r = aga();
        assert_eq!(r.selected(), 1);
        cell(&mut r, 6);
        assert_eq!(r.selected(), 6);
        cell(&mut r, 31);
        assert_eq!(r.selected(), 31);
        // Every cell maps back to its entry, for every palette size.
        for n in [2usize, 4, 8, 16, 32, 64, 128, 256] {
            let pal = Palette::new(vec![Rgb8::BLACK; n]).unwrap();
            let r = PaletteRequester::new(&pal, 0, Chipset::Aga);
            for i in 0..n {
                let c = r.cell_rect(i as u8);
                assert!(c.w >= 8 && c.h >= 4, "{n} colours: cell {c:?}");
                assert_eq!(r.cell_at(c.x + 1, c.y + 1), Some(i as u8), "{n}/{i}");
            }
        }
        // Selection is clamped to the palette.
        let pal = Palette::new(vec![Rgb8::BLACK; 4]).unwrap();
        assert_eq!(PaletteRequester::new(&pal, 9, Chipset::Aga).selected(), 3);
    }

    #[test]
    fn sliders_change_the_selected_colour_aga() {
        let mut r = aga();
        cell(&mut r, 0); // black
        // Dragging the red knob to the right end gives 255.
        let t = PaletteRequester::slider_rect(0);
        drag_knob(&mut r, 0, t.x + t.w + 40);
        assert_eq!(r.colors()[0], Rgb8::new(255, 0, 0));
        // Clicking left of the knob steps one level down; right of it one up.
        r.handle(ReqEvent::Press(t.x + 2, t.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.colors()[0].r, 254);
        let g = PaletteRequester::slider_rect(1);
        r.handle(ReqEvent::Press(g.x + g.w - 2, g.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.colors()[0], Rgb8::new(254, 1, 0));
        // Halfway is about half.
        drag_knob(&mut r, 2, t.x + 1 + PaletteRequester::free() / 2);
        assert!(
            (126..=129).contains(&r.colors()[0].b),
            "{:?}",
            r.colors()[0]
        );
        // Other entries are untouched.
        assert_eq!(
            &r.colors()[1..],
            &Palette::dpaint_default_32().colors()[1..]
        );
    }

    #[test]
    fn ocs_sliders_have_16_levels_stored_times_17() {
        let mut r = PaletteRequester::new(&Palette::dpaint_default_32(), 0, Chipset::Ocs);
        assert_eq!(r.max(0), 15);
        let t = PaletteRequester::slider_rect(1);
        r.handle(ReqEvent::Press(t.x + t.w - 2, t.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.colors()[0], Rgb8::new(0, 17, 0));
        drag_knob(&mut r, 1, t.x + t.w + 40);
        assert_eq!(r.colors()[0], Rgb8::new(0, 255, 0));
        drag_knob(&mut r, 1, t.x + 1 + PaletteRequester::free() / 3);
        assert_eq!(r.colors()[0].g % 17, 0);
        assert_eq!(r.value(1), 5);
    }

    #[test]
    fn hsv_sliders() {
        assert_eq!(rgb_to_hsv(Rgb8::new(255, 0, 0)), [0, 100, 100]);
        assert_eq!(rgb_to_hsv(Rgb8::new(0, 255, 0)), [120, 100, 100]);
        assert_eq!(rgb_to_hsv(Rgb8::new(0, 0, 255)), [240, 100, 100]);
        assert_eq!(rgb_to_hsv(Rgb8::new(0, 0, 0)), [0, 0, 0]);
        assert_eq!(rgb_to_hsv(Rgb8::new(255, 255, 255)), [0, 0, 100]);
        let mut r = aga();
        cell(&mut r, 2); // 0xE00 red
        button(&mut r, Button::System);
        assert_eq!(r.system, System::Hsv);
        assert_eq!(r.hsv, [0, 100, 93]);
        // Value to zero gives black; hue and saturation are kept, so value back gives red.
        let v = PaletteRequester::slider_rect(2);
        drag_knob(&mut r, 2, v.x - 40);
        assert_eq!(r.colors()[2], Rgb8::BLACK);
        drag_knob(&mut r, 2, v.x + v.w + 40);
        assert_eq!(r.colors()[2], Rgb8::new(255, 0, 0));
        // Hue to 120 degrees gives green.
        let h = PaletteRequester::slider_rect(0);
        drag_knob(&mut r, 0, h.x + 1 + PaletteRequester::free() / 3);
        assert_eq!(r.value(0), 119);
        // A click right of the knob adds one degree.
        r.handle(ReqEvent::Press(h.x + h.w - 2, h.y + 4));
        r.handle(ReqEvent::Release);
        assert_eq!(r.value(0), 120);
        assert_eq!(r.colors()[2], Rgb8::new(0, 255, 0));
        // Back to RGB.
        button(&mut r, Button::System);
        assert_eq!(r.value(1), 255);
        // OCS: HSV results are quantised to 4 bits.
        let mut r = PaletteRequester::new(&Palette::dpaint_default_32(), 2, Chipset::Ocs);
        button(&mut r, Button::System);
        drag_knob(&mut r, 2, v.x + 1 + PaletteRequester::free() / 2);
        let c = r.colors()[2];
        assert!(
            [c.r, c.g, c.b].iter().all(|v| v.is_multiple_of(17)),
            "{c:?}"
        );
    }

    #[test]
    fn spread_makes_an_even_gradient() {
        let mut colors = vec![Rgb8::new(9, 9, 9); 32];
        colors[0] = Rgb8::BLACK;
        colors[15] = Rgb8::WHITE;
        let pal = Palette::new(colors).unwrap();
        let mut r = PaletteRequester::new(&pal, 0, Chipset::Aga);
        button(&mut r, Button::Spread);
        assert_eq!(r.pending, Some(Op::Spread));
        cell(&mut r, 15);
        assert_eq!(r.pending, None);
        for k in 0..16 {
            let v = 17 * k as u8;
            assert_eq!(r.colors()[k], Rgb8::new(v, v, v), "entry {k}");
        }
        assert_eq!(r.colors()[16], Rgb8::new(9, 9, 9));
        assert_eq!(r.selected(), 15);
        // Backwards works too, and OCS rounds to 4-bit levels.
        let mut colors = vec![Rgb8::BLACK; 8];
        colors[7] = Rgb8::new(0, 0, 255);
        let pal = Palette::new(colors).unwrap();
        let mut r = PaletteRequester::new(&pal, 7, Chipset::Ocs);
        button(&mut r, Button::Spread);
        cell(&mut r, 0);
        // Levels 15 * k / 7 rounded: 0 2 4 6 9 11 13 15.
        let b: Vec<u8> = r.colors().iter().map(|c| c.b / 17).collect();
        assert_eq!(b, [0, 2, 4, 6, 9, 11, 13, 15]);
    }

    #[test]
    fn copy_and_exchange() {
        let def = Palette::dpaint_default_32();
        let mut r = aga();
        cell(&mut r, 2);
        button(&mut r, Button::Copy);
        cell(&mut r, 10);
        assert_eq!(r.colors()[10], def.colors()[2]);
        assert_eq!(r.colors()[2], def.colors()[2]);
        assert_eq!(r.selected(), 10);

        let mut r = aga();
        cell(&mut r, 4);
        button(&mut r, Button::Ex);
        cell(&mut r, 5);
        assert_eq!(r.colors()[4], def.colors()[5]);
        assert_eq!(r.colors()[5], def.colors()[4]);
        // Pressing the button again withdraws it; Esc too.
        button(&mut r, Button::Copy);
        button(&mut r, Button::Copy);
        assert_eq!(r.pending, None);
        button(&mut r, Button::Copy);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            PaletteResponse::Continue
        );
        assert_eq!(r.pending, None);
    }

    /// ADR-46: every edit on an OCS document keeps the palette 12-bit, so Ok's SetPalette is
    /// never refused by the engine's OCS rule.
    #[test]
    fn ocs_edits_keep_every_entry_12_bit() {
        let twelve = |r: &PaletteRequester| {
            r.colors()
                .iter()
                .all(|c| Rgb8::from_rgb4(c.to_rgb4()) == *c)
        };
        let mut r = PaletteRequester::new(&Palette::dpaint_default_32(), 3, Chipset::Ocs);
        assert!(twelve(&r));
        // Spread between two odd levels, then Copy and Exchange, then RGB and HSV sliders.
        button(&mut r, Button::Spread);
        cell(&mut r, 17);
        cell(&mut r, 6);
        button(&mut r, Button::Copy);
        cell(&mut r, 29);
        button(&mut r, Button::Ex);
        cell(&mut r, 1);
        for ch in 0..3 {
            let t = PaletteRequester::slider_rect(ch);
            drag_knob(
                &mut r,
                ch,
                t.x + 1 + PaletteRequester::free() * (ch as i32 + 1) / 7,
            );
        }
        button(&mut r, Button::System);
        for ch in 0..3 {
            let t = PaletteRequester::slider_rect(ch);
            drag_knob(
                &mut r,
                ch,
                t.x + 1 + PaletteRequester::free() * (ch as i32 + 2) / 9,
            );
        }
        assert!(twelve(&r), "{:?}", r.colors());
        assert_ne!(r.colors(), Palette::dpaint_default_32().colors());
    }

    /// Wave-8 integration of ADR-46: in an older project's OCS palette with colours that are
    /// not 12-bit, an edit makes the entries it writes 12-bit and leaves every other entry as
    /// stored, so Ok's SetPalette passes the engine's rule (only changed entries must be
    /// 12-bit) and nothing else changes.
    #[test]
    fn ocs_edits_write_12_bit_entries_and_keep_the_others() {
        let old: Vec<Rgb8> = (0..8u8)
            .map(|i| Rgb8::new(i * 11, i * 11, i * 11))
            .collect();
        let twelve = |c: Rgb8| Rgb8::from_rgb4(c.to_rgb4()) == c;
        let fresh = || PaletteRequester::new(&Palette::new(old.clone()).unwrap(), 2, Chipset::Ocs);
        // A slider: the whole entry becomes 12-bit, the others stay.
        let mut r = fresh();
        let t = PaletteRequester::slider_rect(0);
        drag_knob(&mut r, 0, t.x + t.w + 40);
        let mut want = Rgb8::from_rgb4(old[2].to_rgb4());
        want.r = 255;
        assert_eq!(r.colors()[2], want);
        assert!(twelve(r.colors()[2]) && !twelve(old[2]));
        for i in (0..8).filter(|&i| i != 2) {
            assert_eq!(r.colors()[i], old[i], "entry {i} untouched");
        }
        // Copy writes the 12-bit colour; Exchange both entries.
        let mut r = fresh();
        button(&mut r, Button::Copy);
        cell(&mut r, 5);
        assert_eq!(r.colors()[5], Rgb8::from_rgb4(old[2].to_rgb4()));
        assert_eq!(r.colors()[2], old[2], "the source stays");
        let mut r = fresh();
        button(&mut r, Button::Ex);
        cell(&mut r, 6);
        assert_eq!(r.colors()[2], Rgb8::from_rgb4(old[6].to_rgb4()));
        assert_eq!(r.colors()[6], Rgb8::from_rgb4(old[2].to_rgb4()));
        assert_eq!(r.colors()[3], old[3]);
        // Spread keeps its two ends and writes 12-bit colours between them.
        let mut r = fresh();
        button(&mut r, Button::Spread);
        cell(&mut r, 7);
        assert_eq!((r.colors()[2], r.colors()[7]), (old[2], old[7]));
        assert!((3..7).all(|i| twelve(r.colors()[i])));
        // AGA writes colours as they are.
        let mut r = PaletteRequester::new(&Palette::new(old.clone()).unwrap(), 2, Chipset::Aga);
        button(&mut r, Button::Copy);
        cell(&mut r, 5);
        assert_eq!(r.colors()[5], old[2]);
    }

    #[test]
    fn undo_reverts_the_last_edit() {
        let def = Palette::dpaint_default_32();
        let mut r = aga();
        cell(&mut r, 0);
        // A whole drag is one step.
        let t = PaletteRequester::slider_rect(0);
        let kx = r.knob_x(0);
        r.handle(ReqEvent::Press(kx + 2, t.y + 4));
        for x in (kx..t.x + t.w).step_by(7) {
            r.handle(ReqEvent::Drag(x, t.y + 4));
        }
        r.handle(ReqEvent::Release);
        assert_ne!(r.colors()[0], Rgb8::BLACK);
        button(&mut r, Button::Copy);
        cell(&mut r, 3);
        assert_eq!(r.undo.len(), 2);
        button(&mut r, Button::Undo);
        assert_eq!(r.colors()[3], def.colors()[3]);
        assert_ne!(r.colors()[0], Rgb8::BLACK);
        r.handle(ReqEvent::Key(ReqKey::Char('u')));
        assert_eq!(r.colors(), def.colors());
        // Nothing more to undo.
        button(&mut r, Button::Undo);
        assert_eq!(r.colors(), def.colors());
        // A press on a knob that does not move it leaves no undo step.
        let kx = r.knob_x(1);
        r.handle(ReqEvent::Press(kx + 2, t.y + 16));
        r.handle(ReqEvent::Release);
        assert!(r.undo.is_empty());
    }

    #[test]
    fn ok_cancel_and_keys() {
        let mut r = aga();
        assert_eq!(button(&mut r, Button::Ok), PaletteResponse::Ok);
        assert_eq!(button(&mut r, Button::Cancel), PaletteResponse::Cancel);
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Enter)), PaletteResponse::Ok);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Char('p'))),
            PaletteResponse::Ok
        );
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            PaletteResponse::Cancel
        );
        // Released away from the button: nothing.
        let b = Button::Ok.rect();
        r.handle(ReqEvent::Press(b.x + 2, b.y + 2));
        r.handle(ReqEvent::Drag(0, 0));
        assert_eq!(r.handle(ReqEvent::Release), PaletteResponse::Continue);
        assert_eq!(r.handle(ReqEvent::Scroll(1)), PaletteResponse::Continue);
        // Cancel never changes the colours it hands back: the caller just drops them.
        let mut r = aga();
        cell(&mut r, 0);
        drag_knob(&mut r, 0, 400);
        assert_ne!(r.colors(), Palette::dpaint_default_32().colors());
        assert_eq!(button(&mut r, Button::Cancel), PaletteResponse::Cancel);
    }

    #[test]
    fn render_size_layout_and_pixels() {
        let p = Pens::from_palette(&Palette::dpaint_default_32());
        let r = aga();
        let img = r.render(&p);
        assert_eq!((img.width() as i32, img.height() as i32), (WIDTH, HEIGHT));
        const { assert!(WIDTH <= 320 && HEIGHT <= 200) };
        assert_eq!(PaletteRequester::origin(320, 200), (16, 39));
        assert_eq!(PaletteRequester::origin(100, 50), (0, 0));
        // Cells show their own palette index; the sample box the selected one.
        let c = r.cell_rect(6);
        assert_eq!(img.get(c.x + c.w / 2, c.y + c.h / 2), Some(6));
        assert_eq!(img.get(geom::SAMPLE_X + 10, geom::SAMPLE_Y + 10), Some(1));
        // Everything lies inside the requester and nothing overlaps.
        let mut rects: Vec<Rect> = Button::ALL.iter().map(|b| b.rect()).collect();
        rects.extend((0..3).map(PaletteRequester::slider_rect));
        rects.push(Rect::new(
            geom::GRID_X,
            geom::GRID_Y,
            geom::GRID_W + 1,
            geom::GRID_H + 1,
        ));
        for (i, a) in rects.iter().enumerate() {
            assert!(
                a.x >= 1 && a.y > geom::TITLE_LINE_Y && a.x + a.w < WIDTH && a.y + a.h < HEIGHT
            );
            for b in &rects[i + 1..] {
                assert!(
                    a.intersect(*b).w <= 0 || a.intersect(*b).h <= 0,
                    "{a:?} {b:?}"
                );
            }
        }
        // Labels fit their buttons.
        let font = Font::builtin();
        for (b, l) in [
            (Button::Spread, "Spread"),
            (Button::Cancel, "Cancel"),
            (Button::Undo, "Undo"),
            (Button::Copy, "Copy"),
        ] {
            assert!(font.text_width(l) + 4 <= b.rect().w, "{l}");
        }
        assert!(geom::LABEL_X + font.text_width("Exchange 255 with?") <= WIDTH - 4);
        // A pending operation shows its button pressed.
        let mut r = aga();
        button(&mut r, Button::Spread);
        assert!(r.pressed(Button::Spread) && !r.pressed(Button::Copy));
        assert_ne!(r.render(&p), aga().render(&p));
    }

    #[test]
    fn ui_state_has_no_palette_requester_by_default() {
        assert!(UiState::default().palette_requester.is_none());
    }
}
