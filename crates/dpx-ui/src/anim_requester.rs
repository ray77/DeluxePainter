//! The Anim menu requesters (ADR-28): Set Frame Count, Add Frames, Copy Frames, Delete
//! Frames, Go to Frame and Set Rate. They are small forms (fields, radio buttons, Cancel / Ok)
//! in the style of the toolbox requesters (ADR-20). Frame numbers count from 1, as on screen;
//! the app turns the [`AnimChoice`] into document commands.
//!
//! Deviation from DeluxePaint IV: its Copy and Delete requesters act through buttons next to
//! each choice (Range, All Frames, …); here the choice is a radio button and Ok acts.

use dpx_core::{IndexedImage, Rect, anim};

use crate::font::Font;
use crate::form::{Form, FormResponse};
use crate::pens::Pens;
use crate::requester::ReqEvent;

/// Where [`Form`] draws its message line: this far above the bottom edge.
const MESSAGE_FROM_BOTTOM: i32 = 29;
const FIELD_W: i32 = 34;
const FIELD_H: i32 = 9;

/// What the user chose; frame numbers count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimChoice {
    /// Set Frame Count: the new number of frames.
    FrameCount(u32),
    /// Add Frames: how many.
    AddFrames(u32),
    /// Copy Frames: the current frame to every frame.
    CopyToAll,
    /// Copy Frames: the current frame to frames `first..=last`.
    CopyToRange {
        first: u32,
        last: u32,
    },
    /// Copy Frames: copies of frames `first..=last` inserted before frame `before`.
    InsertCopies {
        first: u32,
        last: u32,
        before: u32,
    },
    DeleteCurrent,
    DeleteRange {
        first: u32,
        last: u32,
    },
    DeleteAll,
    /// Go to Frame.
    Goto(u32),
    /// Set Rate: frames per second.
    Rate(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimReqResponse {
    Continue,
    Cancel,
    Ok(AnimChoice),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    FrameCount,
    AddFrames,
    Copy,
    Delete,
    Goto,
    Rate,
}

/// One Anim menu requester (modal).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimRequester {
    kind: Kind,
    form: Form,
    /// A message from the app (e.g. why a command was refused), until the next event.
    message: Option<String>,
}

/// A field box of the usual size at (`x`, `y`).
fn field_rect(x: i32, y: i32) -> Rect {
    Rect::new(x, y, FIELD_W, FIELD_H)
}

