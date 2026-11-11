//! The Anim control requesters (ADR-38): Anim ▸ Control ▸ Set Range.. ("Set Play Range")
//! and Anim ▸ Move.. ("Move"). Drawn in UI (LoRes) pixels with square corners and
//! palette-derived pens, centred on the screen and modal, like the other Anim requesters
//! (ADR-28). The app turns their answers into document commands and playback settings.
//!
//! Set Range follows the Copy Frames deviation of ADR-28: DeluxePaint acts through a Range
//! and an All Frames button (tutorial p.182); here they are radio buttons and Ok acts.

use dpx_core::anim_move::{MAX_AMOUNT, MAX_COUNT, MAX_EASE, MovePath, Record};
use dpx_core::{IndexedImage, Rect};

use crate::anim_panel::{GO_TO, draw_icon};
use crate::font::Font;
use crate::form::{Form, FormResponse};
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{TextField, button_face, centred, checker, draw_button, draw_field, frame};

// ------------------------------------------------------------------------- Set Range

/// Set Range's answer; frames count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayRange {
    /// Every frame (the default).
    All,
    /// Frames `from..=to`, `from < to`.
    Frames { from: u32, to: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayRangeResponse {
    Continue,
    Cancel,
    Ok(PlayRange),
}

/// Anim ▸ Control ▸ Set Range..: the frames Play once and Ping-pong play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayRangeRequester {
    form: Form,
    message: Option<String>,
}

const LEFT: i32 = 8;
const FIELD_W: i32 = 34;
const FIELD_H: i32 = 9;

fn text_w(s: &str) -> i32 {
    Font::builtin().text_width(s)
}

fn as_i32(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

impl PlayRangeRequester {
    /// For an animation of `count` frames; `range` = the current range (from 1), if any.
    pub fn new(count: usize, range: Option<(usize, usize)>) -> Self {
        let n = as_i32(count).max(1);
        let (from, to) = range.map_or((1, n), |(a, b)| (as_i32(a), as_i32(b)));
        let from_x = LEFT + 11 + text_w("Frames") + 6;
        let to_text = from_x + FIELD_W + 6;
        let to_x = to_text + text_w("to") + 6;
        let w = (to_x + FIELD_W + LEFT).max(168);
        let mut form = Form::new("Set Play Range", w, 72);
        form.radio(0, 0, (LEFT, 17), "All frames", range.is_none());
        let r = form.radio(0, 1, (LEFT, 29), "Frames", range.is_some());
        let a = form.field(
            "From",
            (-1, 0),
            Rect::new(from_x, 28, FIELD_W, FIELD_H),
            (1, n),
            from,
            Some((0, 1)),
        );
        form.text((to_text, 29), "to");
        form.field(
            "To",
            (-1, 0),
            Rect::new(to_x, 28, FIELD_W, FIELD_H),
            (1, n),
            to,
            Some((0, 1)),
        );
        form.link(r, a);
        Self {
            form,
            message: None,
        }
    }

    pub fn size(&self) -> (i32, i32) {
        self.form.size()
    }

    /// Handles one event in requester coordinates.
    pub fn handle(&mut self, event: ReqEvent) -> PlayRangeResponse {
        if !matches!(event, ReqEvent::Drag(..)) {
            self.message = None;
        }
        match self.form.handle(event) {
            FormResponse::Continue => PlayRangeResponse::Continue,
            FormResponse::Cancel => PlayRangeResponse::Cancel,
            FormResponse::Ok if self.form.selected(0) == 0 => PlayRangeResponse::Ok(PlayRange::All),
            FormResponse::Ok => {
                let (from, to) = (self.form.number(0), self.form.number(1));
                if from < to {
                    PlayRangeResponse::Ok(PlayRange::Frames {
                        from: from as u32,
                        to: to as u32,
                    })
                } else {
                    // The manual: From must be smaller than To (AGA p.257).
                    self.message = Some(format!("From {from} must be before To {to}"));
                    PlayRangeResponse::Continue
                }
            }
        }
    }

    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let mut img = self.form.render(pens);
        if let Some(m) = &self.message {
            let (w, h) = self.size();
            let y = h - 29;
            img.fill_rect(1, y, w - 2, FIELD_H, pens.face);
            Font::builtin().draw(&mut img, 6, y, m, pens.select);
        }
        img
    }
}

