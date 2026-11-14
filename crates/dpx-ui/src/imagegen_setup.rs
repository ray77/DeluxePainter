//! The Image Generator setup requester (ADR-45), a DeluxePainter addition: which generator
//! Picture ▸ Generate.. uses (None or OpenAI Images), its API key, its model, a free key test
//! and a link to the provider's key page.
//!
//! The key field is masked: it shows one `*` per typed character and never the key, and a
//! key already stored is not read into it (a line says where it is kept). Typing or pasting
//! (Cmd/Ctrl+V) a key replaces the stored one at Ok; Remove key forgets it at Ok; Cancel
//! changes nothing. The requester only holds the entries; storing, testing and opening the
//! web page are the app's.
//!
//! Drawn in UI (LoRes) pixels with palette-derived pens, square corners: Tab moves between the
//! two fields, Return = Ok, Esc = Cancel.

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::form::draw_box;
use crate::note_requester::wrap;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{Inset, TextField, centred, checker, draw_button, draw_field, frame};

const TITLE: &str = "Image Generator";
const MAX_W: i32 = 360;
const MAX_H: i32 = 232;
const MIN_W: i32 = 304;
const MIN_H: i32 = 190;
const LEFT: i32 = 6;
const BOX: i32 = 7;
const ROW_H: i32 = 9;
const LINE_STEP: i32 = 9;
const BUTTON_H: i32 = 13;
const FIELD_H: i32 = 10;
/// Longest key accepted.
pub const MAX_KEY_CHARS: usize = 200;
/// Longest model id (OpenAI ids are short; ADR-45 accepts 1-64 characters).
pub const MAX_MODEL_CHARS: usize = 64;

mod geom {
    pub const PROVIDER_Y: i32 = 16;
    pub const KEY_Y: i32 = 28;
    pub const KEY_STATUS_Y: i32 = 39;
    pub const MODEL_Y: i32 = 50;
    pub const MODEL_HINT_Y: i32 = 61;
    pub const BUTTONS_Y: i32 = 72;
    pub const TEST_Y: i32 = 89;
    pub const NOTE_Y: i32 = 112;
}

/// The controls of the requester's description for the Modern skin
/// ([`ImageGenSetupRequester::view`]): the ids are the places in
/// [`ImageGenSetupRequester::controls`].
pub mod ctl {
    use crate::dialog::ControlId;

    /// None, OpenAI Images: `PROVIDER + 0..2`.
    pub const PROVIDER: ControlId = 0;
    pub const KEY: ControlId = 2;
    pub const MODEL: ControlId = 3;
    pub const TEST: ControlId = 4;
    pub const GET_KEY: ControlId = 5;
    pub const REMOVE_KEY: ControlId = 6;
    pub const CANCEL: ControlId = 7;
    pub const OK: ControlId = 8;
}

