//! Prefs ▸ AI & MCP.. (ADR-43, part 3): the requester that turns the MCP server on and off,
//! shows its status, connects Claude Desktop and Claude Code, sets the AI clients'
//! permissions, shows the image generator's state with a "Set up.." button for it (ADR-45)
//! and lists their last calls.
//!
//! It only draws what the app gives it ([`McpView`], refreshed every pass) and reports which
//! control was used ([`McpResponse::Pick`]); the app changes its settings and runs the
//! connect flows. Drawn in UI (LoRes) pixels with palette-derived pens and square corners,
//! like the other requesters; Return and Esc close it. It is a DeluxePainter addition in
//! DeluxePaint's look (DeluxePaint IV has no such requester).
//!
//! The layout fits the smallest UI screen (320×200: three activity lines) and shows up to
//! five activity lines on taller screens; it is 288 to 360 pixels wide.

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::form::draw_box;
use crate::note_requester::wrap;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{Inset, centred, checker, draw_button, draw_clipped, frame};

/// Widest requester (UI pixels); smaller screens get a narrower one.
const MAX_W: i32 = 360;
/// Narrowest requester: the buttons and labels need this much.
const MIN_W: i32 = 288;
/// Activity lines shown at most and at least.
pub const MAX_ACTIVITY: usize = 5;
const MIN_ACTIVITY: usize = 2;
const TITLE: &str = "AI & MCP";
const LEFT: i32 = 6;
const BOX: i32 = 7;
const ROW_H: i32 = 9;
const BUTTON_H: i32 = 12;
const PAD: i32 = 12;
const GAP: i32 = 4;

mod geom {
    pub const CONTROL_Y: i32 = 15;
    pub const STATUS_Y: i32 = 25;
    pub const CONNECT_Y: i32 = 45;
    pub const MANAGE_Y: i32 = 59;
    pub const MESSAGE_Y: i32 = 72;
    pub const RULE_1: i32 = 82;
    pub const PERMISSION_Y: i32 = 84;
    pub const FOLDER_Y: i32 = 112;
    /// The image generator's two lines (ADR-45): "Image generation:", then its state; the
    /// "Set up.." button right of them is centred on both.
    pub const GENERATION_Y: i32 = 121;
    pub const GENERATION_STATE_Y: i32 = 130;
    pub const SET_UP_Y: i32 = 124;
    pub const RULE_2: i32 = 140;
    pub const ACTIVITY_Y: i32 = 143;
    /// Above the activity lines, below the Close button.
    pub const FIXED_H: i32 = 171;
}

const CONTROL_LABEL: &str = "MCP control:";
const CONNECT_LABEL: &str = "Connect:";
const ACTIVITY_LABEL: &str = "Last calls:";
const GENERATION_LABEL: &str = "Image generation:";

/// What the requester shows: the app's MCP state, made again every pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpView {
    /// MCP control is on (the setting, or `--mcp` for this session).
    pub on: bool,
    /// The status after "Status: ", e.g. "Ready on 127.0.0.1:8765" (wrapped to two lines).
    pub status: String,
    /// The three permissions.
    pub read: bool,
    pub edit: bool,
    pub overwrite: bool,
    /// The approved folder as shown, e.g. "~/Pictures/DeluxePainter".
    pub folder: String,
    /// The last calls, oldest first ("12s draw_rectangle 10,20 20x10 colour 5").
    pub activity: Vec<String>,
    /// What the last connect action did, or what it is doing.
    pub message: Option<String>,
    /// The image generator's state (ADR-45), e.g. "OpenAI, key set", shown under
    /// "Image generation:" with the "Set up.." button right of both. Never the key.
    pub generation: String,
    /// A connect action runs: the connect buttons are ghosted until it ends.
    pub busy: bool,
}

/// The requester's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpControl {
    On,
    Off,
    ConnectDesktop,
    ConnectCode,
    Disconnect,
    Revoke,
    Read,
    Edit,
    Overwrite,
    /// The image generator's "Set up.." (ADR-45): the app opens the Image Generator setup
    /// over this requester.
    SetUpGenerator,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpResponse {
    Continue,
    /// A control was used (radios and check boxes on press, buttons on release; Return and
    /// Esc pick Close). The app acts and gives the requester a new view.
    Pick(McpControl),
}

