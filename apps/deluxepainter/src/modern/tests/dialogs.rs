//! Tests of the requesters the Modern skin draws itself (ADR-47, slice 4a). Part of
//! `modern::tests`: they drive a Classic and a Modern window with real `ui()` passes.
//!
//! A script addresses a requester's controls by their ids. In the Classic window the click
//! goes to the rectangle in which the requester takes that control's press; in the Modern
//! window to the control's place on the card. After every step both windows must be in the
//! same state: the document and the whole interface state, and of the converted requesters
//! everything they show (their descriptions) and the command they would send. Only the raw
//! pointer position a requester remembers is left out: beside the controls the two skins
//! have nothing in common to compare it by.

use super::*;
use crate::autosave::Pending;
use crate::modern_dialog::Card;
use dpx_ui::dialog::ControlId;

type Seen<'a> = &'a mut dyn FnMut(&mut Rig, String);

/// What both skins must agree on while and after a converted requester is open.
fn dialog_state(rig: &mut Rig) -> String {
    let app = &mut rig.app;
    let open = app.modal_open();
    let pending = app.autosave.pending.clone();
    let confirmed = app.autosave.quit_confirmed;
    // The converted requesters by what they show; then the rest without them.
    let note = app.ui.note_requester.take();
    let choice = app.ui.choice_requester.take();
    let page = app.ui.page_size_requester.take();
    let format = app.requester.take();
    let palette = app.palette().clone();
    let tool = app
        .ui
        .tool_requester
        .take_if(|r| r.view(&palette).is_some());
    let anim = app.ui.anim_requester.take();
    let control = app.ui.anim_control.take_if(|r| r.has_view());
    let percent = (
        app.menu_data.effects.percent,
        app.engine.document().fps(),
        app.anim_control.range,
    );
    let shown = format!(
        "{:?}",
        (
            note.as_ref().map(|r| r.view()),
            choice.as_ref().map(|r| r.view()),
            page.as_ref().map(|r| r.view()),
            format.as_ref().map(|r| (r.view(), r.command())),
            tool.as_ref().map(|r| r.view(&palette)),
            anim.as_ref().map(|r| r.view()),
            control.as_ref().map(|r| r.view()),
            percent,
            open,
            pending,
            confirmed,
        )
    );
    let closes = rig
        .commands
        .iter()
        .filter(|c| matches!(c, egui::ViewportCommand::Close))
        .count();
    let rest = rig.state();
    let app = &mut rig.app;
    app.ui.note_requester = note;
    app.ui.choice_requester = choice;
    app.ui.page_size_requester = page;
    app.requester = format;
    if tool.is_some() {
        app.ui.tool_requester = tool;
    }
    app.ui.anim_requester = anim;
    if control.is_some() {
        app.ui.anim_control = control;
    }
    format!("{shown} closes: {closes} {rest}")
}

/// Runs `script` in a Classic and in a Modern window; after every step both must be in the
/// same state ([`dialog_state`]).
fn both(modern: Rig, script: fn(&mut Rig, Seen)) {
    let run = |mut rig: Rig| {
        let mut states = vec![("start".to_string(), dialog_state(&mut rig))];
        script(&mut rig, &mut |rig: &mut Rig, step: String| {
            if !rig.app.modal_open() {
                // A requester's button is not at the same place of the two windows: the
                // pointer rests on the same screen pixel before they are compared.
                rig.move_to(Place::Screen(30, 200));
            }
            let state = dialog_state(rig);
            states.push((step, state));
        });
        states
    };
    let (classic, modern) = (run(Rig::classic()), run(modern));
    assert_eq!(classic.len(), modern.len());
    assert!(classic.len() > 3, "the script reported its steps");
    for ((step, c), (_, m)) in classic.iter().zip(&modern) {
        if c != m {
            let at = c
                .bytes()
                .zip(m.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            let from = at.saturating_sub(160);
            panic!(
                "the skins differ after '{step}':\n classic ..{}\n modern  ..{}",
                &c[from..(at + 160).min(c.len())],
                &m[from..(at + 160).min(m.len())]
            );
        }
    }
}

/// The same script at device scale 1, at 2, and in an odd window at 1.5.
fn both_at_every_scale(script: fn(&mut Rig, Seen)) {
    both(Rig::modern(), script);
    both(Rig::new(UiStyle::Modern, (1280.0, 800.0), 2.0), script);
    both(Rig::new(UiStyle::Modern, (1001.0, 700.0), 1.5), script);
}

impl Rig {
    /// A window point beside every requester and card: the lower left corner.
    fn beside(&self) -> egui::Pos2 {
        egui::pos2(9.0, self.size.y - 9.0)
    }

    /// A left click beside the requester.
    fn click_beside(&mut self) {
        let pos = self.beside();
        self.pass(vec![
            egui::Event::PointerMoved(pos),
            pointer_button(pos, Primary, true),
        ]);
        self.pass(vec![pointer_button(pos, Primary, false)]);
    }

    /// Press on control `id`, drag off it, let go beside the requester.
    fn press_and_leave(&mut self, id: ControlId) {
        let pos = self.control_point(id, (0.5, 0.5));
        self.pass(vec![
            egui::Event::PointerMoved(pos),
            pointer_button(pos, Primary, true),
        ]);
        let away = self.beside();
        self.pass(vec![egui::Event::PointerMoved(away)]);
        self.pass(vec![pointer_button(away, Primary, false)]);
    }

    /// Press beside the requester, drag onto control `id`, let go there.
    fn press_beside_and_enter(&mut self, id: ControlId) {
        let away = self.beside();
        self.pass(vec![
            egui::Event::PointerMoved(away),
            pointer_button(away, Primary, true),
        ]);
        let pos = self.control_point(id, (0.5, 0.5));
        self.pass(vec![egui::Event::PointerMoved(pos)]);
        self.pass(vec![pointer_button(pos, Primary, false)]);
    }

    fn type_text(&mut self, text: &str) {
        self.pass(vec![egui::Event::Text(text.into())]);
    }

    fn wheel(&mut self, lines: f32) {
        self.pass(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, lines),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        }]);
    }

    fn show_note(&mut self) {
        self.app.ui.note_requester = crate::note_requester(
            "Picture loaded",
            &[
                "mask plane not kept".to_string(),
                "the picture had more colours than the screen shows, so they were reduced"
                    .to_string(),
            ],
        );
        self.app.dirty = true;
        self.pass(vec![]);
        assert!(self.app.ui.note_requester.is_some());
    }

    /// The card the Modern window shows; in the Classic window always `None`.
    fn card(&self) -> Option<Card> {
        self.app.modern_card()
    }
}

/// A note: nothing but Ok, Return, Esc and Space closes it.
fn note_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::note_requester::OK;
    type Close = fn(&mut Rig);
    let ways: [(&str, Close); 4] = [
        ("Ok", |rig| rig.click_control(OK, (0.5, 0.5))),
        ("Return", |rig| rig.tap(egui::Key::Enter)),
        ("Esc", |rig| rig.tap(egui::Key::Escape)),
        ("Space", |rig| rig.type_text(" ")),
    ];
    for (name, close) in ways {
        rig.show_note();
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::Note));
        }
        seen(rig, "a note".into());
        rig.click_beside();
        seen(rig, "a click beside it".into());
        rig.press_and_leave(OK);
        seen(rig, "Ok pressed and left".into());
        rig.press_beside_and_enter(OK);
        seen(rig, "pressed beside, let go on Ok".into());
        rig.tap(egui::Key::Tab);
        rig.type_text("x");
        rig.wheel(-2.0);
        rig.tap(egui::Key::ArrowDown);
        assert!(rig.app.ui.note_requester.is_some(), "still open");
        seen(rig, "Tab, a letter, the wheel, an arrow".into());
        close(rig);
        assert!(
            rig.app.ui.note_requester.is_none(),
            "{name} closes the note"
        );
        seen(rig, name.to_string());
    }
    // The first and the last pixel of Ok are Ok.
    for corner in [(0.02, 0.05), (0.98, 0.95)] {
        rig.show_note();
        rig.click_control(OK, corner);
        assert!(rig.app.ui.note_requester.is_none(), "Ok at {corner:?}");
        seen(rig, format!("Ok at {corner:?}"));
    }
}

