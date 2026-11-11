//! The AnimBrush requesters (ADR-36): Pick up AnimBrush and Make AnimBrush (the number of
//! cels, AGA p.250 and p.243) and AnimBrush Settings (Number of cels, Duration, Current,
//! Direction; figure 8.28, AGA p.250-251). Small forms in the ADR-20 style with square
//! corners; the Settings requester draws the three Direction buttons (forward, backward,
//! ping-pong) as icons, as the figure shows. The app reads the values after Ok.

use dpx_core::{CelDirection, IndexedImage, Rect, anim_brush};

use crate::font::Font;
use crate::form::{Form, FormResponse};
use crate::pens::Pens;
use crate::requester::ReqEvent;
use crate::tool_requesters::ToolReqResponse;
use crate::widgets::button_face;

const LEFT: i32 = 8;
const FIELD_W: i32 = 34;
const FIELD_H: i32 = 9;
const GAP: i32 = 6;
/// The Direction buttons: size, gap, and their row in the Settings requester.
const DIR_W: i32 = 19;
const DIR_H: i32 = 11;
const DIR_GAP: i32 = 3;
const DIR_Y: i32 = 54;
/// Fields start here in the Settings requester (right of the longest label).
const SETTINGS_FIELD_X: i32 = 76;

/// Which requester.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimBrushReq {
    /// Anim ▸ AnimBrush ▸ Pick up.. (or Alt with the Brush Selector): how many cels.
    PickUp,
    /// Brush ▸ Spare ▸ Metamorph..: how many cels the morph gets.
    Make,
    /// Anim ▸ AnimBrush ▸ Settings..
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnimBrushRequester {
    kind: AnimBrushReq,
    form: Form,
    direction: CelDirection,
    /// The Direction button the mouse button went down on.
    held: Option<CelDirection>,
    /// The pointer is still over that button.
    over: bool,
}

fn text_w(s: &str) -> i32 {
    Font::builtin().text_width(s)
}

fn int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

const DIRECTIONS: [CelDirection; 3] = [
    CelDirection::Forward,
    CelDirection::Backward,
    CelDirection::PingPong,
];

impl AnimBrushRequester {
    /// "Number of cels" between 2 and `max`, `value` in the field.
    fn cel_count(kind: AnimBrushReq, title: &'static str, value: usize, max: usize) -> Self {
        let max = max.max(anim_brush::MIN_CELS);
        let label = "Number of cels";
        let note = format!("2 to {max}");
        let field_x = LEFT + text_w(label) + GAP;
        let note_x = field_x + FIELD_W + GAP;
        // Wide enough for the form's message about a number out of range, too.
        let refused = format!("{label} must be {} to {max}", anim_brush::MIN_CELS);
        let w = (note_x + text_w(&note) + LEFT)
            .max(text_w(title) + 4 * LEFT)
            .max(text_w(&refused) + 2 * LEFT)
            .max(168);
        let mut form = Form::new(title, w, 64);
        form.field(
            label,
            (LEFT, 20),
            Rect::new(field_x, 19, FIELD_W, FIELD_H),
            (int(anim_brush::MIN_CELS), int(max)),
            int(value.clamp(anim_brush::MIN_CELS, max)),
            None,
        );
        form.text((note_x, 20), &note);
        Self {
            kind,
            form,
            direction: CelDirection::Forward,
            held: None,
            over: false,
        }
    }

    /// Pick up AnimBrush: `value` cels suggested, at most `max` (the frames there are).
    pub fn pick_up(value: usize, max: usize) -> Self {
        Self::cel_count(AnimBrushReq::PickUp, "Pick up AnimBrush", value, max)
    }

    /// Make AnimBrush (Metamorph): `value` cels suggested, at most `max` (the cels that fit
    /// the AnimBrush byte limit, [`anim_brush::max_morph_cels`]).
    pub fn make(value: usize, max: usize) -> Self {
        let max = max.min(anim_brush::MAX_CELS);
        Self::cel_count(AnimBrushReq::Make, "Make AnimBrush", value, max)
    }

    /// AnimBrush Settings for a brush of `cels` cels: Duration, the current cel (from 1) and
    /// the direction.
    pub fn settings(cels: usize, duration: u32, current: usize, direction: CelDirection) -> Self {
        let mut form = Form::new("AnimBrush Settings", 168, 96);
        form.text((LEFT, 17), &format!("Number of cels: {cels}"));
        form.field(
            "Duration",
            (LEFT, 30),
            Rect::new(SETTINGS_FIELD_X, 29, FIELD_W, FIELD_H),
            (1, anim_brush::MAX_DURATION as i32),
            duration as i32,
            None,
        );
        form.text((SETTINGS_FIELD_X + FIELD_W + GAP, 30), "stamps");
        form.field(
            "Current",
            (LEFT, 42),
            Rect::new(SETTINGS_FIELD_X, 41, FIELD_W, FIELD_H),
            (1, int(cels.max(1))),
            int(current.clamp(1, cels.max(1))),
            None,
        );
        form.text(
            (SETTINGS_FIELD_X + FIELD_W + GAP, 42),
            &format!("of {cels}"),
        );
        form.text((LEFT, DIR_Y + 2), "Direction");
        Self {
            kind: AnimBrushReq::Settings,
            form,
            direction,
            held: None,
            over: false,
        }
    }

