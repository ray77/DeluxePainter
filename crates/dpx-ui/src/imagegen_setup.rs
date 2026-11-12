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
                self.provider = p;
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
                self.test = None;
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
                        let before = self.key.chars.len();
                        self.key.key(k);
                        if self.key.chars.len() > before {
                            // A new key replaces the stored one instead of removing it.
                            self.remove = false;
                            self.test = None;
                        }
                    }
                    Some(SetupField::Model) => self.model.key(k),
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
}