#[test]
fn the_note_requester_does_the_same_in_both_skins() {
    both_at_every_scale(note_script);
}

/// Picture ▸ About.. and the `?` key: our text in the note requester.
fn about_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::note_requester::OK;
    rig.pick(Action::Command("app.about"));
    let about = rig.app.ui.note_requester.as_ref().expect("About");
    assert_eq!(about.view().kind, dpx_ui::dialog::Kind::About);
    seen(rig, "About..".into());
    rig.press_and_leave(OK);
    seen(rig, "Ok pressed and left".into());
    rig.click_control(OK, (0.5, 0.5));
    assert!(rig.app.ui.note_requester.is_none());
    seen(rig, "Ok".into());
    rig.type_text("?");
    assert!(rig.app.ui.note_requester.is_some(), "? opens About");
    seen(rig, "the ? key".into());
    rig.tap(egui::Key::Enter);
    assert!(rig.app.ui.note_requester.is_none());
    seen(rig, "Return".into());
}

#[test]
fn about_does_the_same_in_both_skins() {
    both_at_every_scale(about_script);
}

/// The questions: Quit (Yes / No, Return and Esc say No) and Recover Picture (Recover /
/// Discard, Return recovers, Esc discards), by both buttons and both keys.
fn question_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::choice_requester::BUTTONS;
    // Something unsaved, so Quit asks.
    rig.click(Place::Page(40, 40), Primary);
    seen(rig, "a dot".into());
    type Answer = fn(&mut Rig);
    let no: [(&str, Answer); 4] = [
        ("No", |rig| rig.click_control(BUTTONS[1], (0.5, 0.5))),
        ("Return", |rig| rig.tap(egui::Key::Enter)),
        ("Esc", |rig| rig.tap(egui::Key::Escape)),
        ("No, at its last pixel", |rig| {
            rig.click_control(BUTTONS[1], (0.98, 0.95))
        }),
    ];
    for (name, answer) in no {
        rig.pick(Action::Command("app.quit"));
        assert!(rig.app.ui.choice_requester.is_some(), "Quit asks");
        assert_eq!(rig.app.autosave.pending, Some(Pending::Quit));
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::Choice));
        }
        seen(rig, "Quit?".into());
        rig.click_beside();
        rig.type_text("y");
        rig.tap(egui::Key::Tab);
        assert!(rig.app.ui.choice_requester.is_some(), "still asking");
        seen(rig, "a click beside it, a letter, Tab".into());
        for button in BUTTONS {
            rig.press_and_leave(button);
            seen(rig, format!("button {button} pressed and left"));
        }
        rig.press_beside_and_enter(BUTTONS[0]);
        assert!(rig.app.ui.choice_requester.is_some(), "still asking");
        seen(rig, "pressed beside, let go on Yes".into());
        answer(rig);
        assert!(rig.app.ui.choice_requester.is_none(), "{name} answers");
        assert!(!rig.app.autosave.quit_confirmed, "{name} is No");
        seen(rig, name.to_string());
    }
    // Recover Picture, as at a start after a crash (no recovery folder in this test).
    let recover: [(&str, Answer); 5] = [
        ("Recover", |rig| rig.click_control(BUTTONS[0], (0.5, 0.5))),
        ("Discard", |rig| rig.click_control(BUTTONS[1], (0.5, 0.5))),
        ("Return", |rig| rig.tap(egui::Key::Enter)),
        ("Esc", |rig| rig.tap(egui::Key::Escape)),
        ("Recover, at its first pixel", |rig| {
            rig.click_control(BUTTONS[0], (0.02, 0.05))
        }),
    ];
    for (name, answer) in recover {
        rig.app.ui.choice_requester = Some(crate::autosave::recover_requester(
            "Castle",
            "2026-11-14 10:00 UTC".into(),
        ));
        rig.app.autosave.pending = Some(Pending::Recover);
        rig.app.dirty = true;
        rig.pass(vec![]);
        seen(rig, "Recover Picture?".into());
        rig.press_and_leave(BUTTONS[0]);
        seen(rig, "Recover pressed and left".into());
        answer(rig);
        assert!(rig.app.ui.choice_requester.is_none(), "{name} answers");
        assert_eq!(rig.app.autosave.pending, None, "{name}");
        seen(rig, name.to_string());
    }
    // Quit: Yes closes the window.
    rig.pick(Action::Command("app.quit"));
    seen(rig, "Quit?".into());
    rig.click_control(BUTTONS[0], (0.5, 0.5));
    assert!(rig.app.autosave.quit_confirmed, "Yes");
    assert!(
        rig.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
    seen(rig, "Yes".into());
}

#[test]
fn the_questions_do_the_same_in_both_skins() {
    both_at_every_scale(question_script);
}

