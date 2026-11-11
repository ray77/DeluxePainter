//! The Animation Control Panel (Anim ▸ Control ▸ Panel On/Off, Alt-a; ADR-38): a bar at the
//! bottom of the screen, left of the toolbox, with the frame slider between First Frame and
//! Last Frame buttons, ten control buttons and the five Light Table toggles (AGA p.257,
//! figure p.257; tutorial p.148-149, figure p.159). Drawn in UI (LoRes) pixels with square
//! corners and palette-derived pens; the icons are drawn for this project.
//!
//! The panel only shows state and reports what was hit; the app runs the frame commands.

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::screen::geom::TOOLBOX_W;
use crate::widgets::button_face;

/// Panel geometry (UI pixels).
pub mod geom {
    /// Whole panel: a dark top line, the slider row and the button row.
    pub const PANEL_H: i32 = 21;
    pub const SLIDER_H: i32 = 9;
    pub const BUTTON_H: i32 = 11;
    /// First Frame and Last Frame buttons at the ends of the slider.
    pub const END_W: i32 = 12;
    /// The ten control buttons.
    pub const CONTROL_W: i32 = 19;
    /// The Light Table button and its four image toggles.
    pub const TOGGLE_W: i32 = 16;
    /// Narrowest slider knob.
    pub const KNOB_MIN: i32 = 6;
}

/// A button of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelButton {
    /// Slider end buttons.
    First,
    Last,
    LoopBackward,
    OnceBackward,
    Previous,
    Next,
    OnceForward,
    LoopForward,
    PingPong,
    Add,
    Delete,
    /// Repeat the last Go To (Ctrl-click: the Go to Frame requester).
    GoTo,
    LightTable,
    TwoBack,
    LightPrevious,
    LightNext,
    Spare,
}

/// The control buttons left to right, as in the manual's figure (p.257).
pub const CONTROLS: [PanelButton; 10] = [
    PanelButton::LoopBackward,
    PanelButton::OnceBackward,
    PanelButton::Previous,
    PanelButton::Next,
    PanelButton::OnceForward,
    PanelButton::LoopForward,
    PanelButton::PingPong,
    PanelButton::Add,
    PanelButton::Delete,
    PanelButton::GoTo,
];

/// The Light Table group at the right end.
pub const TOGGLES: [PanelButton; 5] = [
    PanelButton::LightTable,
    PanelButton::TwoBack,
    PanelButton::LightPrevious,
    PanelButton::LightNext,
    PanelButton::Spare,
];

/// What lies under a panel position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelHit {
    Button(PanelButton),
    /// The slider track left (`-1`) or right (`1`) of the knob: one frame that way.
    Step(i32),
    /// The knob, grabbed this many pixels right of its left edge.
    Knob(i32),
    /// The panel elsewhere (gaps): nothing happens.
    Background,
}

/// Where the panel's parts lie on a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub panel: Rect,
    pub first: Rect,
    pub last: Rect,
    pub track: Rect,
    /// Top of the button row.
    pub buttons_y: i32,
}

impl Layout {
    /// The panel on a `screen_w`×`screen_h` UI screen: the bottom rows, left of the toolbox
    /// when it is shown (full width when it is hidden).
    pub fn new(screen_w: i32, screen_h: i32, toolbox: bool) -> Self {
        let w = if toolbox {
            screen_w - TOOLBOX_W
        } else {
            screen_w
        };
        let y = screen_h - geom::PANEL_H;
        let row = y + 1;
        Self {
            panel: Rect::new(0, y, w, geom::PANEL_H),
            first: Rect::new(0, row, geom::END_W, geom::SLIDER_H),
            last: Rect::new(w - geom::END_W, row, geom::END_W, geom::SLIDER_H),
            track: Rect::new(geom::END_W, row, w - 2 * geom::END_W, geom::SLIDER_H),
            buttons_y: row + geom::SLIDER_H,
        }
    }

    /// The rectangle of `button`.
    pub fn button(&self, button: PanelButton) -> Rect {
        let y = self.buttons_y;
        if button == PanelButton::First {
            return self.first;
        }
        if button == PanelButton::Last {
            return self.last;
        }
        if let Some(i) = CONTROLS.iter().position(|&b| b == button) {
            return Rect::new(
                i as i32 * geom::CONTROL_W,
                y,
                geom::CONTROL_W,
                geom::BUTTON_H,
            );
        }
        let i = TOGGLES.iter().position(|&b| b == button).unwrap_or(0) as i32;
        let x0 = self.panel.w - TOGGLES.len() as i32 * geom::TOGGLE_W;
        Rect::new(x0 + i * geom::TOGGLE_W, y, geom::TOGGLE_W, geom::BUTTON_H)
    }
}

