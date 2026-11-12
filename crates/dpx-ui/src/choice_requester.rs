//! Choice requester: a title, a few lines of text and two buttons (ADR-34). Used for
//! "Recover Picture" (Recover / Discard) and "Quit without saving?" (Yes / No).
//!
//! Drawn like the note requester (ADR-12): LoRes UI pixels, palette-derived pens, square
//! corners. Return presses the `enter` button and Esc the `esc` button.

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::note_requester::wrap;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{centred, draw_button, frame};

/// Requester width in LoRes UI pixels; the smallest UI screen is 320×200.
pub const WIDTH: i32 = 256;
/// Most wrapped text lines shown.
pub const MAX_LINES: usize = 8;
/// Most lines of a question that has to show a whole command (ADR-43, Prefs > AI & MCP's
/// connect questions): 16 lines still fit the 200-line UI screen.
pub const MAX_LINES_TALL: usize = 16;

mod geom {
    use super::WIDTH;

    pub const TITLE_Y: i32 = 2;
    pub const TITLE_LINE_Y: i32 = 12;
    pub const TEXT_Y: i32 = 17;
    pub const LINE_STEP: i32 = 9;
    pub const TEXT_X: i32 = 12;
    pub const TEXT_W: i32 = WIDTH - 2 * TEXT_X;
    pub const BUTTON_GAP: i32 = 6;
    pub const BUTTON_W: i32 = 72;
    pub const BUTTON_H: i32 = 13;
    pub const BUTTON_SPACE: i32 = 16;
    pub const BOTTOM: i32 = 5;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceResponse {
    Continue,
    /// The button with this index (0 = left, 1 = right) was chosen.
    Pick(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceRequester {
    title: String,
    lines: Vec<String>,
    /// Text was cut to fit.
    truncated: bool,
    labels: [String; 2],
    /// Buttons chosen by Return and Esc.
    enter: usize,
    esc: usize,
    /// Button held down, and the pointer position.
    held: Option<usize>,
    pointer: (i32, i32),
}

impl ChoiceRequester {
    /// `text` is word-wrapped (one paragraph per entry), at most [`MAX_LINES`] lines.
    pub fn new(title: &str, text: &[String], labels: [&str; 2], enter: usize, esc: usize) -> Self {
        Self::with_max_lines(title, text, labels, enter, esc, MAX_LINES)
    }

    /// [`Self::new`] with up to `max_lines` lines (at most [`MAX_LINES_TALL`]); more are cut,
    /// the last shown line being "..." ([`Self::truncated`]).
    pub fn with_max_lines(
        title: &str,
        text: &[String],
        labels: [&str; 2],
        enter: usize,
        esc: usize,
        max_lines: usize,
    ) -> Self {
        let max_lines = max_lines.clamp(2, MAX_LINES_TALL);
        let mut lines: Vec<String> = text
            .iter()
            .flat_map(|t| wrap(t, geom::TEXT_W).into_iter())
            .collect();
        let truncated = lines.len() > max_lines;
        if truncated {
            lines.truncate(max_lines - 1);
            lines.push("...".into());
        }
        Self {
            title: title.to_string(),
            lines,
            truncated,
            labels: [labels[0].to_string(), labels[1].to_string()],
            enter: enter.min(1),
            esc: esc.min(1),
            held: None,
            pointer: (0, 0),
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The wrapped text lines.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Not all of the text is shown.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn labels(&self) -> [&str; 2] {
        [&self.labels[0], &self.labels[1]]
    }

    pub fn height(&self) -> i32 {
        geom::TEXT_Y
            + self.lines.len() as i32 * geom::LINE_STEP
            + geom::BUTTON_GAP
            + geom::BUTTON_H
            + geom::BOTTOM
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(&self, screen_w: i32, screen_h: i32) -> (i32, i32) {
        (
            ((screen_w - WIDTH) / 2).max(0),
            ((screen_h - self.height()) / 2).max(0),
        )
    }

    /// The buttons are 72 pixels wide, or wider for a longer label (ADR-43: "Get Claude
    /// Code"), as long as both fit.
    fn button_widths(&self) -> (i32, i32) {
        let room = WIDTH - 8 - geom::BUTTON_SPACE;
        let want =
            |i: usize| (Font::builtin().text_width(&self.labels[i]) + 12).max(geom::BUTTON_W);
        let (w0, w1) = (want(0), want(1));
        if w0 + w1 <= room {
            (w0, w1)
        } else {
            (room / 2, room / 2)
        }
    }

    fn button_rect(&self, i: usize) -> Rect {
        let y = geom::TEXT_Y + self.lines.len() as i32 * geom::LINE_STEP + geom::BUTTON_GAP;
        let (w0, w1) = self.button_widths();
        let total = w0 + w1 + geom::BUTTON_SPACE;
        let x0 = (WIDTH - total) / 2;
        let (x, w) = if i == 0 {
            (x0, w0)
        } else {
            (x0 + w0 + geom::BUTTON_SPACE, w1)
        };
        Rect::new(x, y, w, geom::BUTTON_H)
    }

    /// Centre of button `i` in requester coordinates (for tests and snapshots).
    pub fn button_centre(&self, i: usize) -> (i32, i32) {
        let r = self.button_rect(i);
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        (0..2).find(|&i| self.button_rect(i).contains(x, y))
    }

    fn pressed(&self, i: usize) -> bool {
        self.held == Some(i) && self.hit(self.pointer.0, self.pointer.1) == Some(i)
    }

    /// Handles one event in requester coordinates (see [`ChoiceRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> ChoiceResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.hit(x, y);
                ChoiceResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                ChoiceResponse::Continue
            }
            ReqEvent::Release => {
                let fire = self.held.filter(|&i| self.pressed(i));
                self.held = None;
                fire.map_or(ChoiceResponse::Continue, ChoiceResponse::Pick)
            }
            ReqEvent::Key(ReqKey::Enter) => ChoiceResponse::Pick(self.enter),
            ReqEvent::Key(ReqKey::Escape) => ChoiceResponse::Pick(self.esc),
            _ => ChoiceResponse::Continue,
        }
    }

    /// Draws the requester onto a UI screen at [`ChoiceRequester::origin`].
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = self.origin(img.width() as i32, img.height() as i32);
        img.blit(&self.render(pens), x, y);
    }

    /// Renders the requester alone (WIDTH×height()).
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let h = self.height();
        let mut img = IndexedImage::new(WIDTH as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, WIDTH, h), pens.dark);
        centred(&mut img, 0, WIDTH, geom::TITLE_Y, &self.title, pens.dark);
        img.hline(1, WIDTH - 2, geom::TITLE_LINE_Y, pens.dark);
        for (i, line) in self.lines.iter().enumerate() {
            let y = geom::TEXT_Y + i as i32 * geom::LINE_STEP;
            font.draw(&mut img, geom::TEXT_X, y, line, pens.dark);
        }
        for i in 0..2 {
            draw_button(
                &mut img,
                self.button_rect(i),
                &self.labels[i],
                self.pressed(i),
                pens,
            );
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

    fn req() -> ChoiceRequester {
        ChoiceRequester::new(
            "Recover Picture",
            &["Castle, saved 2026-11-11 10:00 UTC".to_string()],
            ["Recover", "Discard"],
            0,
            1,
        )
    }

    #[test]
    fn return_and_escape_pick_their_buttons() {
        assert_eq!(
            req().handle(ReqEvent::Key(ReqKey::Enter)),
            ChoiceResponse::Pick(0)
        );
        assert_eq!(
            req().handle(ReqEvent::Key(ReqKey::Escape)),
            ChoiceResponse::Pick(1)
        );
        assert_eq!(
            req().handle(ReqEvent::Key(ReqKey::Char('x'))),
            ChoiceResponse::Continue
        );
    }

    #[test]
    fn clicks_pick_a_button_only_when_released_on_it() {
        let mut r = req();
        let b = r.button_rect(1);
        r.handle(ReqEvent::Press(b.x + 2, b.y + 2));
        assert_eq!(r.handle(ReqEvent::Release), ChoiceResponse::Pick(1));
        r.handle(ReqEvent::Press(b.x + 2, b.y + 2));
        r.handle(ReqEvent::Drag(1, 1));
        assert_eq!(r.handle(ReqEvent::Release), ChoiceResponse::Continue);
        let a = r.button_rect(0);
        r.handle(ReqEvent::Press(a.x + 2, a.y + 2));
        assert_eq!(r.handle(ReqEvent::Release), ChoiceResponse::Pick(0));
    }

    #[test]
    fn a_tall_question_shows_up_to_sixteen_lines_and_fits_the_smallest_screen() {
        let long = vec!["word ".repeat(200)];
        let short = ChoiceRequester::new("T", &long, ["A", "B"], 0, 1);
        assert_eq!(short.lines().len(), MAX_LINES);
        assert!(short.truncated());
        assert_eq!(short.lines().last().map(String::as_str), Some("..."));
        let tall = ChoiceRequester::with_max_lines("T", &long, ["A", "B"], 0, 1, MAX_LINES_TALL);
        assert_eq!(tall.lines().len(), MAX_LINES_TALL);
        assert!(tall.truncated());
        assert!(tall.height() <= 200, "{}", tall.height());
        assert!(tall.origin(320, 200).1 + tall.height() <= 200);
        let fits = ChoiceRequester::with_max_lines(
            "T",
            &["one line".into()],
            ["A", "B"],
            0,
            1,
            MAX_LINES_TALL,
        );
        assert!(!fits.truncated());
        assert!(!req().truncated());
    }

    #[test]
    fn a_long_label_gets_a_wider_button_inside_the_requester() {
        let short = req();
        assert_eq!(short.button_rect(0).w, geom::BUTTON_W);
        assert_eq!(short.button_rect(1).w, geom::BUTTON_W);
        let r = ChoiceRequester::new("T", &[], ["Get Claude Code", "Close"], 1, 1);
        let (a, b) = (r.button_rect(0), r.button_rect(1));
        assert!(a.w >= Font::builtin().text_width("Get Claude Code") + 8);
        assert_eq!(b.w, geom::BUTTON_W);
        assert!(a.x > 0 && a.x + a.w + geom::BUTTON_SPACE <= b.x && b.x + b.w < WIDTH);
    }

    #[test]
    fn renders_at_its_height() {
        let img = req().render(&Pens::from_palette(&Palette::dpaint_default_32()));
        assert_eq!(
            (img.width() as i32, img.height() as i32),
            (WIDTH, req().height())
        );
    }
}