/// Picture ▸ Page Size: the fields with every editing key, the presets, Cancel, Ok, Return
/// and Esc, a missing and a refused size.
fn page_size_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::page_size_requester::ctl;
    let open = |rig: &mut Rig, seen: Seen| {
        rig.pick(Action::Command("picture.page_size"));
        assert!(rig.app.ui.page_size_requester.is_some());
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::PageSize));
        }
        seen(rig, "Page Size..".into());
    };
    let page = |rig: &Rig| {
        let p = rig.app.engine.document().page();
        (p.width(), p.height())
    };
    open(rig, seen);
    for preset in 0..3 {
        rig.click_control(ctl::PRESET + preset, (0.5, 0.5));
        seen(rig, format!("preset {preset}"));
        rig.press_and_leave(ctl::PRESET + (preset + 1) % 3);
        seen(rig, "another preset pressed and left".into());
    }
    rig.click_control(ctl::PRESET + 2, (0.97, 0.9));
    seen(rig, "the last preset by its last pixel".into());
    // The Height field: typing, the editing keys, what is not a digit, pasted text.
    rig.click_control(ctl::HEIGHT, (0.5, 0.5));
    seen(rig, "the Height field".into());
    let keys = [
        egui::Key::Backspace,
        egui::Key::ArrowLeft,
        egui::Key::Delete,
        egui::Key::Home,
        egui::Key::ArrowRight,
        egui::Key::End,
        egui::Key::ArrowUp,
        egui::Key::ArrowDown,
    ];
    for key in keys {
        rig.tap(key);
        seen(rig, format!("{key:?}"));
        rig.type_text("7");
        seen(rig, format!("7 after {key:?}"));
        rig.tap(egui::Key::Backspace);
    }
    rig.type_text("ab-9");
    seen(rig, "letters and a digit".into());
    rig.pass(vec![egui::Event::Paste("1x2\n3456".into())]);
    seen(rig, "pasted text".into());
    rig.tap(egui::Key::Tab);
    seen(rig, "Tab".into());
    for _ in 0..5 {
        rig.tap(egui::Key::Backspace);
    }
    seen(rig, "the Width field emptied".into());
    // An empty field: Return and Ok keep the requester open with a message.
    rig.tap(egui::Key::Enter);
    assert!(
        rig.app.ui.page_size_requester.is_some(),
        "a width is missing"
    );
    seen(rig, "Return with an empty field".into());
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(rig.app.ui.page_size_requester.is_some());
    seen(rig, "Ok with an empty field".into());
    rig.type_text("0");
    rig.tap(egui::Key::Enter);
    assert!(rig.app.ui.page_size_requester.is_some(), "zero is no width");
    seen(rig, "a zero width".into());
    // A size the screen format refuses: the message with the limits.
    rig.tap(egui::Key::Backspace);
    rig.type_text("9999");
    rig.click_control(ctl::WIDTH, (0.1, 0.5));
    rig.tap(egui::Key::Tab);
    for _ in 0..5 {
        rig.tap(egui::Key::Backspace);
    }
    rig.type_text("9999");
    seen(rig, "9999 x 9999".into());
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(rig.app.ui.page_size_requester.is_some(), "too large");
    seen(rig, "Ok with a size that is refused".into());
    rig.tap(egui::Key::Escape);
    assert!(rig.app.ui.page_size_requester.is_none(), "Esc closes it");
    assert_eq!(page(rig), (320, 256));
    seen(rig, "Esc".into());
    // Cancel; Ok pressed and left; a click beside.
    open(rig, seen);
    rig.click_control(ctl::PRESET + 1, (0.5, 0.5));
    rig.press_and_leave(ctl::OK);
    rig.press_and_leave(ctl::CANCEL);
    rig.click_beside();
    rig.press_beside_and_enter(ctl::OK);
    assert!(rig.app.ui.page_size_requester.is_some());
    seen(rig, "Ok and Cancel pressed and left".into());
    rig.click_control(ctl::CANCEL, (0.03, 0.06));
    assert!(rig.app.ui.page_size_requester.is_none(), "Cancel closes it");
    assert_eq!(page(rig), (320, 256));
    seen(rig, "Cancel".into());
    // A preset and Ok: the page has the size.
    open(rig, seen);
    rig.click_control(ctl::PRESET + 1, (0.5, 0.5));
    rig.click_control(ctl::OK, (0.97, 0.94));
    assert!(rig.app.ui.page_size_requester.is_none(), "Ok closes it");
    assert_ne!(page(rig), (320, 256));
    seen(rig, "Full Page and Ok".into());
    // A typed size and Return.
    open(rig, seen);
    for _ in 0..4 {
        rig.tap(egui::Key::Backspace);
    }
    rig.type_text("400");
    rig.tap(egui::Key::Enter);
    assert!(rig.app.ui.page_size_requester.is_none(), "Return accepts");
    assert_eq!(page(rig).0, 400);
    seen(rig, "400 wide and Return".into());
    rig.key('u');
    seen(rig, "undo".into());
}

#[test]
fn the_page_size_requester_does_the_same_in_both_skins() {
    both_at_every_scale(page_size_script);
}

/// Picture ▸ Screen Format: every gadget with the pointer, every key and hotkey.
fn screen_format_all_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::requester::ctl;
    let open = |rig: &mut Rig, seen: Seen| {
        rig.pick(Action::Command("picture.screen_format"));
        rig.pass(vec![]);
        assert!(rig.app.requester.is_some());
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::ScreenFormat));
        }
        seen(rig, "Screen Format..".into());
    };
    let colours = |rig: &Rig| rig.app.requester.as_ref().expect("open").colors();
    open(rig, seen);
    // The Palette Size slider: every step by a click, the ends by a drag beyond them.
    for step in 0..8u16 {
        rig.click_control(
            ctl::COLORS,
            ((2.0 * f32::from(step) + 1.0) / 16.0 + 0.004, 0.5),
        );
        assert_eq!(colours(rig), 2 << step, "step {step}");
        seen(rig, format!("{} colours", 2 << step));
    }
    for (to, most) in [
        (egui::pos2(rig.size.x - 3.0, 40.0), 256),
        (egui::pos2(3.0, 300.0), 2),
    ] {
        let from = rig.control_point(ctl::COLORS, (0.5, 0.5));
        rig.pass(vec![
            egui::Event::PointerMoved(from),
            pointer_button(from, Primary, true),
        ]);
        seen(rig, "the slider held".into());
        rig.pass(vec![egui::Event::PointerMoved(to)]);
        assert_eq!(colours(rig), most, "dragged beyond the end");
        seen(rig, format!("dragged to {most} colours"));
        rig.pass(vec![pointer_button(to, Primary, false)]);
        seen(rig, "let go".into());
    }
    rig.click_control(ctl::COLORS, (9.0 / 16.0 + 0.004, 0.5));
    assert_eq!(colours(rig), 32);
    seen(rig, "32 colours again".into());
    // The mode list: every row, the arrow keys, the wheel, the scroll bar.
    for row in 0..5 {
        rig.click_control(ctl::ROW + row, (0.5, 0.5));
        seen(rig, format!("row {row}"));
    }
    rig.click_control(ctl::ROW + 4, (0.99, 0.95));
    seen(rig, "the last row by its last pixel".into());
    for key in [
        egui::Key::ArrowDown,
        egui::Key::ArrowDown,
        egui::Key::ArrowUp,
    ] {
        rig.tap(key);
        seen(rig, format!("{key:?}"));
    }
    for lines in [3.0, -1.0, -9.0, 2.0] {
        rig.wheel(lines);
        seen(rig, format!("the wheel by {lines}"));
    }
    rig.click_control(ctl::SCROLL, (0.5, 0.98));
    seen(rig, "a click at the end of the scroll bar".into());
    rig.click_control(ctl::SCROLL, (0.5, 0.02));
    seen(rig, "a click at its start".into());
    // The knob dragged down and back: pressed in its middle.
    let knob = |rig: &Rig| {
        let dialog = rig.dialog();
        let list = dialog.columns[0]
            .iter()
            .find_map(|item| match item {
                dpx_ui::dialog::Item::List(list) => list.scroll,
                _ => None,
            })
            .expect("the scroll bar");
        let (area, knob) = (list.control.area, list.knob);
        ((knob.y - area.y) as f32 + knob.h as f32 / 2.0) / area.h as f32
    };
    let from = rig.control_point(ctl::SCROLL, (0.5, knob(rig)));
    rig.pass(vec![
        egui::Event::PointerMoved(from),
        pointer_button(from, Primary, true),
    ]);
    seen(rig, "the knob held".into());
    for to in [0.6, 0.95, 0.3] {
        let pos = rig.control_point(ctl::SCROLL, (0.5, to));
        rig.pass(vec![egui::Event::PointerMoved(pos)]);
        seen(rig, format!("the knob dragged to {to}"));
    }
    let pos = rig.control_point(ctl::SCROLL, (0.5, 0.3));
    rig.pass(vec![pointer_button(pos, Primary, false)]);
    seen(rig, "the knob let go".into());
    // Screen Size: Standard, Overscan, Custom with its two number boxes.
    rig.click_control(ctl::SCREEN, (0.5, 0.5));
    seen(rig, "Overscan".into());
    rig.click_control(ctl::SCREEN, (0.5, 0.5));
    seen(rig, "Custom".into());
    rig.type_text("5");
    seen(rig, "a digit with no box active".into());
    rig.click_control(ctl::SCREEN_H, (0.5, 0.5));
    seen(rig, "the height box".into());
    for key in [
        egui::Key::Backspace,
        egui::Key::ArrowLeft,
        egui::Key::Delete,
        egui::Key::Home,
        egui::Key::ArrowRight,
        egui::Key::End,
    ] {
        rig.tap(key);
        rig.type_text("2");
        seen(rig, format!("{key:?} and a digit"));
    }
    rig.tap(egui::Key::Tab);
    seen(rig, "Tab".into());
    rig.tap(egui::Key::Backspace);
    rig.type_text("4x");
    seen(rig, "a digit and a letter in the width box".into());
    rig.pass(vec![egui::Event::Paste("1r".into())]);
    seen(rig, "pasted text".into());
    rig.click_control(ctl::SCREEN_W, (0.9, 0.5));
    rig.tap(egui::Key::Tab);
    seen(rig, "the width box by a click, Tab".into());
    rig.click_control(ctl::SCREEN, (0.5, 0.5));
    seen(rig, "Standard".into());
    // Page Size, Retain Picture, Preset, Chipset: by the pointer and by their keys.
    for step in ["Keep Same", "Screen"] {
        rig.click_control(ctl::PAGE, (0.5, 0.5));
        seen(rig, format!("Page Size: {step}"));
    }
    rig.click_control(ctl::RETAIN, (0.5, 0.5));
    seen(rig, "Retain Picture off".into());
    rig.key('r');
    seen(rig, "the r key".into());
    for _ in 0..3 {
        rig.click_control(ctl::PRESET, (0.5, 0.5));
        seen(rig, "the next preset".into());
    }
    rig.key('p');
    seen(rig, "the p key".into());
    rig.click_control(ctl::CHIPSET, (0.5, 0.5));
    seen(rig, "the other chipset".into());
    rig.key('h');
    seen(rig, "the h key".into());
    rig.key('H');
    rig.key('h');
    seen(rig, "H and h".into());
    // Buttons pressed and left, a click beside: still open.
    rig.press_and_leave(ctl::USE);
    rig.press_and_leave(ctl::CANCEL);
    rig.click_beside();
    rig.press_beside_and_enter(ctl::CANCEL);
    assert!(rig.app.requester.is_some());
    seen(rig, "Use and Cancel pressed and left".into());
    // Every way out without a change.
    type Close = fn(&mut Rig);
    let ways: [(&str, Close); 3] = [
        ("the c key", |rig| rig.key('c')),
        ("Esc", |rig| rig.tap(egui::Key::Escape)),
        ("Cancel", |rig| rig.click_control(ctl::CANCEL, (0.5, 0.5))),
    ];
    for (i, (name, close)) in ways.into_iter().enumerate() {
        if i > 0 {
            open(rig, seen);
            rig.click_control(ctl::RETAIN, (0.5, 0.5));
        }
        close(rig);
        assert!(rig.app.requester.is_none(), "{name} closes it");
        rig.pass(vec![]);
        seen(rig, name.to_string());
    }
    // Use: by the u key, by Return and by the button, each with another palette size.
    let uses: [(&str, f32, Close); 3] = [
        ("the u key", 11.0, |rig| rig.key('u')),
        ("Return", 7.0, |rig| rig.tap(egui::Key::Enter)),
        ("Use", 9.0, |rig| rig.click_control(ctl::USE, (0.5, 0.5))),
    ];
    for (name, sixteenth, accept) in uses {
        open(rig, seen);
        rig.click_control(ctl::COLORS, (sixteenth / 16.0 + 0.004, 0.5));
        let wanted = usize::from(colours(rig));
        seen(rig, format!("{wanted} colours"));
        accept(rig);
        assert!(rig.app.requester.is_none(), "{name} closes it");
        assert_eq!(rig.app.engine.document().palette().len(), wanted, "{name}");
        rig.pass(vec![]);
        seen(rig, name.to_string());
        if rig.app.ui.note_requester.is_some() {
            rig.tap(egui::Key::Enter);
            seen(rig, "the note after it".into());
        }
    }
}

