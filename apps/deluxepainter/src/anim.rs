//! Animation in the app (ADR-28): Anim ▸ Frames and Control, Effect ▸ LightTable, their keys,
//! playback and the Anim requesters.
//!
//! Frames change only through the document's frame commands (`SetFrameCount`, `AddFrames`,
//! `CopyFrame`, `InsertFrameCopies`, `DeleteFrames`, `GotoFrame`, `SetFrameRate`). This module
//! keeps what is not part of the document: whether the Light Table is on and which images it
//! shows, and the playback clock. Both only change what is shown, never pixels (except
//! LightTable ▸ Merge, an undoable `SetPage`).

use std::time::Duration;

use dpx_core::anim::{self, LightTable, PlayMode};
use dpx_core::{Command, Document, IndexedImage};
use dpx_ui::{AnimChoice, AnimReqResponse, AnimRequester, ReqEvent};
use eframe::egui;

use crate::{PainterApp, View};

/// Title of the note after Play on a single frame.
pub const NOTHING_TO_PLAY: &str = "Nothing to play";

/// A playback in progress (Play, Play once, Ping-pong).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
    pub mode: PlayMode,
    /// Step position at `since` (see [`anim::frame_in`]).
    pub start: u64,
    /// The frames played (from 0): Set Range for the play-once modes and Ping-pong (ADR-38).
    pub first: usize,
    pub last: usize,
    /// When the clock started (egui time, seconds).
    pub since: f64,
    /// The frame on screen.
    pub shown: usize,
}

/// App state of the animation features.
#[derive(Clone, Debug, Default)]
pub struct AnimState {
    /// Effect ▸ LightTable ▸ On/Off (`l`).
    pub light_on: bool,
    /// Which images the Light Table shows, and Dim.
    pub light: LightTable,
    /// The animation playing; while it plays only Esc, Space, a click and the arrows act.
    pub playing: Option<Playback>,
    /// The page shown instead of the document page while the screen is composed (Light
    /// Table or playback); `None` the rest of the time.
    pub shown: Option<IndexedImage>,
}

/// The Light Table picture of `doc` (ADR-28): `base` (the current frame as it shows,
/// perhaps with a stroke preview) with the chosen other frames and the `spare` page showing
/// through its `empty` (BG colour) pixels, nearest first, dimmed if Dim is on. The other
/// frames show as they look, all visible layers (ADR-44); `spare` is the spare page as it
/// shows. Display only.
pub fn light_table_page(
    doc: &Document,
    base: &IndexedImage,
    light: LightTable,
    spare: Option<&IndexedImage>,
    empty: u8,
    dim: bool,
) -> IndexedImage {
    let frames: Vec<_> = light
        .frames(doc.current_frame(), doc.frame_count())
        .into_iter()
        .filter_map(|f| doc.composite_frame(f))
        .collect();
    let mut under: Vec<&IndexedImage> = frames.iter().map(|f| f.as_ref()).collect();
    if light.spare
        && let Some(s) = spare
    {
        under.push(s);
    }
    let table = dim.then(|| anim::dim_table(doc.palette(), empty));
    anim::light_table(base, &under, empty, table.as_deref())
}

/// `--light-table [LIST]` for snapshots: `previous`, `next`, `two_back`, `spare`, `dim`
/// separated by commas; without a list the defaults (Previous, Dim).
pub fn parse_light_table(spec: Option<&str>) -> Result<LightTable, String> {
    let Some(spec) = spec else {
        return Ok(LightTable::default());
    };
    let mut lt = LightTable {
        two_back: false,
        previous: false,
        next: false,
        spare: false,
        dim: false,
    };
    for part in spec.split(',').map(str::trim) {
        match part {
            "previous" => lt.previous = true,
            "next" => lt.next = true,
            "two_back" => lt.two_back = true,
            "spare" => lt.spare = true,
            "dim" => lt.dim = true,
            other => return Err(format!("unknown light table image '{other}'")),
        }
    }
    Ok(lt)
}

