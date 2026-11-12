//! Tooltips in the app (ADR-41): when the pointer has rested half a second on a toolbox tool
//! button (on split buttons: one half) or a button of the Animation Control Panel, the screen
//! shows a box naming it with its key. Texts, layout and look live in `dpx_ui::tooltip`; this
//! module decides when one shows.
//!
//! The state machine is fed once per pass from the hover branch of `PainterApp::ui` with what
//! lies under the pointer, and checked before every upload: any mouse button, the menu strip,
//! a requester, note, Show Page or playback, and a button that is not shown (toolbox hidden
//! with F10, panel off) hide it, and the delay starts again afterwards. egui only runs a pass
//! on input, so while the pointer rests the pass asks for one when the delay is over; once the
//! tooltip shows, it asks for nothing more. The key in the text follows the keyboard's mode:
//! perspective mode and typing text take keys for themselves ([`KeyMode`]).

use std::time::Duration;

use dpx_ui::{Hit, KeyMode, Screen, Tooltip, TooltipTarget, UiState};
use eframe::egui;

use crate::PainterApp;

/// How long the pointer rests on a button before its tooltip shows (seconds).
pub const DELAY: f64 = 0.5;

/// A pass this much before the end of the delay counts as on time (the wake-up time of
/// `t + delay` is rounded, and timers may fire a little early).
const SLACK: f64 = 1e-3;

/// Tooltip state of the window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    /// The target under the pointer and when the pointer came to it (egui time, seconds).
    target: Option<(TooltipTarget, f64)>,
    /// The tooltip shows. While the pointer goes straight from one target to the next, the
    /// next one shows at once.
    shown: bool,
    /// The last upload had the menu strip or a modal open.
    blocked: bool,
}

impl State {
    /// Hidden; the next target waits the whole delay.
    fn hide(&mut self) {
        self.target = None;
        self.shown = false;
    }
}

impl PainterApp {
    /// The pointer on the paint screen (the hover branch of `ui()`): `hit` is what lies under
    /// it, `None` outside the window or off the page.
    pub(crate) fn tooltip_hover(&mut self, ctx: &egui::Context, hit: Option<Hit>) {
        let (now, buttons) = ctx.input(|i| {
            let p = &i.pointer;
            (i.time, p.any_down() || p.any_pressed())
        });
        let target = hit.and_then(TooltipTarget::of_hit);
        let tip = &mut self.tooltip;
        match target {
            // A press or a held button hides it; the delay starts again after the release.
            _ if buttons => tip.hide(),
            // Page, palette, gaps, outside the window: hidden, the next one waits again.
            None => tip.hide(),
            // Moving on the same button (and half) keeps the time it came.
            Some(t) if tip.target.is_some_and(|(old, _)| old == t) => {}
            Some(t) => tip.target = Some((t, now)),
        }
        if let State {
            target: Some((_, since)),
            shown: false,
            ..
        } = *tip
        {
            let left = DELAY - (now - since);
            if left <= SLACK {
                tip.shown = true;
            } else {
                // An idle window gets no pass by itself: ask for one when the delay is over.
                ctx.request_repaint_after(Duration::from_secs_f64(left));
            }
        }
        self.sync_tooltip();
    }

    /// Before every upload: no tooltip while a requester, note, Show Page or playback is open
    /// (the pass's keys may have opened one) or the menu strip is shown (it opens with a
    /// press, which hides the tooltip already). A button that is no longer shown (toolbox
    /// hidden with F10, panel off) needs nothing here: it is not hit any more, and
    /// `Tooltip::layout` draws no box for it meanwhile. The text follows a key mode changed
    /// in this pass (Return into perspective mode, Esc ending the text).
    pub(crate) fn tooltip_guard(&mut self, ctx: &egui::Context) {
        if self.ui.menu.active || self.modal_open() {
            self.tooltip.hide();
            self.tooltip.blocked = true;
        } else if std::mem::take(&mut self.tooltip.blocked) {
            // It closed in this pass. Play Once ends by itself, with no input that brings the
            // next pass: one more, so the hover branch starts the delay under a resting pointer.
            ctx.request_repaint();
        }
        self.sync_tooltip();
    }

    /// The keys that do their paint-screen job now: perspective mode takes the digits and
    /// `<` / `>` (`perspective_keys` runs first), typing text takes every key.
    fn tooltip_keys(&self) -> KeyMode {
        if self.typing() {
            KeyMode::Typing
        } else if self.perspective.active {
            KeyMode::Perspective
        } else {
            KeyMode::Paint
        }
    }

    /// `UiState::tooltip` follows the state; the screen is redrawn only when it changes.
    fn sync_tooltip(&mut self) {
        let keys = self.tooltip_keys();
        let shown = match self.tooltip {
            State {
                target: Some((t, _)),
                shown: true,
                ..
            } => Some((t, keys)),
            _ => None,
        };
        if self.ui.tooltip.as_ref().map(|t| (t.target, t.keys)) != shown {
            self.ui.tooltip = shown.map(|(t, keys)| Tooltip::new(t, keys));
            self.dirty = true;
        }
    }
}