#[test]
fn the_screen_format_requester_does_the_same_in_both_skins() {
    both_at_every_scale(screen_format_all_script);
}

/// The requesters built on the shared form (slice 4b): Gridding, Symmetry, Spacing, Fill
/// Type and Translucency. Every field with every editing key, Tab, a refused number, every
/// radio button and check box, Cancel, Ok, Return and Esc.
fn form_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::form_ctl as ctl;
    use dpx_ui::{SpacingMode, ToolRequester};
    let opened = |rig: &mut Rig, seen: Seen, name: &str| {
        assert!(rig.app.ui.tool_requester.is_some(), "{name} is open");
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::Tool), "{name}");
        }
        seen(rig, format!("{name}.."));
    };
    let open = |rig: &Rig| rig.app.ui.tool_requester.is_some();
    let erase = |rig: &mut Rig, n: usize| {
        for _ in 0..n {
            rig.tap(egui::Key::Backspace);
        }
    };
    // Gridding: two fields.
    let grid = |rig: &mut Rig, seen: Seen| {
        rig.click(Place::Tool(Tool::Grid, false), Secondary);
        assert!(matches!(
            rig.app.ui.tool_requester,
            Some(ToolRequester::Grid(_))
        ));
        opened(rig, seen, "Gridding");
    };
    grid(rig, seen);
    rig.click_control(ctl::FIELD + 1, (0.5, 0.5));
    seen(rig, "the Y field".into());
    for key in [
        egui::Key::Backspace,
        egui::Key::ArrowLeft,
        egui::Key::Delete,
        egui::Key::Home,
        egui::Key::ArrowRight,
        egui::Key::End,
        egui::Key::ArrowUp,
        egui::Key::ArrowDown,
    ] {
        rig.tap(key);
        seen(rig, format!("{key:?}"));
        rig.type_text("7");
        seen(rig, format!("7 after {key:?}"));
        rig.tap(egui::Key::Backspace);
    }
    rig.type_text("ab-9");
    seen(rig, "letters and a digit".into());
    rig.pass(vec![egui::Event::Paste("1x2\n34".into())]);
    seen(rig, "pasted text".into());
    rig.tap(egui::Key::Tab);
    seen(rig, "Tab".into());
    erase(rig, 3);
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "an X spacing is missing");
    seen(rig, "Return with an empty field".into());
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(open(rig));
    seen(rig, "Ok with an empty field".into());
    rig.type_text("0");
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "zero is no spacing");
    seen(rig, "a zero spacing".into());
    rig.click_control(ctl::FIELD, (0.9, 0.5));
    rig.click_control(ctl::FIELD + 1, (0.04, 0.1));
    seen(rig, "both fields by a click".into());
    rig.tap(egui::Key::Escape);
    assert!(!open(rig), "Esc closes it");
    assert_eq!(
        (rig.app.tools.grid_size.x, rig.app.tools.grid_size.y),
        (16, 16)
    );
    seen(rig, "Esc".into());
    grid(rig, seen);
    rig.press_and_leave(ctl::OK);
    rig.press_and_leave(ctl::CANCEL);
    rig.click_beside();
    rig.press_beside_and_enter(ctl::OK);
    rig.wheel(-2.0);
    assert!(open(rig));
    seen(rig, "Ok and Cancel pressed and left, the wheel".into());
    rig.click_control(ctl::CANCEL, (0.03, 0.06));
    assert!(!open(rig), "Cancel closes it");
    seen(rig, "Cancel".into());
    grid(rig, seen);
    erase(rig, 3);
    rig.type_text("24");
    rig.click_control(ctl::OK, (0.97, 0.94));
    assert!(!open(rig), "Ok closes it");
    assert_eq!(rig.app.tools.grid_size.x, 24);
    seen(rig, "24 and Ok".into());
    grid(rig, seen);
    rig.tap(egui::Key::ArrowUp);
    rig.tap(egui::Key::ArrowUp);
    rig.tap(egui::Key::Enter);
    assert_eq!(rig.app.tools.grid_size.x, 26);
    seen(rig, "two steps up and Return".into());
    // Symmetry: two radio buttons side by side and a field.
    rig.click(Place::Tool(Tool::Symmetry, false), Secondary);
    opened(rig, seen, "Symmetry");
    rig.click_control(ctl::RADIO + 1, (0.03, 0.1));
    seen(rig, "Mirror by its first pixel".into());
    rig.click_control(ctl::RADIO, (0.97, 0.9));
    seen(rig, "Cyclic by its last pixel".into());
    rig.press_and_leave(ctl::RADIO + 1);
    seen(rig, "Mirror pressed and left".into());
    erase(rig, 2);
    rig.type_text("41");
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "41 points are too many");
    seen(rig, "an order that is refused".into());
    erase(rig, 2);
    rig.type_text("8");
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(!open(rig));
    let s = rig.app.tools.symmetry_settings;
    assert_eq!((s.mirror, s.order), (true, 8));
    seen(rig, "Mirror, 8 points, Ok".into());
    // Spacing: every radio button activates its field.
    rig.click(Place::Tool(Tool::Line, false), Secondary);
    opened(rig, seen, "Spacing");
    for radio in 1..4 {
        rig.click_control(ctl::RADIO + radio, (0.5, 0.5));
        seen(rig, format!("radio button {radio}"));
        rig.type_text("3");
        seen(rig, "a digit in its field".into());
    }
    rig.click_control(ctl::RADIO, (0.5, 0.5));
    seen(rig, "Continuous".into());
    rig.click_control(ctl::FIELD + 3, (0.5, 0.5));
    erase(rig, 2);
    rig.type_text("25");
    seen(rig, "the airbrush size".into());
    rig.tap(egui::Key::Tab);
    seen(rig, "Tab to the first field".into());
    rig.click_control(ctl::RADIO + 2, (0.5, 0.5));
    erase(rig, 3);
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "the chosen mode has no number");
    seen(rig, "Return without a number".into());
    rig.type_text("12");
    rig.tap(egui::Key::Enter);
    assert!(!open(rig));
    let s = rig.app.tools.spacing;
    assert_eq!(
        (s.mode, s.n, s.airbrush_radius),
        (SpacingMode::EveryN, 12, 25)
    );
    seen(rig, "every 12th dot, Return".into());
    // Fill Type: the gradient directions and Dither.
    rig.click(Place::Tool(Tool::Fill, false), Secondary);
    opened(rig, seen, "Fill Type");
    for radio in [1, 2, 3, 4, 0, 3] {
        rig.click_control(ctl::RADIO + radio, (0.5, 0.5));
        seen(rig, format!("radio button {radio}"));
    }
    for corner in [(0.5, 0.5), (0.03, 0.1), (0.97, 0.9)] {
        rig.click_control(ctl::CHECK, corner);
        seen(rig, format!("Dither at {corner:?}"));
    }
    rig.type_text("x5");
    rig.tap(egui::Key::Tab);
    seen(rig, "keys that mean nothing here".into());
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(!open(rig));
    let fill = rig.app.tools.fill;
    assert_eq!(
        (fill.gradient, fill.dither),
        (Some(dpx_core::GradientDirection::VerticalLine), true)
    );
    seen(rig, "Vertical Line, Dither, Ok".into());
    // Translucency: one field.
    rig.pick(Action::Command("translucency.settings"));
    opened(rig, seen, "Translucency");
    erase(rig, 3);
    rig.type_text("101");
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "101 % is refused");
    seen(rig, "101 %".into());
    rig.tap(egui::Key::Backspace);
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(!open(rig));
    assert_eq!(rig.app.menu_data.effects.percent, 10);
    seen(rig, "10 % and Ok".into());
}