// ------------------------------------------------------------------------------ Move

/// The Move requester's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveButton {
    Clear,
    GoBack,
    Preview,
    Trails,
    /// Ghosted: a filled perspective plane is not implemented.
    Fill,
    Draw,
    Cancel,
    /// Ghosted: the move file format is not described.
    Load,
    Save,
    Exit,
}

/// What the Move requester asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveResponse {
    Continue,
    /// Close; the settings from before opening stay.
    Cancel,
    /// Close and keep these settings.
    Exit(MovePath),
    /// Show the move as a wire frame, then come back (the settings are not kept yet).
    Preview(MovePath),
    /// Paint the move (with `trails` set by the Trails button), close and keep the settings.
    Draw(MovePath),
    /// Start the next move where the brush was last clicked down again (stays open).
    GoBack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Field(usize),
    DistBrush,
    AngleBrush,
    Cyclic,
    GoFrom,
    ComeTo,
    Record(Record),
    Button(MoveButton),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NumField {
    label: &'static str,
    rect: Rect,
    min: i32,
    max: i32,
    text: TextField,
}

/// Move requester geometry (requester pixels).
mod geom {
    pub const W: i32 = 248;
    pub const H: i32 = 142;
    pub const COL_X: [i32; 3] = [54, 98, 142];
    pub const NUM_W: i32 = 40;
    pub const ROW_H: i32 = 9;
    pub const DIST_Y: i32 = 24;
    pub const ANGLE_Y: i32 = 35;
    pub const CHECK_X: i32 = 188;
    pub const SMALL_Y: i32 = 47;
    pub const SMALL_W: i32 = 56;
    pub const SMALL_H: i32 = 11;
    pub const RULE_1: i32 = 61;
    pub const EASE_X: i32 = 78;
    pub const EASE_W: i32 = 30;
    pub const LOWER_Y: [i32; 3] = [65, 76, 87];
    pub const DIR_X: i32 = 126;
    pub const RECORD_X: i32 = 174;
    pub const ICON_Y: i32 = 86;
    pub const ICON_W: i32 = 20;
    pub const RULE_2: i32 = 100;
    pub const MESSAGE_Y: i32 = 102;
    pub const BUTTON_Y: [i32; 2] = [112, 126];
    pub const BUTTON_W: i32 = 56;
    pub const BUTTON_H: i32 = 12;
    pub const BUTTON_X: [i32; 4] = [6, 66, 126, 186];
}

/// The bottom two rows of buttons, left to right.
const BUTTON_ROWS: [[MoveButton; 4]; 2] = [
    [
        MoveButton::Preview,
        MoveButton::Trails,
        MoveButton::Fill,
        MoveButton::Draw,
    ],
    [
        MoveButton::Cancel,
        MoveButton::Load,
        MoveButton::Save,
        MoveButton::Exit,
    ],
];

fn button_label(b: MoveButton) -> &'static str {
    match b {
        MoveButton::Clear => "Clear",
        MoveButton::GoBack => "Go Back",
        MoveButton::Preview => "Preview",
        MoveButton::Trails => "Trails",
        MoveButton::Fill => "Fill",
        MoveButton::Draw => "Draw",
        MoveButton::Cancel => "Cancel",
        MoveButton::Load => "Load",
        MoveButton::Save => "Save",
        MoveButton::Exit => "Exit",
    }
}