/// `--anim-requester KIND` for snapshots: count, add, copy, delete, goto or rate.
pub fn snapshot_requester(kind: &str, doc: &Document) -> Option<AnimRequester> {
    let (count, current) = (doc.frame_count(), doc.current_frame() + 1);
    let page = doc.page();
    // ADR-44: every frame holds one picture per layer.
    let max = anim::max_frames_for_layers(page.width(), page.height(), doc.layer_count());
    Some(match kind {
        "count" => AnimRequester::frame_count(count, max),
        "add" => AnimRequester::add_frames(max.saturating_sub(count)),
        "copy" => AnimRequester::copy_frames(count, current),
        "delete" => AnimRequester::delete_frames(count, current),
        "goto" => AnimRequester::goto(count, current),
        "rate" => AnimRequester::rate(doc.fps()),
        _ => return None,
    })
}

impl PainterApp {
    /// The Anim menu's frame and control items and Effect ▸ LightTable.
    pub(crate) fn anim_action(&mut self, id: &str, now: f64) {
        // ADR-32: IFF ANIM files.
        match id {
            "anim.load" => {
                return self.open_file_requester(dpx_ui::file_requester::FileKind::LoadAnim);
            }
            "anim.save" => {
                return self.open_file_requester(dpx_ui::file_requester::FileKind::SaveAnim);
            }
            _ => {}
        }
        let doc = self.engine.document();
        let (count, current) = (doc.frame_count(), doc.current_frame());
        // ADR-44: every frame holds one picture per layer.
        let page = doc.page();
        let max = anim::max_frames_for_layers(page.width(), page.height(), doc.layer_count());
        let open = |req: AnimRequester| Some(req);
        let requester = match id {
            "anim.set_frame_count" => open(AnimRequester::frame_count(count, max)),
            "anim.add_frames" => open(AnimRequester::add_frames(max.saturating_sub(count))),
            "anim.copy_frames" => open(AnimRequester::copy_frames(count, current + 1)),
            "anim.delete_frames" => open(AnimRequester::delete_frames(count, current + 1)),
            "anim.goto" => open(AnimRequester::goto(count, current + 1)),
            "anim.set_rate" => open(AnimRequester::rate(doc.fps())),
            _ => None,
        };
        if let Some(req) = requester {
            self.finish_stroke();
            self.ui.anim_requester = Some(req);
            self.dirty = true;
            return;
        }
        match id {
            "anim.previous" => self.step_frame(-1),
            "anim.next" => self.step_frame(1),
            "anim.play" => self.start_playback(PlayMode::Loop, now),
            "anim.play_once" => self.start_playback(PlayMode::Once, now),
            "anim.ping_pong" => self.start_playback(PlayMode::PingPong, now),
            "light_table.toggle" => self.anim.light_on = !self.anim.light_on,
            "light_table.merge" => self.merge_light_table(),
            "light_table.dim" => self.anim.light.dim = !self.anim.light.dim,
            "light_table.two_back" => self.anim.light.two_back = !self.anim.light.two_back,
            "light_table.previous" => self.anim.light.previous = !self.anim.light.previous,
            "light_table.next" => self.anim.light.next = !self.anim.light.next,
            "light_table.spare" => self.anim.light.spare = !self.anim.light.spare,
            _ => {}
        }
        self.dirty = true;
    }

    /// Runs a frame command (the menus, the keys and the Control Panel all come here);
    /// returns the engine's message if it refused.
    pub(crate) fn frame_command(&mut self, command: Command) -> Result<(), String> {
        self.finish_stroke();
        // ADR-39: the area stencil follows its picture.
        let result = self.execute_frames(command).map(|_| ()).map_err(|e| {
            eprintln!("{e}");
            e.message
        });
        self.clamp_scroll();
        self.dirty = true;
        result
    }

    /// Previous (`1`) / Next (`2`): one frame back or on, wrapping (AGA p.255).
    pub(crate) fn step_frame(&mut self, step: i64) {
        let doc = self.engine.document();
        let count = doc.frame_count() as i64;
        if count < 2 {
            return;
        }
        let to = (doc.current_frame() as i64 + step).rem_euclid(count);
        let _ = self.frame_command(Command::GotoFrame { frame: to as u32 });
    }

