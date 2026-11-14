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
    let tool = app.ui.tool_requester.take();
    let range = app.ui.range_requester.take();
    let anim = app.ui.anim_requester.take();
    let control = app.ui.anim_control.take();
    let file = app.ui.file_requester.take();
    let percent = (
        app.menu_data.effects.percent,
        app.engine.document().fps(),
        app.anim_control.range,
        app.animbrush
            .brush
            .as_ref()
            .map(|b| (b.len(), b.duration, b.current(), b.direction)),
        app.perspective.settings,
        app.stencil.clone(),
        app.extras.ranges.clone(),
        (app.anim_control.settings, app.anim_control.moved_to),
    );
    let shown = format!(
        "{:?}",
        (
            note.as_ref().map(|r| r.view()),
            choice.as_ref().map(|r| r.view()),
            page.as_ref().map(|r| r.view()),
            format.as_ref().map(|r| (r.view(), r.command())),
            tool.as_ref().map(|r| r.view(&palette)),
            range.as_ref().map(|r| (r.view(&palette), r.ranges())),
            anim.as_ref().map(|r| r.view()),
            control.as_ref().map(|r| r.view()),
            // Slice 4c (a tuple holds at most twelve values).
            (file.as_ref().map(|r| (r.view(), r.drawer().to_string())),),
            percent,
            (open, pending, confirmed),
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
    app.ui.tool_requester = tool;
    app.ui.range_requester = range;
    app.ui.anim_requester = anim;
    app.ui.anim_control = control;
    app.ui.file_requester = file;
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

/// The AnimBrush requesters and Perspective (slice 4b): forms with buttons of their own.
/// Pick up AnimBrush through the real flow (four frames, Pick up.., a frame with the Brush
/// Selector), the Settings with their three Direction buttons, Make AnimBrush with a
/// refused number, and the Perspective settings with From Brush.
fn animbrush_perspective_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::animbrush_requester::DIRECTION;
    use dpx_ui::form_ctl as ctl;
    use dpx_ui::perspective_requester::FROM_BRUSH_ID;
    use dpx_ui::{AnimBrushRequester, ToolRequester};
    let erase = |rig: &mut Rig, n: usize| {
        for _ in 0..n {
            rig.tap(egui::Key::Backspace);
        }
    };
    let open = |rig: &Rig| rig.app.ui.tool_requester.is_some();
    let card = |rig: &mut Rig, seen: Seen, name: &str| {
        assert!(rig.app.ui.tool_requester.is_some(), "{name} is open");
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::Tool), "{name}");
        }
        seen(rig, format!("{name}.."));
    };
    // Four frames with something on them.
    rig.click(Place::Page(20, 20), Primary);
    rig.pick(Action::Command("anim.set_frame_count"));
    erase(rig, 1);
    rig.type_text("4");
    rig.tap(egui::Key::Enter);
    seen(rig, "four frames".into());
    // Perspective without a brush: From Brush is ghosted and does nothing.
    rig.app.open_perspective_requester();
    rig.pass(vec![]);
    card(rig, seen, "Perspective");
    let without = rig.dialog();
    assert!(without.control(FROM_BRUSH_ID).is_some());
    rig.tap(egui::Key::Escape);
    assert!(!open(rig));
    seen(rig, "Esc".into());
    // Pick up AnimBrush: three cels.
    rig.pick(Action::Command("animbrush.pick_up"));
    seen(rig, "Pick up.. is armed".into());
    rig.drag(
        &[
            Place::Page(60, 60),
            Place::Page(75, 70),
            Place::Page(90, 80),
        ],
        Primary,
    );
    assert!(matches!(
        rig.app.ui.tool_requester,
        Some(ToolRequester::AnimBrush(_))
    ));
    card(rig, seen, "Pick up AnimBrush");
    erase(rig, 1);
    rig.type_text("9");
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(open(rig), "nine cels of four frames are refused");
    seen(rig, "nine cels".into());
    rig.press_and_leave(ctl::OK);
    rig.press_and_leave(ctl::CANCEL);
    rig.click_beside();
    rig.click_control(ctl::FIELD, (0.5, 0.5));
    erase(rig, 1);
    rig.type_text("3");
    seen(rig, "three cels".into());
    rig.tap(egui::Key::Enter);
    assert!(!open(rig), "Return picks it up");
    assert!(rig.app.animbrush.brush.is_some(), "an AnimBrush");
    seen(rig, "the AnimBrush is picked up".into());
    // AnimBrush Settings: the fields and the three Direction buttons.
    let settings = |rig: &mut Rig, seen: Seen| {
        rig.pick(Action::Command("animbrush.settings"));
        card(rig, seen, "AnimBrush Settings");
    };
    let direction = |rig: &Rig| rig.app.animbrush.brush.as_ref().expect("brush").direction;
    settings(rig, seen);
    for button in [1, 2, 0, 2] {
        rig.click_control(DIRECTION + button, (0.5, 0.5));
        seen(rig, format!("direction button {button}"));
    }
    rig.click_control(DIRECTION, (0.04, 0.1));
    seen(rig, "forward by its first pixel".into());
    rig.click_control(DIRECTION + 1, (0.96, 0.9));
    seen(rig, "backward by its last pixel".into());
    rig.press_and_leave(DIRECTION + 2);
    seen(rig, "ping-pong pressed and left".into());
    let held = rig.control_point(DIRECTION + 2, (0.5, 0.5));
    rig.pass(vec![
        egui::Event::PointerMoved(held),
        pointer_button(held, Primary, true),
    ]);
    seen(rig, "ping-pong held".into());
    rig.pass(vec![pointer_button(held, Primary, false)]);
    seen(rig, "ping-pong let go".into());
    erase(rig, 2);
    rig.type_text("7");
    rig.tap(egui::Key::Tab);
    erase(rig, 1);
    rig.type_text("5");
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "cel 5 of 3 is refused");
    seen(rig, "duration 7, cel 5".into());
    erase(rig, 1);
    rig.type_text("2");
    rig.click_control(ctl::OK, (0.5, 0.5));
    assert!(!open(rig));
    let brush = rig.app.animbrush.brush.as_ref().expect("brush");
    assert_eq!((brush.duration, brush.current() + 1), (7, 2));
    assert_eq!(direction(rig), dpx_core::CelDirection::PingPong);
    seen(rig, "the settings are kept".into());
    settings(rig, seen);
    rig.click_control(DIRECTION, (0.5, 0.5));
    rig.click_control(ctl::CANCEL, (0.5, 0.5));
    assert!(!open(rig));
    assert_eq!(direction(rig), dpx_core::CelDirection::PingPong, "Cancel");
    seen(rig, "Cancel keeps the direction".into());
    // Make AnimBrush (set as Brush ▸ Spare ▸ Metamorph.. sets it): a refused number, Esc.
    rig.app.ui.tool_requester = Some(ToolRequester::AnimBrush(AnimBrushRequester::make(8, 20)));
    rig.app.dirty = true;
    rig.pass(vec![]);
    card(rig, seen, "Make AnimBrush");
    erase(rig, 1);
    rig.type_text("21");
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "21 cels are refused");
    seen(rig, "21 cels".into());
    rig.tap(egui::Key::ArrowDown);
    seen(rig, "one step down".into());
    rig.tap(egui::Key::Escape);
    assert!(!open(rig));
    seen(rig, "Esc".into());
    // Perspective with the brush: every field, both choices, From Brush.
    rig.app.open_perspective_requester();
    rig.pass(vec![]);
    card(rig, seen, "Perspective");
    for field in 0..4 {
        rig.click_control(ctl::FIELD + field, (0.5, 0.5));
        erase(rig, 4);
        rig.type_text(["99", "88", "77", "45"][usize::from(field)]);
        seen(rig, format!("field {field}"));
    }
    for radio in [1, 3, 0, 2, 1, 3] {
        rig.click_control(ctl::RADIO + radio, (0.5, 0.5));
        seen(rig, format!("radio button {radio}"));
    }
    rig.press_and_leave(FROM_BRUSH_ID);
    seen(rig, "From Brush pressed and left".into());
    rig.click_control(FROM_BRUSH_ID, (0.04, 0.1));
    seen(rig, "From Brush by its first pixel".into());
    rig.click_control(ctl::FIELD + 2, (0.5, 0.5));
    erase(rig, 4);
    rig.tap(egui::Key::Enter);
    assert!(open(rig), "Grid Z is missing");
    seen(rig, "Return without Grid Z".into());
    rig.type_text("12");
    rig.click_control(ctl::OK, (0.97, 0.94));
    assert!(!open(rig));
    let s = rig.app.perspective.settings;
    assert_eq!((s.grid[2], s.angle_step), (12, 45));
    assert!(s.brush_axes && s.show_position);
    assert_ne!((s.grid[0], s.grid[1]), (99, 88), "From Brush set X and Y");
    seen(rig, "the settings are kept".into());
}