/// Direction icons, 9×7 (`#` = ink).
const GO_FROM: [&str; 7] = [
    ".........",
    ".##...#..",
    "####...#.",
    "#########",
    "####...#.",
    ".##...#..",
    ".........",
];
const FORWARD: [&str; 7] = [
    "#...#....",
    ".#...#...",
    "..#...#..",
    "...#...#.",
    "..#...#..",
    ".#...#...",
    "#...#....",
];
const IN_PLACE: [&str; 7] = [
    "..##.##..",
    "..##.##..",
    "..##.##..",
    "..##.##..",
    "..##.##..",
    "..##.##..",
    "..##.##..",
];

/// Anim ▸ Move.. (AGA p.252-255).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveRequester {
    /// Dist X/Y/Z, Angle X/Y/Z, Ease-Out, Ease-In, Count.
    fields: Vec<NumField>,
    dist_brush: bool,
    angle_brush: bool,
    cyclic: bool,
    come_to: bool,
    record: Record,
    /// More than one frame: Record and Trails are available.
    frames: bool,
    active: Option<usize>,
    held: Option<Target>,
    pointer: (i32, i32),
    message: Option<String>,
}

impl MoveRequester {
    /// The requester showing `path`; `frames` = the animation's frame count.
    pub fn new(path: &MovePath, frames: usize) -> Self {
        let mut fields = Vec::new();
        let mut num = |label, rect, (min, max): (i32, i32), value: i32| {
            let digits = max.to_string().len() + usize::from(min < 0);
            let mut text = TextField::new(digits);
            text.set(&value.to_string());
            fields.push(NumField {
                label,
                rect,
                min,
                max,
                text,
            });
        };
        let amount = (-MAX_AMOUNT, MAX_AMOUNT);
        for (i, axis) in ["Dist X", "Dist Y", "Dist Z"].into_iter().enumerate() {
            let r = Rect::new(geom::COL_X[i], geom::DIST_Y, geom::NUM_W, geom::ROW_H);
            num(axis, r, amount, path.dist[i]);
        }
        for (i, axis) in ["Angle X", "Angle Y", "Angle Z"].into_iter().enumerate() {
            let r = Rect::new(geom::COL_X[i], geom::ANGLE_Y, geom::NUM_W, geom::ROW_H);
            num(axis, r, amount, path.angle[i]);
        }
        let ease = (0, MAX_EASE as i32);
        let lower = |i: usize| {
            Rect::new(
                geom::EASE_X,
                geom::LOWER_Y[i] - 1,
                geom::EASE_W,
                geom::ROW_H,
            )
        };
        num("Ease-Out", lower(0), ease, path.ease_out as i32);
        num("Ease-In", lower(1), ease, path.ease_in as i32);
        num("Count", lower(2), (1, MAX_COUNT as i32), path.count as i32);
        Self {
            fields,
            dist_brush: path.dist_brush,
            angle_brush: path.angle_brush,
            cyclic: path.cyclic,
            come_to: path.come_to,
            record: path.record,
            frames: frames > 1,
            active: Some(0),
            held: None,
            pointer: (0, 0),
            message: None,
        }
    }

    pub fn size(&self) -> (i32, i32) {
        (geom::W, geom::H)
    }

    /// Shows `text` on the message line (e.g. why the move was refused).
    pub fn set_message(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
    }

    fn check_rect(y: i32, label: &str) -> Rect {
        Rect::new(geom::CHECK_X, y, 11 + text_w(label), geom::ROW_H)
    }

    fn small_rect(i: i32) -> Rect {
        Rect::new(
            LEFT + i * (geom::SMALL_W + 6),
            geom::SMALL_Y,
            geom::SMALL_W,
            geom::SMALL_H,
        )
    }

    fn icon_rect(x: i32, i: i32) -> Rect {
        Rect::new(
            x + i * geom::ICON_W,
            geom::ICON_Y,
            geom::ICON_W,
            geom::SMALL_H,
        )
    }