    /// Play (`4`), Play once (`5`), Ping-pong (`6`): Loop and Ping-pong start at the current
    /// frame, Play once at the first. With a single frame a note says how to add frames (the
    /// manual is silent; owner's choice, ADR-35).
    pub(crate) fn start_playback(&mut self, mode: PlayMode, now: f64) {
        self.finish_stroke();
        let doc = self.engine.document();
        if doc.frame_count() < 2 {
            let note = ["only one frame: Anim ▸ Frames ▸ Set # adds frames".to_string()];
            self.ui.note_requester = crate::note_requester(NOTHING_TO_PLAY, &note);
            self.dirty = true;
            return;
        }
        // ADR-38: the play range and the backward modes.
        let (first, last) = self.play_frames(mode);
        let start = anim::start_position(mode, first, last, doc.current_frame());
        self.anim.playing = Some(Playback {
            mode,
            start,
            first,
            last,
            since: now,
            shown: anim::frame_in(mode, first, last, start).0,
        });
        self.dirty = true;
    }

    /// Stops playing; the frame on screen becomes the current frame.
    pub(crate) fn stop_playback(&mut self) {
        if let Some(p) = self.anim.playing.take()
            && p.shown != self.engine.document().current_frame()
        {
            let _ = self.frame_command(Command::GotoFrame {
                frame: p.shown as u32,
            });
        }
        self.dirty = true;
    }

    /// Advances playback to time `now`; stops after Play once.
    pub(crate) fn playback_tick(&mut self, now: f64) {
        let Some(p) = self.anim.playing else {
            return;
        };
        let doc = self.engine.document();
        let position = anim::play_position(p.start, doc.fps(), now - p.since);
        let last = p.last.min(doc.frame_count() - 1);
        let (frame, done) = anim::frame_in(p.mode, p.first.min(last), last, position);
        if frame != p.shown {
            self.dirty = true;
        }
        if let Some(playing) = &mut self.anim.playing {
            playing.shown = frame;
        }
        if done {
            self.stop_playback();
        }
    }

    /// Left / Right while playing: one frame per second slower / faster (AGA p.296). The
    /// rate is the document's (a setting, no undo step); the clock restarts where it is.
    fn change_play_rate(&mut self, delta: i32, now: f64) {
        let Some(p) = self.anim.playing else {
            return;
        };
        let doc = self.engine.document();
        let fps = (i32::from(doc.fps()) + delta)
            .clamp(i32::from(anim::MIN_FPS), i32::from(anim::MAX_FPS)) as u8;
        let position = anim::play_position(p.start, doc.fps(), now - p.since);
        if self.engine.execute(Command::SetFrameRate { fps }).is_ok() {
            self.anim.playing = Some(Playback {
                start: position,
                since: now,
                ..p
            });
        }
    }

    /// Input while an animation plays: Esc, Space or a mouse button stop it, Left/Right
    /// change the rate; the screen is redrawn for every new frame.
    pub(crate) fn playback_input(&mut self, ctx: &egui::Context) {
        let (now, events, clicked) =
            ctx.input(|i| (i.time, i.events.clone(), i.pointer.any_pressed()));
        if clicked {
            return self.stop_playback();
        }
        for event in events {
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
            {
                match key {
                    egui::Key::Escape | egui::Key::Space => return self.stop_playback(),
                    // Colour cycling plays along (AGA p.194-196); Tab switches it here too.
                    egui::Key::Tab if !repeat && !modifiers.any() => {
                        self.toggle_cycling(now);
                        ctx.request_repaint();
                    }
                    egui::Key::ArrowLeft => self.change_play_rate(-1, now),
                    egui::Key::ArrowRight => self.change_play_rate(1, now),
                    _ => {}
                }
            }
        }
        self.playback_tick(now);
        if self.anim.playing.is_some() {
            let fps = f64::from(self.engine.document().fps().max(1));
            ctx.request_repaint_after(Duration::from_secs_f64(0.5 / fps));
        }
    }

