//! Picture ▸ Page Size requester ("Set Page Size", DeluxePaint IV AGA).
//!
//! Drawn in LoRes UI pixels with palette-derived pens, like the file requester. Two string
//! gadgets take the width and height; three preset buttons (Standard, Full Page, Overscan)
//! fill them with sizes supplied by the caller. The requester knows no page limits: the caller
//! validates [`PageSizeResponse::Ok`] and reports problems with
//! [`PageSizeRequester::set_message`].

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{TextField, button_face, centred, draw_button, draw_field, frame};

/// Requester size in LoRes pixels.
pub const WIDTH: i32 = 216;
pub const HEIGHT: i32 = 118;
/// Most digits a size field accepts.
const MAX_DIGITS: usize = 4;
/// Shown when Return or Ok finds an empty or zero field.
const MISSING: &str = "Type a width and a height";

mod geom {
    use super::WIDTH;

    pub const TITLE_Y: i32 = 2;
    pub const TITLE_LINE_Y: i32 = 12;
    pub const PROMPT_X: i32 = 6;
    pub const PROMPT_Y: i32 = 15;
    pub const FIELD_Y: i32 = 26;
    pub const FIELD_H: i32 = 9;
    pub const FIELD_W: i32 = 40;
    pub const WIDTH_LABEL_X: i32 = 6;
    pub const WIDTH_FIELD_X: i32 = 52;
    pub const HEIGHT_LABEL_X: i32 = 98;
    pub const HEIGHT_FIELD_X: i32 = 152;
    pub const SEPARATOR_Y: i32 = 39;
    pub const SELECT_Y: i32 = 43;
    pub const PRESET_X: i32 = 30;
    pub const PRESET_W: i32 = 80;
    pub const PRESET_Y: i32 = 54;
    pub const PRESET_STEP: i32 = 14;
    pub const SIZE_X: i32 = 118;
    pub const BUTTON_H: i32 = 13;
    pub const BUTTON_W: i32 = 56;
    pub const BUTTONS_Y: i32 = 101;
    pub const CANCEL_X: i32 = 6;
    pub const OK_X: i32 = WIDTH - 62;
}

