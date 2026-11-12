//! A small form for the toolbox settings requesters (ADR-20): radio buttons in groups, check
//! boxes, numeric fields, static text, and Cancel / Ok. Drawn in UI (LoRes) pixels with
//! palette-derived pens, square corners. The requesters (Grid, Symmetry, Spacing, Fill Type)
//! build one with explicit coordinates and read the values back after Ok.

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{TextField, button_face, centred, checker, draw_button, draw_field, frame};

const BOX: i32 = 7;
const ROW_H: i32 = 9;
const BUTTON_W: i32 = 56;
const BUTTON_H: i32 = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormResponse {
    Continue,
    Cancel,
    Ok,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Radio {
    group: usize,
    value: u8,
    at: (i32, i32),
    label: String,
    enabled: bool,
    /// Selecting this radio activates this field.
    field: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Check {
    at: (i32, i32),
    label: String,
    enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Field {
    label: String,
    label_at: (i32, i32),
    rect: Rect,
    min: i32,
    max: i32,
    text: TextField,
    /// Only checked on Ok while this radio group has this value.
    when: Option<(usize, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Radio(usize),
    Check(usize),
    Field(usize),
    Cancel,
    Ok,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    title: &'static str,
    size: (i32, i32),
    radios: Vec<Radio>,
    selected: Vec<u8>,
    checks: Vec<Check>,
    on: Vec<bool>,
    fields: Vec<Field>,
    texts: Vec<((i32, i32), String)>,
    rules: Vec<i32>,
    active: Option<usize>,
    message: Option<String>,
    held: Option<Target>,
    pointer: (i32, i32),
}

impl Form {
    pub fn new(title: &'static str, width: i32, height: i32) -> Self {
        Self {
            title,
            size: (width, height),
            radios: Vec::new(),
            selected: Vec::new(),
            checks: Vec::new(),
            on: Vec::new(),
            fields: Vec::new(),
            texts: Vec::new(),
            rules: Vec::new(),
            active: None,
            message: None,
            held: None,
            pointer: (0, 0),
        }
    }

    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// A radio button of `group` (groups are numbered from 0 in any order); `selected`
    /// makes it the group's choice. Returns its index.
    pub fn radio(
        &mut self,
        group: usize,
        value: u8,
        at: (i32, i32),
        label: &str,
        selected: bool,
    ) -> usize {
        if self.selected.len() <= group {
            self.selected.resize(group + 1, 0);
        }
        if selected {
            self.selected[group] = value;
        }
        self.radios.push(Radio {
            group,
            value,
            at,
            label: label.to_string(),
            enabled: true,
            field: None,
        });
        self.radios.len() - 1
    }

    /// A radio button that is not available (ghosted, cannot be selected).
    pub fn ghost_radio(&mut self, group: usize, value: u8, at: (i32, i32), label: &str) {
        let i = self.radio(group, value, at, label, false);
        self.radios[i].enabled = false;
    }

    pub fn check(&mut self, at: (i32, i32), label: &str, on: bool) -> usize {
        self.checks.push(Check {
            at,
            label: label.to_string(),
            enabled: true,
        });
        self.on.push(on);
        self.checks.len() - 1
    }

    /// A numeric field: `label` at `label_at` (not drawn when its x is negative; it still
    /// names the field in messages), the value box at `rect`, `min..=max`.
    pub fn field(
        &mut self,
        label: &str,
        label_at: (i32, i32),
        rect: Rect,
        range: (i32, i32),
        value: i32,
        when: Option<(usize, u8)>,
    ) -> usize {
        let digits = range.1.to_string().len();
        let mut text = TextField::new(digits);
        text.set(&value.to_string());
        self.fields.push(Field {
            label: label.to_string(),
            label_at,
            rect,
            min: range.0,
            max: range.1,
            text,
            when,
        });
        if self.active.is_none() {
            self.active = Some(self.fields.len() - 1);
        }
        self.fields.len() - 1
    }

    /// Selecting `radio` also activates `field`.
    pub fn link(&mut self, radio: usize, field: usize) {
        self.radios[radio].field = Some(field);
    }

    pub fn text(&mut self, at: (i32, i32), text: &str) {
        self.texts.push((at, text.to_string()));
    }

    /// A horizontal rule across the form at `y`.
    pub fn rule(&mut self, y: i32) {
        self.rules.push(y);
    }

    /// The selected value of radio `group`.
    pub fn selected(&self, group: usize) -> u8 {
        self.selected.get(group).copied().unwrap_or(0)
    }

    pub fn select(&mut self, group: usize, value: u8) {
        if self.selected.len() <= group {
            self.selected.resize(group + 1, 0);
        }
        self.selected[group] = value;
    }

    pub fn checked(&self, check: usize) -> bool {
        self.on[check]
    }

    /// The value of numeric field `field` (valid after Ok).
    pub fn number(&self, field: usize) -> i32 {
        self.fields[field].text.text().parse().unwrap_or(0)
    }

    /// Sets numeric field `field` (a requester's own button filled it in, ADR-37).
    pub fn set_number(&mut self, field: usize, value: i32) {
        self.fields[field].text.set(&value.to_string());
        self.message = None;
    }

    fn button_rect(&self, ok: bool) -> Rect {
        let y = self.size.1 - BUTTON_H - 5;
        let x = if ok { self.size.0 - BUTTON_W - 6 } else { 6 };
        Rect::new(x, y, BUTTON_W, BUTTON_H)
    }

    fn radio_rect(&self, r: &Radio) -> Rect {
        let w = BOX + 4 + Font::builtin().text_width(&r.label);
        Rect::new(r.at.0, r.at.1, w, ROW_H)
    }

    fn check_rect(&self, c: &Check) -> Rect {
        let w = BOX + 4 + Font::builtin().text_width(&c.label);
        Rect::new(c.at.0, c.at.1, w, ROW_H)
    }

    fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        for (i, r) in self.radios.iter().enumerate() {
            if r.enabled && self.radio_rect(r).contains(x, y) {
                return Some(Target::Radio(i));
            }
        }
        for (i, c) in self.checks.iter().enumerate() {
            if c.enabled && self.check_rect(c).contains(x, y) {
                return Some(Target::Check(i));
            }
        }
        for (i, f) in self.fields.iter().enumerate() {
            if f.rect.contains(x, y) {
                return Some(Target::Field(i));
            }
        }
        if self.button_rect(false).contains(x, y) {
            return Some(Target::Cancel);
        }
        if self.button_rect(true).contains(x, y) {
            return Some(Target::Ok);
        }
        None
    }

    fn pressed(&self, t: Target) -> bool {
        self.held == Some(t) && self.target_at(self.pointer.0, self.pointer.1) == Some(t)
    }

    fn activate(&mut self, i: usize) {
        self.active = Some(i);
        let f = &mut self.fields[i];
        f.text.cursor = f.text.chars.len();
    }

    fn click(&mut self, t: Target) -> FormResponse {
        match t {
            Target::Radio(i) => {
                let (group, value, field) = {
                    let r = &self.radios[i];
                    (r.group, r.value, r.field)
                };
                self.select(group, value);
                self.message = None;
                if let Some(f) = field {
                    self.activate(f);
                }
            }
            Target::Check(i) => self.on[i] = !self.on[i],
            Target::Field(i) => self.activate(i),
            Target::Cancel => return FormResponse::Cancel,
            Target::Ok => return self.accept(),
        }
        FormResponse::Continue
    }

    /// Checks the numeric fields that matter; the first bad one keeps the form open.
    fn accept(&mut self) -> FormResponse {
        for (i, f) in self.fields.iter().enumerate() {
            if let Some((g, v)) = f.when
                && self.selected(g) != v
            {
                continue;
            }
            let ok = f
                .text
                .text()
                .parse::<i32>()
                .is_ok_and(|n| (f.min..=f.max).contains(&n));
            if !ok {
                self.message = Some(format!("{} must be {} to {}", f.label, f.min, f.max));
                self.active = Some(i);
                return FormResponse::Continue;
            }
        }
        FormResponse::Ok
    }

    /// Handles one event in requester coordinates (origin: [`Form::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> FormResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.target_at(x, y);
                // Fields and boxes act on press, buttons on release.
                if let Some(t @ (Target::Radio(_) | Target::Check(_) | Target::Field(_))) =
                    self.held
                {
                    self.held = None;
                    return self.click(t);
                }
                FormResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                FormResponse::Continue
            }
            ReqEvent::Release => match self.held.take() {
                Some(t) if self.target_at(self.pointer.0, self.pointer.1) == Some(t) => {
                    self.click(t)
                }
                _ => FormResponse::Continue,
            },
            ReqEvent::Scroll(_) => FormResponse::Continue,
            ReqEvent::Key(key) => self.key(key),
        }
    }

    fn key(&mut self, key: ReqKey) -> FormResponse {
        match key {
            ReqKey::Enter => return self.accept(),
            ReqKey::Escape => return FormResponse::Cancel,
            ReqKey::Tab if !self.fields.is_empty() => {
                let next = self.active.map_or(0, |a| (a + 1) % self.fields.len());
                self.activate(next);
            }
            ReqKey::Up | ReqKey::Down => {
                if let Some(a) = self.active {
                    let f = &mut self.fields[a];
                    let n = f.text.text().parse::<i32>().unwrap_or(f.min);
                    let n = if key == ReqKey::Up { n + 1 } else { n - 1 };
                    f.text.set(&n.clamp(f.min, f.max).to_string());
                    self.message = None;
                }
            }
            ReqKey::Char(c) | ReqKey::Paste(c) if !c.is_ascii_digit() => {}
            k => {
                if let Some(a) = self.active {
                    let f = &mut self.fields[a];
                    let before = f.text.chars.clone();
                    f.text.key(k);
                    if f.text.chars != before {
                        self.message = None;
                    }
                }
            }
        }
        FormResponse::Continue
    }

    /// Renders the form alone.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let (w, h) = self.size;
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, self.title, pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        for &y in &self.rules {
            img.hline(1, w - 2, y, pens.dark);
        }
        for ((x, y), t) in &self.texts {
            font.draw(&mut img, *x, *y, t, pens.dark);
        }
        for (i, r) in self.radios.iter().enumerate() {
            let on = self.selected(r.group) == r.value && r.enabled;
            let held = self.pressed(Target::Radio(i));
            draw_box(&mut img, r.at, on || held, false, pens);
            self.draw_label(&mut img, r.at, &r.label, r.enabled, pens);
        }
        for (i, c) in self.checks.iter().enumerate() {
            draw_box(&mut img, c.at, self.on[i], true, pens);
            self.draw_label(&mut img, c.at, &c.label, c.enabled, pens);
        }
        for (i, f) in self.fields.iter().enumerate() {
            if f.label_at.0 >= 0 {
                font.draw(&mut img, f.label_at.0, f.label_at.1, &f.label, pens.dark);
            }
            draw_field(&mut img, f.rect, &f.text, self.active == Some(i), pens);
        }
        if let Some(m) = &self.message {
            font.draw(&mut img, 6, self.size.1 - BUTTON_H - 16, m, pens.select);
        }
        for (ok, label) in [(false, "Cancel"), (true, "Ok")] {
            let t = if ok { Target::Ok } else { Target::Cancel };
            draw_button(&mut img, self.button_rect(ok), label, self.pressed(t), pens);
        }
        img
    }

    fn draw_label(
        &self,
        img: &mut IndexedImage,
        at: (i32, i32),
        label: &str,
        enabled: bool,
        pens: &Pens,
    ) {
        let font = Font::builtin();
        let pen = if enabled {
            pens.dark
        } else {
            pens.ghost.unwrap_or(pens.dark)
        };
        font.draw(img, at.0 + BOX + 4, at.1, label, pen);
        if !enabled && pens.ghost.is_none() {
            let r = Rect::new(at.0 + BOX + 4, at.1, font.text_width(label), ROW_H);
            checker(img, r, pens.face);
        }
    }
}