#[test]
fn the_animbrush_and_perspective_requesters_do_the_same_in_both_skins() {
    both_at_every_scale(animbrush_perspective_script);
}

/// Where swatch `cell` of a grid of 16 columns and `rows` rows lies in the grid's control,
/// as a fraction (`at` inside the cell).
fn swatch(cell: u16, rows: u16, at: (f32, f32)) -> (f32, f32) {
    (
        (f32::from(cell % 16) + at.0) / 16.0,
        (f32::from(cell / 16) + at.1) / f32::from(rows),
    )
}

impl Rig {
    /// Press on the first point of control `id` (fractions of it, also beyond its edges),
    /// move over the others, let go on the last; `seen` after every move.
    fn drag_in_control(&mut self, id: ControlId, path: &[(f32, f32)], seen: Seen, what: &str) {
        let first = self.control_point(id, path[0]);
        self.pass(vec![
            egui::Event::PointerMoved(first),
            pointer_button(first, Primary, true),
        ]);
        seen(self, format!("{what}: pressed"));
        let mut last = first;
        for (i, frac) in path.iter().enumerate().skip(1) {
            // (The control's place is asked while the button is held: it does not move.)
            last = self.control_point(id, *frac);
            self.pass(vec![egui::Event::PointerMoved(last)]);
            seen(self, format!("{what}: moved to point {i}"));
        }
        self.pass(vec![pointer_button(last, Primary, false)]);
        seen(self, format!("{what}: let go"));
    }
}

/// Effect ▸ Stencil ▸ Make.. (slice 4b): swatches toggled by a click and painted by a drag
/// over the grid and beyond its edge, Clear, Invert, Make, Cancel, Return and Esc.
fn stencil_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::ToolRequester;
    use dpx_ui::stencil_requester::ctl;
    const MID: (f32, f32) = (0.5, 0.5);
    let open = |rig: &mut Rig, seen: Seen| {
        rig.pick(Action::Command("stencil.make"));
        assert!(matches!(
            rig.app.ui.tool_requester,
            Some(ToolRequester::Stencil(_))
        ));
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::Tool));
        }
        seen(rig, "Make Stencil..".into());
    };
    let protected = |rig: &Rig| -> Vec<usize> {
        match &rig.app.ui.tool_requester {
            Some(ToolRequester::Stencil(r)) => (0..32).filter(|c| r.is_protected(*c)).collect(),
            _ => panic!("the stencil requester"),
        }
    };
    open(rig, seen);
    for cell in [5, 5, 3, 20, 31, 0] {
        rig.click_control(ctl::GRID, swatch(cell, 2, MID));
        seen(rig, format!("swatch {cell}"));
    }
    assert_eq!(protected(rig), [0, 3, 20, 31]);
    // The corners of a swatch are that swatch.
    rig.click_control(ctl::GRID, swatch(17, 2, (0.06, 0.06)));
    rig.click_control(ctl::GRID, swatch(18, 2, (0.8, 0.8)));
    assert_eq!(protected(rig), [0, 3, 17, 18, 20, 31]);
    seen(rig, "two swatches by their corners".into());
    // A drag paints the state of the first swatch over the others.
    let path: Vec<(f32, f32)> = [8, 9, 10, 11].map(|c| swatch(c, 2, MID)).to_vec();
    rig.drag_in_control(ctl::GRID, &path, seen, "a drag over four swatches");
    assert_eq!(protected(rig), [0, 3, 8, 9, 10, 11, 17, 18, 20, 31]);
    // From a protected swatch on: the drag clears; it leaves the grid at its right edge.
    let path = [swatch(17, 2, MID), swatch(19, 2, MID), (1.2, 0.75)];
    rig.drag_in_control(ctl::GRID, &path, seen, "a drag out of the grid");
    assert_eq!(protected(rig)[..6], [0, 3, 8, 9, 10, 11]);
    assert!(protected(rig).iter().all(|c| *c < 17), "the row is cleared");
    rig.click_control(ctl::INVERT, MID);
    assert_eq!(protected(rig).len(), 26);
    seen(rig, "Invert".into());
    rig.press_and_leave(ctl::CLEAR);
    rig.press_and_leave(ctl::MAKE);
    rig.click_beside();
    rig.type_text("x");
    rig.tap(egui::Key::Tab);
    assert_eq!(protected(rig).len(), 26);
    seen(
        rig,
        "Clear and Make pressed and left, keys that mean nothing".into(),
    );
    rig.click_control(ctl::CLEAR, (0.97, 0.9));
    assert!(protected(rig).is_empty());
    seen(rig, "Clear".into());
    rig.click_control(ctl::GRID, swatch(6, 2, MID));
    rig.click_control(ctl::CANCEL, MID);
    assert!(rig.app.ui.tool_requester.is_none() && rig.app.stencil.is_none());
    seen(rig, "Cancel".into());
    open(rig, seen);
    rig.click_control(ctl::GRID, swatch(6, 2, MID));
    rig.tap(egui::Key::Escape);
    assert!(rig.app.ui.tool_requester.is_none() && rig.app.stencil.is_none());
    seen(rig, "Esc".into());
    open(rig, seen);
    rig.click_control(ctl::GRID, swatch(6, 2, MID));
    rig.click_control(ctl::MAKE, (0.04, 0.1));
    assert!(rig.app.ui.tool_requester.is_none());
    assert!(rig.app.stencil.as_ref().is_some_and(|s| s[6] && !s[7]));
    seen(rig, "Make".into());
    // It opens on the stencil there is; Return makes it.
    open(rig, seen);
    assert_eq!(protected(rig), [6]);
    rig.click_control(ctl::GRID, swatch(7, 2, MID));
    rig.tap(egui::Key::Enter);
    assert!(rig.app.stencil.as_ref().is_some_and(|s| s[6] && s[7]));
    seen(rig, "Return".into());
}