/// What the panel shows: the frame position and the Light Table toggles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimPanel {
    /// The frame shown (from 0) and the number of frames.
    pub frame: usize,
    pub count: usize,
    /// Effect ▸ LightTable on, and the images it shows.
    pub light_on: bool,
    pub two_back: bool,
    pub previous: bool,
    pub next: bool,
    pub spare: bool,
    /// The button held down, drawn pressed.
    pub held: Option<PanelButton>,
}

/// Icons, 9×7 (`#` = ink), centred on their button.
const LOOP_BACKWARD: [&str; 7] = [
    "...#.....",
    "..######.",
    "...#....#",
    "#.......#",
    "#....#...",
    ".######..",
    ".....#...",
];
const ONCE_BACKWARD: [&str; 7] = [
    "....#....",
    "...##....",
    "..###....",
    ".####....",
    "..###....",
    "...##....",
    "....#....",
];
const PREVIOUS: [&str; 7] = [
    "...#..#..",
    "..##..#..",
    ".###..#..",
    "####..#..",
    ".###..#..",
    "..##..#..",
    "...#..#..",
];
const PING_PONG: [&str; 7] = [
    "...#.#...",
    "..##.##..",
    ".###.###.",
    "####.####",
    ".###.###.",
    "..##.##..",
    "...#.#...",
];
const ADD: [&str; 7] = [
    "....#....",
    "....#....",
    "....#....",
    ".#######.",
    "....#....",
    "....#....",
    "....#....",
];
const DELETE: [&str; 7] = [
    ".........",
    ".........",
    ".........",
    ".#######.",
    ".........",
    ".........",
    ".........",
];
/// Go To (the panel) and Come To (the Move requester): an arrow into a dot.
pub(crate) const GO_TO: [&str; 7] = [
    ".........",
    "..#...##.",
    "...#.####",
    "#########",
    "...#.####",
    "..#...##.",
    ".........",
];
const BULB: [&str; 7] = [
    "..###..", ".#...#.", "#.....#", "#.....#", ".#...#.", "..###..", "..###..",
];
/// First / Last Frame: a bar and a triangle, 6×7.
const FIRST: [&str; 7] = [
    "#...#.", "#..##.", "#.###.", "#####.", "#.###.", "#..##.", "#...#.",
];

/// The pixels of an icon, mirrored left to right when `mirror`.
fn icon_points(rows: &[&str], mirror: bool) -> (i32, i32, Vec<(i32, i32)>) {
    let w = rows.iter().map(|r| r.len()).max().unwrap_or(0) as i32;
    let mut pts = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        for (x, c) in row.chars().enumerate() {
            if c == '#' {
                let x = x as i32;
                pts.push((if mirror { w - 1 - x } else { x }, y as i32));
            }
        }
    }
    (w, rows.len() as i32, pts)
}

/// Draws icon `rows` centred on `r` in `pen`, mirrored left to right when `mirror`.
pub(crate) fn draw_icon(img: &mut IndexedImage, r: Rect, rows: &[&str], mirror: bool, pen: u8) {
    let (w, h, pts) = icon_points(rows, mirror);
    let (x0, y0) = (r.x + (r.w - w) / 2, r.y + (r.h - h) / 2);
    for (x, y) in pts {
        img.set(x0 + x, y0 + y, pen);
    }
}

impl AnimPanel {
    /// Whether a Light Table toggle is on.
    fn on(&self, button: PanelButton) -> bool {
        match button {
            PanelButton::LightTable => self.light_on,
            PanelButton::TwoBack => self.two_back,
            PanelButton::LightPrevious => self.previous,
            PanelButton::LightNext => self.next,
            PanelButton::Spare => self.spare,
            _ => false,
        }
    }

    /// The knob: its rectangle on the track.
    pub fn knob(&self, layout: &Layout) -> Rect {
        let t = layout.track;
        let count = self.count.max(1) as i32;
        let kw = (t.w / count).clamp(geom::KNOB_MIN.min(t.w), t.w.max(1));
        let room = t.w - kw;
        let x = if count > 1 {
            room * self.frame.min(self.count - 1) as i32 / (count - 1)
        } else {
            0
        };
        Rect::new(t.x + x, t.y + 1, kw, t.h - 2)
    }