/// The generator chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SetupProvider {
    /// No generator: Picture ▸ Generate.. says how to set one up.
    #[default]
    None,
    OpenAi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupField {
    Key,
    Model,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupControl {
    Provider(SetupProvider),
    Field(SetupField),
    Test,
    GetKey,
    RemoveKey,
    Cancel,
    Ok,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupResponse {
    Continue,
    /// Check the key (the typed one, else the stored one) without generating.
    Test,
    /// Open the provider's API key page in the web browser.
    GetKey,
    Cancel,
    Ok,
}

/// What the app tells about the generator (never the key itself).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupInfo {
    /// Where the stored key is, e.g. "API key saved in the system keychain".
    pub key_status: String,
    /// A key is set up (stored or in the environment): Test can check it.
    pub has_key: bool,
    /// The key is stored by DeluxePainter (keychain or session): Remove key can forget it.
    pub removable: bool,
    /// The default model and when it was checked.
    pub model_hint: String,
    /// Cost, network and storage, shown at the bottom (wrapped).
    pub note: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImageGenSetupRequester {
    provider: SetupProvider,
    key: TextField,
    model: TextField,
    active: Option<SetupField>,
    /// Remove key was pressed (forgotten at Ok).
    remove: bool,
    info: SetupInfo,
    /// The last test's result, or why Ok was refused.
    test: Option<String>,
    /// Counts changes of what a key test depends on (the key, the model, the generator,
    /// Remove key): a test answer made for an older revision is not shown.
    revision: u64,
    screen: (i32, i32),
    held: Option<SetupControl>,
    pointer: (i32, i32),
}

/// Never shows the typed key (logs, panics, test failures).
impl std::fmt::Debug for ImageGenSetupRequester {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageGenSetupRequester")
            .field("provider", &self.provider)
            .field("key_chars", &self.key.chars.len())
            .field("model", &self.model.text())
            .field("remove", &self.remove)
            .field("test", &self.test)
            .finish_non_exhaustive()
    }
}

impl ImageGenSetupRequester {
    pub fn new(provider: SetupProvider, model: &str, info: SetupInfo, screen: (i32, i32)) -> Self {
        let mut model_field = TextField::new(MAX_MODEL_CHARS);
        model_field.set(model);
        Self {
            provider,
            key: TextField::new(MAX_KEY_CHARS),
            model: model_field,
            active: (provider == SetupProvider::OpenAi).then_some(SetupField::Key),
            remove: false,
            info,
            test: None,
            revision: 0,
            screen,
            held: None,
            pointer: (0, 0),
        }
    }

    pub fn provider(&self) -> SetupProvider {
        self.provider
    }

    /// The key typed or pasted, trimmed; `None` when the field is empty.
    pub fn typed_key(&self) -> Option<String> {
        let k = self.key.text();
        let k = k.trim();
        (!k.is_empty()).then(|| k.to_string())
    }

    /// Remove key was pressed and no new key was typed since.
    pub fn remove_key(&self) -> bool {
        self.remove
    }

    /// The model id, trimmed.
    pub fn model(&self) -> String {
        self.model.text().trim().to_string()
    }

    pub fn active(&self) -> Option<SetupField> {
        self.active
    }

    pub fn info(&self) -> &SetupInfo {
        &self.info
    }

    pub fn set_info(&mut self, info: SetupInfo) {
        self.info = info;
    }

    pub fn test(&self) -> Option<&str> {
        self.test.as_deref()
    }

    /// The test's result, or why Ok was refused.
    pub fn set_test(&mut self, text: Option<String>) {
        self.test = text;
    }

    /// Changes with every edit of the key, the model, the generator choice and Remove key:
    /// the app shows a key test's answer only while it is the revision the test was made
    /// for.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Something a key test depends on changed: an earlier answer no longer applies.
    fn edited(&mut self) {
        self.revision += 1;
        self.test = None;
    }

    /// Empties the key field and forgets Remove key (the app saved the typed key and keeps
    /// the requester open to say where it went).
    pub fn clear_key(&mut self) {
        self.key.set("");
        self.remove = false;
        self.edited();
    }

    /// Lines the note gets above the buttons.
    fn note_room(&self) -> usize {
        let (_, h) = self.size();
        ((h - BUTTON_H - 8 - geom::NOTE_Y) / LINE_STEP).max(0) as usize
    }

    /// Whether the whole note shows (the GUI keeps its note short enough).
    pub fn note_fits(&self) -> bool {
        let (w, _) = self.size();
        wrap(&self.info.note, w - 2 * LEFT).len() <= self.note_room()
    }

    pub fn size(&self) -> (i32, i32) {
        (
            (self.screen.0 - 8).clamp(MIN_W, MAX_W),
            (self.screen.1 - 8).clamp(MIN_H, MAX_H),
        )
    }

    pub fn origin(&self) -> (i32, i32) {
        let (w, h) = self.size();
        (
            ((self.screen.0 - w) / 2).max(0),
            ((self.screen.1 - h) / 2).max(0),
        )
    }

    fn label_w() -> i32 {
        let font = Font::builtin();
        ["Generator:", "API key:", "Model:"]
            .iter()
            .map(|l| font.text_width(l))
            .max()
            .unwrap_or(0)
            + 6
    }

    fn field_rect(&self, f: SetupField) -> Rect {
        let (w, _) = self.size();
        let x = LEFT + Self::label_w();
        let y = match f {
            SetupField::Key => geom::KEY_Y - 1,
            SetupField::Model => geom::MODEL_Y - 1,
        };
        Rect::new(x, y, w - LEFT - x, FIELD_H)
    }

    fn radio_rect(at: (i32, i32), label: &str) -> Rect {
        Rect::new(
            at.0,
            at.1,
            BOX + 4 + Font::builtin().text_width(label),
            ROW_H,
        )
    }

    fn provider_label(p: SetupProvider) -> &'static str {
        match p {
            SetupProvider::None => "None",
            SetupProvider::OpenAi => "OpenAI Images",
        }
    }

    /// Every control with its rectangle (requester coordinates).
    pub fn controls(&self) -> Vec<(SetupControl, Rect)> {
        let font = Font::builtin();
        let (w, h) = self.size();
        let mut out = Vec::new();
        let mut x = LEFT + Self::label_w();
        for p in [SetupProvider::None, SetupProvider::OpenAi] {
            let r = Self::radio_rect((x, geom::PROVIDER_Y), Self::provider_label(p));
            x += r.w + 10;
            out.push((SetupControl::Provider(p), r));
        }
        for f in [SetupField::Key, SetupField::Model] {
            out.push((SetupControl::Field(f), self.field_rect(f)));
        }
        let width = |label: &str| (font.text_width(label) + 14).max(44);
        let mut x = LEFT;
        for (c, label) in [
            (SetupControl::Test, "Test"),
            (SetupControl::GetKey, "Get key.."),
            (SetupControl::RemoveKey, "Remove key"),
        ] {
            let bw = width(label);
            out.push((c, Rect::new(x, geom::BUTTONS_Y, bw, BUTTON_H)));
            x += bw + 6;
        }
        let by = h - BUTTON_H - 5;
        out.push((SetupControl::Cancel, Rect::new(LEFT, by, 56, BUTTON_H)));
        out.push((SetupControl::Ok, Rect::new(w - LEFT - 56, by, 56, BUTTON_H)));
        out
    }

    fn label(c: SetupControl) -> &'static str {
        match c {
            SetupControl::Test => "Test",
            SetupControl::GetKey => "Get key..",
            SetupControl::RemoveKey => "Remove key",
            SetupControl::Cancel => "Cancel",
            SetupControl::Ok => "Ok",
            _ => "",
        }
    }

    pub fn enabled(&self, c: SetupControl) -> bool {
        let openai = self.provider == SetupProvider::OpenAi;
        match c {
            SetupControl::Field(_) | SetupControl::GetKey => openai,
            SetupControl::Test => openai && (self.typed_key().is_some() || self.info.has_key),
            SetupControl::RemoveKey => {
                openai && (self.typed_key().is_some() || (self.info.removable && !self.remove))
            }
            SetupControl::Provider(_) | SetupControl::Cancel | SetupControl::Ok => true,
        }
    }

    fn control_at(&self, x: i32, y: i32) -> Option<SetupControl> {
        self.controls()
            .into_iter()
            .find(|&(c, r)| r.contains(x, y) && self.enabled(c))
            .map(|(c, _)| c)
    }

    fn pressed(&self, c: SetupControl) -> bool {
        self.held == Some(c) && self.control_at(self.pointer.0, self.pointer.1) == Some(c)
    }

    fn apply(&mut self, c: SetupControl) -> SetupResponse {
        match c {
            SetupControl::Provider(p) => {
                if p != self.provider {
                    self.provider = p;
                    self.edited();
                }
                self.active = (p == SetupProvider::OpenAi).then_some(SetupField::Key);
                SetupResponse::Continue
            }
            SetupControl::Field(f) => {
                self.active = Some(f);
                SetupResponse::Continue
            }
            SetupControl::Test => SetupResponse::Test,
            SetupControl::GetKey => SetupResponse::GetKey,
            SetupControl::RemoveKey => {
                self.key.set("");
                self.remove = self.info.removable;
                self.edited();
                SetupResponse::Continue
            }
            SetupControl::Cancel => SetupResponse::Cancel,
            SetupControl::Ok => SetupResponse::Ok,
        }
    }

    /// Handles one event in requester coordinates (origin: [`Self::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> SetupResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.control_at(x, y);
                // Radios and fields act on press, buttons on release.
                match self.held {
                    Some(c @ (SetupControl::Provider(_) | SetupControl::Field(_))) => {
                        self.held = None;
                        self.apply(c)
                    }
                    _ => SetupResponse::Continue,
                }
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                SetupResponse::Continue
            }
            ReqEvent::Release => match self.held.take() {
                Some(c) if self.control_at(self.pointer.0, self.pointer.1) == Some(c) => {
                    self.apply(c)
                }
                _ => SetupResponse::Continue,
            },
            ReqEvent::Scroll(_) => SetupResponse::Continue,
            ReqEvent::Key(key) => self.key(key),
        }
    }

    fn key(&mut self, key: ReqKey) -> SetupResponse {
        match key {
            ReqKey::Enter => SetupResponse::Ok,
            ReqKey::Escape => SetupResponse::Cancel,
            ReqKey::Tab => {
                if let Some(f) = self.active {
                    self.active = Some(match f {
                        SetupField::Key => SetupField::Model,
                        SetupField::Model => SetupField::Key,
                    });
                }
                SetupResponse::Continue
            }
            k => {
                match self.active {
                    Some(SetupField::Key) => {
                        let before = self.key.chars.clone();
                        self.key.key(k);
                        if self.key.chars.len() > before.len() {
                            // A new key replaces the stored one instead of removing it.
                            self.remove = false;
                        }
                        if self.key.chars != before {
                            self.edited();
                        }
                    }
                    Some(SetupField::Model) => {
                        let before = self.model.chars.clone();
                        self.model.key(k);
                        if self.model.chars != before {
                            self.edited();
                        }
                    }
                    None => {}
                }
                SetupResponse::Continue
            }
        }
    }

    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = self.origin();
        img.blit(&self.render(pens), x, y);
    }

    /// The key field as shown: one `*` per character, the cursor where it is.
    fn masked(&self) -> TextField {
        let mut m = TextField::new(MAX_KEY_CHARS);
        m.chars = vec!['*'; self.key.chars.len()];
        m.cursor = self.key.cursor;
        m
    }

    /// The line under the key field.
    pub fn key_line(&self) -> String {
        if self.typed_key().is_some() {
            "The new key is saved at Ok.".into()
        } else if self.remove {
            "The API key is removed at Ok.".into()
        } else {
            self.info.key_status.clone()
        }
    }

    /// What the requester shows, for the Modern skin (ADR-47). The key field is described
    /// as the Classic skin draws it: one `*` per character and the caret, never the key.
    pub fn view(&self) -> crate::dialog::Dialog {
        use crate::dialog::{
            Button as ViewButton, Control, ControlId, Dialog, Item, Kind, Part, Push, Radio, Role,
            Size, Tone,
        };
        let controls = self.controls();
        let control = |id: ControlId| Control::new(id, controls[usize::from(id)].1);
        let kind = |id: ControlId| controls[usize::from(id)].0;
        let openai = self.provider == SetupProvider::OpenAi;
        let radio = |id: ControlId, p: SetupProvider| {
            Part::Radio(Radio {
                control: control(id),
                label: Self::provider_label(p).into(),
                on: self.provider == p,
                enabled: true,
            })
        };
        let field = |id: ControlId, f: SetupField, label: &str| {
            let shown = match f {
                SetupField::Key => self.masked(),
                SetupField::Model => self.model.clone(),
            };
            let mut field = shown.view(control(id), openai && self.active == Some(f));
            // A press chooses the field; it does not move the caret (as in Classic).
            field.stops.clear();
            field.enabled = openai;
            if !openai {
                // Ghosted: nothing of the field is shown in Classic.
                field.text.clear();
            }
            Item::Setting {
                label: label.into(),
                focused: false,
                enabled: openai,
                parts: vec![Part::Field(field)],
            }
        };
        let push = |id: ControlId| {
            let c = kind(id);
            Part::Button(Push {
                control: control(id),
                label: Self::label(c).into(),
                on: false,
                pressed: self.pressed(c),
                enabled: self.enabled(c),
            })
        };
        let text = |text: &str, tone| Item::Text {
            text: text.into(),
            tone,
            bullet: false,
        };
        // One line under a field, where the field starts (as in Classic, which shows the
        // first line of it).
        let under = |line: String| Item::Line {
            label: String::new(),
            key: None,
            parts: vec![Part::Text(line, Tone::Dim)],
        };
        let mut items = vec![
            Item::Line {
                label: "Generator".into(),
                key: None,
                parts: vec![
                    radio(ctl::PROVIDER, SetupProvider::None),
                    radio(ctl::PROVIDER + 1, SetupProvider::OpenAi),
                ],
            },
            field(ctl::KEY, SetupField::Key, "API key"),
            under(self.key_line()),
            field(ctl::MODEL, SetupField::Model, "Model"),
            under(self.info.model_hint.clone()),
            Item::Line {
                label: String::new(),
                key: None,
                parts: vec![push(ctl::TEST), push(ctl::GET_KEY), push(ctl::REMOVE_KEY)],
            },
        ];
        if let Some(t) = &self.test {
            items.push(text(t, Tone::Normal));
        }
        items.push(Item::Rule);
        items.push(text(&self.info.note, Tone::Dim));
        // A line the app left empty takes no room.
        items.retain(|i| match i {
            Item::Text { text, .. } => !text.trim().is_empty(),
            Item::Line { parts, .. } => {
                !matches!(&parts[..], [Part::Text(text, _)] if text.trim().is_empty())
            }
            _ => true,
        });
        let button = |id: ControlId, role| {
            let c = kind(id);
            ViewButton::new(control(id), Self::label(c), role, self.pressed(c))
        };
        Dialog {
            title: TITLE.into(),
            kind: Kind::Plain,
            size: Size::Medium,
            columns: vec![items],
            buttons: vec![
                button(ctl::CANCEL, Role::Cancel),
                button(ctl::OK, Role::Default),
            ],
        }
    }

    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let (w, h) = self.size();
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, TITLE, pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        let openai = self.provider == SetupProvider::OpenAi;
        let text = |img: &mut IndexedImage, x: i32, y: i32, s: &str, on: bool| {
            font.draw(
                img,
                x,
                y,
                s,
                if on {
                    pens.dark
                } else {
                    pens.ghost.unwrap_or(pens.dark)
                },
            );
            if !on && pens.ghost.is_none() {
                checker(img, Rect::new(x, y, font.text_width(s), ROW_H), pens.face);
            }
        };
        text(&mut img, LEFT, geom::PROVIDER_Y, "Generator:", true);
        text(&mut img, LEFT, geom::KEY_Y, "API key:", openai);
        text(&mut img, LEFT, geom::MODEL_Y, "Model:", openai);
        let vx = LEFT + Self::label_w();
        let line_w = w - LEFT - vx;
        if let Some(line) = wrap(&self.key_line(), line_w).first() {
            text(&mut img, vx, geom::KEY_STATUS_Y, line, openai);
        }
        if let Some(line) = wrap(&self.info.model_hint, line_w).first() {
            text(&mut img, vx, geom::MODEL_HINT_Y, line, openai);
        }
        for (c, r) in self.controls() {
            match c {
                SetupControl::Provider(p) => {
                    draw_box(&mut img, (r.x, r.y), self.provider == p, false, pens);
                    font.draw(
                        &mut img,
                        r.x + BOX + 4,
                        r.y,
                        Self::provider_label(p),
                        pens.dark,
                    );
                }
                SetupControl::Field(f) => {
                    if openai {
                        let shown = match f {
                            SetupField::Key => self.masked(),
                            SetupField::Model => self.model.clone(),
                        };
                        draw_field(&mut img, r, &shown, self.active == Some(f), pens);
                    } else {
                        img.fill_rect(r.x, r.y, r.w, r.h, pens.dark);
                        checker(&mut img, r, pens.face);
                    }
                }
                _ => {
                    draw_button(&mut img, r, Self::label(c), self.pressed(c), pens);
                    if !self.enabled(c) {
                        checker(&mut img, r.inset(2), pens.face);
                    }
                }
            }
        }
        let full = w - 2 * LEFT;
        if let Some(t) = &self.test {
            for (i, line) in wrap(t, full).iter().take(2).enumerate() {
                font.draw(
                    &mut img,
                    LEFT,
                    geom::TEST_Y + i as i32 * LINE_STEP,
                    line,
                    pens.dark,
                );
            }
        }
        img.hline(1, w - 2, geom::NOTE_Y - 4, pens.dark);
        for (i, line) in wrap(&self.info.note, full)
            .iter()
            .take(self.note_room())
            .enumerate()
        {
            font.draw(
                &mut img,
                LEFT,
                geom::NOTE_Y + i as i32 * LINE_STEP,
                line,
                pens.dark,
            );
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

    fn info(has_key: bool) -> SetupInfo {
        SetupInfo {
            key_status: if has_key {
                "API key saved in the system keychain".into()
            } else {
                "No API key set up".into()
            },
            has_key,
            removable: has_key,
            model_hint: "Default gpt-image-2.6-flare (checked 2026-11-12)".into(),
            note: "Sends your description over the internet to OpenAI, which generates the \
                   picture. OpenAI bills every picture to your own API account."
                .into(),
        }
    }

    fn requester(p: SetupProvider, has_key: bool) -> ImageGenSetupRequester {
        ImageGenSetupRequester::new(p, "gpt-image-2.6-flare", info(has_key), (320, 256))
    }

    fn rect_of(r: &ImageGenSetupRequester, c: SetupControl) -> Rect {
        r.controls().into_iter().find(|&(k, _)| k == c).unwrap().1
    }

    fn click(r: &mut ImageGenSetupRequester, c: SetupControl) -> SetupResponse {
        let rect = rect_of(r, c);
        let (x, y) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
        let a = r.handle(ReqEvent::Press(x, y));
        let b = r.handle(ReqEvent::Release);
        if a == SetupResponse::Continue { b } else { a }
    }

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    #[test]
    fn layout_fits_small_and_large_screens() {
        for screen in [(320, 200), (320, 256), (368, 283)] {
            let r = ImageGenSetupRequester::new(SetupProvider::OpenAi, "m", info(true), screen);
            let (w, h) = r.size();
            let (ox, oy) = r.origin();
            assert!(ox >= 0 && oy >= 0 && ox + w <= screen.0 && oy + h <= screen.1);
            let bounds = Rect::new(1, 1, w - 2, h - 2);
            let controls = r.controls();
            for (i, &(c, rect)) in controls.iter().enumerate() {
                assert_eq!(rect.intersect(bounds), rect, "{c:?} inside");
                for &(d, other) in &controls[i + 1..] {
                    assert!(rect.intersect(other).is_empty(), "{c:?} overlaps {d:?}");
                }
            }
            // The note starts below the test lines and ends above the buttons.
            const { assert!(geom::TEST_Y + 2 * LINE_STEP <= geom::NOTE_Y - 4) };
            assert!(geom::NOTE_Y + 3 * LINE_STEP <= h - BUTTON_H - 5);
        }
    }

    #[test]
    fn the_key_is_masked_pasted_and_never_shown() {
        let pens = pens();
        let mut r = requester(SetupProvider::OpenAi, false);
        assert_eq!(r.active(), Some(SetupField::Key));
        for k in crate::requester::paste_keys("sk-secret-123\n") {
            r.handle(ReqEvent::Key(k));
        }
        // The trailing line break became a space; the key is trimmed.
        assert_eq!(r.typed_key().as_deref(), Some("sk-secret-123"));
        assert_eq!(r.key_line(), "The new key is saved at Ok.");
        assert!(!format!("{r:?}").contains("secret"), "Debug hides the key");
        // The field shows stars only: the same pixels as a field of stars.
        let img = r.render(&pens);
        let mut stars = r.clone();
        stars.key.set(&"*".repeat(r.key.chars.len()));
        stars.key.cursor = r.key.cursor;
        assert_eq!(img.pixels(), stars.render(&pens).pixels());
        let mut other = r.clone();
        other.key.set(&"x".repeat(r.key.chars.len()));
        other.key.cursor = r.key.cursor;
        assert_eq!(img.pixels(), other.render(&pens).pixels(), "length only");
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Enter)), SetupResponse::Ok);
    }

    #[test]
    fn tab_moves_between_fields_and_the_model_is_edited() {
        let mut r = requester(SetupProvider::OpenAi, true);
        r.handle(ReqEvent::Key(ReqKey::Tab));
        assert_eq!(r.active(), Some(SetupField::Model));
        for _ in 0..5 {
            r.handle(ReqEvent::Key(ReqKey::Backspace));
        }
        for c in "sunburst".chars() {
            r.handle(ReqEvent::Key(ReqKey::Char(c)));
        }
        assert_eq!(r.model(), "gpt-image-2.6-sunburst");
        assert_eq!(r.typed_key(), None, "typing went to the model");
        click(&mut r, SetupControl::Field(SetupField::Key));
        assert_eq!(r.active(), Some(SetupField::Key));
    }

    #[test]
    fn remove_key_waits_for_ok_and_a_new_key_cancels_it() {
        let mut r = requester(SetupProvider::OpenAi, true);
        assert!(r.enabled(SetupControl::RemoveKey));
        assert!(
            r.enabled(SetupControl::Test),
            "the stored key can be tested"
        );
        assert_eq!(
            click(&mut r, SetupControl::RemoveKey),
            SetupResponse::Continue
        );
        assert!(r.remove_key());
        assert_eq!(r.key_line(), "The API key is removed at Ok.");
        assert!(!r.enabled(SetupControl::RemoveKey));
        r.handle(ReqEvent::Key(ReqKey::Char('s')));
        assert!(!r.remove_key(), "a typed key replaces instead");
        // Without a stored key, Remove only clears the field.
        let mut r = requester(SetupProvider::OpenAi, false);
        assert!(!r.enabled(SetupControl::RemoveKey));
        assert!(!r.enabled(SetupControl::Test), "nothing to test");
        r.handle(ReqEvent::Key(ReqKey::Char('s')));
        assert!(r.enabled(SetupControl::Test));
        click(&mut r, SetupControl::RemoveKey);
        assert_eq!(r.typed_key(), None);
        assert!(!r.remove_key());
    }

    #[test]
    fn every_edit_a_key_test_depends_on_changes_the_revision_and_drops_the_answer() {
        fn changed(r: &mut ImageGenSetupRequester, last: &mut u64, what: &str) {
            assert!(r.revision() > *last, "{what} changes the revision");
            assert_eq!(r.test(), None, "{what} drops the answer");
            *last = r.revision();
            r.set_test(Some("The key works.".into()));
        }
        let mut r = requester(SetupProvider::OpenAi, true);
        let mut last = r.revision();
        r.set_test(Some("The key works.".into()));
        r.handle(ReqEvent::Key(ReqKey::Char('s')));
        changed(&mut r, &mut last, "typing");
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        changed(&mut r, &mut last, "deleting");
        // Moving the cursor or deleting nothing is no edit.
        r.handle(ReqEvent::Key(ReqKey::Left));
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        assert_eq!(r.revision(), last);
        assert_eq!(r.test(), Some("The key works."));
        r.handle(ReqEvent::Key(ReqKey::Tab));
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        changed(&mut r, &mut last, "the model");
        click(&mut r, SetupControl::RemoveKey);
        changed(&mut r, &mut last, "Remove key");
        click(&mut r, SetupControl::Provider(SetupProvider::None));
        changed(&mut r, &mut last, "another generator");
        click(&mut r, SetupControl::Provider(SetupProvider::None));
        assert_eq!(r.revision(), last, "the same generator again");
        click(&mut r, SetupControl::Provider(SetupProvider::OpenAi));
        for k in crate::requester::paste_keys("sk-1") {
            r.handle(ReqEvent::Key(k));
        }
        assert!(r.typed_key().is_some());
        let before = r.revision();
        r.clear_key();
        assert!(r.revision() > before);
        assert_eq!(r.typed_key(), None);
        assert!(!r.remove_key());
        assert_eq!(r.key_line(), r.info().key_status);
    }

    #[test]
    fn none_ghosts_the_openai_entries_and_buttons_answer() {
        let mut r = requester(SetupProvider::None, true);
        assert_eq!(r.active(), None);
        for c in [
            SetupControl::Test,
            SetupControl::GetKey,
            SetupControl::RemoveKey,
            SetupControl::Field(SetupField::Key),
        ] {
            assert!(!r.enabled(c), "{c:?}");
        }
        r.handle(ReqEvent::Key(ReqKey::Char('x')));
        assert_eq!(r.typed_key(), None, "no field takes keys");
        assert_eq!(
            click(&mut r, SetupControl::Provider(SetupProvider::OpenAi)),
            SetupResponse::Continue
        );
        assert_eq!(r.provider(), SetupProvider::OpenAi);
        assert_eq!(click(&mut r, SetupControl::GetKey), SetupResponse::GetKey);
        assert_eq!(click(&mut r, SetupControl::Test), SetupResponse::Test);
        assert_eq!(click(&mut r, SetupControl::Cancel), SetupResponse::Cancel);
        assert_eq!(click(&mut r, SetupControl::Ok), SetupResponse::Ok);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            SetupResponse::Cancel
        );
        let pens = pens();
        let img = r.render(&pens);
        assert_eq!(img.get(0, 0), Some(pens.dark), "square corner");
        r.set_test(Some("The key works.".into()));
        assert_ne!(r.render(&pens).pixels(), img.pixels());
    }
    #[test]
    fn the_view_names_every_control_and_never_shows_the_key() {
        use crate::dialog::{Item, Part, Role, Tone};
        let mut r = requester(SetupProvider::OpenAi, true);
        let view = r.view();
        assert_eq!(view.title, "Image Generator");
        // The ids are the places in `controls()`, with the same rectangles.
        let controls = r.controls();
        assert_eq!(view.controls().len(), controls.len());
        for c in view.controls() {
            assert_eq!(c.area, controls[usize::from(c.id)].1, "control {}", c.id);
        }
        assert_eq!(
            view.buttons
                .iter()
                .map(|b| (b.label.as_str(), b.role))
                .collect::<Vec<_>>(),
            [("Cancel", Role::Cancel), ("Ok", Role::Default)]
        );
        let fields = |r: &ImageGenSetupRequester| -> Vec<(String, bool, crate::dialog::Field)> {
            r.view().columns[0]
                .iter()
                .filter_map(|i| match i {
                    Item::Setting {
                        label,
                        enabled,
                        parts,
                        ..
                    } => match &parts[0] {
                        Part::Field(f) => Some((label.clone(), *enabled, f.clone())),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        };
        let buttons = |r: &ImageGenSetupRequester| -> Vec<(String, bool)> {
            r.view().columns[0]
                .iter()
                .filter_map(|i| match i {
                    Item::Line { parts, .. } => Some(parts),
                    _ => None,
                })
                .flatten()
                .filter_map(|p| match p {
                    Part::Button(b) => Some((b.label.clone(), b.enabled)),
                    _ => None,
                })
                .collect()
        };
        let texts = |r: &ImageGenSetupRequester| -> Vec<String> {
            r.view().columns[0]
                .iter()
                .filter_map(|i| match i {
                    Item::Text { text, .. } => Some(text.clone()),
                    Item::Line { parts, .. } => match &parts[..] {
                        [Part::Text(text, _)] => Some(text.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        };
        // The key field has the keys; a typed or pasted key is shown as stars only.
        let f = fields(&r);
        assert_eq!((f[0].0.as_str(), f[0].2.caret), ("API key", Some(0)));
        assert_eq!((f[1].0.as_str(), f[1].2.caret), ("Model", None));
        assert_eq!(f[1].2.text, "gpt-image-2.6-flare");
        let secret = "sk-secret-Key_0123456789";
        for k in crate::paste_keys(secret) {
            r.handle(ReqEvent::Key(k));
        }
        r.handle(ReqEvent::Key(ReqKey::Left));
        let key = &fields(&r)[0].2;
        assert_eq!(key.text, "*".repeat(secret.len()));
        assert_eq!(key.caret, Some(secret.len() - 1));
        assert!(key.stops.is_empty());
        let dump = format!("{:?}", r.view());
        assert!(!dump.contains(secret) && !dump.contains("sk-") && !dump.contains("Key_0"));
        assert!(!format!("{r:?}").contains("secret"));
        assert_eq!(texts(&r)[0], "The new key is saved at Ok.");
        assert_eq!(
            buttons(&r),
            [
                ("Test".to_string(), true),
                ("Get key..".to_string(), true),
                ("Remove key".to_string(), true)
            ]
        );
        // A press in the middle of every control's rectangle reaches it.
        let press = |r: &mut ImageGenSetupRequester, id| {
            let a = r.view().control(id).expect("the control").area;
            let first = r.handle(ReqEvent::Press(a.x + a.w / 2, a.y + a.h / 2));
            let second = r.handle(ReqEvent::Release);
            if first == SetupResponse::Continue {
                second
            } else {
                first
            }
        };
        assert_eq!(press(&mut r, ctl::TEST), SetupResponse::Test);
        assert_eq!(press(&mut r, ctl::GET_KEY), SetupResponse::GetKey);
        assert_eq!(press(&mut r, ctl::MODEL), SetupResponse::Continue);
        assert_eq!(fields(&r)[1].2.caret, Some("gpt-image-2.6-flare".len()));
        assert_eq!(press(&mut r, ctl::KEY), SetupResponse::Continue);
        assert_eq!(r.active(), Some(SetupField::Key));
        assert_eq!(press(&mut r, ctl::REMOVE_KEY), SetupResponse::Continue);
        assert_eq!(fields(&r)[0].2.text, "");
        assert_eq!(texts(&r)[0], "The API key is removed at Ok.");
        assert_eq!(buttons(&r)[2], ("Remove key".to_string(), false));
        assert_eq!(press(&mut r, ctl::CANCEL), SetupResponse::Cancel);
        assert_eq!(press(&mut r, ctl::OK), SetupResponse::Ok);
        // A test answer is shown; None ghosts the fields and their buttons.
        r.set_test(Some("The key works.".into()));
        assert!(texts(&r).contains(&"The key works.".to_string()));
        assert_eq!(press(&mut r, ctl::PROVIDER), SetupResponse::Continue);
        let f = fields(&r);
        assert!(!f[0].1 && !f[0].2.enabled && f[0].2.caret.is_none());
        assert!(
            !f[1].1 && f[1].2.text.is_empty(),
            "nothing of a ghosted field is shown"
        );
        assert!(buttons(&r).iter().all(|b| !b.1));
        assert!(matches!(&r.view().columns[0][0],
            Item::Line { parts, .. } if matches!(&parts[0], Part::Radio(n) if n.on)));
        // The note is the last, quiet text.
        assert!(matches!(r.view().columns[0].last(),
            Some(Item::Text { text, tone: Tone::Dim, .. }) if text.starts_with("Sends your")));
    }
}