#[test]
fn the_stencil_requester_does_the_same_in_both_skins() {
    both_at_every_scale(stencil_script);
}

/// Color ▸ Palette ▸ Arrange (slice 4b): a swatch dragged to another place moves there or,
/// with Ex, changes places; Undo, Revert, Cancel, Ok, Return and Esc.
fn arrange_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::ToolRequester;
    use dpx_ui::arrange_requester::ctl;
    const MID: (f32, f32) = (0.5, 0.5);
    let open = |rig: &mut Rig, seen: Seen| {
        rig.pick(Action::Command("palette.arrange"));
        assert!(matches!(
            rig.app.ui.tool_requester,
            Some(ToolRequester::Arrange(_))
        ));
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::Tool));
        }
        seen(rig, "Arrange..".into());
    };
    let order = |rig: &Rig| -> Vec<u8> {
        match &rig.app.ui.tool_requester {
            Some(ToolRequester::Arrange(r)) => r.order().to_vec(),
            _ => panic!("the arrange requester"),
        }
    };
    // Something painted, so that the renumbered picture can be compared.
    rig.click(Place::Cell(9), Primary);
    rig.click(Place::Page(40, 40), Primary);
    seen(rig, "a dot in colour 9".into());
    let colours = rig.app.engine.document().palette().clone();
    open(rig, seen);
    let cell = |c: u16| swatch(c, 2, MID);
    // Move: 9 goes to place 2, what lies between shifts by one.
    rig.drag_in_control(ctl::GRID, &[cell(9), cell(5), cell(2)], seen, "9 to 2");
    assert_eq!(order(rig)[..4], [0, 1, 9, 2]);
    // A click on a swatch and a drag that ends beside the grid move nothing.
    rig.click_control(ctl::GRID, swatch(20, 2, (0.1, 0.9)));
    rig.drag_in_control(ctl::GRID, &[cell(4), cell(6), (0.4, -0.9)], seen, "out");
    assert_eq!(order(rig)[..4], [0, 1, 9, 2]);
    // Ex: the two change places.
    rig.click_control(ctl::EXCHANGE, MID);
    seen(rig, "Ex".into());
    rig.drag_in_control(ctl::GRID, &[cell(0), cell(17), cell(31)], seen, "0 with 31");
    assert_eq!((order(rig)[0], order(rig)[31]), (31, 0));
    rig.click_control(ctl::MOVE, (0.05, 0.1));
    seen(rig, "Move".into());
    rig.press_and_leave(ctl::EXCHANGE);
    rig.press_and_leave(ctl::UNDO);
    rig.press_and_leave(ctl::REVERT);
    rig.press_and_leave(ctl::OK);
    rig.click_beside();
    rig.type_text("x");
    assert_eq!((order(rig)[0], order(rig)[31]), (31, 0));
    seen(rig, "buttons pressed and left".into());
    rig.click_control(ctl::UNDO, MID);
    assert_eq!((order(rig)[0], order(rig)[2]), (0, 9));
    seen(rig, "Undo".into());
    rig.click_control(ctl::REVERT, MID);
    assert_eq!(order(rig)[..4], [0, 1, 2, 3]);
    seen(rig, "Revert".into());
    rig.click_control(ctl::UNDO, (0.96, 0.9));
    assert_eq!(order(rig)[..4], [0, 1, 9, 2]);
    seen(rig, "Undo of Revert".into());
    rig.click_control(ctl::CANCEL, MID);
    assert!(rig.app.ui.tool_requester.is_none());
    assert_eq!(rig.app.engine.document().palette(), &colours, "Cancel");
    seen(rig, "Cancel".into());
    open(rig, seen);
    rig.drag_in_control(ctl::GRID, &[cell(9), cell(3), cell(2)], seen, "9 to 2");
    rig.tap(egui::Key::Escape);
    assert_eq!(rig.app.engine.document().palette(), &colours, "Esc");
    seen(rig, "Esc".into());
    open(rig, seen);
    rig.drag_in_control(ctl::GRID, &[cell(9), cell(3), cell(2)], seen, "9 to 2");
    rig.click_control(ctl::OK, (0.95, 0.9));
    assert!(rig.app.ui.tool_requester.is_none());
    let now = rig.app.engine.document().palette().clone();
    assert_eq!(now.get(2), colours.get(9), "Ok arranges the palette");
    seen(rig, "Ok".into());
    open(rig, seen);
    rig.click_control(ctl::EXCHANGE, MID);
    rig.drag_in_control(ctl::GRID, &[cell(1), cell(16), cell(30)], seen, "1 with 30");
    rig.tap(egui::Key::Enter);
    let then = rig.app.engine.document().palette().clone();
    assert_eq!((then.get(1), then.get(30)), (now.get(30), now.get(1)));
    seen(rig, "Return".into());
    rig.key('u');
    seen(rig, "undo".into());
}