    fn button_rect(b: MoveButton) -> Rect {
        match b {
            MoveButton::Clear => Self::small_rect(0),
            MoveButton::GoBack => Self::small_rect(1),
            _ => {
                let (row, col) = BUTTON_ROWS
                    .iter()
                    .enumerate()
                    .find_map(|(r, row)| row.iter().position(|&x| x == b).map(|c| (r, c)))
                    .expect("a bottom button");
                Rect::new(
                    geom::BUTTON_X[col],
                    geom::BUTTON_Y[row],
                    geom::BUTTON_W,
                    geom::BUTTON_H,
                )
            }
        }
    }

    fn enabled(&self, t: Target) -> bool {
        match t {
            Target::Button(MoveButton::Fill | MoveButton::Load | MoveButton::Save) => false,
            Target::Button(MoveButton::Trails) | Target::Record(_) => self.frames,
            _ => true,
        }
    }

    /// Every target with its rectangle.
    fn targets(&self) -> Vec<(Target, Rect)> {
        let mut out: Vec<(Target, Rect)> = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| (Target::Field(i), f.rect))
            .collect();
        out.push((Target::DistBrush, Self::check_rect(geom::DIST_Y, "Brush")));
        out.push((Target::AngleBrush, Self::check_rect(geom::ANGLE_Y, "Brush")));
        out.push((
            Target::Cyclic,
            Self::check_rect(geom::SMALL_Y + 1, "Cyclic"),
        ));
        out.push((Target::GoFrom, Self::icon_rect(geom::DIR_X, 0)));
        out.push((Target::ComeTo, Self::icon_rect(geom::DIR_X, 1)));
        for (i, r) in [Record::Forward, Record::InPlace, Record::Backward]
            .into_iter()
            .enumerate()
        {
            out.push((Target::Record(r), Self::icon_rect(geom::RECORD_X, i as i32)));
        }
        out.push((
            Target::Button(MoveButton::Clear),
            Self::button_rect(MoveButton::Clear),
        ));
        out.push((
            Target::Button(MoveButton::GoBack),
            Self::button_rect(MoveButton::GoBack),
        ));
        for b in BUTTON_ROWS.iter().flatten() {
            out.push((Target::Button(*b), Self::button_rect(*b)));
        }
        out
    }

    fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        self.targets()
            .into_iter()
            .find(|&(t, r)| r.contains(x, y) && self.enabled(t))
            .map(|(t, _)| t)
    }

    fn pressed(&self, t: Target) -> bool {
        self.held == Some(t) && self.target_at(self.pointer.0, self.pointer.1) == Some(t)
    }

    /// The settings as typed, or why a field is out of range (the field becomes active).
    fn path(&mut self, trails: bool) -> Option<MovePath> {
        for (i, f) in self.fields.iter().enumerate() {
            let ok = f
                .text
                .text()
                .parse::<i32>()
                .is_ok_and(|n| (f.min..=f.max).contains(&n));
            if !ok {
                self.message = Some(format!("{} must be {} to {}", f.label, f.min, f.max));
                self.active = Some(i);
                return None;
            }
        }
        let n = |i: usize| self.fields[i].text.text().parse::<i32>().unwrap_or(0);
        Some(MovePath {
            dist: [n(0), n(1), n(2)],
            angle: [n(3), n(4), n(5)],
            dist_brush: self.dist_brush,
            angle_brush: self.angle_brush,
            cyclic: self.cyclic,
            ease_out: n(6) as u32,
            ease_in: n(7) as u32,
            count: n(8) as u32,
            come_to: self.come_to,
            record: self.record,
            trails: trails && self.frames,
        })
    }

    fn activate(&mut self, i: usize) {
        self.active = Some(i);
        let f = &mut self.fields[i];
        f.text.cursor = f.text.chars.len();
    }

    fn click(&mut self, t: Target) -> MoveResponse {
        match t {
            Target::Field(i) => self.activate(i),
            Target::DistBrush => self.dist_brush = !self.dist_brush,
            Target::AngleBrush => self.angle_brush = !self.angle_brush,
            Target::Cyclic => self.cyclic = !self.cyclic,
            Target::GoFrom => self.come_to = false,
            Target::ComeTo => self.come_to = true,
            Target::Record(r) => self.record = r,
            Target::Button(b) => return self.button(b),
        }
        MoveResponse::Continue
    }

    fn button(&mut self, b: MoveButton) -> MoveResponse {
        match b {
            MoveButton::Clear => {
                // Zero every Dist and Angle number (AGA p.253).
                for f in &mut self.fields[..6] {
                    f.text.set("0");
                }
                MoveResponse::Continue
            }
            MoveButton::GoBack => MoveResponse::GoBack,
            MoveButton::Preview => self
                .path(false)
                .map_or(MoveResponse::Continue, MoveResponse::Preview),
            MoveButton::Trails => self
                .path(true)
                .map_or(MoveResponse::Continue, MoveResponse::Draw),
            MoveButton::Draw => self
                .path(false)
                .map_or(MoveResponse::Continue, MoveResponse::Draw),
            MoveButton::Exit => self
                .path(false)
                .map_or(MoveResponse::Continue, MoveResponse::Exit),
            MoveButton::Cancel => MoveResponse::Cancel,
            MoveButton::Fill | MoveButton::Load | MoveButton::Save => MoveResponse::Continue,
        }
    }

    /// Handles one event in requester coordinates. Return draws (the requester's main
    /// action, like Ok elsewhere), Esc cancels, Tab steps through the numbers, Up/Down
    /// change the active one.
    pub fn handle(&mut self, event: ReqEvent) -> MoveResponse {
        if !matches!(event, ReqEvent::Drag(..)) {
            self.message = None;
        }
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.target_at(x, y);
                // Fields, boxes and icons act on press, buttons on release.
                match self.held {
                    Some(Target::Button(_)) | None => MoveResponse::Continue,
                    Some(t) => {
                        self.held = None;
                        self.click(t)
                    }
                }
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                MoveResponse::Continue
            }
            ReqEvent::Release => match self.held.take() {
                Some(t) if self.target_at(self.pointer.0, self.pointer.1) == Some(t) => {
                    self.click(t)
                }
                _ => MoveResponse::Continue,
            },
            ReqEvent::Scroll(_) => MoveResponse::Continue,
            ReqEvent::Key(key) => self.key(key),
        }
    }

    fn key(&mut self, key: ReqKey) -> MoveResponse {
        match key {
            ReqKey::Enter => return self.button(MoveButton::Draw),
            ReqKey::Escape => return MoveResponse::Cancel,
            ReqKey::Tab => {
                let next = self.active.map_or(0, |a| (a + 1) % self.fields.len());
                self.activate(next);
            }
            ReqKey::Up | ReqKey::Down => {
                if let Some(a) = self.active {
                    let f = &mut self.fields[a];
                    let n = f.text.text().parse::<i32>().unwrap_or(0);
                    let n = if key == ReqKey::Up { n + 1 } else { n - 1 };
                    f.text.set(&n.clamp(f.min, f.max).to_string());
                }
            }
            ReqKey::Char(c) if !(c.is_ascii_digit() || c == '-') => {}
            k => {
                if let Some(a) = self.active {
                    let f = &mut self.fields[a];
                    if k == ReqKey::Char('-') && (f.min >= 0 || f.text.cursor != 0) {
                        return MoveResponse::Continue;
                    }
                    f.text.key(k);
                }
            }
        }
        MoveResponse::Continue
    }

    /// Renders the requester alone.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let (w, h) = self.size();
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, "Move", pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            centred(&mut img, geom::COL_X[i], geom::NUM_W, 14, axis, pens.dark);
        }
        font.draw(&mut img, LEFT, geom::DIST_Y + 1, "Dist:", pens.dark);
        font.draw(&mut img, LEFT, geom::ANGLE_Y + 1, "Angle:", pens.dark);
        for (i, label) in ["Ease-Out:", "Ease-In:", "Count:"].into_iter().enumerate() {
            font.draw(&mut img, LEFT, geom::LOWER_Y[i], label, pens.dark);
        }
        for (i, f) in self.fields.iter().enumerate() {
            draw_field(&mut img, f.rect, &f.text, self.active == Some(i), pens);
        }
        let checks = [
            (Target::DistBrush, geom::DIST_Y, "Brush", self.dist_brush),
            (Target::AngleBrush, geom::ANGLE_Y, "Brush", self.angle_brush),
            (Target::Cyclic, geom::SMALL_Y + 1, "Cyclic", self.cyclic),
        ];
        for (_, y, label, on) in checks {
            draw_check(&mut img, (geom::CHECK_X, y), on, pens);
            font.draw(&mut img, geom::CHECK_X + 11, y, label, pens.dark);
        }
        img.hline(1, w - 2, geom::RULE_1, pens.dark);
        img.vline(
            geom::DIR_X - 6,
            geom::RULE_1 + 1,
            geom::RULE_2 - 1,
            pens.dark,
        );
        font.draw(
            &mut img,
            geom::DIR_X,
            geom::LOWER_Y[0],
            "Direction:",
            pens.dark,
        );
        font.draw(&mut img, geom::DIR_X, geom::LOWER_Y[1], "Move", pens.dark);
        font.draw(
            &mut img,
            geom::RECORD_X,
            geom::LOWER_Y[1],
            "Record",
            pens.dark,
        );
        let icons: [(Target, &[&str], bool, bool); 5] = [
            (Target::GoFrom, &GO_FROM, false, !self.come_to),
            (Target::ComeTo, &GO_TO, false, self.come_to),
            (
                Target::Record(Record::Forward),
                &FORWARD,
                false,
                self.record == Record::Forward,
            ),
            (
                Target::Record(Record::InPlace),
                &IN_PLACE,
                false,
                self.record == Record::InPlace,
            ),
            (
                Target::Record(Record::Backward),
                &FORWARD,
                true,
                self.record == Record::Backward,
            ),
        ];
        let rects: Vec<(Target, Rect)> = self.targets();
        let rect_of = |t: Target| rects.iter().find(|(x, _)| *x == t).map(|(_, r)| *r);
        for (t, rows, mirror, on) in icons {
            let Some(r) = rect_of(t) else { continue };
            let ink = button_face(&mut img, r, on || self.pressed(t), pens);
            draw_icon(&mut img, r, rows, mirror, ink);
            if !self.enabled(t) {
                checker(&mut img, r, pens.face);
            }
        }
        img.hline(1, w - 2, geom::RULE_2, pens.dark);
        if let Some(m) = &self.message {
            font.draw(&mut img, 6, geom::MESSAGE_Y, m, pens.select);
        }
        let buttons = [MoveButton::Clear, MoveButton::GoBack]
            .into_iter()
            .chain(BUTTON_ROWS.iter().flatten().copied());
        for b in buttons {
            let r = Self::button_rect(b);
            let t = Target::Button(b);
            draw_button(&mut img, r, button_label(b), self.pressed(t), pens);
            if !self.enabled(t) {
                checker(&mut img, r, pens.face);
            }
        }
        img
    }
}