    pub fn kind(&self) -> AnimBrushReq {
        self.kind
    }

    /// Pick up / Make: the number of cels (valid after Ok).
    pub fn cels(&self) -> usize {
        usize::try_from(self.form.number(0)).unwrap_or(anim_brush::MIN_CELS)
    }

    /// Settings: the Duration (valid after Ok).
    pub fn duration(&self) -> u32 {
        u32::try_from(self.form.number(0)).unwrap_or(1)
    }

    /// Settings: the current cel, from 1 (valid after Ok).
    pub fn current(&self) -> usize {
        usize::try_from(self.form.number(1)).unwrap_or(1)
    }

    /// Settings: the chosen direction.
    pub fn direction(&self) -> CelDirection {
        self.direction
    }

    pub fn size(&self) -> (i32, i32) {
        self.form.size()
    }

    /// The Direction button of `d` (Settings only), in requester coordinates.
    pub fn direction_rect(d: CelDirection) -> Rect {
        let i = DIRECTIONS.iter().position(|&x| x == d).unwrap_or(0) as i32;
        Rect::new(
            SETTINGS_FIELD_X + i * (DIR_W + DIR_GAP),
            DIR_Y,
            DIR_W,
            DIR_H,
        )
    }

    fn direction_at(&self, x: i32, y: i32) -> Option<CelDirection> {
        if self.kind != AnimBrushReq::Settings {
            return None;
        }
        DIRECTIONS
            .into_iter()
            .find(|&d| Self::direction_rect(d).contains(x, y))
    }

    /// Handles one event in requester coordinates. A Direction button is chosen when the
    /// button goes up over the one it went down on (like the Cancel and Ok buttons);
    /// everything else goes to the form.
    pub fn handle(&mut self, event: ReqEvent) -> ToolReqResponse {
        match event {
            ReqEvent::Press(x, y) if self.direction_at(x, y).is_some() => {
                self.held = self.direction_at(x, y);
                self.over = true;
                return ToolReqResponse::Continue;
            }
            ReqEvent::Drag(x, y) if self.held.is_some() => {
                self.over = self.direction_at(x, y) == self.held;
                return ToolReqResponse::Continue;
            }
            ReqEvent::Release if self.held.is_some() => {
                if let Some(d) = self.held.take()
                    && self.over
                {
                    self.direction = d;
                }
                return ToolReqResponse::Continue;
            }
            _ => {}
        }
        match self.form.handle(event) {
            FormResponse::Continue => ToolReqResponse::Continue,
            FormResponse::Cancel => ToolReqResponse::Cancel,
            FormResponse::Ok => ToolReqResponse::Ok,
        }
    }

    /// Chooses a direction as a click on its button does.
    pub fn choose(&mut self, d: CelDirection) {
        self.direction = d;
    }

    /// Renders the requester alone.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let mut img = self.form.render(pens);
        if self.kind == AnimBrushReq::Settings {
            for d in DIRECTIONS {
                let r = Self::direction_rect(d);
                let on = d == self.direction || (self.held == Some(d) && self.over);
                let ink = button_face(&mut img, r, on, pens);
                draw_direction(&mut img, r, d, ink);
            }
        }
        img
    }
}

/// A triangle 4 pixels wide and 7 high with its tip at `tip_x` (pointing right or left).
fn triangle(img: &mut IndexedImage, tip_x: i32, cy: i32, right: bool, pen: u8) {
    for i in 0..4 {
        let x = if right { tip_x - i } else { tip_x + i };
        img.vline(x, cy - i, cy + i, pen);
    }
}