#[test]
fn the_form_requesters_do_the_same_in_both_skins() {
    both_at_every_scale(form_script);
}

/// The Anim requesters (slice 4b): Set Frame Count, Add Frames, Go to Frame, Set Rate, Set
/// Range, Copy Frames and Delete Frames, with a refused number, a backwards range, every
/// radio button, Cancel, Ok, Return and Esc.
fn anim_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::form_ctl as ctl;
    let open = |rig: &mut Rig, seen: Seen, command: &'static str| {
        rig.pick(Action::Command(command));
        let control = command == "anim.set_range";
        assert_eq!(rig.app.ui.anim_requester.is_some(), !control, "{command}");
        assert_eq!(rig.app.ui.anim_control.is_some(), control, "{command}");
        if rig.app.is_modern() {
            let card = if control {
                Card::AnimControl
            } else {
                Card::Anim
            };
            assert_eq!(rig.card(), Some(card), "{command}");
        }
        seen(rig, format!("{command}.."));
    };
    let is_open =
        |rig: &Rig| rig.app.ui.anim_requester.is_some() || rig.app.ui.anim_control.is_some();
    let erase = |rig: &mut Rig, n: usize| {
        for _ in 0..n {
            rig.tap(egui::Key::Backspace);
        }
    };
    // Whatever the command answers with (a note) is closed in both skins alike.
    let settle = |rig: &mut Rig, seen: Seen, step: &str| {
        seen(rig, step.to_string());
        for _ in 0..2 {
            if rig.app.modal_open() {
                rig.tap(egui::Key::Enter);
                seen(rig, format!("Return after {step}"));
            }
        }
        assert!(!rig.app.modal_open(), "{step}: closed");
    };
    let frames = |rig: &Rig| rig.app.engine.document().frame_count();
    // Set Frame Count: a refused number, then four frames.
    open(rig, seen, "anim.set_frame_count");
    erase(rig, 1);
    rig.type_text("0");
    rig.tap(egui::Key::Enter);
    assert!(is_open(rig), "no frames is refused");
    seen(rig, "a count of 0".into());
    rig.click_control(ctl::FIELD, (0.5, 0.5));
    for key in [
        egui::Key::Home,
        egui::Key::Delete,
        egui::Key::End,
        egui::Key::ArrowLeft,
        egui::Key::ArrowRight,
    ] {
        rig.tap(key);
        seen(rig, format!("{key:?}"));
    }
    rig.type_text("x4");
    rig.press_and_leave(ctl::OK);
    rig.press_and_leave(ctl::CANCEL);
    rig.click_beside();
    assert!(is_open(rig));
    seen(rig, "4, buttons pressed and left".into());
    rig.click_control(ctl::OK, (0.97, 0.94));
    settle(rig, seen, "four frames");
    assert_eq!(frames(rig), 4);
    // Add Frames: Cancel, Esc, then two more by the arrow key and Return.
    open(rig, seen, "anim.add_frames");
    rig.click_control(ctl::CANCEL, (0.03, 0.06));
    assert!(!is_open(rig), "Cancel");
    seen(rig, "Cancel".into());
    open(rig, seen, "anim.add_frames");
    rig.tap(egui::Key::Escape);
    assert!(!is_open(rig), "Esc");
    seen(rig, "Esc".into());
    open(rig, seen, "anim.add_frames");
    rig.tap(egui::Key::ArrowUp);
    rig.tap(egui::Key::Tab);
    seen(rig, "one step up, Tab".into());
    rig.tap(egui::Key::Enter);
    settle(rig, seen, "two frames added");
    assert_eq!(frames(rig), 6);
    // Go to Frame and Set Rate.
    open(rig, seen, "anim.goto");
    erase(rig, 1);
    rig.type_text("3");
    rig.tap(egui::Key::Enter);
    settle(rig, seen, "frame 3");
    assert_eq!(rig.app.engine.document().current_frame(), 2);
    open(rig, seen, "anim.set_rate");
    erase(rig, 2);
    rig.type_text("61");
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(is_open(rig), "61 frames per second are refused");
    seen(rig, "a rate of 61".into());
    erase(rig, 2);
    rig.type_text("12");
    rig.click_control(ctl::OK, (0.5, 0.5));
    settle(rig, seen, "12 frames per second");
    assert_eq!(rig.app.engine.document().fps(), 12);
    // Set Range: a backwards range is refused, then frames 2 to 5, then all frames.
    open(rig, seen, "anim.set_range");
    rig.click_control(ctl::RADIO + 1, (0.5, 0.5));
    seen(rig, "Frames".into());
    erase(rig, 1);
    rig.type_text("5");
    rig.tap(egui::Key::Tab);
    erase(rig, 1);
    rig.type_text("2");
    rig.tap(egui::Key::Enter);
    assert!(is_open(rig), "5 to 2 is refused");
    seen(rig, "from 5 to 2".into());
    rig.click_control(ctl::FIELD, (0.5, 0.5));
    seen(rig, "the From field".into());
    erase(rig, 1);
    rig.type_text("2");
    rig.click_control(ctl::FIELD + 1, (0.5, 0.5));
    erase(rig, 1);
    rig.type_text("5");
    rig.press_and_leave(ctl::OK);
    seen(rig, "2 to 5, Ok pressed and left".into());
    rig.click_control(ctl::OK, (0.5, 0.5));
    settle(rig, seen, "the range 2 to 5");
    assert_eq!(rig.app.anim_control.range, Some((1, 4)));
    open(rig, seen, "anim.set_range");
    rig.click_control(ctl::CANCEL, (0.5, 0.5));
    assert!(!is_open(rig));
    seen(rig, "Cancel".into());
    open(rig, seen, "anim.set_range");
    rig.click_control(ctl::RADIO, (0.03, 0.1));
    rig.tap(egui::Key::Enter);
    settle(rig, seen, "all frames");
    assert_eq!(rig.app.anim_control.range, None);
    // Copy Frames: every choice; a backwards range; copies inserted before frame 1.
    open(rig, seen, "anim.copy_frames");
    for radio in [1, 2, 0, 1] {
        rig.click_control(ctl::RADIO + radio, (0.5, 0.5));
        seen(rig, format!("radio button {radio}"));
    }
    erase(rig, 1);
    rig.type_text("6");
    rig.tap(egui::Key::Tab);
    erase(rig, 1);
    rig.type_text("2");
    rig.tap(egui::Key::Enter);
    assert!(is_open(rig), "6 to 2 is refused");
    seen(rig, "from 6 to 2".into());
    rig.click_control(ctl::RADIO + 2, (0.97, 0.9));
    seen(rig, "Copy frames".into());
    for field in [4, 3, 2] {
        rig.click_control(ctl::FIELD + field, (0.5, 0.5));
        erase(rig, 1);
        rig.type_text("1");
        seen(rig, format!("1 in field {field}"));
    }
    rig.click_control(ctl::OK, (0.5, 0.5));
    settle(rig, seen, "a copy of frame 1 before it");
    assert_eq!(frames(rig), 7);
    open(rig, seen, "anim.copy_frames");
    rig.tap(egui::Key::Enter);
    settle(rig, seen, "the current frame to all frames");
    // Delete Frames: a range, the current frame, Esc, all frames.
    open(rig, seen, "anim.delete_frames");
    rig.click_control(ctl::RADIO + 1, (0.5, 0.5));
    erase(rig, 1);
    rig.type_text("6");
    seen(rig, "frames 6 to the last".into());
    rig.tap(egui::Key::Enter);
    settle(rig, seen, "frames 6 and 7 deleted");
    assert_eq!(frames(rig), 5);
    open(rig, seen, "anim.delete_frames");
    rig.click_control(ctl::RADIO + 2, (0.5, 0.5));
    rig.click_control(ctl::RADIO, (0.5, 0.5));
    seen(rig, "All frames, then Current frame".into());
    rig.click_control(ctl::OK, (0.5, 0.5));
    settle(rig, seen, "the current frame deleted");
    assert_eq!(frames(rig), 4);
    open(rig, seen, "anim.delete_frames");
    rig.click_control(ctl::RADIO + 2, (0.5, 0.5));
    rig.tap(egui::Key::Escape);
    assert!(!is_open(rig));
    seen(rig, "All frames and Esc".into());
    assert_eq!(frames(rig), 4);
    open(rig, seen, "anim.delete_frames");
    rig.click_control(ctl::RADIO + 2, (0.5, 0.5));
    rig.click_control(ctl::OK, (0.5, 0.5));
    settle(rig, seen, "all frames deleted");
    assert_eq!(frames(rig), 1);
    rig.key('u');
    seen(rig, "undo".into());
}