#[test]
fn the_arrange_requester_does_the_same_in_both_skins() {
    both_at_every_scale(arrange_script);
}

/// Color ▸ Ranges.. (slice 4b): the range numbers by button and key, a span by two clicks
/// and by a drag over the strip, the rate slider, direction, Active, Clear, Cancel, Ok,
/// Return and Esc.
fn range_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::range_requester::ctl;
    const MID: (f32, f32) = (0.5, 0.5);
    let open = |rig: &mut Rig, seen: Seen| {
        rig.pick(Action::Command("color.ranges"));
        assert!(rig.app.ui.range_requester.is_some());
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::Range));
        }
        seen(rig, "Ranges..".into());
    };
    let req = |rig: &Rig| rig.app.ui.range_requester.clone().expect("open");
    let span = |rig: &Rig| {
        let r = req(rig).current();
        (r.low, r.high)
    };
    let step = |rig: &Rig| dpx_ui::range_requester::rate_to_step(req(rig).current().rate);
    let cell = |c: u16| swatch(c, 2, MID);
    let kept = rig.app.extras.ranges.clone();
    open(rig, seen);
    // The range numbers.
    for n in [4u16, 7, 0, 2] {
        rig.click_control(ctl::SELECT + n, MID);
        assert_eq!(req(rig).selected(), usize::from(n));
        seen(rig, format!("range {}", n + 1));
    }
    rig.click_control(ctl::SELECT + 5, (0.08, 0.1));
    rig.press_and_leave(ctl::SELECT + 1);
    assert_eq!(req(rig).selected(), 5);
    seen(
        rig,
        "range 6 by its first pixel, range 2 pressed and left".into(),
    );
    for key in ['1', '8', '3'] {
        rig.key(key);
        seen(rig, format!("the {key} key"));
    }
    assert_eq!(req(rig).selected(), 2);
    assert!(!req(rig).current().defined(), "range 3 is empty");
    // A span: the first colour, Esc takes it back; then first and last.
    rig.click_control(ctl::GRID, cell(3));
    assert!(req(rig).awaiting_high());
    seen(rig, "the first colour".into());
    rig.tap(egui::Key::Escape);
    assert!(rig.app.ui.range_requester.is_some() && !req(rig).awaiting_high());
    seen(rig, "Esc takes it back".into());
    rig.click_control(ctl::GRID, cell(3));
    rig.click_control(ctl::GRID, swatch(9, 2, (0.9, 0.9)));
    assert_eq!(span(rig), (3, 9));
    seen(rig, "colours 3 to 9".into());
    // A drag over the strip sets the span, backwards too, and across the rows.
    rig.drag_in_control(ctl::GRID, &[cell(12), cell(13), cell(15)], seen, "12 to 15");
    assert_eq!(span(rig), (12, 15));
    rig.drag_in_control(ctl::GRID, &[cell(22), cell(21), cell(18)], seen, "22 to 18");
    assert_eq!(span(rig), (18, 22));
    // The rate: a click beside the knob is one step; the knob dragged beyond the ends.
    assert_eq!(step(rig), 16);
    rig.click_control(ctl::RATE, (0.9, 0.5));
    assert_eq!(step(rig), 17);
    seen(rig, "one step up".into());
    rig.click_control(ctl::RATE, (0.04, 0.5));
    rig.click_control(ctl::RATE, (0.04, 0.5));
    assert_eq!(step(rig), 15);
    seen(rig, "two steps down".into());
    for (to, most) in [(1.6, 63), (-0.7, 0), (1.3, 63)] {
        let knob = |rig: &Rig| {
            let dialog = rig.dialog();
            let slider = dialog.columns[0]
                .iter()
                .find_map(|item| match item {
                    dpx_ui::dialog::Item::Line { parts, .. } => {
                        parts.iter().find_map(|p| match p {
                            dpx_ui::dialog::Part::Slider(s) => Some(s.clone()),
                            _ => None,
                        })
                    }
                    _ => None,
                })
                .expect("the slider");
            let (area, knob) = (slider.control.area, slider.knob);
            ((knob.x - area.x) as f32 + knob.w as f32 / 2.0) / area.w as f32
        };
        let from = rig.control_point(ctl::RATE, (knob(rig), 0.5));
        let before = step(rig);
        rig.pass(vec![
            egui::Event::PointerMoved(from),
            pointer_button(from, Primary, true),
        ]);
        assert_eq!(step(rig), before, "a press on the knob changes nothing");
        seen(rig, "the knob held".into());
        let end = rig.control_point(ctl::RATE, (to, 2.5));
        rig.pass(vec![egui::Event::PointerMoved(end)]);
        assert_eq!(step(rig), most, "dragged beyond the end");
        seen(rig, format!("the knob dragged to {most}"));
        rig.pass(vec![pointer_button(end, Primary, false)]);
        seen(rig, "let go".into());
    }
    // Direction, Active, Clear.
    for way in ["Reverse", "Forward"] {
        rig.click_control(ctl::DIRECTION, MID);
        seen(rig, way.to_string());
    }
    rig.key('r');
    assert!(req(rig).current().reverse());
    seen(rig, "the r key".into());
    rig.click_control(ctl::ACTIVE, MID);
    assert!(!req(rig).current().active());
    seen(rig, "Active off".into());
    rig.click_control(ctl::ACTIVE, (0.95, 0.9));
    rig.press_and_leave(ctl::CLEAR);
    assert!(req(rig).current().active() && req(rig).current().defined());
    seen(rig, "Active on, Clear pressed and left".into());
    rig.click_control(ctl::CLEAR, MID);
    assert!(!req(rig).current().defined());
    seen(rig, "Clear".into());
    rig.key('r');
    rig.type_text("x9");
    rig.tap(egui::Key::Tab);
    rig.wheel(2.0);
    seen(rig, "keys that mean nothing for an empty range".into());
    rig.press_and_leave(ctl::OK);
    rig.press_and_leave(ctl::CANCEL);
    rig.click_beside();
    assert!(rig.app.ui.range_requester.is_some());
    seen(rig, "Ok and Cancel pressed and left".into());
    rig.click_control(ctl::CANCEL, (0.04, 0.1));
    assert!(rig.app.ui.range_requester.is_none());
    assert_eq!(rig.app.extras.ranges, kept, "Cancel keeps the ranges");
    seen(rig, "Cancel".into());
    open(rig, seen);
    rig.key('4');
    rig.drag_in_control(ctl::GRID, &[cell(1), cell(2), cell(6)], seen, "1 to 6");
    rig.tap(egui::Key::Escape);
    assert!(rig.app.ui.range_requester.is_none());
    assert_eq!(rig.app.extras.ranges, kept, "Esc keeps the ranges");
    seen(rig, "Esc".into());
    open(rig, seen);
    rig.key('4');
    rig.drag_in_control(ctl::GRID, &[cell(1), cell(2), cell(6)], seen, "1 to 6");
    rig.click_control(ctl::OK, (0.95, 0.9));
    assert!(rig.app.ui.range_requester.is_none());
    assert_eq!(rig.app.extras.ranges.len(), 4);
    assert_eq!(
        (rig.app.extras.ranges[3].low, rig.app.extras.ranges[3].high),
        (1, 6)
    );
    seen(rig, "Ok".into());
    open(rig, seen);
    rig.key('4');
    rig.click_control(ctl::CLEAR, MID);
    rig.tap(egui::Key::Enter);
    assert_eq!(
        rig.app.extras.ranges, kept,
        "Return: range 4 is cleared again"
    );
    seen(rig, "Return".into());
}