/// A 7×7 box: frame, and when `on` a filled centre (radio) or a cross (check box).
pub(crate) fn draw_box(img: &mut IndexedImage, at: (i32, i32), on: bool, check: bool, pens: &Pens) {
    let r = Rect::new(at.0, at.1 + 1, BOX, BOX);
    button_face(img, r, false, pens);
    if !on {
        return;
    }
    if check {
        for i in 1..BOX - 1 {
            img.set(r.x + i, r.y + i, pens.dark);
            img.set(r.x + BOX - 1 - i, r.y + i, pens.dark);
        }
    } else {
        img.fill_rect(r.x + 2, r.y + 2, BOX - 4, BOX - 4, pens.dark);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn sample() -> Form {
        let mut f = Form::new("Sample", 150, 90);
        f.radio(0, 0, (8, 20), "One", true);
        let r2 = f.radio(0, 1, (8, 32), "Two", false);
        f.ghost_radio(0, 2, (8, 44), "Three");
        let n = f.field(
            "N",
            (80, 33),
            Rect::new(100, 32, 30, 9),
            (1, 99),
            5,
            Some((0, 1)),
        );
        f.link(r2, n);
        f.check((8, 56), "Dither", false);
        f
    }

    fn click(f: &mut Form, x: i32, y: i32) -> FormResponse {
        f.handle(ReqEvent::Press(x, y));
        f.handle(ReqEvent::Release)
    }

    #[test]
    fn radios_select_activate_their_field_and_skip_ghosts() {
        let mut f = sample();
        assert_eq!(f.selected(0), 0);
        click(&mut f, 10, 33);
        assert_eq!(f.selected(0), 1);
        assert_eq!(f.active, Some(0));
        // The ghosted option cannot be chosen.
        click(&mut f, 10, 45);
        assert_eq!(f.selected(0), 1);
        // Clicking the label counts too.
        click(&mut f, 24, 21);
        assert_eq!(f.selected(0), 0);
    }

    #[test]
    fn check_toggles() {
        let mut f = sample();
        assert!(!f.checked(0));
        click(&mut f, 10, 58);
        assert!(f.checked(0));
        click(&mut f, 10, 58);
        assert!(!f.checked(0));
    }

    #[test]
    fn fields_take_digits_arrows_and_validate_on_ok() {
        let mut f = sample();
        f.select(0, 1);
        f.activate(0);
        f.handle(ReqEvent::Key(ReqKey::Char('x')));
        f.handle(ReqEvent::Key(ReqKey::Char('7')));
        assert_eq!(f.fields[0].text.text(), "57");
        f.handle(ReqEvent::Key(ReqKey::Up));
        assert_eq!(f.number(0), 58);
        f.handle(ReqEvent::Key(ReqKey::Backspace));
        f.handle(ReqEvent::Key(ReqKey::Backspace));
        assert_eq!(
            f.handle(ReqEvent::Key(ReqKey::Enter)),
            FormResponse::Continue
        );
        assert_eq!(f.message.as_deref(), Some("N must be 1 to 99"));
        f.handle(ReqEvent::Key(ReqKey::Char('0')));
        assert_eq!(
            f.handle(ReqEvent::Key(ReqKey::Enter)),
            FormResponse::Continue,
            "0 is below the minimum"
        );
        f.handle(ReqEvent::Key(ReqKey::Char('4')));
        assert_eq!(f.fields[0].text.text(), "04");
        assert_eq!(f.handle(ReqEvent::Key(ReqKey::Enter)), FormResponse::Ok);
        assert_eq!(f.number(0), 4);
    }

    #[test]
    fn a_field_of_another_choice_is_not_checked() {
        let mut f = sample();
        f.fields[0].text.set("");
        assert_eq!(f.selected(0), 0);
        assert_eq!(f.handle(ReqEvent::Key(ReqKey::Enter)), FormResponse::Ok);
    }

    #[test]
    fn buttons_act_on_release_inside_and_escape_cancels() {
        let mut f = sample();
        let ok = f.button_rect(true);
        let cancel = f.button_rect(false);
        f.handle(ReqEvent::Press(ok.x + 2, ok.y + 2));
        f.handle(ReqEvent::Drag(0, 0));
        assert_eq!(f.handle(ReqEvent::Release), FormResponse::Continue);
        assert_eq!(click(&mut f, ok.x + 2, ok.y + 2), FormResponse::Ok);
        assert_eq!(
            click(&mut f, cancel.x + 2, cancel.y + 2),
            FormResponse::Cancel
        );
        assert_eq!(
            f.handle(ReqEvent::Key(ReqKey::Escape)),
            FormResponse::Cancel
        );
    }

    #[test]
    fn tab_cycles_fields_and_render_shows_state() {
        let mut f = sample();
        f.handle(ReqEvent::Key(ReqKey::Tab));
        assert_eq!(f.active, Some(0));
        let p = pens();
        let a = f.render(&p);
        assert_eq!((a.width() as i32, a.height() as i32), (150, 90));
        click(&mut f, 10, 58);
        assert_ne!(f.render(&p), a, "check box is drawn");
    }

    #[test]
    fn ghosted_radios_use_the_ghost_pen() {
        let p = pens();
        let img = sample().render(&p);
        let ghost = p.ghost.unwrap();
        let count = |pen: u8| {
            (44..53)
                .flat_map(|y| (19..60).map(move |x| (x, y)))
                .filter(|&(x, y)| img.get(x, y) == Some(pen))
                .count()
        };
        assert!(count(ghost) > 0);
    }
}