#[test]
fn the_anim_requesters_do_the_same_in_both_skins() {
    both_at_every_scale(anim_script);
}

/// Several requesters open at once: the pointer and the keys belong to the one `pass()`
/// asks first, and that is the one the Modern skin shows as its card.
fn stack_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::choice_requester::BUTTONS;
    rig.pick(Action::Command("picture.page_size"));
    seen(rig, "Page Size..".into());
    rig.show_note();
    seen(rig, "a note over it".into());
    rig.app.ui.choice_requester = Some(crate::autosave::quit_requester());
    rig.app.dirty = true;
    rig.pass(vec![]);
    seen(rig, "a question over both".into());
    let open = |rig: &Rig| {
        let ui = &rig.app.ui;
        (
            ui.choice_requester.is_some(),
            ui.note_requester.is_some(),
            ui.page_size_requester.is_some(),
        )
    };
    if rig.app.is_modern() {
        assert_eq!(rig.card(), Some(Card::Choice));
    }
    rig.type_text("7");
    rig.click_control(BUTTONS[0], (0.5, 0.5));
    assert_eq!(
        open(rig),
        (false, true, true),
        "the question took the click"
    );
    seen(rig, "the question answered".into());
    if rig.app.is_modern() {
        assert_eq!(rig.card(), Some(Card::Note));
    }
    rig.type_text("7");
    rig.click_control(dpx_ui::note_requester::OK, (0.5, 0.5));
    assert_eq!(open(rig), (false, false, true), "the note took the click");
    seen(rig, "the note closed".into());
    if rig.app.is_modern() {
        assert_eq!(rig.card(), Some(Card::PageSize));
    }
    rig.type_text("7");
    seen(rig, "a digit, now for the field".into());
    rig.click_control(dpx_ui::page_size_requester::ctl::CANCEL, (0.5, 0.5));
    assert_eq!(open(rig), (false, false, false));
    seen(rig, "Cancel".into());
}

#[test]
fn stacked_requesters_answer_in_the_same_order_in_both_skins() {
    both_at_every_scale(stack_script);
}