#[test]
fn the_range_requester_does_the_same_in_both_skins() {
    both_at_every_scale(range_script);
}

/// Anim ▸ Move.. (slice 4b): the nine number fields, the Brush and Cyclic boxes, the Move
/// and Record icon buttons, Clear, Go Back, a refused number, Preview (which runs and comes
/// back), Draw by its button and by Return, Trails, Exit, Cancel and Esc.
fn move_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::AnimControlRequester;
    use dpx_ui::anim_control::move_ctl as ctl;
    const MID: (f32, f32) = (0.5, 0.5);
    let erase = |rig: &mut Rig, n: usize| {
        for _ in 0..n {
            rig.tap(egui::Key::Backspace);
        }
    };
    let open = |rig: &mut Rig, seen: Seen| {
        rig.pick(Action::Command("anim.move"));
        assert!(matches!(
            rig.app.ui.anim_control,
            Some(AnimControlRequester::Move(_))
        ));
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::AnimControl));
        }
        seen(rig, "Move..".into());
    };
    let is_open = |rig: &Rig| rig.app.ui.anim_control.is_some();
    // Four frames, a colour, and the brush clicked down once.
    rig.pick(Action::Command("anim.set_frame_count"));
    erase(rig, 1);
    rig.type_text("4");
    rig.tap(egui::Key::Enter);
    rig.click(Place::Cell(5), Primary);
    rig.click(Place::Page(40, 30), Primary);
    seen(rig, "four frames and a dot".into());
    open(rig, seen);
    // Every field: a click, a number, the editing keys, a minus sign.
    for field in 0..9 {
        rig.click_control(ctl::FIELD + field, MID);
        erase(rig, 6);
        rig.type_text(["12", "-3", "0", "x7", "45", "-0", "2", "1", "4"][usize::from(field)]);
        seen(rig, format!("field {field}"));
    }
    rig.click_control(ctl::FIELD + 1, (0.9, 0.5));
    for key in [
        egui::Key::Home,
        egui::Key::Delete,
        egui::Key::End,
        egui::Key::ArrowLeft,
        egui::Key::ArrowRight,
        egui::Key::ArrowUp,
        egui::Key::ArrowDown,
        egui::Key::Tab,
    ] {
        rig.tap(key);
        seen(rig, format!("{key:?}"));
    }
    rig.pass(vec![egui::Event::Paste("-1a2".into())]);
    seen(rig, "pasted text".into());
    // The boxes and the icon buttons act on the press.
    for id in [
        ctl::DIST_BRUSH,
        ctl::ANGLE_BRUSH,
        ctl::CYCLIC,
        ctl::COME_TO,
        ctl::GO_FROM,
        ctl::RECORD + 2,
        ctl::RECORD + 1,
        ctl::RECORD,
        ctl::ANGLE_BRUSH,
    ] {
        rig.click_control(id, MID);
        seen(rig, format!("control {id}"));
    }
    rig.click_control(ctl::COME_TO, (0.06, 0.1));
    rig.click_control(ctl::RECORD + 1, (0.94, 0.9));
    rig.press_and_leave(ctl::GO_FROM);
    seen(
        rig,
        "icon buttons by their corners, one pressed and left".into(),
    );
    // Buttons pressed and left, a click beside, keys that mean nothing.
    for id in [
        ctl::CLEAR,
        ctl::GO_BACK,
        ctl::PREVIEW,
        ctl::TRAILS,
        ctl::DRAW,
        ctl::CANCEL,
        ctl::EXIT,
    ] {
        rig.press_and_leave(id);
    }
    rig.click_beside();
    rig.type_text("q");
    rig.wheel(-1.0);
    assert!(is_open(rig));
    seen(rig, "buttons pressed and left".into());
    // A refused number: the field gets the keys and the message says why.
    rig.click_control(ctl::FIELD + 8, MID);
    erase(rig, 3);
    rig.type_text("0");
    rig.click_control(ctl::DRAW, MID);
    assert!(is_open(rig), "a count of 0 is refused");
    seen(rig, "Draw with a count of 0".into());
    erase(rig, 1);
    rig.type_text("4");
    seen(rig, "a count of 4".into());
    // Exit keeps the settings; they are there when it opens again.
    rig.click_control(ctl::EXIT, (0.95, 0.9));
    assert!(!is_open(rig));
    let kept = rig
        .app
        .anim_control
        .settings
        .expect("Exit keeps the settings");
    assert_eq!((kept.dist[0], kept.count, kept.cyclic), (12, 4, false));
    seen(rig, "Exit".into());
    open(rig, seen);
    rig.click_control(ctl::CLEAR, MID);
    seen(rig, "Clear".into());
    rig.click_control(ctl::CANCEL, (0.05, 0.1));
    assert!(!is_open(rig));
    assert_eq!(rig.app.anim_control.settings, Some(kept), "Cancel");
    seen(rig, "Cancel".into());
    open(rig, seen);
    rig.click_control(ctl::GO_BACK, MID);
    assert!(is_open(rig), "Go Back stays open");
    seen(rig, "Go Back".into());
    rig.tap(egui::Key::Escape);
    assert!(!is_open(rig));
    seen(rig, "Esc".into());
    // Preview: the wire frame runs on the picture (no card), then the requester is back.
    open(rig, seen);
    rig.click_control(ctl::PREVIEW, MID);
    assert!(rig.app.anim_control.preview.is_some(), "the preview runs");
    assert_eq!(rig.card(), None, "the picture alone");
    for _ in 0..400 {
        if rig.app.anim_control.preview.is_none() {
            break;
        }
        rig.pass(vec![]);
    }
    assert!(rig.app.anim_control.preview.is_none() && is_open(rig));
    seen(rig, "after the preview".into());
    // Draw: by Return, by its button, and with trails.
    rig.tap(egui::Key::Enter);
    assert!(!is_open(rig), "Return draws");
    seen(rig, "Return".into());
    rig.key('u');
    seen(rig, "undo".into());
    open(rig, seen);
    rig.click_control(ctl::DRAW, (0.05, 0.1));
    assert!(!is_open(rig), "Draw");
    seen(rig, "Draw".into());
    open(rig, seen);
    rig.click_control(ctl::TRAILS, MID);
    assert!(!is_open(rig), "Trails draws");
    seen(rig, "Trails".into());
}