/// The icon of a Direction button: ▶ forward, ◀ backward, ◀▶ ping-pong.
fn draw_direction(img: &mut IndexedImage, r: Rect, d: CelDirection, pen: u8) {
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    match d {
        CelDirection::Forward => triangle(img, cx + 1, cy, true, pen),
        CelDirection::Backward => triangle(img, cx - 2, cy, false, pen),
        CelDirection::PingPong => {
            triangle(img, cx - 5, cy, false, pen);
            triangle(img, cx + 4, cy, true, pen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requester::ReqKey;
    use dpx_core::Palette;

    fn key(r: &mut AnimBrushRequester, k: ReqKey) -> ToolReqResponse {
        r.handle(ReqEvent::Key(k))
    }

    fn retype(r: &mut AnimBrushRequester, text: &str) {
        for _ in 0..4 {
            key(r, ReqKey::Backspace);
        }
        for c in text.chars() {
            key(r, ReqKey::Char(c));
        }
    }

    fn click(r: &mut AnimBrushRequester, p: (i32, i32)) -> ToolReqResponse {
        r.handle(ReqEvent::Press(p.0, p.1));
        r.handle(ReqEvent::Release)
    }

    #[test]
    fn cel_counts_from_two_to_the_frames_there_are() {
        let mut r = AnimBrushRequester::pick_up(20, 20);
        assert_eq!(r.kind(), AnimBrushReq::PickUp);
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        assert_eq!(r.cels(), 20);
        retype(&mut r, "1");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Continue);
        retype(&mut r, "21");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Continue);
        retype(&mut r, "5");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        assert_eq!(r.cels(), 5);
        assert_eq!(key(&mut r, ReqKey::Escape), ToolReqResponse::Cancel);
        let mut m = AnimBrushRequester::make(11, anim_brush::MAX_CELS);
        assert_eq!(m.kind(), AnimBrushReq::Make);
        retype(&mut m, "999");
        assert_eq!(key(&mut m, ReqKey::Enter), ToolReqResponse::Ok);
        assert_eq!(m.cels(), 999);
        // Big brushes: no more cels than fit the byte limit.
        let mut m = AnimBrushRequester::make(200, 134);
        assert_eq!(m.cels(), 134, "the suggestion is cut to the most");
        retype(&mut m, "135");
        assert_eq!(key(&mut m, ReqKey::Enter), ToolReqResponse::Continue);
        let refused = text_w("Number of cels must be 2 to 134");
        assert!(m.size().0 >= refused + 2 * LEFT, "the message fits");
        retype(&mut m, "134");
        assert_eq!(key(&mut m, ReqKey::Enter), ToolReqResponse::Ok);
    }

    #[test]
    fn settings_edit_duration_current_and_direction() {
        let mut r = AnimBrushRequester::settings(20, 20, 3, CelDirection::Forward);
        retype(&mut r, "40");
        key(&mut r, ReqKey::Tab);
        retype(&mut r, "21");
        assert_eq!(
            key(&mut r, ReqKey::Enter),
            ToolReqResponse::Continue,
            "21 of 20"
        );
        retype(&mut r, "7");
        let back = AnimBrushRequester::direction_rect(CelDirection::Backward);
        assert_eq!(
            click(&mut r, (back.x + 2, back.y + 2)),
            ToolReqResponse::Continue
        );
        assert_eq!(r.direction(), CelDirection::Backward);
        let pp = AnimBrushRequester::direction_rect(CelDirection::PingPong);
        click(&mut r, (pp.x + 2, pp.y + 2));
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        assert_eq!(
            (r.duration(), r.current(), r.direction()),
            (40, 7, CelDirection::PingPong)
        );
        // Pressing a button and letting go elsewhere changes nothing.
        let fwd = AnimBrushRequester::direction_rect(CelDirection::Forward);
        r.handle(ReqEvent::Press(fwd.x + 1, fwd.y + 1));
        r.handle(ReqEvent::Drag(1, 1));
        r.handle(ReqEvent::Release);
        assert_eq!(r.direction(), CelDirection::PingPong);
    }

    #[test]
    fn requesters_fit_and_show_the_chosen_direction() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        for r in [
            AnimBrushRequester::pick_up(2, 999),
            AnimBrushRequester::make(8, anim_brush::MAX_CELS),
            AnimBrushRequester::settings(999, 999, 999, CelDirection::Backward),
        ] {
            let (w, h) = r.size();
            assert!(w <= 320 && h <= 200, "fits the smallest UI screen");
            let img = r.render(&pens);
            assert_eq!((img.width() as i32, img.height() as i32), (w, h));
        }
        let mut r = AnimBrushRequester::settings(4, 4, 1, CelDirection::Forward);
        let fwd = AnimBrushRequester::direction_rect(CelDirection::Forward);
        let back = AnimBrushRequester::direction_rect(CelDirection::Backward);
        let img = r.render(&pens);
        assert_eq!(
            img.get(fwd.x + 1, fwd.y + 1),
            Some(pens.select),
            "chosen: pressed in"
        );
        assert_eq!(img.get(back.x + 1, back.y + 1), Some(pens.face));
        r.choose(CelDirection::Backward);
        let img = r.render(&pens);
        assert_eq!(img.get(back.x + 1, back.y + 1), Some(pens.select));
    }
}
