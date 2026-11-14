//! A small form for the toolbox settings requesters (ADR-20): radio buttons in groups, check
//! boxes, numeric fields, static text, and Cancel / Ok. Drawn in UI (LoRes) pixels with
//! palette-derived pens, square corners. The requesters (Grid, Symmetry, Spacing, Fill Type)
//! build one with explicit coordinates and read the values back after Ok.

use dpx_core::{IndexedImage, Rect};

use crate::dialog::{
    self, Button as ViewButton, Check as ViewCheck, Control, Dialog, Field as ViewField, Item,
    Kind, Part, Radio as ViewRadio, Role, Size, Tone,
};
use crate::font::Font;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{TextField, button_face, centred, checker, draw_button, draw_field, frame};

const BOX: i32 = 7;
const ROW_H: i32 = 9;
const BUTTON_W: i32 = 56;
const BUTTON_H: i32 = 13;

/// What lies at most this many pixels lower than the first part of a line belongs to it
/// (a label is drawn one pixel lower than its field's box).
const ROW_JOIN: i32 = 3;

/// Ids of the controls in [`Form::view`].
pub mod ctl {
    use crate::dialog::ControlId;

    pub const CANCEL: ControlId = 0;
    pub const OK: ControlId = 1;
    /// The radio buttons in the order the form got them: `RADIO + i`.
    pub const RADIO: ControlId = 100;
    /// The check boxes: `CHECK + i`.
    pub const CHECK: ControlId = 200;
    /// The number fields: `FIELD + i`.
    pub const FIELD: ControlId = 300;
    /// A requester's own controls beside its form start here.
    pub const EXTRA: ControlId = 400;
}

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

    /// The form as the Modern skin shows it (ADR-47).
    pub fn view(&self) -> Dialog {
        self.view_with(Vec::new(), None)
    }

    /// [`Form::view`] for a requester that adds to its form: `extra` are its own controls
    /// with the place they have in the Classic requester (they join the form line at that
    /// height); `message` is shown in place of the form's own message, as the requester
    /// draws it over that line.
    pub(crate) fn view_with(
        &self,
        extra: Vec<((i32, i32), Part)>,
        message: Option<&str>,
    ) -> Dialog {
        enum El {
            Text(String),
            Part(Part),
            Rule,
        }
        let mut els: Vec<((i32, i32), El)> = Vec::new();
        for (at, t) in &self.texts {
            els.push(((at.1, at.0), El::Text(t.clone())));
        }
        for &y in &self.rules {
            els.push(((y, 0), El::Rule));
        }
        for (i, r) in self.radios.iter().enumerate() {
            let part = Part::Radio(ViewRadio {
                control: Control::new(ctl::RADIO + i as dialog::ControlId, self.radio_rect(r)),
                label: r.label.clone(),
                on: r.enabled && self.selected(r.group) == r.value,
                enabled: r.enabled,
            });
            els.push(((r.at.1, r.at.0), El::Part(part)));
        }
        for (i, c) in self.checks.iter().enumerate() {
            let part = Part::Check(ViewCheck {
                control: Control::new(ctl::CHECK + i as dialog::ControlId, self.check_rect(c)),
                label: c.label.clone(),
                on: self.on[i],
                enabled: c.enabled,
                key: None,
            });
            els.push(((c.at.1, c.at.0), El::Part(part)));
        }
        for (i, f) in self.fields.iter().enumerate() {
            if f.label_at.0 >= 0 {
                els.push(((f.label_at.1, f.label_at.0), El::Text(f.label.clone())));
            }
            let part = Part::Field(ViewField {
                control: Control::new(ctl::FIELD + i as dialog::ControlId, f.rect),
                text: f.text.text(),
                caret: (self.active == Some(i)).then_some(f.text.cursor),
                enabled: true,
                numeric: true,
                max_chars: f.max.to_string().len(),
            });
            els.push(((f.rect.y + 1, f.rect.x), El::Part(part)));
        }
        for ((x, y), part) in extra {
            els.push(((y, x), El::Part(part)));
        }
        els.sort_by_key(|(at, _)| *at);
        // Lines: what stands at (nearly) the same height, from left to right.
        let mut rows: Vec<Vec<((i32, i32), El)>> = Vec::new();
        for (at, el) in els {
            let rule = matches!(el, El::Rule);
            match rows.last_mut() {
                Some(row)
                    if !rule && !matches!(row[0].1, El::Rule) && at.0 - row[0].0.0 <= ROW_JOIN =>
                {
                    row.push((at, el));
                }
                _ => rows.push(vec![(at, el)]),
            }
        }
        for row in &mut rows {
            row.sort_by_key(|(at, _)| at.1);
        }
        let texts_only =
            |row: &[((i32, i32), El)]| row.iter().all(|(_, el)| matches!(el, El::Text(_)));
        let fields = |row: &[((i32, i32), El)]| {
            row.iter()
                .filter(|(_, el)| matches!(el, El::Part(Part::Field(_))))
                .count()
        };
        let starts_with_radio = |row: &[((i32, i32), El)]| {
            row.iter()
                .find_map(|(_, el)| match el {
                    El::Part(p) => Some(matches!(p, Part::Radio(_))),
                    _ => None,
                })
                .unwrap_or(false)
                && !matches!(row[0].1, El::Text(_))
        };
        let last_controls = rows
            .iter()
            .rposition(|r| !texts_only(r) && !matches!(r[0].1, El::Rule));
        let trim = |t: &str| t.trim_end_matches(':').to_string();
        let mut items: Vec<Item> = Vec::new();
        let mut captions: Vec<String> = Vec::new();
        let mut rows = rows.into_iter().enumerate().peekable();
        while let Some((i, row)) = rows.next() {
            if matches!(row[0].1, El::Rule) {
                items.push(Item::Rule);
                continue;
            }
            if texts_only(&row) {
                let words: Vec<String> = row
                    .into_iter()
                    .filter_map(|(_, el)| match el {
                        El::Text(t) => Some(t),
                        _ => None,
                    })
                    .collect();
                let next = rows.peek().map(|(_, r)| r.as_slice());
                // Captions over a line of fields stand before the fields.
                if words.len() > 1 && next.is_some_and(|r| fields(r) == words.len()) {
                    captions = words;
                    continue;
                }
                let text = words.join(" ");
                if next.is_some_and(starts_with_radio) {
                    items.push(Item::Heading(trim(&text)));
                } else if last_controls.is_some_and(|l| i < l) {
                    items.push(Item::Text {
                        text,
                        tone: Tone::Normal,
                        bullet: false,
                    });
                } else if let Some(Item::Text {
                    text: before,
                    tone: Tone::Dim,
                    ..
                }) = items.last_mut()
                {
                    // An explanation that runs over several Classic lines is one text.
                    before.push(' ');
                    before.push_str(&text);
                } else {
                    items.push(Item::Text {
                        text,
                        tone: Tone::Dim,
                        bullet: false,
                    });
                }
                continue;
            }
            let mut label = String::new();
            let mut parts = Vec::new();
            let mut caption = std::mem::take(&mut captions).into_iter();
            for (n, (_, el)) in row.into_iter().enumerate() {
                match el {
                    El::Text(t) if n == 0 => label = trim(&t),
                    El::Text(t) => parts.push(Part::Text(t, Tone::Dim)),
                    El::Part(p) => {
                        if matches!(p, Part::Field(_))
                            && let Some(c) = caption.next()
                        {
                            parts.push(Part::Text(c, Tone::Dim));
                        }
                        parts.push(p);
                    }
                    El::Rule => {}
                }
            }
            let leads = matches!(parts.first(), Some(Part::Radio(_) | Part::Check(_)));
            items.push(if label.is_empty() && leads {
                Item::Row(parts)
            } else {
                Item::Line {
                    label,
                    key: None,
                    parts,
                }
            });
        }
        if let Some(m) = message.or(self.message.as_deref()) {
            items.push(Item::Text {
                text: m.to_string(),
                tone: Tone::Alert,
                bullet: false,
            });
        }
        Dialog {
            title: self.title.to_string(),
            kind: Kind::Plain,
            size: Size::Small,
            columns: vec![items],
            buttons: vec![
                ViewButton::new(
                    Control::new(ctl::CANCEL, self.button_rect(false)),
                    "Cancel",
                    Role::Cancel,
                    self.pressed(Target::Cancel),
                ),
                ViewButton::new(
                    Control::new(ctl::OK, self.button_rect(true)),
                    "Ok",
                    Role::Default,
                    self.pressed(Target::Ok),
                ),
            ],
        }
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

    /// The view (ADR-47): what stands at one height is one line, and the centre of every
    /// control's rectangle is where the form takes that control's press.
    #[test]
    fn the_view_groups_lines_and_names_every_control() {
        let mut f = sample();
        f.text((8, 70), "An explanation");
        f.text((8, 79), "in two lines");
        let v = f.view();
        assert_eq!((v.title.as_str(), v.size), ("Sample", Size::Small));
        let ids: Vec<dialog::ControlId> = v.controls().iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                ctl::RADIO,
                ctl::RADIO + 1,
                ctl::FIELD,
                ctl::RADIO + 2,
                ctl::CHECK,
                ctl::CANCEL,
                ctl::OK
            ]
        );
        for c in v.controls() {
            let (x, y) = (c.area.x + c.area.w / 2, c.area.y + c.area.h / 2);
            let wanted = match c.id {
                ctl::CANCEL => Some(Target::Cancel),
                ctl::OK => Some(Target::Ok),
                // The third radio button is ghosted: it takes no press.
                id if id == ctl::RADIO + 2 => None,
                id if id >= ctl::FIELD => Some(Target::Field(usize::from(id - ctl::FIELD))),
                id if id >= ctl::CHECK => Some(Target::Check(usize::from(id - ctl::CHECK))),
                id => Some(Target::Radio(usize::from(id - ctl::RADIO))),
            };
            assert_eq!(f.target_at(x, y), wanted, "control {}", c.id);
        }
        let items = &v.columns[0];
        // One, Two with its field (the caption "N" between them), Three, Dither, the text.
        assert!(matches!(&items[0], Item::Row(p) if p.len() == 1));
        let Item::Row(two) = &items[1] else {
            panic!("the second line: {:?}", items[1]);
        };
        assert!(matches!(&two[0], Part::Radio(r) if r.label == "Two" && !r.on && r.enabled));
        assert!(matches!(&two[1], Part::Text(t, Tone::Dim) if t == "N"));
        assert!(
            matches!(&two[2], Part::Field(x) if x.text == "5" && x.caret == Some(1) && x.max_chars == 2)
        );
        assert!(matches!(&items[2], Item::Row(p) if matches!(&p[0], Part::Radio(r) if !r.enabled)));
        assert!(matches!(&items[3], Item::Row(p) if matches!(&p[0], Part::Check(c) if !c.on)));
        assert_eq!(
            items[4],
            Item::Text {
                text: "An explanation in two lines".into(),
                tone: Tone::Dim,
                bullet: false
            }
        );
        assert_eq!(items.len(), 5);
        // State: a chosen radio, a checked box, a held button, a message.
        click(&mut f, 10, 33);
        click(&mut f, 10, 58);
        f.fields[0].text.set("");
        f.handle(ReqEvent::Key(ReqKey::Enter));
        let ok = f.button_rect(true);
        f.handle(ReqEvent::Press(ok.x + 1, ok.y + 1));
        let v = f.view();
        let items = &v.columns[0];
        assert!(matches!(&items[1], Item::Row(p) if matches!(&p[0], Part::Radio(r) if r.on)));
        assert!(matches!(&items[3], Item::Row(p) if matches!(&p[0], Part::Check(c) if c.on)));
        assert_eq!(
            items[5],
            Item::Text {
                text: "N must be 1 to 99".into(),
                tone: Tone::Alert,
                bullet: false
            }
        );
        assert!(v.buttons[1].pressed && !v.buttons[0].pressed);
        assert_eq!(
            f.view_with(Vec::new(), Some("Refused")).columns[0][5],
            Item::Text {
                text: "Refused".into(),
                tone: Tone::Alert,
                bullet: false
            }
        );
    }
}