    /// The frame (from 0) the knob shows when dragged with its left edge `grab` pixels left
    /// of `x`.
    pub fn frame_for_knob(&self, layout: &Layout, x: i32, grab: i32) -> usize {
        let t = layout.track;
        let kw = self.knob(layout).w;
        let room = (t.w - kw).max(1);
        let last = self.count.max(1) - 1;
        let off = (x - grab - t.x).clamp(0, room);
        // Nearest frame.
        ((i64::from(off) * last as i64 * 2 + i64::from(room)) / (2 * i64::from(room))) as usize
    }

    /// What lies at UI pixel (`x`, `y`), if it is on the panel.
    pub fn hit(&self, layout: &Layout, x: i32, y: i32) -> Option<PanelHit> {
        if !layout.panel.contains(x, y) {
            return None;
        }
        if layout.first.contains(x, y) {
            return Some(PanelHit::Button(PanelButton::First));
        }
        if layout.last.contains(x, y) {
            return Some(PanelHit::Button(PanelButton::Last));
        }
        if layout.track.contains(x, y) {
            let k = self.knob(layout);
            return Some(if x < k.x {
                PanelHit::Step(-1)
            } else if x >= k.x + k.w {
                PanelHit::Step(1)
            } else {
                PanelHit::Knob(x - k.x)
            });
        }
        let button = CONTROLS
            .iter()
            .chain(&TOGGLES)
            .copied()
            .find(|&b| layout.button(b).contains(x, y));
        Some(button.map_or(PanelHit::Background, PanelHit::Button))
    }

