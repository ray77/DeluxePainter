//! Note requester: lists the notes produced by Picture ▸ Load and Save (things that were not
//! kept, were reduced or cannot be stored), with a single Ok button.
//!
//! Drawn on the UI layer in LoRes UI pixels with palette-derived pens, like the other
//! requesters. DeluxePaint IV shows its messages in small requesters with buttons; the layout
//! measurements here are our own (ADR-12).

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{centred, draw_button, frame};

/// Requester width in LoRes UI pixels; the smallest UI screen is 320×200.
pub const WIDTH: i32 = 256;
/// Most wrapped text lines shown.
pub const MAX_LINES: usize = 14;

mod geom {
    use super::WIDTH;

    pub const TITLE_Y: i32 = 2;
    pub const TITLE_LINE_Y: i32 = 12;
    pub const TEXT_Y: i32 = 17;
    pub const LINE_STEP: i32 = 9;
    pub const BULLET_X: i32 = 8;
    pub const BULLET_SIZE: i32 = 2;
    pub const TEXT_X: i32 = 16;
    pub const RIGHT_MARGIN: i32 = 8;
    pub const TEXT_W: i32 = WIDTH - TEXT_X - RIGHT_MARGIN;
    pub const BUTTON_GAP: i32 = 6;
    pub const BUTTON_W: i32 = 56;
    pub const BUTTON_H: i32 = 13;
    pub const BOTTOM: i32 = 5;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteResponse {
    Continue,
    Close,
}

/// One shown text line; `bullet` marks the first line of a note.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    text: String,
    bullet: bool,
}

/// State of the note requester.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRequester {
    title: String,
    lines: Vec<Line>,
    /// Ok button held down, and the pointer position.
    held: bool,
    pointer: (i32, i32),
}