/// The card is drawn over the veiled window, picture included; under it the chrome takes no
/// input and the skin does not switch; the control under the pointer is raised, a held
/// button shows it; without a card nothing of the pointer is kept.
#[test]
fn the_card_veils_the_window_and_follows_the_pointer() {
    use dpx_ui::page_size_requester::ctl;
    for (kind, ppp) in [(ThemeKind::Dark, 1.0), (ThemeKind::Light, 2.0)] {
        let theme = Theme::of(kind);
        let mut rig = Rig::new(UiStyle::Modern, (1280.0, 800.0), ppp);
        rig.app.ui.theme = match kind {
            ThemeKind::Dark => ThemeChoice::Dark,
            ThemeKind::Light => ThemeChoice::Light,
        };
        rig.click(Place::Cell(5), Primary);
        rig.key('R');
        rig.drag(&[Place::Page(2, 2), Place::Page(60, 40)], Primary);
        rig.pass(vec![]);
        let main = rig.app.modern_layout().expect("layout");
        let lit = shot(&mut rig, kind, "live-dialog-lit");
        let (bar, dock) = (
            main.bar.expect("bar"),
            main.dock.as_ref().expect("dock").rect,
        );
        let chrome = [
            (bar.right() - 3.0, bar.y + 3.0),
            (dock.x + 3.0, dock.bottom() - 3.0),
        ];
        let page = modern_point(&rig.app, Place::Page(4, 4));
        let before = rig.app.modern.overlay();
        rig.pick(Action::Command("picture.page_size"));
        rig.pass(vec![]);
        assert_eq!(rig.card(), Some(Card::PageSize));
        let l = rig.app.modern_layout().expect("layout");
        assert_eq!(l, main, "the card moves nothing");
        assert!(!l.over_chrome);
        let after = rig.app.modern.overlay();
        assert!(after.0 > before.0 && after.1, "the card is the overlay");
        let dialog = rig.app.modern_dialog().expect("the card");
        let built =
            dpx_ui_modern::dialog::build(&dialog, l.window, l.scale, &theme, &Default::default());
        let name = format!("live-dialog-page-size-{kind:?}").to_lowercase();
        let image = shot(&mut rig, kind, &name);
        // The card's ground; the page under the scrim; the chrome under the scrim; no
        // requester in the Classic look inside the picture.
        assert_eq!(
            probe(&image, &l, built.card.x + 8.0, built.card.y + 30.0),
            rgb(theme.raised)
        );
        let (was, now) = (
            probe(&lit, &l, page.x, page.y),
            probe(&image, &l, page.x, page.y),
        );
        let a = u32::from(theme.scrim.a);
        let veiled = |under: [u8; 3]| {
            let over = rgb(theme.scrim);
            [0, 1, 2].map(|i| {
                ((u32::from(over[i]) * a + u32::from(under[i]) * (255 - a) + 127) / 255) as u8
            })
        };
        // (The rasteriser's blend may round a level differently.)
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 1);
        assert!(
            now != was && near(now, veiled(was)),
            "the picture is veiled"
        );
        for (x, y) in chrome {
            let c = probe(&image, &l, x, y);
            assert!(near(c, veiled(rgb(theme.panel))), "{c:?}");
        }
        let screen = rig.app.compose_for_skin();
        let mut plain = rig.app.ui.clone();
        crate::modern_dialog::without_cards(&mut plain);
        assert_eq!(
            screen,
            rig.app
                .view()
                .compose(&rig.app.page(), rig.app.scroll, rig.app.palette(), &plain),
            "the quad shows the picture alone"
        );
        // The pointer over Cancel raises it; over Ok it lightens the accent.
        let fill_at = |rig: &mut Rig, id: ControlId| {
            let r = built.spot(id).expect("a button").rect;
            let image = shot(rig, kind, "live-dialog-hover");
            probe(&image, &l, r.x + 6.0, r.y + r.h / 2.0)
        };
        use dpx_ui_modern::dialog::{button_fill, default_fill};
        assert_eq!(
            fill_at(&mut rig, ctl::CANCEL),
            rgb(button_fill(&theme, false, false))
        );
        let renders = rig.app.modern.overlay().0;
        let cancel = rig.control_point(ctl::CANCEL, (0.5, 0.5));
        rig.pass(vec![egui::Event::PointerMoved(cancel)]);
        assert!(
            rig.app.modern.overlay().0 > renders,
            "the card is drawn again"
        );
        assert_eq!(
            fill_at(&mut rig, ctl::CANCEL),
            rgb(button_fill(&theme, true, false))
        );
        assert_eq!(fill_at(&mut rig, ctl::OK), rgb(theme.accent));
        let renders = rig.app.modern.overlay().0;
        rig.pass(vec![egui::Event::PointerMoved(cancel)]);
        assert_eq!(
            rig.app.modern.overlay().0,
            renders,
            "a resting pointer costs nothing"
        );
        let ok = rig.control_point(ctl::OK, (0.5, 0.5));
        rig.pass(vec![egui::Event::PointerMoved(ok)]);
        assert_eq!(
            fill_at(&mut rig, ctl::OK),
            rgb(default_fill(&theme, true, false))
        );
        assert_eq!(
            fill_at(&mut rig, ctl::CANCEL),
            rgb(button_fill(&theme, false, false))
        );
        // Held on Cancel: the requester shows it held; dragged off: not; back on: held.
        rig.pass(vec![
            egui::Event::PointerMoved(cancel),
            pointer_button(cancel, Primary, true),
        ]);
        assert_eq!(rig.app.modern.pointer.captured, Some(ctl::CANCEL));
        assert_eq!(
            fill_at(&mut rig, ctl::CANCEL),
            rgb(button_fill(&theme, true, true))
        );
        rig.pass(vec![egui::Event::PointerMoved(ok)]);
        assert_eq!(
            fill_at(&mut rig, ctl::CANCEL),
            rgb(button_fill(&theme, false, false))
        );
        assert_eq!(
            fill_at(&mut rig, ctl::OK),
            rgb(theme.accent),
            "Ok is not the held one"
        );
        rig.pass(vec![egui::Event::PointerMoved(cancel)]);
        assert_eq!(
            fill_at(&mut rig, ctl::CANCEL),
            rgb(button_fill(&theme, true, true))
        );
        rig.pass(vec![egui::Event::PointerMoved(ok)]);
        rig.pass(vec![pointer_button(ok, Primary, false)]);
        assert!(
            rig.app.ui.page_size_requester.is_some(),
            "let go on another button"
        );
        assert_eq!(rig.app.modern.pointer.captured, None);
        // The chrome takes nothing, the keys of the main window do nothing.
        let state = (
            rig.app.ui.tool,
            rig.app.ui.fg,
            rig.app.ui.brush,
            rig.app.engine.document().revision(),
            rig.app.ui.title_visible,
            rig.app.ui.toolbox_visible,
        );
        let title = l.menu_titles[1];
        for pos in [
            modern_point(&rig.app, Place::Tool(Tool::Line, false)),
            modern_point(&rig.app, Place::Cell(12)),
            modern_point(&rig.app, Place::Brush(4)),
            modern_point(&rig.app, Place::Page(100, 200)),
            egui::pos2(title.x + title.w / 2.0, title.y + title.h / 2.0),
        ] {
            for button in [Primary, Secondary] {
                rig.pass(vec![
                    egui::Event::PointerMoved(pos),
                    pointer_button(pos, button, true),
                ]);
                assert!(!rig.app.ui.menu.active, "no menu under the card");
                assert_eq!(
                    rig.app.modern.hover, None,
                    "nothing of the chrome is raised"
                );
                rig.pass(vec![pointer_button(pos, button, false)]);
            }
        }
        for key in [egui::Key::F9, egui::Key::F10, egui::Key::F12] {
            rig.tap(key);
        }
        assert!(rig.app.is_modern(), "F12 does not switch under a requester");
        assert!(rig.app.ui.page_size_requester.is_some());
        assert_eq!(
            state,
            (
                rig.app.ui.tool,
                rig.app.ui.fg,
                rig.app.ui.brush,
                rig.app.engine.document().revision(),
                rig.app.ui.title_visible,
                rig.app.ui.toolbox_visible,
            )
        );
        // Closed: the window as before, nothing of the pointer kept.
        rig.tap(egui::Key::Escape);
        rig.pass(vec![egui::Event::PointerMoved(page)]);
        assert_eq!(rig.card(), None);
        assert_eq!(rig.app.modern.pointer, Default::default());
        let again = shot(&mut rig, kind, "live-dialog-closed");
        assert_eq!(probe(&again, &l, page.x, page.y), was);
        for (x, y) in chrome {
            assert_eq!(probe(&again, &l, x, y), rgb(theme.panel));
        }
    }
}