    /// Draws the panel at the bottom of a UI screen.
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens, layout: &Layout) {
        let p = layout.panel;
        img.fill_rect(p.x, p.y, p.w, p.h, pens.face);
        img.hline(p.x, p.x + p.w - 1, p.y, pens.dark);
        // Slider: dark track, raised knob, end buttons.
        let t = layout.track;
        img.fill_rect(t.x, t.y, t.w, t.h, pens.dark);
        let k = self.knob(layout);
        button_face(img, k, false, pens);
        for (button, mirror) in [(PanelButton::First, false), (PanelButton::Last, true)] {
            let r = layout.button(button);
            let ink = button_face(img, r, self.held == Some(button), pens);
            draw_icon(img, r, &FIRST, mirror, ink);
        }
        // Control buttons.
        for &button in &CONTROLS {
            let r = layout.button(button);
            let ink = button_face(img, r, self.held == Some(button), pens);
            let (rows, mirror): (&[&str], bool) = match button {
                PanelButton::LoopBackward => (&LOOP_BACKWARD, false),
                PanelButton::OnceBackward => (&ONCE_BACKWARD, false),
                PanelButton::Previous => (&PREVIOUS, false),
                PanelButton::Next => (&PREVIOUS, true),
                PanelButton::OnceForward => (&ONCE_BACKWARD, true),
                PanelButton::LoopForward => (&LOOP_BACKWARD, true),
                PanelButton::PingPong => (&PING_PONG, false),
                PanelButton::Add => (&ADD, false),
                PanelButton::Delete => (&DELETE, false),
                _ => (&GO_TO, false),
            };
            draw_icon(img, r, rows, mirror, ink);
        }
        // Light Table toggles: pressed while on.
        let font = Font::builtin();
        for &button in &TOGGLES {
            let r = layout.button(button);
            let pressed = self.on(button) || self.held == Some(button);
            let ink = button_face(img, r, pressed, pens);
            let label = match button {
                PanelButton::TwoBack => "2",
                PanelButton::LightPrevious => "P",
                PanelButton::LightNext => "N",
                PanelButton::Spare => "S",
                _ => {
                    draw_icon(img, r, &BULB, false, ink);
                    continue;
                }
            };
            let tw = font.text_width(label);
            font.draw(img, r.x + (r.w - tw) / 2, r.y + 2, label, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    #[test]
    fn layout_sits_at_the_bottom_left_of_the_toolbox() {
        let l = Layout::new(320, 256, true);
        assert_eq!(l.panel, Rect::new(0, 235, 296, 21));
        assert_eq!(l.first, Rect::new(0, 236, 12, 9));
        assert_eq!(l.last, Rect::new(284, 236, 12, 9));
        assert_eq!(
            l.button(PanelButton::LoopBackward),
            Rect::new(0, 245, 19, 11)
        );
        assert_eq!(l.button(PanelButton::GoTo), Rect::new(171, 245, 19, 11));
        assert_eq!(l.button(PanelButton::Spare), Rect::new(280, 245, 16, 11));
        assert_eq!(l.button(PanelButton::LightTable).x, 216);
        // Without the toolbox it is as wide as the screen.
        assert_eq!(
            Layout::new(320, 200, false).panel,
            Rect::new(0, 179, 320, 21)
        );
    }

    #[test]
    fn every_button_is_hit_where_it_is_drawn() {
        let l = Layout::new(320, 256, true);
        let panel = AnimPanel {
            count: 10,
            ..AnimPanel::default()
        };
        for &b in CONTROLS
            .iter()
            .chain(&TOGGLES)
            .chain(&[PanelButton::First, PanelButton::Last])
        {
            let r = l.button(b);
            assert_eq!(
                panel.hit(&l, r.x + r.w / 2, r.y + r.h / 2),
                Some(PanelHit::Button(b))
            );
        }
        assert_eq!(
            panel.hit(&l, 200, 250),
            Some(PanelHit::Background),
            "the gap"
        );
        assert_eq!(panel.hit(&l, 100, 100), None, "off the panel");
        assert_eq!(panel.hit(&l, 300, 250), None, "the toolbox");
    }

    #[test]
    fn the_knob_follows_the_frame_and_drags_to_frames() {
        let l = Layout::new(320, 256, true);
        let mut panel = AnimPanel {
            count: 10,
            ..AnimPanel::default()
        };
        let k = panel.knob(&l);
        assert_eq!((k.x, k.w), (l.track.x, 27));
        assert_eq!(panel.hit(&l, k.x + 3, k.y + 2), Some(PanelHit::Knob(3)));
        assert_eq!(panel.hit(&l, k.x + k.w + 5, k.y), Some(PanelHit::Step(1)));
        panel.frame = 9;
        let k = panel.knob(&l);
        assert_eq!(
            k.x + k.w,
            l.track.x + l.track.w,
            "last frame at the right end"
        );
        assert_eq!(panel.hit(&l, k.x - 1, k.y), Some(PanelHit::Step(-1)));
        // Dragging: the left end is frame 0, the right end the last frame, and the knob's
        // own position for a frame drags to that frame.
        assert_eq!(panel.frame_for_knob(&l, l.track.x + 3, 3), 0);
        assert_eq!(panel.frame_for_knob(&l, 400, 3), 9);
        for f in 0..10 {
            panel.frame = f;
            let k = panel.knob(&l);
            assert_eq!(panel.frame_for_knob(&l, k.x + 2, 2), f);
        }
        // One frame: the knob fills the track.
        let one = AnimPanel {
            count: 1,
            ..AnimPanel::default()
        };
        assert_eq!(one.knob(&l).w, l.track.w);
        assert_eq!(one.frame_for_knob(&l, 200, 0), 0);
    }

    #[test]
    fn toggles_show_their_state_and_held_buttons_press() {
        let p = pens();
        let l = Layout::new(320, 256, true);
        let draw = |panel: &AnimPanel| {
            let mut img = IndexedImage::new(320, 256, 0);
            panel.draw(&mut img, &p, &l);
            img
        };
        let off = draw(&AnimPanel::default());
        let r = l.button(PanelButton::LightNext);
        assert_eq!(off.get(r.x + 1, r.y + 1), Some(p.face));
        let on = draw(&AnimPanel {
            next: true,
            ..AnimPanel::default()
        });
        assert_eq!(on.get(r.x + 1, r.y + 1), Some(p.select), "pressed while on");
        let held = draw(&AnimPanel {
            held: Some(PanelButton::Add),
            ..AnimPanel::default()
        });
        let a = l.button(PanelButton::Add);
        assert_eq!(held.get(a.x + 1, a.y + 1), Some(p.select));
        // Square corners: the panel's top line is dark to the very edge, nothing above it.
        assert_eq!(off.get(0, l.panel.y), Some(p.dark));
        assert_eq!(off.get(0, l.panel.y - 1), Some(0));
        // Every control button has an icon.
        for &b in &CONTROLS {
            let r = l.button(b);
            let inked = (r.y + 2..r.y + r.h - 2)
                .flat_map(|y| (r.x + 2..r.x + r.w - 2).map(move |x| (x, y)))
                .any(|(x, y)| off.get(x, y) == Some(p.dark));
            assert!(inked, "{b:?}");
        }
    }
}