/// `--hover X,Y` for snapshots: the tooltip a pointer resting at UI pixel X,Y shows (none
/// while a menu or requester is shown, as in the window).
pub fn snapshot_hover(
    args: &[String],
    palette_len: usize,
    screen: &Screen,
    ui: &mut UiState,
) -> Result<(), String> {
    let Some(spec) = crate::arg_value(args, "--hover") else {
        return Ok(());
    };
    let (x, y) = spec
        .split_once(',')
        .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
        .ok_or_else(|| format!("--hover '{spec}': expected X,Y (UI pixels)"))?;
    let open = ui.menu.active
        || ui.file_requester.is_some()
        || ui.page_size_requester.is_some()
        || ui.note_requester.is_some()
        || ui.choice_requester.is_some()
        || ui.palette_requester.is_some()
        || ui.range_requester.is_some()
        || ui.tool_requester.is_some()
        || ui.anim_requester.is_some()
        || ui.anim_control.is_some();
    let target = TooltipTarget::of_hit(screen.hit_test(palette_len, ui, x, y));
    // Snapshots start on the paint screen: every key works.
    ui.tooltip = target
        .filter(|_| !open)
        .map(|t| Tooltip::new(t, KeyMode::Paint));
    if ui.tooltip.is_none() {
        eprintln!("note: --hover {x},{y}: no tooltip there");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{chord, click_ui, menu_item_path, pointer_button, tap, ui_pass, ui_point};
    use dpx_core::Command;
    use dpx_ui::anim_panel::{CONTROLS, Layout, TOGGLES};
    use dpx_ui::toolbox::TOOL_ROWS;
    use dpx_ui::{PanelButton, Tool};
    use egui::PointerButton::{Primary, Secondary};

    /// A UI pixel inside a tool button: its upper-left half, or the lower-right (filled) one.
    /// The tests' screen: PAL LoRes, 320×256 UI pixels (checked in `started`).
    const SCREEN: (i32, i32) = (320, 256);

    fn tool_at(tool: Tool, filled: bool) -> (i32, i32) {
        let b = Screen::new(SCREEN.0, SCREEN.1).tool_button(tool);
        if filled {
            (b.x + 9, b.y + 9)
        } else {
            (b.x + 2, b.y + 2)
        }
    }

    /// The centre of a panel button; `toolbox`: the toolbox is shown.
    fn panel_at(button: PanelButton, toolbox: bool) -> (i32, i32) {
        let r = Layout::new(SCREEN.0, SCREEN.1, toolbox).button(button);
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    /// One pass at `t` with `events`, as the window runs it: after a pass with input egui asks
    /// for two more at once (they run here at the same time); returns the repaint delay egui
    /// asks for after them.
    fn input(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        events: Vec<egui::Event>,
    ) -> std::time::Duration {
        let had_input = !events.is_empty();
        let mut delay = ui_pass(app, ctx, t, events);
        for _ in 0..4 {
            if !(had_input && delay.is_zero()) {
                break;
            }
            delay = ui_pass(app, ctx, t, vec![]);
        }
        delay
    }

    /// The pointer moved to UI pixel `at` (see [`input`]).
    fn move_to(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        at: (i32, i32),
    ) -> std::time::Duration {
        let pos = ui_point(app, at);
        input(app, ctx, t, vec![egui::Event::PointerMoved(pos)])
    }

    /// Passes as the idle window runs them (only when egui asks for one) from `t` with the
    /// `delay` of the last pass, until egui asks for none or `until`; returns the time of the
    /// last pass.
    fn idle(
        app: &mut PainterApp,
        ctx: &egui::Context,
        mut t: f64,
        mut delay: std::time::Duration,
        until: f64,
    ) -> f64 {
        while delay < std::time::Duration::from_secs(10) && t < until {
            t += delay.as_secs_f64();
            delay = ui_pass(app, ctx, t, vec![]);
        }
        t
    }

    fn shown(app: &PainterApp) -> Option<&str> {
        app.ui.tooltip.as_ref().map(|t| t.text.as_str())
    }

    /// The pointer comes to `at` at `t` and rests until the tooltip shows; returns the time.
    fn rest_on(app: &mut PainterApp, ctx: &egui::Context, t: f64, at: (i32, i32)) -> f64 {
        let delay = move_to(app, ctx, t, at);
        let t = idle(app, ctx, t, delay, t + 2.0);
        assert!(app.ui.tooltip.is_some(), "a tooltip at {at:?}");
        t
    }

    fn started() -> (PainterApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        input(&mut app, &ctx, 0.0, vec![]);
        assert_eq!(app.view().ui_size(), SCREEN);
        (app, ctx)
    }

    #[test]
    fn a_resting_pointer_shows_the_tooltip_after_half_a_second_then_asks_for_nothing() {
        let (mut app, ctx) = started();
        let line = app.view().ui_screen().tool_button(Tool::Line);
        let secs = std::time::Duration::from_secs_f64;
        let delay = move_to(&mut app, &ctx, 1.0, (line.x + 2, line.y + 2));
        assert_eq!(shown(&app), None, "not at once");
        // The idle window wakes up when the delay is over, not later.
        assert!(delay <= secs(DELAY), "{delay:?}");
        assert!(delay >= secs(DELAY - 0.01), "{delay:?}");
        // Moving on the same button does not start the delay again.
        let delay = move_to(&mut app, &ctx, 1.3, (line.x + 8, line.y + 5));
        assert_eq!(shown(&app), None);
        assert!(delay <= secs(0.2 + 1e-9), "{delay:?}");
        let t = 1.3 + delay.as_secs_f64();
        let delay = input(&mut app, &ctx, t, vec![]);
        assert_eq!(shown(&app), Some("Straight Line  v"), "at {t}");
        // Shown: no more passes for the tooltip, also when the pointer moves on the button.
        assert_eq!(delay, std::time::Duration::MAX);
        assert_eq!(
            move_to(&mut app, &ctx, 2.0, (line.x + 3, line.y + 7)),
            std::time::Duration::MAX
        );
        assert_eq!(input(&mut app, &ctx, 3.0, vec![]), std::time::Duration::MAX);
        assert_eq!(shown(&app), Some("Straight Line  v"));
        // It is on the uploaded screen: the box left of the toolbox, at the button's top.
        let screen = app.compose_screen();
        let tip = app.ui.tooltip.clone().unwrap();
        let s = app.view().ui_screen();
        let r = tip.layout(s.width, s.height, &app.ui).unwrap();
        assert_eq!((r.x + r.w, r.y), (s.toolbox_x() - 2, line.y));
        let pens = dpx_ui::Pens::from_palette(app.palette());
        assert_eq!(screen.get(r.x, r.y), Some(pens.dark));
        assert_eq!(screen.get(r.x + 1, r.y + 1), Some(pens.face));
    }

    #[test]
    fn the_next_button_shows_at_once_until_the_pointer_leaves_the_buttons() {
        let (mut app, ctx) = started();
        let t = rest_on(&mut app, &ctx, 1.0, tool_at(Tool::Line, false));
        // Straight to the next button: at once.
        let at = tool_at(Tool::Curve, false);
        move_to(&mut app, &ctx, t + 0.1, at);
        assert_eq!(shown(&app), Some("Curve  q"));
        // Across the diagonal of a split button: the other half at once.
        move_to(&mut app, &ctx, t + 0.2, tool_at(Tool::Rectangle, false));
        assert_eq!(shown(&app), Some("Rectangle  r"));
        move_to(&mut app, &ctx, t + 0.3, tool_at(Tool::Rectangle, true));
        assert_eq!(shown(&app), Some("Filled Rectangle  R"));
        move_to(&mut app, &ctx, t + 0.4, tool_at(Tool::Ellipse, true));
        assert_eq!(shown(&app), Some("Filled Ellipse  E"));
        // A gap between buttons, a built-in brush, the palette, the page: hidden, and the next
        // button waits the whole delay again.
        let s = app.view().ui_screen();
        let gap = (s.toolbox_x() + 11, tool_at(Tool::Line, false).1);
        let brush = (s.toolbox_x() + 3, 14);
        let palette = (s.toolbox_x() + 1, s.height - 3);
        for (i, away) in [gap, brush, palette, (100, 100)].into_iter().enumerate() {
            let t = 10.0 * (i + 1) as f64;
            rest_on(&mut app, &ctx, t, tool_at(Tool::Fill, false));
            let delay = move_to(&mut app, &ctx, t + 3.0, away);
            assert_eq!(shown(&app), None, "away at {away:?}");
            assert_eq!(delay, std::time::Duration::MAX, "nothing to wait for");
            move_to(&mut app, &ctx, t + 3.1, tool_at(Tool::Airbrush, false));
            assert_eq!(shown(&app), None, "after {away:?}: the delay again");
            input(&mut app, &ctx, t + 3.1 + DELAY, vec![]);
            assert_eq!(shown(&app), Some("Airbrush"));
        }
        // The pointer leaves the window.
        input(&mut app, &ctx, 60.0, vec![egui::Event::PointerGone]);
        assert_eq!(shown(&app), None);
    }

    #[test]
    fn a_press_hides_it_and_after_the_release_it_waits_again() {
        let (mut app, ctx) = started();
        let at = tool_at(Tool::Circle, true);
        let t = rest_on(&mut app, &ctx, 1.0, at);
        assert_eq!(shown(&app), Some("Filled Circle  C"));
        let pos = ui_point(&app, at);
        input(
            &mut app,
            &ctx,
            t + 0.1,
            vec![pointer_button(pos, Primary, true)],
        );
        assert_eq!(shown(&app), None, "the press hides it");
        assert_eq!((app.ui.tool, app.ui.filled), (Tool::Circle, true));
        // Held down: none, and nothing to wait for.
        let delay = input(&mut app, &ctx, t + 1.0, vec![]);
        assert_eq!(shown(&app), None, "while the button is held");
        assert_eq!(delay, std::time::Duration::MAX);
        let delay = input(
            &mut app,
            &ctx,
            t + 1.5,
            vec![pointer_button(pos, Primary, false)],
        );
        assert_eq!(shown(&app), None, "the delay starts again at the release");
        assert!(delay <= std::time::Duration::from_secs_f64(DELAY));
        input(&mut app, &ctx, t + 1.5 + DELAY, vec![]);
        assert_eq!(shown(&app), Some("Filled Circle  C"));
        // The right button too (on Zoom: smaller), also a click within one pass.
        let t = rest_on(&mut app, &ctx, 10.0, tool_at(Tool::Zoom, false));
        let zoom = app.tools.zoom;
        let pos = ui_point(&app, tool_at(Tool::Zoom, false));
        input(
            &mut app,
            &ctx,
            t + 0.1,
            vec![
                pointer_button(pos, Secondary, true),
                pointer_button(pos, Secondary, false),
            ],
        );
        assert_eq!(shown(&app), None);
        assert_eq!(app.tools.zoom, zoom - 1);
    }

    #[test]
    fn the_menu_strip_and_requesters_hide_it() {
        let (mut app, ctx) = started();
        let line = tool_at(Tool::Line, false);
        let t = rest_on(&mut app, &ctx, 1.0, line);
        // Right button on the title bar: the menu strip, no tooltip; Picture ▸ Page Size..
        let [title, item] = menu_item_path(&app, "picture.page_size");
        let title_pos = ui_point(&app, title);
        input(
            &mut app,
            &ctx,
            t + 0.1,
            vec![
                egui::Event::PointerMoved(title_pos),
                pointer_button(title_pos, Secondary, true),
            ],
        );
        assert!(app.ui.menu.active);
        assert_eq!(shown(&app), None);
        move_to(&mut app, &ctx, t + 0.2, item);
        let item_pos = ui_point(&app, item);
        input(
            &mut app,
            &ctx,
            t + 0.3,
            vec![pointer_button(item_pos, Secondary, false)],
        );
        assert!(app.ui.page_size_requester.is_some());
        // Under the requester, resting on the button shows nothing and waits for nothing.
        let delay = move_to(&mut app, &ctx, t + 0.4, line);
        assert_eq!(delay, std::time::Duration::MAX);
        input(&mut app, &ctx, t + 2.0, vec![]);
        assert_eq!(shown(&app), None, "never with a requester");
        input(&mut app, &ctx, t + 2.1, tap(egui::Key::Escape));
        assert!(app.ui.page_size_requester.is_none());
        let delay = input(&mut app, &ctx, t + 2.2, vec![]);
        assert_eq!(shown(&app), None, "the delay starts again");
        input(&mut app, &ctx, t + 2.2 + delay.as_secs_f64(), vec![]);
        assert_eq!(shown(&app), Some("Straight Line  v"));
        // A key that opens a requester while the tooltip shows: it goes in the same pass, so
        // it is never drawn over the requester.
        input(
            &mut app,
            &ctx,
            t + 3.0,
            chord(egui::Key::F, egui::Key::F, egui::Modifiers::SHIFT, "F"),
        );
        assert!(app.ui.tool_requester.is_some(), "the Fill Type requester");
        assert_eq!(shown(&app), None);
        input(&mut app, &ctx, t + 4.0, vec![]);
        assert_eq!(shown(&app), None);
    }

    #[test]
    fn playback_started_by_a_key_hides_a_panel_tooltip() {
        let (mut app, ctx) = started();
        app.engine
            .execute(Command::SetFrameCount { count: 4 })
            .unwrap();
        app.anim_control.panel = true;
        app.dirty = true;
        input(&mut app, &ctx, 0.1, vec![]);
        let t = rest_on(&mut app, &ctx, 1.0, panel_at(PanelButton::Next, true));
        assert_eq!(shown(&app), Some("Next Frame  2"));
        input(
            &mut app,
            &ctx,
            t + 0.1,
            chord(egui::Key::Num4, egui::Key::Num4, egui::Modifiers::NONE, "4"),
        );
        assert!(app.anim.playing.is_some());
        assert_eq!(shown(&app), None, "not over the playing frames");
        input(&mut app, &ctx, t + 1.0, vec![]);
        assert_eq!(shown(&app), None);
    }

    #[test]
    fn a_hidden_toolbox_shows_no_tool_tooltips_but_the_panel_still_does() {
        let (mut app, ctx) = started();
        let line = tool_at(Tool::Line, false);
        let t = rest_on(&mut app, &ctx, 1.0, line);
        // F10 with the pointer resting on the button: gone in that pass.
        input(&mut app, &ctx, t + 0.1, tap(egui::Key::F10));
        assert!(!app.ui.toolbox_visible);
        assert_eq!(shown(&app), None);
        let delay = move_to(&mut app, &ctx, t + 0.2, (line.0 + 1, line.1));
        assert_eq!(delay, std::time::Duration::MAX, "the page is under it now");
        input(&mut app, &ctx, t + 2.0, vec![]);
        assert_eq!(shown(&app), None);
        // The panel spans the whole width without the toolbox; its buttons name themselves.
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        app.anim_control.panel = true;
        app.dirty = true;
        input(&mut app, &ctx, 5.0, vec![]);
        rest_on(&mut app, &ctx, 6.0, panel_at(PanelButton::Spare, false));
        assert_eq!(shown(&app), Some("Light Table: Spare  Ctrl+4"));
        // Alt-a switches the panel off under the resting pointer: gone.
        input(
            &mut app,
            &ctx,
            9.0,
            chord(egui::Key::A, egui::Key::A, egui::Modifiers::ALT, ""),
        );
        assert!(!app.anim_control.panel);
        assert_eq!(shown(&app), None);
    }

    #[test]
    fn panel_buttons_name_themselves_and_the_slider_does_not() {
        let (mut app, ctx) = started();
        app.engine
            .execute(Command::SetFrameCount { count: 4 })
            .unwrap();
        // Alt-a shows the panel.
        input(
            &mut app,
            &ctx,
            0.1,
            chord(egui::Key::A, egui::Key::A, egui::Modifiers::ALT, ""),
        );
        input(&mut app, &ctx, 0.2, vec![]);
        assert!(app.ui.anim_panel.is_some());
        let mut t = rest_on(&mut app, &ctx, 1.0, panel_at(PanelButton::First, true));
        // Across all buttons, one after the other: each at once.
        for b in [PanelButton::First, PanelButton::Last]
            .into_iter()
            .chain(CONTROLS)
            .chain(TOGGLES)
        {
            t += 0.05;
            move_to(&mut app, &ctx, t, panel_at(b, true));
            assert_eq!(
                shown(&app),
                Some(dpx_ui::tooltip::for_panel_button(b).as_str())
            );
        }
        // The box sits above the panel.
        let s = app.view().ui_screen();
        let layout = Layout::new(s.width, s.height, true);
        let r = app
            .ui
            .tooltip
            .as_ref()
            .unwrap()
            .layout(s.width, s.height, &app.ui)
            .unwrap();
        assert_eq!(r.y + r.h, layout.panel.y - 2);
        // The slider track and the gap between the button groups have none.
        let track = layout.track;
        move_to(
            &mut app,
            &ctx,
            t + 0.1,
            (track.x + track.w - 3, track.y + 4),
        );
        assert_eq!(shown(&app), None, "the track");
        let gap = layout.button(PanelButton::GoTo);
        move_to(&mut app, &ctx, t + 0.2, (gap.x + gap.w + 2, gap.y + 4));
        assert_eq!(shown(&app), None, "the gap");
        input(&mut app, &ctx, t + 2.0, vec![]);
        assert_eq!(shown(&app), None);
    }

    /// The events a key label of a tooltip stands for, as egui-winit sends them on a US
    /// layout: `Shift+`/`Ctrl+`/`Alt+` before a key; a letter as typed (capitals with Shift).
    fn key_events(label: &str) -> Vec<egui::Event> {
        use egui::Modifiers as M;
        const DIGITS: [egui::Key; 10] = [
            egui::Key::Num0,
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        let (modifiers, key) = match label.split_once('+') {
            Some(("Shift", k)) => (M::SHIFT, k),
            Some(("Ctrl", k)) => (M::CTRL, k),
            Some(("Alt", k)) => (M::ALT, k),
            _ => (M::NONE, label),
        };
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            panic!("one key in '{label}'");
        };
        let digit = c.to_digit(10).map(|d| DIGITS[d as usize]);
        match (modifiers, c) {
            // Shifted digits type other characters; the US ones.
            (M::SHIFT, _) => {
                let d = digit.expect("Shift+digit");
                let i = c.to_digit(10).unwrap() as usize - 1;
                chord(d, d, M::SHIFT, &"!@#$%"[i..=i])
            }
            (M::CTRL, _) => {
                let d = digit.expect("Ctrl+digit");
                chord(d, d, M::CTRL, "")
            }
            (M::ALT, '=') => chord(egui::Key::Equals, egui::Key::Equals, M::ALT, ""),
            (_, '/') => chord(egui::Key::Slash, egui::Key::Slash, M::NONE, "/"),
            // `>` is Shift-. on a US keyboard; egui has no key for it.
            (_, '>') => chord(egui::Key::Period, egui::Key::Period, M::SHIFT, ">"),
            (_, c) if c.is_ascii_digit() => {
                let d = digit.unwrap();
                chord(d, d, M::NONE, key)
            }
            (_, c) if c.is_ascii_alphabetic() => {
                let k = egui::Key::from_name(&c.to_ascii_uppercase().to_string())
                    .expect("a letter key");
                let m = if c.is_ascii_uppercase() {
                    M::SHIFT
                } else {
                    M::NONE
                };
                chord(k, k, m, key)
            }
            _ => panic!("no events for '{label}'"),
        }
    }

    /// What a button or its key may change.
    #[derive(Debug, PartialEq)]
    struct Effect {
        tool: Tool,
        filled: bool,
        grid: bool,
        symmetry: bool,
        magnify: bool,
        zoom: usize,
        /// The document's page (not the perspective or text preview).
        pixel: Option<u8>,
        can_undo: bool,
        can_redo: bool,
        frame: usize,
        count: usize,
        playing: Option<dpx_core::anim::PlayMode>,
        goto_requester: bool,
        note: bool,
        light: (bool, bool, bool, bool, bool),
        /// The frame of the area stencil (ADR-39).
        stencil_frame: Option<usize>,
        perspective: bool,
        typing: bool,
    }

    fn effect(app: &PainterApp) -> Effect {
        let doc = app.engine.document();
        let l = app.anim.light;
        Effect {
            tool: app.ui.tool,
            filled: app.ui.filled,
            grid: app.tools.grid,
            symmetry: app.tools.symmetry,
            magnify: app.tools.magnify.is_some(),
            zoom: app.tools.zoom,
            pixel: doc.page().get(0, 0),
            can_undo: app.engine.can_undo(),
            can_redo: app.engine.can_redo(),
            frame: doc.current_frame(),
            count: doc.frame_count(),
            playing: app.anim.playing.map(|p| p.mode),
            goto_requester: app.ui.anim_requester.is_some(),
            note: app.ui.note_requester.is_some(),
            light: (app.anim.light_on, l.two_back, l.previous, l.next, l.spare),
            stencil_frame: app.menu_data.palette_stencil.area.as_ref().map(|a| a.frame),
            perspective: app.perspective.active,
            typing: app.typing(),
        }
    }

    /// Airbrush (no key) selected, the page cleared to 3 (one undo step), BG 4.
    fn tool_app() -> (PainterApp, egui::Context) {
        let (mut app, ctx) = started();
        app.select_tool(Tool::Airbrush, false);
        app.run(Command::Clear { color: 3 });
        app.ui.bg = 4;
        app.dirty = true;
        input(&mut app, &ctx, 0.1, vec![]);
        (app, ctx)
    }

    /// Five frames at frame 3 with the panel on and an area stencil on frame 4;
    /// `last_goto`: the frame of the last Go To.
    fn panel_app(last_goto: Option<usize>) -> (PainterApp, egui::Context) {
        let (mut app, ctx) = started();
        app.engine
            .execute(Command::SetFrameCount { count: 5 })
            .unwrap();
        app.engine.execute(Command::GotoFrame { frame: 2 }).unwrap();
        app.anim_control.panel = true;
        app.anim_control.last_goto = last_goto;
        app.menu_data.palette_stencil.area = Some(area_on(&app, 3));
        app.dirty = true;
        input(&mut app, &ctx, 0.1, vec![]);
        (app, ctx)
    }

    /// An area stencil protecting page pixel (0, 0) of frame `frame`.
    fn area_on(app: &PainterApp, frame: usize) -> crate::palette_stencil::Area {
        let page = app.engine.document().page();
        let (w, h) = (page.width(), page.height());
        let mut mask = vec![false; (w * h) as usize];
        mask[0] = true;
        crate::palette_stencil::Area {
            stencil: dpx_core::stencil::AreaStencil::from_mask(w, h, mask).unwrap(),
            frame,
            on: true,
        }
    }

    /// Effect ▸ Perspective on with a 6×4 brush (Return, as in the window).
    fn in_perspective((mut app, ctx): (PainterApp, egui::Context)) -> (PainterApp, egui::Context) {
        let brush = dpx_core::BrushImage {
            width: 6,
            height: 4,
            pixels: (0..24u8).map(|i| 5 + i % 3).collect(),
            handle: (3, 2),
            transparent: Some(0),
        };
        app.custom_brush = Some(brush.clone());
        app.picked_brush = Some(brush);
        input(&mut app, &ctx, 0.2, tap(egui::Key::Enter));
        assert!(app.perspective.active);
        (app, ctx)
    }

    /// The page pixel where text is typed: left of the toolbox at the height of the Line
    /// button, under the place of its tooltip.
    fn text_at() -> (i32, i32) {
        let b = Screen::new(SCREEN.0, SCREEN.1).tool_button(Tool::Line);
        (b.x - 100, b.y + 1)
    }

    /// The Text tool typing: picked and a click on the page.
    fn typing((mut app, ctx): (PainterApp, egui::Context)) -> (PainterApp, egui::Context) {
        app.select_tool(Tool::Text, false);
        click_ui(&mut app, &ctx, 0.2, text_at(), Primary);
        input(&mut app, &ctx, 0.3, vec![]);
        assert!(app.typing());
        (app, ctx)
    }

    fn tool_targets() -> Vec<TooltipTarget> {
        TOOL_ROWS
            .iter()
            .flatten()
            .flat_map(|&tool| {
                [false, true]
                    .into_iter()
                    .filter(move |&f| !f || tool.is_split())
                    .map(move |filled| TooltipTarget::Tool { tool, filled })
            })
            .collect()
    }

    fn panel_targets() -> Vec<TooltipTarget> {
        [PanelButton::First, PanelButton::Last]
            .into_iter()
            .chain(CONTROLS)
            .chain(TOGGLES)
            .map(TooltipTarget::Panel)
            .collect()
    }

    fn target_at(target: TooltipTarget) -> (i32, i32) {
        match target {
            TooltipTarget::Tool { tool, filled } => tool_at(tool, filled),
            TooltipTarget::Panel(b) => panel_at(b, true),
            // ADR-44: the overlay's button (no key runs one, so no key check reaches it).
            TooltipTarget::Layer(b) => {
                let r = dpx_ui::layer_panel::Layout::overlay(SCREEN.0, SCREEN.1, true, true, false)
                    .button(b);
                (r.x + r.w / 2, r.y + r.h / 2)
            }
        }
    }

    /// For every target with a key, in the state `setup` makes (whose keys are `keys`): the
    /// pointer rests on the button until its tooltip shows, then the key is pressed (as
    /// egui-winit sends it); a second app in the same state gets a click on the button (half).
    /// A key the tooltip names does exactly what the click does; a key it leaves out does
    /// something else (the mode takes it). Returns how many keys were named and left out.
    fn check_keys(
        setup: impl Fn() -> (PainterApp, egui::Context),
        keys: KeyMode,
        targets: &[TooltipTarget],
    ) -> (usize, usize) {
        let (mut named, mut left_out) = (0, 0);
        for &target in targets {
            let Some(key) = target.key() else {
                continue;
            };
            let at = target_at(target);
            let (mut clicked, ctx) = setup();
            let before = effect(&clicked);
            click_ui(&mut clicked, &ctx, 1.0, at, Primary);
            input(&mut clicked, &ctx, 1.1, vec![]);
            let by_click = effect(&clicked);
            assert_ne!(by_click, before, "{target:?}: the click does something");
            let (mut keyed, ctx) = setup();
            assert_eq!(keyed.tooltip_keys(), keys);
            let t = rest_on(&mut keyed, &ctx, 1.0, at);
            let text = target.text_in(keys);
            assert_eq!(shown(&keyed), Some(text.as_str()), "{target:?}");
            input(&mut keyed, &ctx, t + 0.1, key_events(key));
            if keys.keeps(key) {
                assert!(text.ends_with(&format!("  {key}")), "'{text}'");
                assert_eq!(effect(&keyed), by_click, "'{text}' in {keys:?}");
                named += 1;
            } else {
                assert_eq!(text, target.name());
                assert_ne!(
                    effect(&keyed),
                    by_click,
                    "{key} in {keys:?}: taken by the mode"
                );
                left_out += 1;
            }
        }
        (named, left_out)
    }

    #[test]
    fn every_tool_key_a_tooltip_names_does_what_the_button_does() {
        let targets = tool_targets();
        assert_eq!(check_keys(tool_app, KeyMode::Paint, &targets), (20, 0));
    }

    #[test]
    fn every_panel_key_a_tooltip_names_does_what_the_button_does() {
        let targets = panel_targets();
        for last_goto in [None, Some(4)] {
            let setup = || panel_app(last_goto);
            assert_eq!(check_keys(setup, KeyMode::Paint, &targets), (16, 0));
        }
    }

    #[test]
    fn in_perspective_mode_the_tooltips_leave_out_the_keys_it_takes() {
        // The digits (with and without Shift) turn the brush and `>` moves the observer: Zoom
        // and the frame and playback buttons show no key; the letters, Ctrl- and Alt-keys stay.
        let tools = tool_targets();
        let setup = || in_perspective(tool_app());
        assert_eq!(check_keys(setup, KeyMode::Perspective, &tools), (19, 1));
        let panel = panel_targets();
        for last_goto in [None, Some(4)] {
            let setup = || in_perspective(panel_app(last_goto));
            assert_eq!(check_keys(setup, KeyMode::Perspective, &panel), (6, 10));
        }
    }

    #[test]
    fn while_typing_text_the_tooltips_show_no_keys() {
        // Every key types into the text.
        let tools = tool_targets();
        let setup = || typing(tool_app());
        assert_eq!(check_keys(setup, KeyMode::Typing, &tools), (0, 20));
        let panel = panel_targets();
        let setup = || typing(panel_app(Some(4)));
        assert_eq!(check_keys(setup, KeyMode::Typing, &panel), (0, 16));
    }

    #[test]
    fn the_text_follows_the_key_mode_in_the_pass_that_changes_it() {
        let (mut app, ctx) = tool_app();
        app.custom_brush = Some(dpx_core::BrushImage {
            width: 2,
            height: 2,
            pixels: vec![5; 4],
            handle: (1, 1),
            transparent: None,
        });
        app.picked_brush = app.custom_brush.clone();
        let t = rest_on(&mut app, &ctx, 1.0, tool_at(Tool::Zoom, false));
        assert_eq!(shown(&app), Some("Zoom  >"));
        // Return into perspective mode and out again, the pointer resting: in that same pass.
        ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::Enter));
        assert!(app.perspective.active);
        assert_eq!(shown(&app), Some("Zoom"));
        input(&mut app, &ctx, t + 0.2, vec![]);
        ui_pass(&mut app, &ctx, t + 0.3, tap(egui::Key::Enter));
        assert!(!app.perspective.active);
        assert_eq!(shown(&app), Some("Zoom  >"));
        // Esc ends the text: the key is back in that pass.
        let (mut app, ctx) = typing(tool_app());
        let t = rest_on(&mut app, &ctx, 1.0, tool_at(Tool::Line, false));
        assert_eq!(shown(&app), Some("Straight Line"));
        ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::Escape));
        assert!(!app.typing());
        assert_eq!(shown(&app), Some("Straight Line  v"));
    }

    #[test]
    fn the_text_cursor_lies_under_the_tooltip() {
        let (mut app, ctx) = tool_app();
        app.ui.fg = 5;
        let (mut app, ctx) = typing((app, ctx));
        let pens = dpx_ui::Pens::from_palette(app.palette());
        assert!(![pens.face, pens.dark].contains(&5));
        // The cursor: one column right of the typing point, the font's height.
        let (cx, cy) = (text_at().0 + 1, text_at().1);
        let cursor = |app: &PainterApp| {
            let screen = app.compose_screen();
            (cy..cy + 8).all(|y| screen.get(cx, y) == Some(5))
        };
        assert!(cursor(&app), "the cursor shows on the page");
        let line = tool_at(Tool::Line, false);
        let t = rest_on(&mut app, &ctx, 1.0, line);
        assert!(app.typing());
        let tip = app.ui.tooltip.clone().unwrap();
        let s = app.view().ui_screen();
        let r = tip.layout(s.width, s.height, &app.ui).unwrap();
        assert!(r.contains(cx, cy) && r.contains(cx, cy + 7), "{r:?}");
        // The box on the shown screen is exactly the box as drawn alone: no cursor in it.
        let screen = app.compose_screen();
        let mut alone = dpx_core::IndexedImage::new(s.width as u32, s.height as u32, 99);
        tip.draw(&mut alone, &pens, s.width, s.height, &app.ui);
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                assert_eq!(screen.get(x, y), alone.get(x, y), "({x}, {y})");
            }
        }
        // Back on the page, the cursor shows again.
        move_to(&mut app, &ctx, t + 0.1, (100, 120));
        assert_eq!(shown(&app), None);
        assert!(cursor(&app));
    }

    #[test]
    fn after_play_once_ends_by_itself_the_resting_pointer_waits_and_shows_it_again() {
        let (mut app, ctx) = panel_app(None);
        let at = panel_at(PanelButton::OnceForward, true);
        let t = rest_on(&mut app, &ctx, 1.0, at);
        assert_eq!(shown(&app), Some("Play Once  5"));
        let mut delay = click_ui(&mut app, &ctx, t + 0.1, at, Primary);
        assert!(app.anim.playing.is_some());
        // The idle window: a pass only when egui asks for one, the pointer on the button.
        let mut t = t + 0.12;
        let mut ended = None;
        while delay < std::time::Duration::from_secs(10) && t < 60.0 && shown(&app).is_none() {
            t += delay.as_secs_f64();
            delay = ui_pass(&mut app, &ctx, t, vec![]);
            if app.anim.playing.is_none() {
                ended.get_or_insert(t);
            }
        }
        let ended = ended.expect("Play Once ends by itself");
        assert_eq!(
            shown(&app),
            Some("Play Once  5"),
            "at {t}, ended at {ended}"
        );
        assert!(
            (t - ended - DELAY).abs() < 0.01,
            "the whole delay after the end: {ended} -> {t}"
        );
        assert_eq!(delay, std::time::Duration::MAX, "then nothing more");
    }

    #[test]
    fn the_delete_frame_button_takes_the_area_stencil_with_its_frames() {
        // The stencil on frame 4 (index 3) follows its picture to index 2 when frame 3 goes;
        // the stencil of the deleted frame goes with it (ADR-39).
        let (mut app, ctx) = panel_app(None);
        click_ui(
            &mut app,
            &ctx,
            1.0,
            panel_at(PanelButton::Delete, true),
            Primary,
        );
        assert_eq!(app.engine.document().frame_count(), 4);
        assert_eq!(effect(&app).stencil_frame, Some(2));
        let (mut app, ctx) = panel_app(None);
        app.menu_data.palette_stencil.area = Some(area_on(&app, 2));
        click_ui(
            &mut app,
            &ctx,
            1.0,
            panel_at(PanelButton::Delete, true),
            Primary,
        );
        assert_eq!(effect(&app).stencil_frame, None);
        // Add Frame (and Alt-=, checked above) inserts after the current frame.
        let (mut app, ctx) = panel_app(None);
        click_ui(
            &mut app,
            &ctx,
            1.0,
            panel_at(PanelButton::Add, true),
            Primary,
        );
        assert_eq!(app.engine.document().frame_count(), 6);
        assert_eq!(effect(&app).stencil_frame, Some(4));
    }

    #[test]
    fn hover_option_puts_the_tooltip_of_that_pixel_on_a_snapshot() {
        let args = |spec: &str| -> Vec<String> {
            ["deluxepainter", "--snapshot", "x.png", "--hover", spec]
                .map(String::from)
                .to_vec()
        };
        let screen = Screen::new(320, 256);
        let mut ui = UiState::default();
        let b = screen.tool_button(Tool::Rectangle);
        snapshot_hover(
            &args(&format!("{},{}", b.x + 9, b.y + 9)),
            32,
            &screen,
            &mut ui,
        )
        .unwrap();
        assert_eq!(
            ui.tooltip.as_ref().map(|t| t.text.as_str()),
            Some("Filled Rectangle  R")
        );
        snapshot_hover(&args("100,100"), 32, &screen, &mut ui).unwrap();
        assert_eq!(ui.tooltip, None, "the page");
        assert!(snapshot_hover(&args("1"), 32, &screen, &mut ui).is_err());
        // Not with the menu strip shown.
        ui.menu.active = true;
        snapshot_hover(
            &args(&format!("{},{}", b.x + 2, b.y + 2)),
            32,
            &screen,
            &mut ui,
        )
        .unwrap();
        assert_eq!(ui.tooltip, None);
    }
}