/// Upper-cases the first letter.
fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Word-wraps `text` at spaces to at most `width` pixels per line; a word wider than a line is
/// broken by characters.
fn wrap(text: &str, width: i32) -> Vec<String> {
    let font = Font::builtin();
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let joined = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if font.text_width(&joined) <= width {
            cur = joined;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        // Break the word by characters.
        for c in word.chars() {
            let mut next = cur.clone();
            next.push(c);
            if !cur.is_empty() && font.text_width(&next) > width {
                lines.push(std::mem::take(&mut cur));
                cur.push(c);
            } else {
                cur = next;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

impl NoteRequester {
    /// `title` e.g. "Picture loaded"; one entry per note. Notes are wrapped to the text width.
    pub fn new(title: &str, notes: &[String]) -> Self {
        // (note index, line) for every wrapped line.
        let mut all: Vec<(usize, Line)> = Vec::new();
        for (n, note) in notes.iter().enumerate() {
            for (i, text) in wrap(&capitalise(note), geom::TEXT_W)
                .into_iter()
                .enumerate()
            {
                all.push((
                    n,
                    Line {
                        text,
                        bullet: i == 0,
                    },
                ));
            }
        }
        let lines = if all.len() > MAX_LINES {
            let shown = MAX_LINES - 1;
            // A note counts as shown only when its last line is among the shown ones.
            let fully_shown = (0..notes.len())
                .filter(|&n| {
                    all.iter()
                        .rposition(|(i, _)| *i == n)
                        .is_some_and(|last| last < shown)
                })
                .count();
            let mut v: Vec<Line> = all.into_iter().take(shown).map(|(_, l)| l).collect();
            v.push(Line {
                text: format!("...and {} more", notes.len() - fully_shown),
                bullet: false,
            });
            v
        } else {
            all.into_iter().map(|(_, l)| l).collect()
        };
        Self {
            title: title.to_string(),
            lines,
            held: false,
            pointer: (0, 0),
        }
    }

    /// Height in UI pixels (depends on the number of lines).
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

    fn ok_rect(&self) -> Rect {
        let y = geom::TEXT_Y + self.lines.len() as i32 * geom::LINE_STEP + geom::BUTTON_GAP;
        Rect::new(
            (WIDTH - geom::BUTTON_W) / 2,
            y,
            geom::BUTTON_W,
            geom::BUTTON_H,
        )
    }

    fn pressed(&self) -> bool {
        self.held && self.ok_rect().contains(self.pointer.0, self.pointer.1)
    }

    /// Handles one event in requester coordinates (see [`NoteRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> NoteResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.ok_rect().contains(x, y);
                NoteResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                NoteResponse::Continue
            }
            ReqEvent::Release => {
                let fire = self.pressed();
                self.held = false;
                if fire {
                    NoteResponse::Close
                } else {
                    NoteResponse::Continue
                }
            }
            ReqEvent::Scroll(_) => NoteResponse::Continue,
            ReqEvent::Key(ReqKey::Enter | ReqKey::Escape | ReqKey::Char(' ')) => {
                NoteResponse::Close
            }
            ReqEvent::Key(_) => NoteResponse::Continue,
        }
    }

    /// Draws the requester onto a UI screen at [`NoteRequester::origin`].
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
            if line.bullet {
                let o = (8 - geom::BULLET_SIZE) / 2;
                img.fill_rect(
                    geom::BULLET_X,
                    y + o,
                    geom::BULLET_SIZE,
                    geom::BULLET_SIZE,
                    pens.dark,
                );
            }
            font.draw(&mut img, geom::TEXT_X, y, &line.text, pens.dark);
        }
        draw_button(&mut img, self.ok_rect(), "Ok", self.pressed(), pens);
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::UiState;
    use dpx_core::Palette;

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn notes(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn click_ok(r: &mut NoteRequester) -> NoteResponse {
        let b = r.ok_rect();
        r.handle(ReqEvent::Press(b.x + 3, b.y + 3));
        r.handle(ReqEvent::Release)
    }

    #[test]
    fn render_size_matches_height_and_grows_with_lines() {
        let short = NoteRequester::new("Picture loaded", &notes(&["one"]));
        let long = NoteRequester::new("Picture loaded", &notes(&["one", "two", "three"]));
        let empty = NoteRequester::new("Picture loaded", &[]);
        for r in [&short, &long, &empty] {
            let img = r.render(&pens());
            assert_eq!(
                (img.width() as i32, img.height() as i32),
                (WIDTH, r.height())
            );
        }
        assert!(long.height() > short.height());
        assert!(short.height() > empty.height());
        assert_eq!(short.height() - empty.height(), geom::LINE_STEP);
    }

    #[test]
    fn wrap_keeps_words_and_fits() {
        let font = Font::builtin();
        let text =
            "AGA true colour reduced to 256 colours (Floyd-Steinberg dithering) and more words";
        let lines = wrap(text, 150);
        assert!(lines.len() > 1);
        assert_eq!(lines.join(" "), text);
        for l in &lines {
            assert!(font.text_width(l) <= 150, "{l:?}");
        }
        assert!(wrap("", 150).is_empty());
    }

    #[test]
    fn wrap_breaks_long_words() {
        let font = Font::builtin();
        let word = "Supercalifragilisticexpialidocious";
        let lines = wrap(&format!("a {word}"), 60);
        assert!(lines.len() >= 2);
        assert_eq!(lines.concat().replace(' ', ""), format!("a{word}"));
        for l in &lines {
            assert!(font.text_width(l) <= 60, "{l:?}");
        }
    }

    #[test]
    fn first_letter_is_capitalised() {
        let r = NoteRequester::new("T", &notes(&["mask plane not kept", ""]));
        assert_eq!(r.lines[0].text, "Mask plane not kept");
        assert_eq!(capitalise(""), "");
    }

    #[test]
    fn overflow_shows_more_line_with_count() {
        // Ten notes of two lines each: 20 lines > 14.
        let long = "word ".repeat(60);
        let n: Vec<String> = (0..10).map(|_| long.trim().to_string()).collect();
        let lines_per = wrap(&n[0], geom::TEXT_W).len();
        assert!(lines_per >= 2);
        let r = NoteRequester::new("T", &n);
        assert_eq!(r.lines.len(), MAX_LINES);
        let last = r.lines.last().unwrap();
        assert!(!last.bullet);
        let shown = (MAX_LINES - 1) / lines_per; // notes completely shown
        assert_eq!(last.text, format!("...and {} more", 10 - shown));

        // Many single-line notes.
        let n: Vec<String> = (0..20).map(|i| format!("note {i}")).collect();
        let r = NoteRequester::new("T", &n);
        assert_eq!(r.lines.len(), MAX_LINES);
        assert_eq!(r.lines.last().unwrap().text, "...and 7 more");
        // Exactly MAX_LINES lines: no overflow line.
        let n: Vec<String> = (0..MAX_LINES).map(|i| format!("note {i}")).collect();
        let r = NoteRequester::new("T", &n);
        assert_eq!(r.lines.len(), MAX_LINES);
        assert!(!r.lines.last().unwrap().text.starts_with("..."));
    }

    #[test]
    fn ok_click_closes_release_away_does_not() {
        let mut r = NoteRequester::new("T", &notes(&["a"]));
        assert_eq!(click_ok(&mut r), NoteResponse::Close);
        let b = r.ok_rect();
        r.handle(ReqEvent::Press(b.x + 3, b.y + 3));
        assert_ne!(
            r.render(&pens()),
            NoteRequester::new("T", &notes(&["a"])).render(&pens())
        );
        r.handle(ReqEvent::Drag(0, 0));
        assert_eq!(r.handle(ReqEvent::Release), NoteResponse::Continue);
        // Press outside does nothing, even if released over Ok.
        r.handle(ReqEvent::Press(0, 0));
        r.handle(ReqEvent::Drag(b.x + 3, b.y + 3));
        assert_eq!(r.handle(ReqEvent::Release), NoteResponse::Continue);
    }

    #[test]
    fn keys() {
        let mut r = NoteRequester::new("T", &notes(&["a"]));
        for k in [ReqKey::Enter, ReqKey::Escape, ReqKey::Char(' ')] {
            assert_eq!(r.handle(ReqEvent::Key(k)), NoteResponse::Close);
        }
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Tab)), NoteResponse::Continue);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Char('x'))),
            NoteResponse::Continue
        );
        assert_eq!(r.handle(ReqEvent::Scroll(1)), NoteResponse::Continue);
    }

    #[test]
    fn origin_centres_and_clamps() {
        let r = NoteRequester::new("T", &notes(&["a"]));
        assert_eq!(r.origin(320, 256), (32, (256 - r.height()) / 2));
        assert_eq!(r.origin(100, 50), (0, 0));
    }

    #[test]
    fn title_pixels_are_drawn() {
        let p = pens();
        let img = NoteRequester::new("Picture loaded", &notes(&["a"])).render(&p);
        let title_dark = (2..11)
            .flat_map(|y| (60..200).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get(x, y) == Some(p.dark))
            .count();
        assert!(title_dark > 0);
    }

    #[test]
    fn ui_state_has_no_note_requester_by_default() {
        assert!(UiState::default().note_requester.is_none());
    }
}