    /// Animation keys on the paint screen (manual keyboard table, AGA p.296): `1` / `2`
    /// previous / next frame, `3` Go to Frame, `4` Play, `5` Play once, `6` Ping-pong, `l`
    /// LightTable on/off, Alt+`l` Dim, Ctrl+`1`…`4` LightTable Two Back / Previous / Next /
    /// Spare, Alt+`=` add one frame.
    pub(crate) fn anim_keys(&mut self, ctx: &egui::Context) {
        if self.typing() {
            return;
        }
        let (now, events) = ctx.input(|i| (i.time, i.events.clone()));
        for event in events {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            let plain = !modifiers.any();
            let ctrl = modifiers.ctrl && !(modifiers.alt || modifiers.shift || modifiers.mac_cmd);
            let alt = modifiers.alt && !(modifiers.ctrl || modifiers.shift || modifiers.command);
            let id = match key {
                // Holding 1 or 2 keeps stepping.
                egui::Key::Num1 if plain => "anim.previous",
                egui::Key::Num2 if plain => "anim.next",
                _ if repeat => continue,
                egui::Key::Num3 if plain => "anim.goto",
                egui::Key::Num4 if plain => "anim.play",
                egui::Key::Num5 if plain => "anim.play_once",
                egui::Key::Num6 if plain => "anim.ping_pong",
                egui::Key::L if plain => "light_table.toggle",
                egui::Key::L if alt => "light_table.dim",
                egui::Key::Num1 if ctrl => "light_table.two_back",
                egui::Key::Num2 if ctrl => "light_table.previous",
                egui::Key::Num3 if ctrl => "light_table.next",
                egui::Key::Num4 if ctrl => "light_table.spare",
                egui::Key::Equals | egui::Key::Plus if alt => {
                    let _ = self.frame_command(Command::AddFrames { count: 1 });
                    continue;
                }
                _ => continue,
            };
            self.anim_action(id, now);
            if self.anim.playing.is_some()
                || self.ui.anim_requester.is_some()
                || self.ui.note_requester.is_some()
            {
                // The rest of this frame's keys belong to the playback or the requester; the
                // playback needs the next frame at once.
                ctx.request_repaint();
                break;
            }
        }
    }

    /// LightTable ▸ Merge: the images shown (not dimmed) become part of the current frame,
    /// and the Light Table switches off. DeluxePaint cannot undo this; here it is one undo
    /// step (`SetPage`). ADR-44: the picture is worked out on the frame as it shows (as the
    /// Light Table showed it), and only the pixels the other images fill in go into the
    /// active layer; the rest of it stays as it is.
    fn merge_light_table(&mut self) {
        self.finish_stroke();
        if !self.anim.light_on {
            return;
        }
        let doc = self.engine.document();
        let spare = self.light_table_spare();
        let spare = spare.as_deref();
        let shown = doc.composite();
        let table = light_table_page(doc, &shown, self.anim.light, spare, self.ui.bg, false);
        let merged = dpx_core::layer::with_changes(doc.page(), &shown, &table);
        self.anim.light_on = false;
        if &merged != doc.page() {
            let _ = self.frame_command(Command::SetPage {
                width: merged.width(),
                height: merged.height(),
                pixels: merged.pixels().to_vec(),
            });
        }
        self.dirty = true;
    }

    /// Before the screen is composed: the frame counter in the title bar, and the page to
    /// show instead of the document page (a playing frame or the Light Table picture).
    pub(crate) fn prepare_anim_view(&mut self) {
        self.anim.shown = None;
        let doc = self.engine.document();
        let count = doc.frame_count();
        let shown_frame = self.anim.playing.map_or(doc.current_frame(), |p| p.shown);
        self.ui.frame_counter = (count > 1).then_some((shown_frame + 1, count));
        if self.anim.playing.is_some() {
            // ADR-44: the frame as it shows.
            self.anim.shown = doc.composite_frame(shown_frame).map(|f| f.into_owned());
        } else if self.anim.light_on && (count > 1 || self.anim.light.spare) {
            let spare = self.light_table_spare();
            let spare = spare.as_deref();
            let page = light_table_page(
                doc,
                &self.page(),
                self.anim.light,
                spare,
                self.ui.bg,
                self.anim.light.dim,
            );
            self.anim.shown = Some(page);
        }
        // ADR-38: the Control Panel and the Move preview.
        self.sync_anim_control_view();
    }