/// A preset page size: its button label and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub label: &'static str,
    pub size: (u32, u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageSizeResponse {
    Continue,
    Cancel,
    Ok { width: u32, height: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Width,
    Height,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Preset(usize),
    Cancel,
    Ok,
}

impl Button {
    const ALL: [Button; 5] = [
        Button::Preset(0),
        Button::Preset(1),
        Button::Preset(2),
        Button::Cancel,
        Button::Ok,
    ];

    fn rect(self) -> Rect {
        match self {
            Button::Preset(i) => Rect::new(
                geom::PRESET_X,
                geom::PRESET_Y + i as i32 * geom::PRESET_STEP,
                geom::PRESET_W,
                geom::BUTTON_H,
            ),
            Button::Cancel => Rect::new(
                geom::CANCEL_X,
                geom::BUTTONS_Y,
                geom::BUTTON_W,
                geom::BUTTON_H,
            ),
            Button::Ok => Rect::new(geom::OK_X, geom::BUTTONS_Y, geom::BUTTON_W, geom::BUTTON_H),
        }
    }
}

impl Field {
    fn rect(self) -> Rect {
        let x = match self {
            Field::Width => geom::WIDTH_FIELD_X,
            Field::Height => geom::HEIGHT_FIELD_X,
        };
        Rect::new(x, geom::FIELD_Y, geom::FIELD_W, geom::FIELD_H)
    }
}

/// State of the Set Page Size requester.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSizeRequester {
    width: TextField,
    height: TextField,
    active: Field,
    presets: [Preset; 3],
    message: Option<String>,
    /// Button held down, and the pointer position.
    held: Option<Button>,
    pointer: (i32, i32),
}

impl PageSizeRequester {
    /// `current` fills the fields; `presets` are Standard, Full Page, Overscan.
    pub fn new(current: (u32, u32), presets: [Preset; 3]) -> Self {
        let mut r = Self {
            width: TextField::new(MAX_DIGITS),
            height: TextField::new(MAX_DIGITS),
            active: Field::Width,
            presets,
            message: None,
            held: None,
            pointer: (0, 0),
        };
        r.fill(current);
        r
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(screen_w: i32, screen_h: i32) -> (i32, i32) {
        (
            ((screen_w - WIDTH) / 2).max(0),
            ((screen_h - HEIGHT) / 2).max(0),
        )
    }

    /// Shown instead of "Type in size:" until the next edit.
    pub fn set_message(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
    }

    fn fill(&mut self, (w, h): (u32, u32)) {
        self.width.set(&w.to_string());
        self.height.set(&h.to_string());
    }

    fn field_mut(&mut self, f: Field) -> &mut TextField {
        match f {
            Field::Width => &mut self.width,
            Field::Height => &mut self.height,
        }
    }

    /// Both fields as numbers; `None` while one is empty.
    fn parsed(&self) -> Option<(u32, u32)> {
        Some((
            self.width.text().parse().ok()?,
            self.height.text().parse().ok()?,
        ))
    }

    fn pressed(&self, b: Button) -> bool {
        self.held == Some(b) && b.rect().contains(self.pointer.0, self.pointer.1)
    }

    /// Handles one event in requester coordinates (see [`PageSizeRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> PageSizeResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = Button::ALL.into_iter().find(|b| b.rect().contains(x, y));
                if let Some(f) = [Field::Width, Field::Height]
                    .into_iter()
                    .find(|f| f.rect().contains(x, y))
                {
                    self.activate(f);
                }
                PageSizeResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                PageSizeResponse::Continue
            }
            ReqEvent::Release => match self.held.take() {
                Some(b) if b.rect().contains(self.pointer.0, self.pointer.1) => self.button(b),
                _ => PageSizeResponse::Continue,
            },
            ReqEvent::Scroll(_) => PageSizeResponse::Continue,
            ReqEvent::Key(key) => self.key(key),
        }
    }

    fn activate(&mut self, f: Field) {
        self.active = f;
        let field = self.field_mut(f);
        field.cursor = field.chars.len();
    }

    fn button(&mut self, b: Button) -> PageSizeResponse {
        match b {
            Button::Preset(i) => {
                self.message = None;
                self.fill(self.presets[i].size);
                PageSizeResponse::Continue
            }
            Button::Cancel => PageSizeResponse::Cancel,
            Button::Ok => self.accept(),
        }
    }

    fn accept(&mut self) -> PageSizeResponse {
        match self.parsed() {
            Some((width, height)) if width >= 1 && height >= 1 => {
                PageSizeResponse::Ok { width, height }
            }
            _ => {
                self.message = Some(MISSING.to_string());
                PageSizeResponse::Continue
            }
        }
    }

    fn key(&mut self, key: ReqKey) -> PageSizeResponse {
        match key {
            ReqKey::Enter => return self.accept(),
            ReqKey::Escape => return PageSizeResponse::Cancel,
            ReqKey::Tab => {
                let next = match self.active {
                    Field::Width => Field::Height,
                    Field::Height => Field::Width,
                };
                self.activate(next);
            }
            ReqKey::Char(c) | ReqKey::Paste(c) if !c.is_ascii_digit() => {}
            k => {
                let active = self.active;
                let field = self.field_mut(active);
                let before = field.chars.clone();
                field.key(k);
                if field.chars != before {
                    self.message = None;
                }
            }
        }
        PageSizeResponse::Continue
    }

    /// Draws the requester onto a UI screen at [`PageSizeRequester::origin`].
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
            "Set Page Size",
            pens.dark,
        );
        img.hline(1, WIDTH - 2, geom::TITLE_LINE_Y, pens.dark);

        match &self.message {
            Some(m) => font.draw(&mut img, geom::PROMPT_X, geom::PROMPT_Y, m, pens.select),
            None => font.draw(
                &mut img,
                geom::PROMPT_X,
                geom::PROMPT_Y,
                "Type in size:",
                pens.dark,
            ),
        };
        let label_y = geom::FIELD_Y + 1;
        font.draw(&mut img, geom::WIDTH_LABEL_X, label_y, "Width:", pens.dark);
        font.draw(
            &mut img,
            geom::HEIGHT_LABEL_X,
            label_y,
            "Height:",
            pens.dark,
        );
        draw_field(
            &mut img,
            Field::Width.rect(),
            &self.width,
            self.active == Field::Width,
            pens,
        );
        draw_field(
            &mut img,
            Field::Height.rect(),
            &self.height,
            self.active == Field::Height,
            pens,
        );

        img.hline(1, WIDTH - 2, geom::SEPARATOR_Y, pens.dark);
        font.draw(
            &mut img,
            geom::PROMPT_X,
            geom::SELECT_Y,
            "Or select one:",
            pens.dark,
        );
        let parsed = self.parsed();
        for (i, p) in self.presets.iter().enumerate() {
            let b = Button::Preset(i);
            let r = b.rect();
            let selected = parsed == Some(p.size);
            let pressed = self.pressed(b) || selected;
            let ink = button_face(&mut img, r, pressed, pens);
            centred(&mut img, r.x, r.w, r.y + (r.h - 8) / 2, p.label, ink);
            let text = format!("{} x {}", p.size.0, p.size.1);
            font.draw(
                &mut img,
                geom::SIZE_X,
                r.y + (r.h - 8) / 2,
                &text,
                pens.dark,
            );
        }
        for (b, label) in [(Button::Cancel, "Cancel"), (Button::Ok, "Ok")] {
            draw_button(&mut img, b.rect(), label, self.pressed(b), pens);
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

    const PRESETS: [Preset; 3] = [
        Preset {
            label: "Standard",
            size: (320, 200),
        },
        Preset {
            label: "Full Page",
            size: (320, 340),
        },
        Preset {
            label: "Overscan",
            size: (368, 241),
        },
    ];

    fn req() -> PageSizeRequester {
        PageSizeRequester::new((320, 200), PRESETS)
    }

    fn click(r: &mut PageSizeRequester, b: Button) -> PageSizeResponse {
        let rect = b.rect();
        r.handle(ReqEvent::Press(rect.x + 3, rect.y + 3));
        r.handle(ReqEvent::Release)
    }

    fn type_text(r: &mut PageSizeRequester, s: &str) {
        for c in s.chars() {
            r.handle(ReqEvent::Key(ReqKey::Char(c)));
        }
    }

    #[test]
    fn preset_click_fills_fields_and_selects_it() {
        let mut r = req();
        let before = r.render(&pens());
        assert_eq!(click(&mut r, Button::Preset(1)), PageSizeResponse::Continue);
        assert_eq!(r.width.text(), "320");
        assert_eq!(r.height.text(), "340");
        assert_eq!(r.parsed(), Some((320, 340)));
        assert_ne!(r.render(&pens()), before, "selection is drawn");
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            PageSizeResponse::Ok {
                width: 320,
                height: 340
            }
        );
    }

    #[test]
    fn typing_filters_non_digits_and_caps_at_four() {
        let mut r = req();
        for _ in 0..3 {
            r.handle(ReqEvent::Key(ReqKey::Backspace));
        }
        type_text(&mut r, "a1b2c3d4e5");
        assert_eq!(r.width.text(), "1234");
    }

    #[test]
    fn backspace_and_tab() {
        let mut r = req();
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        assert_eq!(r.width.text(), "32");
        r.handle(ReqEvent::Key(ReqKey::Tab));
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        assert_eq!(r.height.text(), "20");
        assert_eq!(r.width.text(), "32");
        r.handle(ReqEvent::Key(ReqKey::Tab));
        type_text(&mut r, "1");
        assert_eq!(r.width.text(), "321");
        // Clicking a field activates it.
        let f = Field::Height.rect();
        r.handle(ReqEvent::Press(f.x + 2, f.y + 2));
        r.handle(ReqEvent::Release);
        type_text(&mut r, "7");
        assert_eq!(r.height.text(), "207");
    }

    #[test]
    fn return_with_values_is_ok() {
        let mut r = req();
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            PageSizeResponse::Ok {
                width: 320,
                height: 200
            }
        );
    }

    #[test]
    fn return_with_empty_field_keeps_open_and_shows_message() {
        let mut r = req();
        r.handle(ReqEvent::Key(ReqKey::Tab));
        for _ in 0..3 {
            r.handle(ReqEvent::Key(ReqKey::Backspace));
        }
        let plain = r.render(&pens());
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            PageSizeResponse::Continue
        );
        assert_eq!(r.message.as_deref(), Some(MISSING));
        assert_ne!(r.render(&pens()), plain);
        // The next edit clears it.
        type_text(&mut r, "5");
        assert_eq!(r.message, None);
        // A zero size is rejected too.
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        type_text(&mut r, "0");
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            PageSizeResponse::Continue
        );
    }

    #[test]
    fn caller_message_is_drawn() {
        let mut r = req();
        let plain = r.render(&pens());
        r.set_message("Too big");
        assert_ne!(r.render(&pens()), plain);
    }

    #[test]
    fn escape_and_cancel_cancel_and_ok_button_accepts() {
        let mut r = req();
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            PageSizeResponse::Cancel
        );
        assert_eq!(click(&mut r, Button::Cancel), PageSizeResponse::Cancel);
        assert_eq!(
            click(&mut r, Button::Ok),
            PageSizeResponse::Ok {
                width: 320,
                height: 200
            }
        );
        // Released away from the button: nothing happens.
        let b = Button::Ok.rect();
        r.handle(ReqEvent::Press(b.x + 3, b.y + 3));
        r.handle(ReqEvent::Drag(0, 0));
        assert_eq!(r.handle(ReqEvent::Release), PageSizeResponse::Continue);
    }

    #[test]
    fn origin_centres() {
        assert_eq!(PageSizeRequester::origin(320, 256), (52, 69));
        assert_eq!(PageSizeRequester::origin(320, 200), (52, 41));
    }

    #[test]
    fn render_has_size_and_title_pixels() {
        let p = pens();
        let img = req().render(&p);
        assert_eq!((img.width() as i32, img.height() as i32), (WIDTH, HEIGHT));
        let title_dark = (2..11)
            .flat_map(|y| (60..160).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get(x, y) == Some(p.dark))
            .count();
        assert!(title_dark > 0);
    }
}