const PERMISSIONS: [(McpControl, &str); 3] = [
    (McpControl::Read, "Read document info"),
    (McpControl::Edit, "Edit pictures and palettes"),
    (McpControl::Overwrite, "Overwrite files without asking"),
];

fn label(c: McpControl) -> &'static str {
    match c {
        McpControl::On => "On",
        McpControl::Off => "Off",
        McpControl::ConnectDesktop => "Claude Desktop",
        McpControl::ConnectCode => "Claude Code",
        McpControl::Disconnect => "Disconnect",
        McpControl::Revoke => "Revoke access",
        McpControl::Read => PERMISSIONS[0].1,
        McpControl::Edit => PERMISSIONS[1].1,
        McpControl::Overwrite => PERMISSIONS[2].1,
        McpControl::SetUpGenerator => "Set up..",
        McpControl::Close => "Close",
    }
}

fn text_w(s: &str) -> i32 {
    Font::builtin().text_width(s)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpRequester {
    view: McpView,
    /// The UI screen (LoRes pixels) the requester is centred on.
    screen: (i32, i32),
    held: Option<McpControl>,
    pointer: (i32, i32),
}

impl McpRequester {
    pub fn new(view: McpView, screen: (i32, i32)) -> Self {
        Self {
            view,
            screen,
            held: None,
            pointer: (0, 0),
        }
    }

    pub fn view(&self) -> &McpView {
        &self.view
    }

    /// Shows `view`; `true` if anything shown changed.
    pub fn set_view(&mut self, view: McpView) -> bool {
        if view == self.view {
            return false;
        }
        self.view = view;
        true
    }

    /// The UI screen changed (another display mode).
    pub fn set_screen(&mut self, screen: (i32, i32)) {
        self.screen = screen;
    }

    /// Activity lines that fit on this screen, with a line free above and below (ADR-45: the
    /// image generator's two lines leave three activity lines on a 200-line screen).
    pub fn activity_rows(&self) -> usize {
        let room = (self.screen.1 - 2 - geom::FIXED_H).max(0) / ROW_H;
        (room as usize).clamp(MIN_ACTIVITY, MAX_ACTIVITY)
    }

    /// The requester's size on its screen.
    pub fn size(&self) -> (i32, i32) {
        (
            (self.screen.0 - 8).clamp(MIN_W, MAX_W),
            geom::FIXED_H + self.activity_rows() as i32 * ROW_H,
        )
    }

    /// Top-left corner on its screen: centred.
    pub fn origin(&self) -> (i32, i32) {
        let (w, h) = self.size();
        (
            ((self.screen.0 - w) / 2).max(0),
            ((self.screen.1 - h) / 2).max(0),
        )
    }

    fn radio_rect(at: (i32, i32), label: &str) -> Rect {
        Rect::new(at.0, at.1, BOX + 4 + text_w(label), ROW_H)
    }

    fn button_w(c: McpControl) -> i32 {
        text_w(label(c)) + PAD
    }

    /// Every control with its rectangle (requester coordinates), in drawing order.
    pub fn controls(&self) -> Vec<(McpControl, Rect)> {
        let mut out = Vec::new();
        let x = LEFT + text_w(CONTROL_LABEL) + 8;
        let on = Self::radio_rect((x, geom::CONTROL_Y), label(McpControl::On));
        let off_x = on.x + on.w + 8;
        out.push((McpControl::On, on));
        out.push((
            McpControl::Off,
            Self::radio_rect((off_x, geom::CONTROL_Y), label(McpControl::Off)),
        ));
        // The connect buttons right of their label; Disconnect and Revoke below them.
        let bx = LEFT + text_w(CONNECT_LABEL) + 6;
        let mut row = |y: i32, buttons: [McpControl; 2]| {
            let mut x = bx;
            for c in buttons {
                let w = Self::button_w(c);
                out.push((c, Rect::new(x, y, w, BUTTON_H)));
                x += w + GAP;
            }
        };
        row(
            geom::CONNECT_Y,
            [McpControl::ConnectDesktop, McpControl::ConnectCode],
        );
        row(geom::MANAGE_Y, [McpControl::Disconnect, McpControl::Revoke]);
        for (i, (c, label)) in PERMISSIONS.iter().enumerate() {
            let at = (LEFT, geom::PERMISSION_Y + i as i32 * ROW_H);
            out.push((*c, Self::radio_rect(at, label)));
        }
        let (w, h) = self.size();
        let sw = Self::button_w(McpControl::SetUpGenerator);
        out.push((
            McpControl::SetUpGenerator,
            Rect::new(w - sw - 6, geom::SET_UP_Y, sw, BUTTON_H),
        ));
        let cw = Self::button_w(McpControl::Close).max(56);
        out.push((
            McpControl::Close,
            Rect::new(w - cw - 6, h - BUTTON_H - 5, cw, BUTTON_H),
        ));
        out
    }

    /// Whether a control can be used now: changing needs reading, overwriting needs both;
    /// the connect buttons wait while a connect action runs.
    pub fn enabled(&self, c: McpControl) -> bool {
        let v = &self.view;
        match c {
            McpControl::Edit => v.read,
            McpControl::Overwrite => v.read && v.edit,
            McpControl::ConnectDesktop
            | McpControl::ConnectCode
            | McpControl::Disconnect
            | McpControl::Revoke => !v.busy,
            _ => true,
        }
    }

    fn control_at(&self, x: i32, y: i32) -> Option<McpControl> {
        self.controls()
            .into_iter()
            .find(|&(c, r)| r.contains(x, y) && self.enabled(c))
            .map(|(c, _)| c)
    }

    fn pressed(&self, c: McpControl) -> bool {
        self.held == Some(c) && self.control_at(self.pointer.0, self.pointer.1) == Some(c)
    }

    fn acts_on_press(c: McpControl) -> bool {
        matches!(
            c,
            McpControl::On
                | McpControl::Off
                | McpControl::Read
                | McpControl::Edit
                | McpControl::Overwrite
        )
    }

    /// Handles one event in requester coordinates (origin: [`McpRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> McpResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.control_at(x, y);
                match self.held {
                    Some(c) if Self::acts_on_press(c) => {
                        self.held = None;
                        McpResponse::Pick(c)
                    }
                    _ => McpResponse::Continue,
                }
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                McpResponse::Continue
            }
            ReqEvent::Release => match self.held.take() {
                Some(c) if self.control_at(self.pointer.0, self.pointer.1) == Some(c) => {
                    McpResponse::Pick(c)
                }
                _ => McpResponse::Continue,
            },
            ReqEvent::Scroll(_) => McpResponse::Continue,
            ReqEvent::Key(ReqKey::Enter | ReqKey::Escape) => McpResponse::Pick(McpControl::Close),
            ReqEvent::Key(_) => McpResponse::Continue,
        }
    }

    /// The status wrapped to at most two lines ("Status: " first).
    pub fn status_lines(&self) -> Vec<String> {
        let (w, _) = self.size();
        let mut lines = wrap(&format!("Status: {}", self.view.status), w - 2 * LEFT);
        if lines.len() > 2 {
            lines.truncate(2);
            lines[1].push_str("..");
        }
        lines
    }

    /// The activity lines shown: the newest that fit, oldest first.
    pub fn shown_activity(&self) -> &[String] {
        let a = &self.view.activity;
        &a[a.len().saturating_sub(self.activity_rows())..]
    }

    /// Draws the requester centred on its UI screen.
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = self.origin();
        img.blit(&self.render(pens), x, y);
    }

    /// Renders the requester alone.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let (w, h) = self.size();
        let room = w - 2 * LEFT;
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, TITLE, pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        font.draw(&mut img, LEFT, geom::CONTROL_Y, CONTROL_LABEL, pens.dark);
        for (i, line) in self.status_lines().iter().enumerate() {
            let y = geom::STATUS_Y + i as i32 * ROW_H;
            draw_clipped(&mut img, LEFT, y, line.chars(), room, pens.dark);
        }
        font.draw(
            &mut img,
            LEFT,
            geom::CONNECT_Y + (BUTTON_H - 8) / 2,
            CONNECT_LABEL,
            pens.dark,
        );
        if let Some(m) = &self.view.message {
            draw_clipped(
                &mut img,
                LEFT,
                geom::MESSAGE_Y,
                m.chars(),
                room,
                pens.select,
            );
        }
        img.hline(1, w - 2, geom::RULE_1, pens.dark);
        draw_clipped(
            &mut img,
            LEFT,
            geom::FOLDER_Y,
            format!("Folder: {}", self.view.folder).chars(),
            room,
            pens.dark,
        );
        // The generator's lines end before its Set up.. button.
        let set_up = Self::button_w(McpControl::SetUpGenerator) + 6 + GAP;
        font.draw(
            &mut img,
            LEFT,
            geom::GENERATION_Y,
            GENERATION_LABEL,
            pens.dark,
        );
        draw_clipped(
            &mut img,
            LEFT,
            geom::GENERATION_STATE_Y,
            self.view.generation.chars(),
            w - LEFT - set_up,
            pens.dark,
        );
        img.hline(1, w - 2, geom::RULE_2, pens.dark);
        let shown = self.shown_activity();
        let header = if shown.is_empty() {
            format!("{ACTIVITY_LABEL} none yet")
        } else {
            ACTIVITY_LABEL.to_string()
        };
        font.draw(&mut img, LEFT, geom::ACTIVITY_Y, &header, pens.dark);
        for (i, line) in shown.iter().enumerate() {
            let y = geom::ACTIVITY_Y + (i as i32 + 1) * ROW_H;
            draw_clipped(&mut img, LEFT, y, line.chars(), room, pens.dark);
        }
        for (c, r) in self.controls() {
            let enabled = self.enabled(c);
            match c {
                McpControl::On | McpControl::Off => {
                    let on = self.view.on == (c == McpControl::On);
                    draw_box(&mut img, (r.x, r.y), on, false, pens);
                    font.draw(&mut img, r.x + BOX + 4, r.y, label(c), pens.dark);
                }
                McpControl::Read | McpControl::Edit | McpControl::Overwrite => {
                    let on = match c {
                        McpControl::Read => self.view.read,
                        McpControl::Edit => self.view.edit,
                        _ => self.view.overwrite,
                    };
                    draw_box(&mut img, (r.x, r.y), on, true, pens);
                    let (tx, text) = (r.x + BOX + 4, label(c));
                    let pen = if enabled {
                        pens.dark
                    } else {
                        pens.ghost.unwrap_or(pens.dark)
                    };
                    font.draw(&mut img, tx, r.y, text, pen);
                    if !enabled {
                        if pens.ghost.is_none() {
                            checker(&mut img, Rect::new(tx, r.y, text_w(text), ROW_H), pens.face);
                        }
                        checker(&mut img, Rect::new(r.x, r.y + 1, BOX, BOX), pens.face);
                    }
                }
                _ => {
                    draw_button(&mut img, r, label(c), self.pressed(c), pens);
                    if !enabled {
                        checker(&mut img, r.inset(2), pens.face);
                    }
                }
            }
        }
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

    fn view() -> McpView {
        McpView {
            on: true,
            status: "Ready on 127.0.0.1:8765".into(),
            read: true,
            edit: true,
            overwrite: false,
            folder: "~/Pictures/DeluxePainter".into(),
            activity: (1..=7)
                .map(|i| format!("{i}s draw_pixel {i},{i}"))
                .collect(),
            message: None,
            generation: "OpenAI, key set".into(),
            busy: false,
        }
    }

    fn rect_of(r: &McpRequester, c: McpControl) -> Rect {
        r.controls().into_iter().find(|&(k, _)| k == c).unwrap().1
    }

    fn click(r: &mut McpRequester, c: McpControl) -> McpResponse {
        let rect = rect_of(r, c);
        let (x, y) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
        let a = r.handle(ReqEvent::Press(x, y));
        let b = r.handle(ReqEvent::Release);
        if a == McpResponse::Continue { b } else { a }
    }

    const ALL: [McpControl; 11] = [
        McpControl::On,
        McpControl::Off,
        McpControl::ConnectDesktop,
        McpControl::ConnectCode,
        McpControl::Disconnect,
        McpControl::Revoke,
        McpControl::Read,
        McpControl::Edit,
        McpControl::Overwrite,
        McpControl::SetUpGenerator,
        McpControl::Close,
    ];

    #[test]
    fn layout_fits_every_ui_screen() {
        for screen in [(320, 200), (320, 256), (368, 241), (368, 283), (288, 200)] {
            let r = McpRequester::new(view(), screen);
            let (w, h) = r.size();
            let (ox, oy) = r.origin();
            assert!(
                ox >= 0 && oy >= 0 && ox + w <= screen.0 && oy + h <= screen.1,
                "{screen:?}: {w}x{h} at {ox},{oy}"
            );
            let bounds = Rect::new(0, 0, w, h).inset(1);
            let controls = r.controls();
            assert_eq!(controls.len(), ALL.len());
            for (i, &(c, rect)) in controls.iter().enumerate() {
                assert_eq!(rect.intersect(bounds), rect, "{c:?} inside on {screen:?}");
                for &(d, other) in &controls[i + 1..] {
                    assert!(rect.intersect(other).is_empty(), "{c:?} overlaps {d:?}");
                }
            }
            // The widest label still fits.
            let longest = text_w(&format!("[x] {}", PERMISSIONS[2].1));
            assert!(longest < w - 2 * LEFT);
        }
        // Three activity lines on a 200-line screen, five on PAL.
        assert_eq!(McpRequester::new(view(), (320, 200)).activity_rows(), 3);
        assert_eq!(McpRequester::new(view(), (320, 256)).activity_rows(), 5);
        assert_eq!(
            McpRequester::new(view(), (320, 200)).shown_activity(),
            [
                "5s draw_pixel 5,5",
                "6s draw_pixel 6,6",
                "7s draw_pixel 7,7"
            ]
        );
    }

    #[test]
    fn every_control_is_hit_where_it_is_drawn() {
        let p = pens();
        let r = McpRequester::new(view(), (320, 256));
        let img = r.render(&p);
        for (c, rect) in r.controls() {
            let drawn = (rect.y..rect.y + rect.h)
                .flat_map(|y| (rect.x..rect.x + rect.w).map(move |x| (x, y)))
                .filter(|&(x, y)| img.get(x, y) != Some(p.face))
                .count();
            assert!(drawn > 4, "{c:?} is drawn");
            let mut probe = r.clone();
            assert_eq!(click(&mut probe, c), McpResponse::Pick(c), "{c:?}");
        }
        // A button released outside does nothing.
        let mut r = r;
        let close = rect_of(&r, McpControl::Close);
        r.handle(ReqEvent::Press(close.x + 2, close.y + 2));
        r.handle(ReqEvent::Drag(0, 0));
        assert_eq!(r.handle(ReqEvent::Release), McpResponse::Continue);
        // Return and Esc close.
        for k in [ReqKey::Enter, ReqKey::Escape] {
            assert_eq!(
                r.handle(ReqEvent::Key(k)),
                McpResponse::Pick(McpControl::Close)
            );
        }
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Tab)), McpResponse::Continue);
    }

    #[test]
    fn permissions_and_busy_ghost_their_controls() {
        let p = pens();
        let mut v = view();
        let mut r = McpRequester::new(v.clone(), (320, 256));
        let plain = r.render(&p);
        v.read = false;
        assert!(r.set_view(v.clone()));
        assert!(!r.set_view(v.clone()), "the same view changes nothing");
        assert!(!r.enabled(McpControl::Edit));
        assert!(!r.enabled(McpControl::Overwrite));
        assert_eq!(click(&mut r, McpControl::Edit), McpResponse::Continue);
        assert_eq!(click(&mut r, McpControl::Overwrite), McpResponse::Continue);
        assert_eq!(
            click(&mut r, McpControl::Read),
            McpResponse::Pick(McpControl::Read)
        );
        assert_ne!(r.render(&p), plain);
        v.read = true;
        v.edit = false;
        r.set_view(v.clone());
        assert!(r.enabled(McpControl::Edit));
        assert!(!r.enabled(McpControl::Overwrite));
        v.edit = true;
        v.busy = true;
        r.set_view(v);
        for c in [
            McpControl::ConnectDesktop,
            McpControl::ConnectCode,
            McpControl::Disconnect,
            McpControl::Revoke,
        ] {
            assert!(!r.enabled(c));
            assert_eq!(click(&mut r, c), McpResponse::Continue, "{c:?}");
        }
        // Close and the switch still work.
        assert_eq!(
            click(&mut r, McpControl::Close),
            McpResponse::Pick(McpControl::Close)
        );
        assert_eq!(
            click(&mut r, McpControl::Off),
            McpResponse::Pick(McpControl::Off)
        );
    }

    #[test]
    fn status_wraps_to_two_lines_and_square_corners() {
        let p = pens();
        let mut v = view();
        v.status = "Error: cannot start the MCP server: cannot keep the token in \
                    /a/very/long/path/to/the/config/directory/DeluxePainter: permission denied"
            .into();
        let r = McpRequester::new(v, (320, 200));
        let lines = r.status_lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("Status: Error"));
        assert!(lines[1].ends_with(".."));
        let img = r.render(&p);
        let (w, h) = r.size();
        assert_eq!((img.width() as i32, img.height() as i32), (w, h));
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(img.get(x, y), Some(p.dark), "square corner {x},{y}");
        }
        // A message is drawn in the select pen.
        let mut v = view();
        v.message = Some("Claude Code is connected.".into());
        let r = McpRequester::new(v, (320, 200));
        let img = r.render(&p);
        let select = (geom::MESSAGE_Y..geom::MESSAGE_Y + 8)
            .flat_map(|y| (LEFT..LEFT + 80).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get(x, y) == Some(p.select))
            .count();
        assert!(select > 0);
    }

    #[test]
    fn on_and_off_show_as_radios() {
        let p = pens();
        let mut v = view();
        let on = McpRequester::new(v.clone(), (320, 256));
        v.on = false;
        let off = McpRequester::new(v, (320, 256));
        let centre = |r: &McpRequester, c: McpControl| {
            let rect = rect_of(r, c);
            r.render(&p).get(rect.x + BOX / 2, rect.y + 1 + BOX / 2)
        };
        assert_eq!(centre(&on, McpControl::On), Some(p.dark));
        assert_ne!(centre(&on, McpControl::Off), Some(p.dark));
        assert_eq!(centre(&off, McpControl::Off), Some(p.dark));
        assert_ne!(centre(&off, McpControl::On), Some(p.dark));
    }

    #[test]
    fn the_generator_line_shows_its_state_beside_set_up() {
        let p = pens();
        for screen in [(320, 200), (320, 256), (288, 200)] {
            let mut v = view();
            v.generation = "OpenAI, no API key, and a very long tail to clip".into();
            v.busy = true;
            let mut r = McpRequester::new(v, screen);
            let set_up = rect_of(&r, McpControl::SetUpGenerator);
            let (w, h) = r.size();
            assert!(h <= screen.1 - 2, "{screen:?}: fits the screen");
            assert_eq!(set_up.x + set_up.w, w - 6, "right-aligned like Close");
            // Between the folder line and the rule above the activity, centred on the
            // label and the state.
            assert!(set_up.y >= geom::FOLDER_Y + 9 && set_up.y + set_up.h <= geom::RULE_2);
            let lines = (geom::GENERATION_Y, geom::GENERATION_STATE_Y + 8);
            let (above, below) = (set_up.y - lines.0, lines.1 - (set_up.y + set_up.h));
            assert!((above - below).abs() <= 1, "{above} above, {below} below");
            // A connect step running does not ghost it.
            assert!(r.enabled(McpControl::SetUpGenerator));
            assert_eq!(
                click(&mut r, McpControl::SetUpGenerator),
                McpResponse::Pick(McpControl::SetUpGenerator)
            );
            // Both lines show; the state stops before the button.
            let img = r.render(&p);
            let inked = |y0: i32, x0: i32, x1: i32| {
                (y0..y0 + 8)
                    .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                    .filter(|&(x, y)| img.get(x, y) == Some(p.dark))
                    .count()
            };
            assert!(
                inked(geom::GENERATION_Y, LEFT, LEFT + 60) > 20,
                "{screen:?}"
            );
            let state = geom::GENERATION_STATE_Y;
            assert!(inked(state, LEFT, LEFT + 60) > 20, "{screen:?}");
            assert_eq!(
                inked(state, set_up.x - GAP, set_up.x),
                0,
                "{screen:?}: a gap before Set up.."
            );
        }
    }

    #[test]
    fn every_generator_state_fits_beside_set_up_on_the_narrowest_screens() {
        // The states Prefs > AI & MCP.. shows (the app's `generator_line`).
        let r = McpRequester::new(view(), (320, 200));
        let room = r.size().0 - LEFT - (McpRequester::button_w(McpControl::SetUpGenerator) + 6);
        for state in [
            "Not set up",
            "OpenAI, key set",
            "OpenAI, no API key",
            "OpenAI, key from OPENAI_API_KEY",
            "OpenAI, key not checked yet",
        ] {
            assert!(text_w(state) + GAP <= room, "{state}");
        }
        assert!(text_w(GENERATION_LABEL) + GAP <= room);
    }
}