#[test]
fn the_move_requester_does_the_same_in_both_skins() {
    both_at_every_scale(move_script);
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

/// A drawer of its own for the file requester scripts: twenty files that are no pictures,
/// a drawer inside, a very long name. Made anew for every run, so the Classic and the
/// Modern window list the same.
fn file_drawer(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("dpx-modern-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("brushes")).expect("a drawer");
    for i in 0..20 {
        std::fs::write(dir.join(format!("pic{i:02}.iff")), b"no picture").expect("a file");
    }
    std::fs::write(dir.join("notes.txt"), b"text").expect("a file");
    let long = format!("a-{}.iff", "very-long-name-".repeat(8));
    std::fs::write(dir.join(long), b"no picture").expect("a file");
    dir.to_string_lossy().into_owned()
}

/// The labels of the list rows the file requester shows, and whether it has a scroll bar.
fn file_rows(rig: &Rig) -> (Vec<String>, bool) {
    let dialog = rig.dialog();
    let list = dialog.columns[0]
        .iter()
        .find_map(|i| match i {
            dpx_ui::dialog::Item::List(list) => Some(list),
            _ => None,
        })
        .expect("the list");
    (
        list.rows.iter().map(|r| r.label.clone()).collect(),
        list.scroll.is_some(),
    )
}

/// The control of the shown row with this label.
fn file_row(rig: &Rig, label: &str) -> ControlId {
    let at = file_rows(rig)
        .0
        .iter()
        .position(|l| l == label)
        .unwrap_or_else(|| panic!("a row '{label}'"));
    dpx_ui::file_requester::ctl::ROW + at as ControlId
}

/// The file requester (slice 4c): Load with the list (wheel, arrow keys, the scroll bar by
/// a press and by a drag of its knob, a file, a double click, a drawer), the three fields
/// (typing, paste, Tab, the caret by a click, a pattern), Disks, Assigns, Parent, messages,
/// Cancel, Esc; Save with the Replace question (Yes, No, the keys); Export; Delete with its
/// question.
fn file_script(rig: &mut Rig, seen: Seen) {
    use dpx_ui::file_requester::ctl;
    const MID: (f32, f32) = (0.5, 0.5);
    let drawer = file_drawer("file");
    let open = |rig: &mut Rig, seen: Seen, command: &'static str| {
        rig.app.last_drawer = Some(drawer.clone());
        rig.pick(Action::Command(command));
        assert!(rig.app.ui.file_requester.is_some(), "{command}");
        if rig.app.is_modern() {
            assert_eq!(rig.card(), Some(Card::File));
        }
        seen(rig, command.to_string());
    };
    let file = |rig: &Rig| rig.app.ui.file_requester.as_ref().expect("open").file();
    let is_open = |rig: &Rig| rig.app.ui.file_requester.is_some();

    // Load: the list.
    open(rig, seen, "picture.load");
    let (rows, bar) = file_rows(rig);
    assert_eq!((rows.len(), bar), (8, true));
    assert_eq!(rows[0], "brushes");
    assert!(rows[1].starts_with("a-very-long-name-"));
    rig.wheel(-3.0);
    assert_eq!(
        file_rows(rig).0[0],
        "pic00.iff",
        "the wheel scrolls three rows"
    );
    seen(rig, "the wheel".into());
    rig.tap(egui::Key::ArrowDown);
    rig.tap(egui::Key::ArrowDown);
    rig.tap(egui::Key::ArrowUp);
    assert_eq!(file_rows(rig).0[0], "pic01.iff");
    seen(rig, "arrow keys".into());
    rig.wheel(40.0);
    assert_eq!(file_rows(rig).0[0], "brushes", "back at the top");
    seen(rig, "the wheel back".into());
    // The scroll bar: a press under the knob pages, a press over it pages back.
    rig.click_control(ctl::SCROLL, (0.5, 0.97));
    assert_eq!(file_rows(rig).0[0], "pic05.iff");
    seen(rig, "a press under the knob".into());
    rig.click_control(ctl::SCROLL, (0.5, 0.02));
    assert_eq!(file_rows(rig).0[0], "brushes");
    seen(rig, "a press over the knob".into());
    // The knob dragged down, beyond the end, and back up a little.
    let from = rig.control_point(ctl::SCROLL, (0.5, 0.1));
    rig.pass(vec![
        egui::Event::PointerMoved(from),
        pointer_button(from, Primary, true),
    ]);
    seen(rig, "the knob held".into());
    for to in [0.4, 0.75, 1.0, 0.6] {
        let pos = rig.control_point(ctl::SCROLL, (0.5, to));
        rig.pass(vec![egui::Event::PointerMoved(pos)]);
        seen(rig, format!("the knob dragged to {to}"));
    }
    let end = rig.control_point(ctl::SCROLL, (0.5, 1.0));
    rig.pass(vec![egui::Event::PointerMoved(end)]);
    assert_eq!(file_rows(rig).0[7], "pic19.iff", "the end of the list");
    rig.pass(vec![pointer_button(end, Primary, false)]);
    seen(rig, "the knob let go at the end".into());
    rig.wheel(40.0);
    // A file: its name goes into the File field, the row is the chosen one.
    let row = file_row(rig, "pic02.iff");
    rig.click_control(row, MID);
    assert_eq!(file(rig), "pic02.iff");
    seen(rig, "a file".into());
    // The first and the last pixel of a row are that row.
    let row = file_row(rig, "pic03.iff");
    rig.click_control(row, (0.01, 0.05));
    assert_eq!(file(rig), "pic03.iff");
    rig.t += 1.0;
    let row = file_row(rig, "pic04.iff");
    rig.click_control(row, (0.99, 0.95));
    assert_eq!(file(rig), "pic04.iff");
    seen(rig, "rows by their corners".into());
    // A double click loads; this file is no picture, so the requester says so and stays.
    rig.t += 1.0;
    rig.click_control(row, MID);
    rig.click_control(row, MID);
    seen(rig, "a double click on a file that is no picture".into());
    if rig.app.ui.note_requester.is_some() {
        rig.tap(egui::Key::Enter);
        seen(rig, "the note closed".into());
    }
    if !is_open(rig) {
        open(rig, seen, "picture.load");
    }
    // The long name ends in an ellipsis on the card and is whole in the File field.
    let long = file_rows(rig).0[1].clone();
    let row = file_row(rig, &long);
    rig.click_control(row, MID);
    assert_eq!(file(rig), long);
    seen(rig, "the long name".into());
    // A drawer, and Parent back.
    let row = file_row(rig, "brushes");
    rig.click_control(row, MID);
    assert!(file_rows(rig).0.is_empty(), "an empty drawer");
    assert!(!file_rows(rig).1, "nothing to scroll");
    seen(rig, "into a drawer".into());
    rig.click_control(ctl::PARENT, MID);
    assert_eq!(file_rows(rig).0[0], "brushes");
    seen(rig, "Parent".into());
    // Disks and Assigns, each by a corner; then the drawer typed into its field.
    rig.click_control(ctl::DISKS, (0.02, 0.05));
    seen(rig, "Disks".into());
    rig.click_control(ctl::ASSIGNS, (0.98, 0.95));
    seen(rig, "Assigns".into());
    rig.click_control(ctl::DRAWER, MID);
    rig.type_text(&drawer);
    seen(rig, "the drawer typed".into());
    rig.tap(egui::Key::Enter);
    assert_eq!(file_rows(rig).0[0], "brushes");
    seen(rig, "Return lists it".into());
    // The fields: Tab goes round, typing and pasting go to the one that has the keys.
    rig.click_control(ctl::FILE, (0.99, 0.5));
    rig.tap(egui::Key::Backspace);
    rig.tap(egui::Key::Home);
    rig.type_text("x");
    rig.tap(egui::Key::ArrowRight);
    rig.tap(egui::Key::Delete);
    rig.tap(egui::Key::End);
    rig.pass(vec![egui::Event::Paste("-pasted\nname".into())]);
    seen(rig, "editing keys and paste in the File field".into());
    rig.tap(egui::Key::Tab);
    rig.tap(egui::Key::Tab);
    rig.type_text("#?.txt");
    seen(rig, "a pattern typed".into());
    rig.tap(egui::Key::Enter);
    assert_eq!(file_rows(rig).0, ["brushes", "notes.txt"]);
    seen(rig, "Return applies the pattern".into());
    rig.click_control(ctl::PATTERN, (0.99, 0.5));
    for _ in 0..6 {
        rig.tap(egui::Key::Backspace);
    }
    rig.type_text("(");
    rig.tap(egui::Key::Tab);
    seen(rig, "a broken pattern says why".into());
    rig.click_control(ctl::PATTERN, (0.99, 0.5));
    rig.tap(egui::Key::Backspace);
    rig.click_control(ctl::FILE, (0.01, 0.5));
    rig.type_text("^");
    assert!(
        file(rig).contains('^'),
        "the caret where the field was pressed"
    );
    seen(rig, "a click at the start of the File field".into());
    // Buttons held and left; a press beside the card.
    for id in [
        ctl::DISKS,
        ctl::ASSIGNS,
        ctl::PARENT,
        ctl::ACTION,
        ctl::CANCEL,
    ] {
        rig.press_and_leave(id);
        assert!(is_open(rig));
        seen(rig, format!("control {id} pressed and left"));
    }
    rig.press_beside_and_enter(ctl::CANCEL);
    rig.click_beside();
    assert!(is_open(rig));
    seen(rig, "pressed beside".into());
    // No name: Load refuses with a message.
    rig.tap(egui::Key::Escape);
    open(rig, seen, "picture.load");
    assert_eq!(file(rig), "");
    rig.click_control(ctl::ACTION, (0.02, 0.05));
    assert!(is_open(rig));
    seen(rig, "Load without a name".into());
    rig.tap(egui::Key::Enter);
    seen(rig, "Return without a name".into());
    rig.click_control(ctl::CANCEL, (0.98, 0.95));
    assert!(!is_open(rig), "Cancel closes it");
    seen(rig, "Cancel".into());
    open(rig, seen, "picture.load");
    rig.tap(egui::Key::Escape);
    assert!(!is_open(rig), "Esc closes it");
    seen(rig, "Esc".into());

    // Save: a new project is written; saving it again asks.
    rig.click(Place::Page(40, 40), Primary);
    open(rig, seen, "picture.save");
    rig.type_text("castle");
    rig.tap(egui::Key::Enter);
    assert!(!is_open(rig), "saved");
    seen(rig, "Save by Return".into());
    assert!(
        std::path::Path::new(&drawer)
            .join("castle.dpxproj")
            .exists()
    );
    type Answer = fn(&mut Rig);
    let answers: [(&str, bool, Answer); 6] = [
        ("No", false, |rig| rig.click_control(ctl::NO, (0.5, 0.5))),
        ("the n key", false, |rig| rig.type_text("n")),
        ("Esc", false, |rig| rig.tap(egui::Key::Escape)),
        ("Yes", true, |rig| rig.click_control(ctl::YES, (0.02, 0.05))),
        ("the y key", true, |rig| rig.type_text("Y")),
        ("Yes, at its last pixel", true, |rig| {
            rig.click_control(ctl::YES, (0.98, 0.95))
        }),
    ];
    for (name, saves, answer) in answers {
        open(rig, seen, "picture.save");
        rig.click_control(ctl::ACTION, MID);
        let asks = rig.dialog();
        assert_eq!(asks.buttons.len(), 2, "the question");
        assert!(asks.control(ctl::ACTION).is_none());
        seen(rig, "Replace it?".into());
        rig.tap(egui::Key::Enter);
        rig.tap(egui::Key::Tab);
        rig.type_text("q");
        rig.wheel(-2.0);
        rig.click_beside();
        rig.press_and_leave(ctl::YES);
        rig.press_and_leave(ctl::NO);
        assert_eq!(rig.dialog(), asks, "still asking");
        seen(rig, "keys and presses that do not answer".into());
        answer(rig);
        assert_eq!(is_open(rig), !saves, "{name}");
        seen(rig, name.to_string());
        if !saves {
            assert!(rig.dialog().control(ctl::ACTION).is_some(), "the requester");
            rig.tap(egui::Key::Escape);
            seen(rig, "Esc".into());
        }
    }

    // Export: IFF beside the project; a second export of the same name asks too.
    open(rig, seen, "picture.export");
    rig.click_control(ctl::FILE, (0.99, 0.5));
    for _ in 0..40 {
        rig.tap(egui::Key::Backspace);
    }
    rig.type_text("castle.iff");
    rig.click_control(ctl::ACTION, MID);
    seen(rig, "Export".into());
    while rig.app.ui.note_requester.is_some() {
        rig.tap(egui::Key::Enter);
        seen(rig, "the export note closed".into());
    }
    assert!(!rig.app.modal_open());
    assert!(std::path::Path::new(&drawer).join("castle.iff").exists());

    // Delete: the question first, then the file is gone.
    open(rig, seen, "picture.delete");
    let row = file_row(rig, "castle.iff");
    rig.click_control(row, MID);
    rig.click_control(ctl::ACTION, MID);
    assert_eq!(rig.dialog().buttons.len(), 2, "Delete this file?");
    seen(rig, "Delete asks".into());
    rig.click_control(ctl::NO, MID);
    seen(rig, "No".into());
    rig.tap(egui::Key::Enter);
    seen(rig, "Return asks again".into());
    rig.click_control(ctl::YES, MID);
    seen(rig, "Yes".into());
    assert!(!std::path::Path::new(&drawer).join("castle.iff").exists());
    if is_open(rig) {
        rig.tap(egui::Key::Escape);
    }
    while rig.app.ui.note_requester.is_some() {
        rig.tap(egui::Key::Enter);
    }
    assert!(!rig.app.modal_open());
    seen(rig, "closed".into());
    let _ = std::fs::remove_dir_all(&drawer);
}

#[test]
fn the_file_requester_does_the_same_in_both_skins() {
    both_at_every_scale(file_script);
}

/// Every converted requester fits the smallest window and is drawn there, in both themes
/// and at device scale 1 and 2 (the images go to `DPX_MODERN_SHOTS` for a look).
#[test]
fn every_card_fits_the_smallest_window() {
    type Open = fn(&mut Rig);
    let open: [(&str, Card, Open); 26] = [
        ("file-load", Card::File, |rig| {
            rig.pick(Action::Command("picture.load"))
        }),
        ("file-save", Card::File, |rig| {
            rig.pick(Action::Command("picture.save"))
        }),
        ("move", Card::AnimControl, |rig| {
            rig.pick(Action::Command("anim.move"))
        }),
        ("stencil", Card::Tool, |rig| {
            rig.pick(Action::Command("stencil.make"))
        }),
        ("arrange", Card::Tool, |rig| {
            rig.pick(Action::Command("palette.arrange"))
        }),
        ("range", Card::Range, |rig| {
            rig.pick(Action::Command("color.ranges"))
        }),
        ("perspective", Card::Tool, |rig| {
            rig.app.open_perspective_requester();
            rig.pass(vec![]);
        }),
        ("animbrush-settings", Card::Tool, |rig| {
            rig.app.ui.tool_requester = Some(dpx_ui::ToolRequester::AnimBrush(
                dpx_ui::AnimBrushRequester::settings(20, 20, 3, dpx_core::CelDirection::Forward),
            ));
            rig.pass(vec![]);
        }),
        ("animbrush-pick", Card::Tool, |rig| {
            rig.app.ui.tool_requester = Some(dpx_ui::ToolRequester::AnimBrush(
                dpx_ui::AnimBrushRequester::pick_up(4, 20),
            ));
            rig.pass(vec![]);
        }),
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

/// A question over another converted requester (here the file requester): only the one on
/// top is a card, nothing is hosted in the picture, the keys belong to the question, and
/// when it is answered the file requester is the card and takes the keys.
#[test]
fn a_question_over_another_card_is_the_only_card() {
    use dpx_ui::choice_requester::BUTTONS;
    let mut rig = Rig::new(UiStyle::Modern, (800.0, 600.0), 1.0);
    let plain = rig.app.modern_layout().expect("layout");
    rig.pick(Action::Command("picture.load"));
    rig.pass(vec![]);
    assert!(rig.app.ui.file_requester.is_some());
    assert_eq!(rig.card(), Some(Card::File));
    assert_eq!(rig.app.modern_layout().expect("layout"), plain);
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
    assert!(rig.app.modern_plain_picture());
    assert_eq!(rig.app.modern_layout().expect("layout"), plain);
    // The quad shows neither of them in the Classic look.
    let screen = rig.app.compose_for_skin();
    let mut bare = rig.app.ui.clone();
    bare.choice_requester = None;
    bare.file_requester = None;
    assert_eq!(
        screen,
        rig.app
            .view()
            .compose(&rig.app.page(), rig.app.scroll, rig.app.palette(), &bare)
    );
    rig.type_text("abc");
    let file = rig.app.ui.file_requester.as_ref().map(|r| r.file());
    assert_eq!(file.as_deref(), Some(""), "the keys belong to the question");
    rig.click_control(BUTTONS[1], (0.5, 0.5));
    assert!(rig.app.ui.choice_requester.is_none(), "Keep answers");
    assert!(rig.app.ui.file_requester.is_some());
    assert_eq!(rig.card(), Some(Card::File));
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
    let others: [(&str, Open); 3] = [
        ("Color Mixer", |rig| rig.key('p')),
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
        assert_eq!(rig.card(), None, "{name}");
    }
}