fn u32_of(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn text_w(s: &str) -> i32 {
    Font::builtin().text_width(s)
}

/// Left margin; a radio button's label starts [`RADIO_TEXT`] to the right of its box.
const LEFT: i32 = 8;
const RADIO_TEXT: i32 = 11;
const GAP: i32 = 6;
/// Narrowest requester (room for Cancel and Ok).
const MIN_W: i32 = 168;

/// A requester with one field: `label` [value] `note`.
fn single(title: &'static str, label: &str, range: (i32, i32), value: i32, note: &str) -> Form {
    let field_x = LEFT + text_w(label) + GAP;
    let note_x = field_x + FIELD_W + GAP;
    let w = (note_x + text_w(note) + LEFT)
        .max(text_w(title) + 4 * LEFT)
        .max(MIN_W);
    let mut form = Form::new(title, w, 64);
    form.field(
        label,
        (LEFT, 20),
        field_rect(field_x, 19),
        range,
        value,
        None,
    );
    form.text((note_x, 20), note);
    form
}

/// Columns of a "[from] to [to]" pair whose fields start at `from_x`: the "to" text, the
/// second field and the right edge of it.
fn pair_columns(from_x: i32) -> (i32, i32, i32) {
    let to_text = from_x + FIELD_W + GAP;
    let to_x = to_text + text_w("to") + GAP;
    (to_text, to_x, to_x + FIELD_W)
}

impl AnimRequester {
    /// Anim ▸ Frames ▸ Set #..: the frame count, 1 to `max`.
    pub fn frame_count(count: usize, max: usize) -> Self {
        let form = single(
            "Set Frame Count",
            "Count",
            (1, u32_of(max)),
            u32_of(count),
            &format!("1 to {max}"),
        );
        Self::new(Kind::FrameCount, form)
    }

    /// Anim ▸ Frames ▸ Add Frames..: how many copies of the current frame, 1 to `max`.
    pub fn add_frames(max: usize) -> Self {
        let form = single("Add Frames", "Add", (1, u32_of(max.max(1))), 1, "frames");
        Self::new(Kind::AddFrames, form)
    }

    /// Anim ▸ Frames ▸ Copy Frames.. for `count` frames, frame `current` (from 1) shown.
    pub fn copy_frames(count: usize, current: usize) -> Self {
        let (n, c) = (u32_of(count), u32_of(current));
        let frames = (1, n.max(1));
        let labels_end = (LEFT + RADIO_TEXT + text_w("Copy frames"))
            .max(LEFT + RADIO_TEXT + text_w("Insert before"));
        let from_x = labels_end + GAP;
        let (to_text, to_x, right) = pair_columns(from_x);
        let w = (right + LEFT)
            .max(2 * LEFT + text_w("Current frame to"))
            .max(MIN_W);
        let mut form = Form::new("Copy Frames", w, 112);
        form.text((LEFT, 17), "Current frame to");
        form.radio(0, 0, (LEFT, 29), "All frames", true);
        let r = form.radio(0, 1, (LEFT, 41), "Frames", false);
        let a = form.field(
            "From",
            (-1, 0),
            field_rect(from_x, 40),
            frames,
            1,
            Some((0, 1)),
        );
        form.text((to_text, 41), "to");
        form.field("To", (-1, 0), field_rect(to_x, 40), frames, n, Some((0, 1)));
        form.link(r, a);
        form.rule(54);
        let r = form.radio(0, 2, (LEFT, 58), "Copy frames", false);
        let b = form.field(
            "From",
            (-1, 0),
            field_rect(from_x, 57),
            frames,
            c,
            Some((0, 2)),
        );
        form.text((to_text, 58), "to");
        form.field("To", (-1, 0), field_rect(to_x, 57), frames, c, Some((0, 2)));
        form.link(r, b);
        form.text((LEFT + RADIO_TEXT, 70), "Insert before");
        form.field(
            "Before",
            (-1, 0),
            field_rect(from_x, 69),
            (1, n + 1),
            c,
            Some((0, 2)),
        );
        Self::new(Kind::Copy, form)
    }

    /// Anim ▸ Frames ▸ Delete Frames.. for `count` frames, frame `current` (from 1) shown.
    pub fn delete_frames(count: usize, current: usize) -> Self {
        let (n, c) = (u32_of(count), u32_of(current));
        let frames = (1, n.max(1));
        let from_x = LEFT + RADIO_TEXT + text_w("Frames") + GAP;
        let (to_text, to_x, right) = pair_columns(from_x);
        let w = (right + LEFT)
            .max(2 * LEFT + RADIO_TEXT + text_w("Current frame"))
            .max(MIN_W);
        let mut form = Form::new("Delete Frames", w, 84);
        form.radio(0, 0, (LEFT, 17), "Current frame", true);
        let r = form.radio(0, 1, (LEFT, 29), "Frames", false);
        let a = form.field(
            "From",
            (-1, 0),
            field_rect(from_x, 28),
            frames,
            c,
            Some((0, 1)),
        );
        form.text((to_text, 29), "to");
        form.field("To", (-1, 0), field_rect(to_x, 28), frames, n, Some((0, 1)));
        form.link(r, a);
        form.radio(0, 2, (LEFT, 41), "All frames", false);
        Self::new(Kind::Delete, form)
    }

    /// Anim ▸ Control ▸ Goto..: frame `current` (from 1) of `count` in the field.
    pub fn goto(count: usize, current: usize) -> Self {
        let n = u32_of(count).max(1);
        let form = single(
            "Go to Frame",
            "Frame",
            (1, n),
            u32_of(current),
            &format!("of {n}"),
        );
        Self::new(Kind::Goto, form)
    }

    /// Anim ▸ Control ▸ Set Rate..: frames per second (1 to 60, AGA p.255).
    pub fn rate(fps: u8) -> Self {
        let range = (i32::from(anim::MIN_FPS), i32::from(anim::MAX_FPS));
        let form = single(
            "Set Rate",
            "Rate",
            range,
            i32::from(fps),
            "frames per second",
        );
        Self::new(Kind::Rate, form)
    }

    fn new(kind: Kind, form: Form) -> Self {
        Self {
            kind,
            form,
            message: None,
        }
    }

    /// Shows `text` on the message line (e.g. why the command was refused).
    pub fn set_message(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
    }

    pub fn size(&self) -> (i32, i32) {
        self.form.size()
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(&self, screen_w: i32, screen_h: i32) -> (i32, i32) {
        let (w, h) = self.size();
        (((screen_w - w) / 2).max(0), ((screen_h - h) / 2).max(0))
    }

    /// Handles one event in requester coordinates (origin: [`AnimRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> AnimReqResponse {
        if !matches!(event, ReqEvent::Drag(..)) {
            self.message = None;
        }
        match self.form.handle(event) {
            FormResponse::Continue => AnimReqResponse::Continue,
            FormResponse::Cancel => AnimReqResponse::Cancel,
            FormResponse::Ok => match self.choice() {
                Ok(choice) => AnimReqResponse::Ok(choice),
                Err(why) => {
                    self.message = Some(why);
                    AnimReqResponse::Continue
                }
            },
        }
    }

    /// The choice after Ok; ranges must run forward.
    fn choice(&self) -> Result<AnimChoice, String> {
        let num = |i: usize| self.form.number(i).max(0) as u32;
        let range = |a: usize, b: usize| {
            if num(a) <= num(b) {
                Ok((num(a), num(b)))
            } else {
                Err(format!("From {} is after To {}", num(a), num(b)))
            }
        };
        Ok(match (self.kind, self.form.selected(0)) {
            (Kind::FrameCount, _) => AnimChoice::FrameCount(num(0)),
            (Kind::AddFrames, _) => AnimChoice::AddFrames(num(0)),
            (Kind::Goto, _) => AnimChoice::Goto(num(0)),
            (Kind::Rate, _) => AnimChoice::Rate(num(0).min(u32::from(u8::MAX)) as u8),
            (Kind::Copy, 0) => AnimChoice::CopyToAll,
            (Kind::Copy, 1) => {
                let (first, last) = range(0, 1)?;
                AnimChoice::CopyToRange { first, last }
            }
            (Kind::Copy, _) => {
                let (first, last) = range(2, 3)?;
                AnimChoice::InsertCopies {
                    first,
                    last,
                    before: num(4),
                }
            }
            (Kind::Delete, 0) => AnimChoice::DeleteCurrent,
            (Kind::Delete, 1) => {
                let (first, last) = range(0, 1)?;
                AnimChoice::DeleteRange { first, last }
            }
            (Kind::Delete, _) => AnimChoice::DeleteAll,
        })
    }

    /// Renders the requester alone.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let mut img = self.form.render(pens);
        if let Some(m) = &self.message {
            let (w, h) = self.size();
            let y = h - MESSAGE_FROM_BOTTOM;
            img.fill_rect(1, y, w - 2, FIELD_H, pens.face);
            Font::builtin().draw(&mut img, 6, y, m, pens.select);
        }
        img
    }

    /// Draws the requester centred on a UI screen.
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = self.origin(img.width() as i32, img.height() as i32);
        img.blit(&self.render(pens), x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requester::ReqKey;
    use dpx_core::Palette;

    fn key(r: &mut AnimRequester, k: ReqKey) -> AnimReqResponse {
        r.handle(ReqEvent::Key(k))
    }

    fn type_text(r: &mut AnimRequester, s: &str) {
        for c in s.chars() {
            key(r, ReqKey::Char(c));
        }
    }

    fn clear(r: &mut AnimRequester, n: usize) {
        for _ in 0..n {
            key(r, ReqKey::Backspace);
        }
    }

    fn click(r: &mut AnimRequester, x: i32, y: i32) {
        r.handle(ReqEvent::Press(x, y));
        r.handle(ReqEvent::Release);
    }

    #[test]
    fn frame_count_takes_a_number_within_the_limit() {
        let mut r = AnimRequester::frame_count(1, 999);
        clear(&mut r, 1);
        type_text(&mut r, "20");
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::FrameCount(20))
        );
        let mut r = AnimRequester::frame_count(5, 130);
        clear(&mut r, 1);
        type_text(&mut r, "131");
        assert_eq!(key(&mut r, ReqKey::Enter), AnimReqResponse::Continue);
        assert_eq!(key(&mut r, ReqKey::Escape), AnimReqResponse::Cancel);
    }

    #[test]
    fn simple_requesters_answer_their_choice() {
        let mut r = AnimRequester::add_frames(10);
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::AddFrames(1))
        );
        let mut r = AnimRequester::goto(12, 3);
        key(&mut r, ReqKey::Up);
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::Goto(4))
        );
        let mut r = AnimRequester::rate(30);
        clear(&mut r, 2);
        type_text(&mut r, "61");
        assert_eq!(key(&mut r, ReqKey::Enter), AnimReqResponse::Continue);
        clear(&mut r, 2);
        type_text(&mut r, "12");
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::Rate(12))
        );
    }

    #[test]
    fn copy_frames_offers_all_range_and_insert() {
        let mut r = AnimRequester::copy_frames(10, 3);
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::CopyToAll)
        );
        // "Frames" radio activates its From field: 4 to 10.
        click(&mut r, 10, 43);
        clear(&mut r, 1);
        type_text(&mut r, "4");
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::CopyToRange { first: 4, last: 10 })
        );
        // A backwards range keeps the requester open with a message.
        click(&mut r, 10, 43);
        clear(&mut r, 1);
        type_text(&mut r, "10");
        key(&mut r, ReqKey::Tab);
        clear(&mut r, 2);
        type_text(&mut r, "2");
        assert_eq!(key(&mut r, ReqKey::Enter), AnimReqResponse::Continue);
        assert!(r.message.as_deref().unwrap().contains("after"));
        // Insert copies of frame 3 before frame 3 (the defaults).
        click(&mut r, 10, 60);
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::InsertCopies {
                first: 3,
                last: 3,
                before: 3
            })
        );
    }

    #[test]
    fn delete_frames_current_range_or_all() {
        let mut r = AnimRequester::delete_frames(8, 2);
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::DeleteCurrent)
        );
        click(&mut r, 10, 31);
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::DeleteRange { first: 2, last: 8 })
        );
        click(&mut r, 10, 43);
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            AnimReqResponse::Ok(AnimChoice::DeleteAll)
        );
    }

    #[test]
    fn requesters_fit_and_show_messages() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        for r in [
            AnimRequester::frame_count(1, 999),
            AnimRequester::add_frames(998),
            AnimRequester::copy_frames(999, 999),
            AnimRequester::delete_frames(999, 1),
            AnimRequester::goto(999, 999),
            AnimRequester::rate(30),
        ] {
            let (w, h) = r.size();
            assert!(w <= 320 && h <= 200, "fits the smallest UI screen");
            let img = r.render(&pens);
            assert_eq!((img.width() as i32, img.height() as i32), (w, h));
            let mut with = r.clone();
            with.set_message("Too many frames");
            assert_ne!(with.render(&pens), img, "message drawn");
            let mut screen = IndexedImage::new(320, 200, 9);
            r.draw(&mut screen, &pens);
            let (x, y) = r.origin(320, 200);
            assert_eq!(screen.get(x, y), Some(pens.dark), "frame corner");
        }
    }
}