/// A 7×7 check box (as in the toolbox requesters), crossed when `on`.
fn draw_check(img: &mut IndexedImage, at: (i32, i32), on: bool, pens: &Pens) {
    let r = Rect::new(at.0, at.1 + 1, 7, 7);
    button_face(img, r, false, pens);
    if on {
        for i in 1..6 {
            img.set(r.x + i, r.y + i, pens.dark);
            img.set(r.x + 6 - i, r.y + i, pens.dark);
        }
    }
}

// ------------------------------------------------------------------------ Both of them

/// The Anim control requester that is open (it is modal).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimControlRequester {
    PlayRange(Box<PlayRangeRequester>),
    Move(Box<MoveRequester>),
}

impl AnimControlRequester {
    pub fn size(&self) -> (i32, i32) {
        match self {
            AnimControlRequester::PlayRange(r) => r.size(),
            AnimControlRequester::Move(r) => r.size(),
        }
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(&self, screen_w: i32, screen_h: i32) -> (i32, i32) {
        let (w, h) = self.size();
        (((screen_w - w) / 2).max(0), ((screen_h - h) / 2).max(0))
    }

    pub fn render(&self, pens: &Pens) -> IndexedImage {
        match self {
            AnimControlRequester::PlayRange(r) => r.render(pens),
            AnimControlRequester::Move(r) => r.render(pens),
        }
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
    use dpx_core::Palette;

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn click(r: &mut MoveRequester, rect: Rect) -> MoveResponse {
        r.handle(ReqEvent::Press(rect.x + 2, rect.y + 2));
        r.handle(ReqEvent::Release)
    }

    fn keys(r: &mut MoveRequester, text: &str) {
        for c in text.chars() {
            r.handle(ReqEvent::Key(ReqKey::Char(c)));
        }
    }

    fn clear_field(r: &mut MoveRequester) {
        for _ in 0..8 {
            r.handle(ReqEvent::Key(ReqKey::Backspace));
        }
    }

    #[test]
    fn play_range_all_or_from_before_to() {
        let mut r = PlayRangeRequester::new(10, None);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            PlayRangeResponse::Ok(PlayRange::All)
        );
        // "Frames" activates From: 3 to 10.
        r.handle(ReqEvent::Press(10, 31));
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        r.handle(ReqEvent::Key(ReqKey::Char('3')));
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            PlayRangeResponse::Ok(PlayRange::Frames { from: 3, to: 10 })
        );
        // From must be smaller than To.
        let mut r = PlayRangeRequester::new(10, Some((4, 4)));
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            PlayRangeResponse::Continue
        );
        assert!(r.message.as_deref().unwrap().contains("before"));
        assert_ne!(
            r.render(&pens()),
            PlayRangeRequester::new(10, Some((4, 4))).render(&pens())
        );
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            PlayRangeResponse::Cancel
        );
    }

    #[test]
    fn move_requester_reads_back_its_settings() {
        let path = MovePath {
            dist: [200, 0, -50],
            angle: [0, 0, 360],
            count: 20,
            ..MovePath::default()
        };
        let mut r = MoveRequester::new(&path, 20);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            MoveResponse::Cancel
        );
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            MoveResponse::Draw(path)
        );
        let exit = click(&mut r, MoveRequester::button_rect(MoveButton::Exit));
        assert_eq!(exit, MoveResponse::Exit(path));
        let preview = click(&mut r, MoveRequester::button_rect(MoveButton::Preview));
        assert_eq!(preview, MoveResponse::Preview(path));
        let trails = click(&mut r, MoveRequester::button_rect(MoveButton::Trails));
        assert_eq!(
            trails,
            MoveResponse::Draw(MovePath {
                trails: true,
                ..path
            })
        );
        assert_eq!(
            click(&mut r, MoveRequester::button_rect(MoveButton::GoBack)),
            MoveResponse::GoBack
        );
    }

    #[test]
    fn typing_negative_numbers_toggles_and_icons() {
        let mut r = MoveRequester::new(&MovePath::default(), 10);
        // Dist Y: -170 (Tab from Dist X).
        r.handle(ReqEvent::Key(ReqKey::Tab));
        clear_field(&mut r);
        keys(&mut r, "-1-70x");
        assert_eq!(r.fields[1].text.text(), "-170", "minus only in front");
        // Count: click its field, type 20; Up makes it 21.
        let f = r.fields[8].rect;
        click(&mut r, f);
        clear_field(&mut r);
        keys(&mut r, "20");
        r.handle(ReqEvent::Key(ReqKey::Up));
        // No minus in Count.
        r.handle(ReqEvent::Key(ReqKey::Home));
        keys(&mut r, "-");
        // Ease-Out 10.
        let f = r.fields[6].rect;
        click(&mut r, f);
        clear_field(&mut r);
        keys(&mut r, "10");
        // Come To, In Place, Cyclic off, Dist on brush axes.
        let rect =
            |r: &MoveRequester, t: Target| r.targets().into_iter().find(|x| x.0 == t).unwrap().1;
        let t = rect(&r, Target::ComeTo);
        click(&mut r, t);
        let t = rect(&r, Target::Record(Record::InPlace));
        click(&mut r, t);
        let t = rect(&r, Target::Cyclic);
        click(&mut r, t);
        let t = rect(&r, Target::DistBrush);
        click(&mut r, t);
        let MoveResponse::Draw(p) = r.handle(ReqEvent::Key(ReqKey::Enter)) else {
            panic!("draws");
        };
        assert_eq!(p.dist, [0, -170, 0]);
        assert_eq!((p.count, p.ease_out, p.ease_in), (21, 10, 0));
        assert!(p.come_to && !p.cyclic && p.dist_brush && p.angle_brush);
        assert_eq!(p.record, Record::InPlace);
        // Clear zeroes the numbers only.
        click(&mut r, MoveRequester::button_rect(MoveButton::Clear));
        let MoveResponse::Exit(p) = click(&mut r, MoveRequester::button_rect(MoveButton::Exit))
        else {
            panic!("exits");
        };
        assert_eq!((p.dist, p.angle, p.count), ([0; 3], [0; 3], 21));
    }

    #[test]
    fn bad_numbers_keep_it_open_and_ghosts_do_nothing() {
        let mut r = MoveRequester::new(&MovePath::default(), 1);
        let f = r.fields[8].rect;
        click(&mut r, f);
        clear_field(&mut r);
        keys(&mut r, "0");
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            MoveResponse::Continue
        );
        assert_eq!(r.message.as_deref(), Some("Count must be 1 to 999"));
        let shown = r.render(&pens());
        r.handle(ReqEvent::Key(ReqKey::Char('5')));
        assert!(r.message.is_none());
        assert_ne!(r.render(&pens()), shown, "message drawn");
        // One frame: Trails and Record are ghosted; Fill, Load and Save always.
        for b in [
            MoveButton::Trails,
            MoveButton::Fill,
            MoveButton::Load,
            MoveButton::Save,
        ] {
            assert_eq!(
                click(&mut r, MoveRequester::button_rect(b)),
                MoveResponse::Continue,
                "{b:?}"
            );
        }
        let backward = r
            .targets()
            .into_iter()
            .find(|x| x.0 == Target::Record(Record::Backward))
            .unwrap()
            .1;
        click(&mut r, backward);
        assert_eq!(r.record, Record::Forward);
    }

    #[test]
    fn requesters_fit_the_smallest_screen_and_draw_centred() {
        let p = pens();
        for req in [
            AnimControlRequester::PlayRange(Box::new(PlayRangeRequester::new(999, Some((1, 999))))),
            AnimControlRequester::Move(Box::new(MoveRequester::new(&MovePath::default(), 30))),
        ] {
            let (w, h) = req.size();
            assert!(w <= 320 && h <= 200);
            let img = req.render(&p);
            assert_eq!((img.width() as i32, img.height() as i32), (w, h));
            let mut screen = IndexedImage::new(320, 200, 9);
            req.draw(&mut screen, &p);
            let (x, y) = req.origin(320, 200);
            assert_eq!(screen.get(x, y), Some(p.dark), "square frame corner");
        }
        // Every target lies inside the requester and none overlap.
        let r = MoveRequester::new(&MovePath::default(), 30);
        let t = r.targets();
        for (i, (a, ra)) in t.iter().enumerate() {
            assert!(
                ra.x >= 1 && ra.y >= 13 && ra.x + ra.w < geom::W && ra.y + ra.h < geom::H,
                "{a:?}"
            );
            for (b, rb) in &t[i + 1..] {
                assert!(ra.intersect(*rb).is_empty(), "{a:?} overlaps {b:?}");
            }
        }
    }
}