/// Every converted requester fits the smallest window and is drawn there, in both themes
/// and at device scale 1 and 2 (the images go to `DPX_MODERN_SHOTS` for a look).
#[test]
fn every_card_fits_the_smallest_window() {
    type Open = fn(&mut Rig);
    let open: [(&str, Card, Open); 17] = [
        ("anim-count", Card::Anim, |rig| {
            rig.pick(Action::Command("anim.set_frame_count"))
        }),
        ("anim-add", Card::Anim, |rig| {
            rig.pick(Action::Command("anim.add_frames"))
        }),
        ("anim-copy", Card::Anim, |rig| {
            rig.pick(Action::Command("anim.copy_frames"))
        }),
        ("anim-delete", Card::Anim, |rig| {
            rig.pick(Action::Command("anim.delete_frames"))
        }),
        ("anim-goto", Card::Anim, |rig| {
            rig.pick(Action::Command("anim.goto"))
        }),
        ("anim-rate", Card::Anim, |rig| {
            rig.pick(Action::Command("anim.set_rate"))
        }),
        ("anim-range", Card::AnimControl, |rig| {
            rig.pick(Action::Command("anim.set_range"))
        }),
        ("grid", Card::Tool, |rig| {
            rig.click(Place::Tool(Tool::Grid, false), Secondary)
        }),
        ("symmetry", Card::Tool, |rig| {
            rig.click(Place::Tool(Tool::Symmetry, false), Secondary)
        }),
        ("spacing", Card::Tool, |rig| {
            rig.click(Place::Tool(Tool::Line, false), Secondary)
        }),
        ("fill", Card::Tool, |rig| {
            rig.click(Place::Tool(Tool::Fill, false), Secondary)
        }),
        ("translucency", Card::Tool, |rig| {
            rig.pick(Action::Command("translucency.settings"))
        }),
        ("note", Card::Note, |rig| rig.show_note()),
        ("about", Card::Note, |rig| {
            rig.pick(Action::Command("app.about"))
        }),
        ("question", Card::Choice, |rig| {
            rig.app.ui.choice_requester = Some(crate::autosave::quit_requester());
            rig.pass(vec![]);
        }),
        ("page-size", Card::PageSize, |rig| {
            rig.pick(Action::Command("picture.page_size"))
        }),
        ("screen-format", Card::ScreenFormat, |rig| {
            rig.pick(Action::Command("picture.screen_format"))
        }),
    ];
    for (kind, ppp) in [(ThemeKind::Dark, 1.0), (ThemeKind::Light, 2.0)] {
        let theme = Theme::of(kind);
        for (name, card, open) in open {
            let mut rig = Rig::new(UiStyle::Modern, (720.0, 520.0), ppp);
            rig.key('L');
            let main = rig.app.modern_layout().expect("layout");
            open(&mut rig);
            rig.pass(vec![]);
            assert_eq!(rig.card(), Some(card), "{name}");
            let l = rig.app.modern_layout().expect("layout");
            assert_eq!(l, main, "{name}: the window stays as it is");
            let dialog = rig.app.modern_dialog().expect("the card");
            let built = dpx_ui_modern::dialog::build(
                &dialog,
                l.window,
                l.scale,
                &theme,
                &Default::default(),
            );
            assert!(l.window.encloses(&built.card), "{name}: {:?}", built.card);
            let file = format!("live-dialog-compact-{name}-{kind:?}").to_lowercase();
            let image = shot(&mut rig, kind, &file);
            assert_eq!(
                probe(&image, &l, built.card.x + 8.0, built.card.y + 8.0),
                rgb(theme.raised),
                "{name}"
            );
            rig.tap(egui::Key::Escape);
            assert!(!rig.app.modal_open(), "{name}: Esc closes it");
        }
    }
}

/// A question over a requester that is still shown in the Classic look (here the file
/// requester): the quad hosts that one as before, the question is the card over it, and
/// when it is answered the file requester takes its clicks through the Classic mapping.
#[test]
fn a_card_lies_over_a_requester_in_the_classic_look() {
    use dpx_ui::choice_requester::BUTTONS;
    let mut rig = Rig::new(UiStyle::Modern, (800.0, 600.0), 1.0);
    rig.pick(Action::Command("picture.load"));
    rig.pass(vec![]);
    assert!(rig.app.ui.file_requester.is_some());
    assert_eq!(rig.card(), None, "the file requester is not converted yet");
    let hosting = rig.app.modern_layout().expect("layout");
    rig.app.ui.choice_requester = Some(dpx_ui::ChoiceRequester::new(
        "Replace file?",
        &["castle.iff exists.".to_string()],
        ["Replace", "Keep"],
        1,
        1,
    ));
    rig.app.dirty = true;
    rig.pass(vec![]);
    assert_eq!(rig.card(), Some(Card::Choice));
    assert!(!rig.app.modern_plain_picture());
    assert_eq!(rig.app.modern_layout().expect("layout"), hosting);
    // The quad still shows the file requester (and not the question in the Classic look).
    let screen = rig.app.compose_for_skin();
    let mut hosted = rig.app.ui.clone();
    hosted.choice_requester = None;
    assert_eq!(
        screen,
        rig.app
            .view()
            .compose(&rig.app.page(), rig.app.scroll, rig.app.palette(), &hosted)
    );
    assert_ne!(
        screen,
        rig.app.view().compose(
            &rig.app.page(),
            rig.app.scroll,
            rig.app.palette(),
            &UiState {
                file_requester: None,
                ..hosted.clone()
            }
        )
    );
    rig.type_text("abc");
    let file = rig.app.ui.file_requester.as_ref().map(|r| r.file());
    assert_eq!(file.as_deref(), Some(""), "the keys belong to the question");
    rig.click_control(BUTTONS[1], (0.5, 0.5));
    assert!(rig.app.ui.choice_requester.is_none(), "Keep answers");
    assert!(rig.app.ui.file_requester.is_some());
    assert_eq!(rig.card(), None);
    rig.type_text("abc");
    let file = rig.app.ui.file_requester.as_ref().map(|r| r.file());
    assert_eq!(file.as_deref(), Some("abc"));
    rig.tap(egui::Key::Escape);
    assert!(!rig.app.modal_open());
}

/// The requesters that are not converted stay in the Classic look: none of them is a card,
/// and the Classic skin never has one.
#[test]
fn only_the_converted_requesters_are_cards() {
    let mut classic = Rig::classic();
    classic.pick(Action::Command("picture.page_size"));
    assert!(classic.app.ui.page_size_requester.is_some());
    assert_eq!(classic.card(), None, "Classic draws its requesters itself");
    assert!(
        classic
            .app
            .modern_dialog_pos(&classic.ctx.clone())
            .is_none()
    );
    type Open = fn(&mut Rig);
    let others: [(&str, Open); 5] = [
        ("Stencil", |rig| rig.pick(Action::Command("stencil.make"))),
        ("Color Mixer", |rig| rig.key('p')),
        ("file", |rig| rig.pick(Action::Command("picture.load"))),
        ("Generate", |rig| {
            rig.pick(Action::Command("picture.generate"))
        }),
        ("AI & MCP", |rig| rig.pick(Action::Command("app.ai_mcp"))),
    ];
    for (name, open) in others {
        let mut rig = Rig::modern();
        open(&mut rig);
        rig.pass(vec![]);
        assert!(rig.app.modal_open(), "{name} is open");
        assert_eq!(rig.card(), None, "{name} keeps the Classic look");
        assert!(rig.app.modern_dialog().is_none());
        // A converted requester under it does not become a card either.
        rig.show_note();
        let top = rig.card();
        let note_first = matches!(name, "file");
        assert_eq!(top.is_some(), note_first, "{name}: {top:?}");
    }
}
