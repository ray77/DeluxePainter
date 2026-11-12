//! Shared requester widgets drawn in UI (LoRes) pixels with palette-derived pens.

use dpx_core::{IndexedImage, Rect};

use crate::font::{Font, GAP, SPACE_ADVANCE};
use crate::pens::Pens;
use crate::requester::ReqKey;

/// An Amiga-style string gadget: text plus cursor (character index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextField {
    pub chars: Vec<char>,
    pub cursor: usize,
    /// Most characters the field accepts.
    max: usize,
}

impl TextField {
    /// An empty field accepting at most `max` characters.
    pub fn new(max: usize) -> Self {
        Self {
            chars: Vec::new(),
            cursor: 0,
            max,
        }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// Replaces the text; the cursor goes to the end.
    pub fn set(&mut self, text: &str) {
        self.chars = text.chars().take(self.max).collect();
        self.cursor = self.chars.len();
    }

    pub fn key(&mut self, key: ReqKey) {
        match key {
            ReqKey::Char(c) | ReqKey::Paste(c)
                if !c.is_control() && self.chars.len() < self.max =>
            {
                self.chars.insert(self.cursor, c);
                self.cursor += 1;
            }
            ReqKey::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.chars.remove(self.cursor);
            }
            ReqKey::Delete if self.cursor < self.chars.len() => {
                self.chars.remove(self.cursor);
            }
            ReqKey::Left => self.cursor = self.cursor.saturating_sub(1),
            ReqKey::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            ReqKey::Home => self.cursor = 0,
            ReqKey::End => self.cursor = self.chars.len(),
            _ => {}
        }
    }

    /// Width of the cursor cell at `i`: the character there, or a space at the end.
    pub fn cell_width(&self, i: usize) -> i32 {
        self.chars
            .get(i)
            .map_or(SPACE_ADVANCE, |&c| Font::builtin().char_advance(c))
    }

    /// First visible character so the cursor cell fits into `width` pixels.
    pub fn first_visible(&self, width: i32) -> usize {
        let font = Font::builtin();
        let mut start = 0;
        loop {
            let w: i32 = self.chars[start..self.cursor]
                .iter()
                .map(|&c| font.char_advance(c))
                .sum::<i32>()
                + self.cell_width(self.cursor);
            if w <= width || start >= self.cursor {
                return start;
            }
            start += 1;
        }
    }
}

pub(crate) trait Inset {
    fn inset(self, d: i32) -> Rect;
}

impl Inset for Rect {
    fn inset(self, d: i32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2 * d, self.h - 2 * d)
    }
}

/// One-pixel outline.
pub(crate) fn frame(img: &mut IndexedImage, r: Rect, pen: u8) {
    let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
    img.hline(r.x, x1, r.y, pen);
    img.hline(r.x, x1, y1, pen);
    img.vline(r.x, r.y, y1, pen);
    img.vline(x1, r.y, y1, pen);
}

/// Draws a button background (raised, or recessed and filled while pressed); returns the ink
/// for its label.
pub(crate) fn button_face(img: &mut IndexedImage, r: Rect, pressed: bool, pens: &Pens) -> u8 {
    let (fill, top_left, bottom_right, ink) = if pressed {
        (pens.select, pens.dark, pens.shine, pens.shine)
    } else {
        (pens.face, pens.shine, pens.dark, pens.dark)
    };
    img.fill_rect(r.x, r.y, r.w, r.h, fill);
    let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
    img.hline(r.x, x1, r.y, top_left);
    img.vline(r.x, r.y, y1, top_left);
    img.hline(r.x, x1, y1, bottom_right);
    img.vline(x1, r.y, y1, bottom_right);
    ink
}

pub(crate) fn draw_button(
    img: &mut IndexedImage,
    r: Rect,
    label: &str,
    pressed: bool,
    pens: &Pens,
) {
    let ink = button_face(img, r, pressed, pens);
    centred(img, r.x, r.w, r.y + (r.h - 8) / 2, label, ink);
}

/// A string gadget: light text on a dark bar; the active one shows an inverse block cursor.
pub(crate) fn draw_field(
    img: &mut IndexedImage,
    r: Rect,
    f: &TextField,
    active: bool,
    pens: &Pens,
) {
    let font = Font::builtin();
    img.fill_rect(r.x, r.y, r.w, r.h, pens.dark);
    let start = f.first_visible(r.w - 4);
    let right = r.x + r.w - 2;
    let mut x = r.x + 2;
    for (i, &c) in f.chars.iter().enumerate().skip(start) {
        let adv = font.char_advance(c);
        if x + adv - GAP > right {
            break;
        }
        if active && i == f.cursor {
            img.fill_rect(x - 1, r.y, adv, r.h, pens.shine);
            font.draw_char(img, x, r.y + 1, c, pens.dark);
        } else {
            font.draw_char(img, x, r.y + 1, c, pens.shine);
        }
        x += adv;
    }
    if active && f.cursor == f.chars.len() {
        img.fill_rect(x - 1, r.y, SPACE_ADVANCE - GAP, r.h, pens.shine);
    }
}

/// Draws characters while they fit into `max_w` pixels.
pub(crate) fn draw_clipped(
    img: &mut IndexedImage,
    x: i32,
    y: i32,
    text: impl Iterator<Item = char>,
    max_w: i32,
    pen: u8,
) {
    let font = Font::builtin();
    let mut cx = x;
    for c in text {
        if cx + font.char_advance(c) - GAP > x + max_w {
            break;
        }
        cx = font.draw_char(img, cx, y, c, pen);
    }
}

pub(crate) fn centred(img: &mut IndexedImage, x: i32, w: i32, y: i32, text: &str, pen: u8) {
    let font = Font::builtin();
    font.draw(img, x + (w - font.text_width(text)) / 2, y, text, pen);
}

/// Ghosts an area: every other pixel becomes `pen`.
pub(crate) fn checker(img: &mut IndexedImage, r: Rect, pen: u8) {
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            if (x + y) % 2 == 0 {
                img.set(x, y, pen);
            }
        }
    }
}