    /// Pointer and keys while an Anim requester is open (it is modal).
    pub(crate) fn anim_requester_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let Some(req) = &self.ui.anim_requester else {
            return;
        };
        let ui_screen = view.ui_screen();
        let at = req.origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.anim_requester.is_none() {
                break;
            }
            self.anim_requester_event(event);
        }
    }

    /// Feeds one event to the Anim requester; Ok runs the chosen frame command (the
    /// requester stays open with the reason if the document refuses it).
    pub(crate) fn anim_requester_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.anim_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        let choice = match response {
            AnimReqResponse::Continue => return,
            AnimReqResponse::Cancel => {
                self.ui.anim_requester = None;
                return;
            }
            AnimReqResponse::Ok(choice) => choice,
        };
        let doc = self.engine.document();
        let (count, current) = (doc.frame_count() as u32, doc.current_frame() as u32);
        let from1 = |n: u32| n.saturating_sub(1);
        let fill = self.ui.bg;
        let command = match choice {
            AnimChoice::FrameCount(n) => Command::SetFrameCount { count: n },
            AnimChoice::AddFrames(n) => Command::AddFrames { count: n },
            AnimChoice::CopyToAll => Command::CopyFrame {
                first: 0,
                last: count - 1,
            },
            AnimChoice::CopyToRange { first, last } => Command::CopyFrame {
                first: from1(first),
                last: from1(last),
            },
            AnimChoice::InsertCopies {
                first,
                last,
                before,
            } => Command::InsertFrameCopies {
                first: from1(first),
                last: from1(last),
                before: from1(before),
            },
            AnimChoice::DeleteCurrent => Command::DeleteFrames {
                first: current,
                last: current,
                fill,
            },
            AnimChoice::DeleteRange { first, last } => Command::DeleteFrames {
                first: from1(first),
                last: from1(last),
                fill,
            },
            AnimChoice::DeleteAll => Command::DeleteFrames {
                first: 0,
                last: count - 1,
                fill,
            },
            AnimChoice::Goto(n) => {
                // ADR-38: the Control Panel's Go To and Shift-3 go here again.
                self.anim_control.last_goto = Some(from1(n) as usize);
                Command::GotoFrame { frame: from1(n) }
            }
            AnimChoice::Rate(fps) => Command::SetFrameRate { fps },
        };
        match self.frame_command(command) {
            Ok(()) => self.ui.anim_requester = None,
            Err(message) => {
                if let Some(req) = &mut self.ui.anim_requester {
                    req.set_message(message);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_ui::{Action, Hit, ReqKey};

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    /// Runs one egui pass with `events` through the animation keys. The keys are released at
    /// the end of the pass (egui marks a key pressed again without release as a repeat).
    fn keys(app: &mut PainterApp, ctx: &egui::Context, mut events: Vec<egui::Event>) {
        let released: Vec<egui::Event> = events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, modifiers, .. } => Some(egui::Event::Key {
                    key: *key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: *modifiers,
                }),
                _ => None,
            })
            .collect();
        events.extend(released);
        let raw = egui::RawInput {
            events,
            ..Default::default()
        };
        ctx.begin_pass(raw);
        app.anim_keys(ctx);
        ctx.end_pass().textures_delta.clear();
    }

    /// What the screen shows instead of the document page (as while composing), if anything.
    fn view(app: &mut PainterApp) -> Option<IndexedImage> {
        app.prepare_anim_view();
        app.anim.shown.take()
    }

    fn type_in(app: &mut PainterApp, text: &str) {
        for c in text.chars() {
            app.anim_requester_event(ReqEvent::Key(ReqKey::Char(c)));
        }
    }

    fn backspace(app: &mut PainterApp, n: usize) {
        for _ in 0..n {
            app.anim_requester_event(ReqEvent::Key(ReqKey::Backspace));
        }
    }

    /// Paints one pixel at (`x`, 0) in `color` on the current frame.
    fn dot(app: &mut PainterApp, x: i32, color: u8) {
        app.ui.fg = color;
        app.handle_press(Hit::Page { x, y: 0 }, false);
        app.finish_stroke();
    }

    #[test]
    fn set_frames_from_the_menu_step_with_keys_and_undo() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run_action(Action::Command("anim.set_frame_count"), &ctx);
        assert!(app.ui.anim_requester.is_some());
        backspace(&mut app, 1);
        type_in(&mut app, "3");
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.anim_requester.is_none());
        assert_eq!(app.engine.document().frame_count(), 3);
        // The title bar counts the frames.
        let _ = view(&mut app);
        assert_eq!(app.ui.frame_counter, Some((1, 3)));

        // `2` next, `1` previous (wrapping); painting goes to the current frame.
        keys(
            &mut app,
            &ctx,
            vec![key_event(egui::Key::Num2, egui::Modifiers::NONE)],
        );
        assert_eq!(app.engine.document().current_frame(), 1);
        dot(&mut app, 4, 7);
        keys(
            &mut app,
            &ctx,
            vec![key_event(egui::Key::Num1, egui::Modifiers::NONE)],
        );
        keys(
            &mut app,
            &ctx,
            vec![key_event(egui::Key::Num1, egui::Modifiers::NONE)],
        );
        assert_eq!(
            app.engine.document().current_frame(),
            2,
            "wraps to the last"
        );
        let doc = app.engine.document();
        assert_eq!(doc.frame(1).unwrap().get(4, 0), Some(7));
        assert_eq!(doc.frame(0).unwrap().get(4, 0), Some(0));
        // Undo goes back to frame 2 and takes the dot away.
        app.undo();
        assert_eq!(app.engine.document().current_frame(), 1);
        assert_eq!(app.page().get(4, 0), Some(0));
        app.undo();
        assert_eq!(app.engine.document().frame_count(), 1);
        let _ = view(&mut app);
        assert_eq!(app.ui.frame_counter, None, "one frame: the program name");
    }

    #[test]
    fn requesters_add_copy_delete_goto_and_rate() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        dot(&mut app, 0, 5);
        // Add Frames: two copies after the current frame; the last copy is current.
        app.run_action(Action::Command("anim.add_frames"), &ctx);
        backspace(&mut app, 1);
        type_in(&mut app, "2");
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        let doc = app.engine.document();
        assert_eq!((doc.frame_count(), doc.current_frame()), (3, 2));
        assert!(doc.frames().all(|f| f.get(0, 0) == Some(5)));
        // Paint frame 3, then Copy Frames (All) copies it everywhere.
        dot(&mut app, 1, 9);
        app.run_action(Action::Command("anim.copy_frames"), &ctx);
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert!(
            app.engine
                .document()
                .frames()
                .all(|f| f.get(1, 0) == Some(9))
        );
        // Go to Frame 1.
        app.run_action(Action::Command("anim.goto"), &ctx);
        backspace(&mut app, 1);
        type_in(&mut app, "1");
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert_eq!(app.engine.document().current_frame(), 0);
        // Delete Frames (current): the next frame becomes current.
        app.run_action(Action::Command("anim.delete_frames"), &ctx);
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert_eq!(app.engine.document().frame_count(), 2);
        // Set Rate.
        app.run_action(Action::Command("anim.set_rate"), &ctx);
        backspace(&mut app, 2);
        type_in(&mut app, "12");
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert_eq!(app.engine.document().fps(), 12);
        // The largest frame count of a 320x256 page, and back.
        app.run_action(Action::Command("anim.set_frame_count"), &ctx);
        backspace(&mut app, 3);
        type_in(&mut app, "999");
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.anim_requester.is_none(), "999 frames of 320x256 fit");
        app.undo();
        assert_eq!(app.engine.document().frame_count(), 2);
        app.run_action(Action::Command("anim.goto"), &ctx);
        app.anim_requester_event(ReqEvent::Key(ReqKey::Escape));
        assert!(app.ui.anim_requester.is_none());
    }

    #[test]
    fn playback_only_redraws_and_stops_on_the_shown_frame() {
        let mut app = PainterApp::new();
        app.engine
            .execute(Command::SetFrameCount { count: 4 })
            .unwrap();
        app.engine
            .execute(Command::SetFrameRate { fps: 10 })
            .unwrap();
        let revision = app.engine.document().revision();
        app.start_playback(PlayMode::Loop, 100.0);
        app.playback_tick(100.25);
        assert_eq!(app.anim.playing.unwrap().shown, 2);
        let _ = view(&mut app);
        assert_eq!(app.ui.frame_counter, Some((3, 4)));
        assert_eq!(
            app.engine.document().revision(),
            revision,
            "no command while playing"
        );
        app.playback_tick(100.55);
        assert_eq!(app.anim.playing.unwrap().shown, 1, "loops");
        app.stop_playback();
        assert_eq!(app.engine.document().current_frame(), 1);
        // Play once from the first frame stops on the last.
        app.start_playback(PlayMode::Once, 0.0);
        app.playback_tick(0.35);
        assert!(app.anim.playing.is_some());
        app.playback_tick(0.4);
        assert!(app.anim.playing.is_none());
        assert_eq!(app.engine.document().current_frame(), 3);
        // One frame: nothing plays.
        let mut single = PainterApp::new();
        single.start_playback(PlayMode::PingPong, 0.0);
        assert!(single.anim.playing.is_none());
    }

    #[test]
    fn light_table_shows_the_previous_frame_dimmed_and_merge_is_undoable() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        dot(&mut app, 3, 3); // white in the default palette
        app.engine.execute(Command::AddFrames { count: 1 }).unwrap();
        app.engine.execute(Command::Clear { color: 0 }).unwrap();
        dot(&mut app, 5, 4);
        keys(
            &mut app,
            &ctx,
            vec![key_event(egui::Key::L, egui::Modifiers::NONE)],
        );
        assert!(app.anim.light_on);
        let shown = view(&mut app).expect("light table picture");
        let dim = anim::dim_table(app.engine.document().palette(), 0)[3];
        assert_eq!(shown.get(3, 0), Some(dim), "previous frame dimmed");
        assert_ne!(dim, 3);
        assert_eq!(shown.get(5, 0), Some(4), "current frame on top");
        assert_eq!(
            app.engine.document().page().get(3, 0),
            Some(0),
            "display only"
        );
        // Alt+l: Dim off shows the own colours.
        let alt = egui::Modifiers {
            alt: true,
            ..Default::default()
        };
        keys(&mut app, &ctx, vec![key_event(egui::Key::L, alt)]);
        assert_eq!(view(&mut app).unwrap().get(3, 0), Some(3));
        // Merge puts the images into the frame, switches off and undoes in one step.
        app.run_action(Action::Command("light_table.merge"), &ctx);
        assert!(!app.anim.light_on);
        assert_eq!(app.page().get(3, 0), Some(3));
        app.undo();
        assert_eq!(app.page().get(3, 0), Some(0));
        // Ctrl+3 toggles Next.
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        keys(&mut app, &ctx, vec![key_event(egui::Key::Num3, ctrl)]);
        assert!(app.anim.light.next);
    }

    #[test]
    fn a_three_frame_project_saves_loads_and_exports_the_current_frame() {
        use crate::files;
        let mut app = PainterApp::new();
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        app.engine.execute(Command::GotoFrame { frame: 1 }).unwrap();
        app.engine
            .execute(Command::SetFrameRate { fps: 8 })
            .unwrap();
        dot(&mut app, 2, 9);
        // A drawer of this test's own (tests run in parallel).
        let dir = std::env::temp_dir().join(format!("dpx-anim-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let project = dir.join("walk.dpxproj");
        let doc = app.engine.document();
        let (bytes, notes) = files::encode_document(doc, &app.extras, &project).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        std::fs::write(&project, bytes).unwrap();
        let info = files::info(&project).unwrap();
        assert!(info.contains("dpxproj format 2"), "{info}");
        assert!(
            info.contains("3 frames (frame 2 current, 8 per second)"),
            "{info}"
        );
        let loaded = files::load(&project).unwrap();
        let (back, _, _) = files::open_picture(loaded.picture, dpx_core::Target::default());
        assert!(back.frames().eq(doc.frames()), "every frame");
        assert_eq!((back.current_frame(), back.fps()), (1, 8));
        // Export writes the current frame as a picture, with a note.
        let iff = dir.join("walk.iff");
        let (bytes, notes) = files::encode_document(doc, &app.extras, &iff).unwrap();
        assert!(
            notes.iter().any(|n| n.contains("frame 2 of 3 exported")),
            "{notes:?}"
        );
        std::fs::write(&iff, bytes).unwrap();
        let picture = files::load(&iff).unwrap().picture;
        assert_eq!(picture.indexed().unwrap().0, doc.page());
        // A page smaller than the screen grows in every frame.
        let small: Vec<IndexedImage> = (0..3).map(|i| IndexedImage::new(10, 10, i)).collect();
        let pal = dpx_core::Palette::dpaint_default_32();
        let doc = Document::from_frames(small, 2, 30, pal, dpx_core::Target::default()).unwrap();
        let (grown, note) = files::page_at_least_screen(doc);
        assert!(note.is_some());
        assert_eq!((grown.frame_count(), grown.current_frame()), (3, 2));
        assert!(
            grown
                .frames()
                .all(|f| (f.width(), f.height()) == (320, 256))
        );
        assert_eq!(grown.frame(1).unwrap().get(9, 9), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn light_table_spec_parsing() {
        assert_eq!(parse_light_table(None), Ok(LightTable::default()));
        let lt = parse_light_table(Some("previous,next,dim")).unwrap();
        assert!(lt.previous && lt.next && lt.dim && !lt.two_back && !lt.spare);
        assert!(parse_light_table(Some("sideways")).is_err());
    }
}
