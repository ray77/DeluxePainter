//! Anim control in the app (ADR-38): Anim ▸ Control ▸ Set Range.. and Panel On/Off (the
//! Animation Control Panel), Anim ▸ Method, Anim ▸ Move.. with its wire-frame preview, and
//! the keys Shift-1 … Shift-5, `M` and Alt-a (AGA p.252-258, keyboard table p.296).
//!
//! Pictures change only through commands: the panel's buttons run the frame commands of
//! ADR-28, Move ▸ Draw and Trails one `MoveBrush` command (one undo step for all its frames).
//! This module keeps what is not part of the document: the play range, whether the panel is
//! shown, the last Go To frame, where the brush was clicked down and where the last move
//! ended, the Move settings and the preview clock.

use std::time::Duration;

use dpx_core::anim::PlayMode;
use dpx_core::anim_move::{self, MAX_COUNT, MovePath, Plan};
use dpx_core::{BrushImage, Command, Document, IndexedImage};
use dpx_ui::anim_panel::Layout;
use dpx_ui::{
    AnimControlRequester, AnimPanel, MoveRequester, MoveResponse, PanelButton, PanelHit, PlayRange,
    PlayRangeRequester, PlayRangeResponse, ReqEvent, UiState,
};
use eframe::egui;

use crate::{PainterApp, View};

/// The Move requester's wire-frame preview while it plays (AGA p.255).
#[derive(Clone, Debug)]
pub struct Preview {
    plan: Plan,
    /// Size and handle of the brush shown.
    size: (u32, u32),
    handle: (i32, i32),
    /// When it started (egui time, seconds).
    since: f64,
    /// The stamp shown (painting order).
    step: usize,
    /// The requester, shown again when the preview ends.
    requester: MoveRequester,
}

/// App state of the animation controls.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// Set Range: the frames (from 0) Play once and Ping-pong play; `None` = every frame.
    pub range: Option<(usize, usize)>,
    /// The Control Panel is shown.
    pub panel: bool,
    /// The panel button held down (drawn pressed until the button goes up).
    pub held: Option<PanelButton>,
    /// The slider knob is dragged, grabbed this many UI pixels right of its left edge.
    pub knob: Option<i32>,
    /// The frame (from 0) the Go to Frame requester went to last: the panel's Go To button
    /// and Shift-3 go there again.
    pub last_goto: Option<usize>,
    /// Where the brush was last clicked down on the page; Go Back starts the next move here.
    pub clicked: Option<(i32, i32)>,
    /// Where the last move ended (page x, y and depth z); the next move starts there.
    pub moved_to: Option<(i32, i32, i32)>,
    /// The Move requester's settings after Exit, Draw or Trails.
    pub settings: Option<MovePath>,
    pub preview: Option<Preview>,
    /// Ctrl or Cmd held in this pass (Ctrl-click on Go To opens the requester).
    pub ctrl: bool,
    /// Time of this pass (egui time, seconds), for playbacks started from the panel.
    pub now: f64,
}

/// `--anim-panel` and `--anim-control range|move` for snapshots (ADR-38).
pub fn snapshot_ui(args: &[String], doc: &Document, ui: &mut UiState) -> Result<(), String> {
    if args.iter().any(|a| a == "--anim-panel") {
        // The Light Table toggles show `--light-table` (its images; on when it is given).
        let light_on = args.iter().position(|a| a == "--light-table");
        let spec = light_on
            .and_then(|i| args.get(i + 1))
            .filter(|v| !v.starts_with("--"));
        let light = crate::anim::parse_light_table(spec.map(String::as_str))?;
        ui.anim_panel = Some(AnimPanel {
            frame: doc.current_frame(),
            count: doc.frame_count(),
            light_on: light_on.is_some(),
            two_back: light.two_back,
            previous: light.previous,
            next: light.next,
            spare: light.spare,
            held: None,
        });
    }
    let Some(i) = args.iter().position(|a| a == "--anim-control") else {
        return Ok(());
    };
    let count = doc.frame_count();
    ui.anim_control = Some(match args.get(i + 1).map(String::as_str) {
        Some("range") => {
            AnimControlRequester::PlayRange(Box::new(PlayRangeRequester::new(count, None)))
        }
        Some("move") => {
            let path = MovePath {
                dist: [200, 0, 400],
                angle: [0, 0, 360],
                count: count.min(MAX_COUNT as usize) as u32,
                ..MovePath::default()
            };
            AnimControlRequester::Move(Box::new(MoveRequester::new(&path, count)))
        }
        _ => return Err("--anim-control expects range or move".into()),
    });
    Ok(())
}

impl PainterApp {
    /// The menu items of ADR-38.
    pub(crate) fn anim_control_action(&mut self, id: &str) {
        match id {
            "anim.set_range" => self.open_play_range(),
            "anim.panel" => self.anim_control.panel = !self.anim_control.panel,
            // Method only describes DeluxePaint's memory use: a check mark, nothing else.
            "anim.method_compressed" => self.ui.anim_expanded = false,
            "anim.method_expanded" => self.ui.anim_expanded = true,
            "anim.move" => self.open_move(),
            _ => {}
        }
        self.dirty = true;
    }

    fn open_play_range(&mut self) {
        self.finish_stroke();
        let count = self.engine.document().frame_count();
        let range = self.anim_control.range.map(|(a, b)| (a + 1, b + 1));
        let req = PlayRangeRequester::new(count, range);
        self.ui.anim_control = Some(AnimControlRequester::PlayRange(Box::new(req)));
    }

    fn open_move(&mut self) {
        self.finish_stroke();
        let frames = self.engine.document().frame_count();
        // Count starts at the number of frames (tutorial p.152).
        let path = self.anim_control.settings.unwrap_or(MovePath {
            count: frames.min(MAX_COUNT as usize) as u32,
            ..MovePath::default()
        });
        let req = MoveRequester::new(&path, frames);
        self.ui.anim_control = Some(AnimControlRequester::Move(Box::new(req)));
    }

    /// The frames (from 0) a playback in `mode` plays: the Set Range frames for the
    /// play-once modes and Ping-pong (AGA p.257), every frame for the loops. A range beyond
    /// the last frame is cut there; what is left of it must still hold two frames.
    pub(crate) fn play_frames(&self, mode: PlayMode) -> (usize, usize) {
        let last = self.engine.document().frame_count().max(1) - 1;
        let ranged = matches!(
            mode,
            PlayMode::Once | PlayMode::OnceBackward | PlayMode::PingPong
        );
        match self.anim_control.range {
            Some((a, b)) if ranged && a < b.min(last) => (a, b.min(last)),
            _ => (0, last),
        }
    }

    /// Notes where the brush was clicked down on the page: the next move starts there.
    pub(crate) fn note_click(&mut self, x: i32, y: i32) {
        self.anim_control.clicked = Some(self.snap((x, y)));
        self.anim_control.moved_to = None;
    }

    /// Where the next move starts: the end of the last move, else the last click, else the
    /// page centre.
    fn move_start(&self) -> (i32, i32, i32) {
        let page = self.engine.document().page();
        let centre = (page.width() as i32 / 2, page.height() as i32 / 2, 0);
        self.anim_control
            .moved_to
            .or(self.anim_control.clicked.map(|(x, y)| (x, y, 0)))
            .unwrap_or(centre)
    }

    /// The brush Move paints with (as a gesture would, ADR-18): the custom brush if its
    /// colours fit the palette, else the built-in brush in FG.
    fn move_brush(&self) -> (Option<BrushImage>, Vec<(i32, i32)>) {
        let colors = self.engine.document().palette().len();
        let fits = |c: u8| usize::from(c) < colors;
        let custom = self
            .custom_brush
            .clone()
            .filter(|b| b.pixels.iter().all(|&c| fits(c)) && b.transparent.is_none_or(fits));
        let builtin = dpx_ui::toolbox::BUILTIN_BRUSHES[self.ui.brush].footprint();
        match custom {
            Some(b) => (Some(b), Vec::new()),
            None => (None, builtin),
        }
    }

    /// The MoveBrush command of `path` with the current brush, mode, range, stencil and
    /// effects.
    fn move_command(&self, path: MovePath) -> Command {
        let colors = self.engine.document().palette().len();
        let fg = self.ui.fg;
        // The colour range that holds FG, for Shade and Cycle (as `tools::ink`).
        let range = self
            .extras
            .ranges
            .iter()
            .find(|r| r.low <= fg && fg <= r.high && usize::from(r.high) < colors)
            .map(|r| (r.low, r.high));
        let (custom, brush) = self.move_brush();
        Command::MoveBrush {
            start: self.move_start(),
            path,
            brush,
            color: fg,
            custom,
            mode: crate::tools::draw_mode(self.ui.mode),
            range,
            stencil: self.stencil.clone(),
            right: false,
            fx: self.menu_data.effects.fx(),
        }
    }

    /// Move ▸ Draw or Trails: one undoable command; the requester closes and keeps the
    /// settings, or stays open with the reason when the document refuses it.
    fn draw_move(&mut self, path: MovePath) {
        self.finish_stroke();
        self.anim_control.settings = Some(MovePath {
            trails: false,
            ..path
        });
        let doc = self.engine.document();
        let start = self.move_start();
        let end = anim_move::plan(&path, start, doc.frame_count(), doc.current_frame()).end;
        match self.engine.execute(self.move_command(path)) {
            Ok(_) => {
                self.ui.anim_control = None;
                self.anim_control.moved_to = Some(end.pixel());
            }
            Err(e) => {
                eprintln!("{e}");
                if let Some(AnimControlRequester::Move(req)) = &mut self.ui.anim_control {
                    req.set_message(e.message);
                }
            }
        }
        self.clamp_scroll();
        self.dirty = true;
    }

    /// Move ▸ Preview: the requester steps aside while the wire frame plays.
    fn start_preview(&mut self, path: MovePath, now: f64) {
        let Some(AnimControlRequester::Move(req)) = self.ui.anim_control.take() else {
            return;
        };
        let doc = self.engine.document();
        let plan = anim_move::plan(
            &path,
            self.move_start(),
            doc.frame_count(),
            doc.current_frame(),
        );
        let (custom, builtin) = self.move_brush();
        let shape = custom.or_else(|| anim_move::brush_of_offsets(&builtin, self.ui.fg));
        let (size, handle) = shape.map_or(((1, 1), (0, 0)), |b| ((b.width, b.height), b.handle));
        self.anim_control.preview = Some(Preview {
            plan,
            size,
            handle,
            since: now,
            step: 0,
            requester: *req,
        });
        self.dirty = true;
    }

    fn end_preview(&mut self) {
        if let Some(p) = self.anim_control.preview.take() {
            self.ui.anim_control = Some(AnimControlRequester::Move(Box::new(p.requester)));
        }
        self.dirty = true;
    }

    /// While the preview plays: one stamp per frame time at the animation's rate; Space, Esc
    /// or a click end it early (AGA p.255).
    fn preview_input(&mut self, ctx: &egui::Context) {
        let (now, stop) = ctx.input(|i| {
            let key = i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Escape);
            (i.time, key || i.pointer.any_pressed())
        });
        let fps = f64::from(self.engine.document().fps().max(1));
        let Some(p) = &mut self.anim_control.preview else {
            return;
        };
        let step = ((now - p.since).max(0.0) * fps).floor() as usize;
        if stop || step >= p.plan.stamps.len() {
            return self.end_preview();
        }
        if step != p.step {
            p.step = step;
            self.dirty = true;
        }
        ctx.request_repaint_after(Duration::from_secs_f64(0.5 / fps));
    }

    /// Once per pass, before the requesters and the paint screen: notes the modifiers and
    /// lets go of panel buttons; while Set Range, Move or the Move preview is open, it takes
    /// the pointer and the keys and returns `true` (the caller only paints).
    pub(crate) fn anim_control_pass(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) -> bool {
        let (now, ctrl, down) = ctx.input(|i| {
            let m = i.modifiers;
            (i.time, m.ctrl || m.mac_cmd, i.pointer.any_down())
        });
        self.anim_control.now = now;
        self.anim_control.ctrl = ctrl;
        if !down && (self.anim_control.held.is_some() || self.anim_control.knob.is_some()) {
            self.anim_control.held = None;
            self.anim_control.knob = None;
            self.dirty = true;
        }
        if self.anim_control.preview.is_some() {
            self.preview_input(ctx);
            return true;
        }
        let Some(req) = &self.ui.anim_control else {
            return false;
        };
        let ui_screen = view.ui_screen();
        let at = req.origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.anim_control.is_none() {
                break;
            }
            self.anim_control_event(event, now);
        }
        if self.anim_control.preview.is_some() {
            ctx.request_repaint();
        }
        true
    }

    /// Feeds one event to the open Set Range or Move requester.
    pub(crate) fn anim_control_event(&mut self, event: ReqEvent, now: f64) {
        self.dirty = true;
        match &mut self.ui.anim_control {
            Some(AnimControlRequester::PlayRange(r)) => match r.handle(event) {
                PlayRangeResponse::Continue => {}
                PlayRangeResponse::Cancel => self.ui.anim_control = None,
                PlayRangeResponse::Ok(range) => {
                    self.anim_control.range = match range {
                        PlayRange::All => None,
                        PlayRange::Frames { from, to } => {
                            Some((from as usize - 1, to as usize - 1))
                        }
                    };
                    self.ui.anim_control = None;
                }
            },
            Some(AnimControlRequester::Move(r)) => match r.handle(event) {
                MoveResponse::Continue => {}
                MoveResponse::Cancel => self.ui.anim_control = None,
                MoveResponse::Exit(path) => {
                    self.anim_control.settings = Some(path);
                    self.ui.anim_control = None;
                }
                MoveResponse::GoBack => self.anim_control.moved_to = None,
                MoveResponse::Preview(path) => self.start_preview(path, now),
                MoveResponse::Draw(path) => self.draw_move(path),
            },
            None => {}
        }
    }

    /// A frame command from the panel or a key (as `anim::frame_command`).
    fn control_command(&mut self, command: Command) {
        self.finish_stroke();
        if let Err(e) = self.engine.execute(command) {
            eprintln!("{e}");
        }
        self.clamp_scroll();
        self.dirty = true;
    }

    fn go_to_frame(&mut self, frame: usize) {
        let doc = self.engine.document();
        if frame < doc.frame_count() && frame != doc.current_frame() {
            self.control_command(Command::GotoFrame {
                frame: frame as u32,
            });
        }
    }

    /// Repeat last Go to (the Go To button, Shift-3, AGA p.258); before any Go To the Go to
    /// Frame requester opens.
    fn repeat_goto(&mut self) {
        let count = self.engine.document().frame_count();
        match self.anim_control.last_goto {
            Some(f) => self.go_to_frame(f.min(count - 1)),
            None => self.anim_action("anim.goto", self.anim_control.now),
        }
    }

    /// A press on the Control Panel (left or right button).
    pub(crate) fn panel_press(&mut self, hit: PanelHit, right: bool) {
        self.finish_stroke();
        self.dirty = true;
        match hit {
            PanelHit::Button(b) => {
                self.anim_control.held = Some(b);
                self.panel_button(b, right);
            }
            // A click beside the knob: one frame that way (AGA p.129 of the tutorial).
            PanelHit::Step(step) => {
                let doc = self.engine.document();
                let to = doc.current_frame() as i64 + i64::from(step);
                if (0..doc.frame_count() as i64).contains(&to) {
                    self.go_to_frame(to as usize);
                }
            }
            PanelHit::Knob(grab) => self.anim_control.knob = Some(grab),
            PanelHit::Background => {}
        }
    }

    fn panel_button(&mut self, b: PanelButton, right: bool) {
        let now = self.anim_control.now;
        let doc = self.engine.document();
        let (count, current) = (doc.frame_count(), doc.current_frame());
        match b {
            PanelButton::First => self.go_to_frame(0),
            PanelButton::Last => self.go_to_frame(count - 1),
            PanelButton::LoopBackward => self.start_playback(PlayMode::LoopBackward, now),
            PanelButton::OnceBackward => self.start_playback(PlayMode::OnceBackward, now),
            PanelButton::Previous => self.step_frame(-1),
            PanelButton::Next => self.step_frame(1),
            PanelButton::OnceForward => self.start_playback(PlayMode::Once, now),
            PanelButton::LoopForward => self.start_playback(PlayMode::Loop, now),
            PanelButton::PingPong => self.start_playback(PlayMode::PingPong, now),
            PanelButton::Add => self.control_command(Command::AddFrames { count: 1 }),
            // Undoable here, so no confirmation (DeluxePaint asks, tutorial p.130).
            PanelButton::Delete => self.control_command(Command::DeleteFrames {
                first: current as u32,
                last: current as u32,
                fill: self.ui.bg,
            }),
            // Ctrl-click (or the right button) opens the Go to Frame requester.
            PanelButton::GoTo if self.anim_control.ctrl || right => {
                self.anim_action("anim.goto", now)
            }
            PanelButton::GoTo => self.repeat_goto(),
            PanelButton::LightTable => self.anim_action("light_table.toggle", now),
            PanelButton::TwoBack => self.anim_action("light_table.two_back", now),
            PanelButton::LightPrevious => self.anim_action("light_table.previous", now),
            PanelButton::LightNext => self.anim_action("light_table.next", now),
            PanelButton::Spare => self.anim_action("light_table.spare", now),
        }
        if self.modal_open() {
            // A playback, requester or note took over; the button is up again when it ends.
            self.anim_control.held = None;
        }
    }

    /// The slider knob held with the pointer at UI x `ux`: the frame under it becomes current.
    pub(crate) fn drag_knob(&mut self, ux: i32) {
        let (Some(grab), Some(panel)) = (self.anim_control.knob, self.ui.anim_panel) else {
            return;
        };
        let screen = self.view().ui_screen();
        let layout = Layout::new(screen.width, screen.height, self.ui.toolbox_visible);
        let frame = panel.frame_for_knob(&layout, ux, grab);
        self.go_to_frame(frame);
    }

    /// Animation keys of ADR-38 on the paint screen (AGA p.296): Shift-1 / Shift-2 first /
    /// last frame, Shift-3 the last Go To frame, Shift-4 / Shift-5 play backward (loop /
    /// once), `M` the Move requester, Alt-a the Control Panel. The digits are read from the
    /// physical keys, as shifted digits type other characters on every layout.
    pub(crate) fn anim_control_keys(&mut self, ctx: &egui::Context) {
        if self.typing() || self.modal_open() {
            return;
        }
        let (now, events) = ctx.input(|i| (i.time, i.events.clone()));
        for event in events {
            let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat,
                modifiers: m,
            } = event
            else {
                continue;
            };
            let shift = m.shift && !(m.ctrl || m.alt || m.command || m.mac_cmd);
            let alt = m.alt && !(m.ctrl || m.shift || m.command || m.mac_cmd);
            let digit = physical_key.unwrap_or(key);
            let count = self.engine.document().frame_count();
            match digit {
                egui::Key::Num1 if shift => self.go_to_frame(0),
                egui::Key::Num2 if shift => self.go_to_frame(count - 1),
                _ if repeat => continue,
                egui::Key::Num3 if shift => self.repeat_goto(),
                egui::Key::Num4 if shift => self.start_playback(PlayMode::LoopBackward, now),
                egui::Key::Num5 if shift => self.start_playback(PlayMode::OnceBackward, now),
                _ if key == egui::Key::M && shift => self.open_move(),
                _ if (key == egui::Key::A || digit == egui::Key::A) && alt => {
                    self.anim_control_action("anim.panel")
                }
                _ => continue,
            }
            self.dirty = true;
            if self.modal_open() {
                // The rest of this pass's keys belong to the playback or the requester.
                ctx.request_repaint();
                break;
            }
        }
    }

    /// Before the screen is composed (after `prepare_anim_view`): the Control Panel's state
    /// and, while the Move preview plays, the frame of its stamp with the wire frame on it.
    pub(crate) fn sync_anim_control_view(&mut self) {
        let doc = self.engine.document();
        self.ui.anim_panel = self.anim_control.panel.then(|| AnimPanel {
            frame: self.anim.playing.map_or(doc.current_frame(), |p| p.shown),
            count: doc.frame_count(),
            light_on: self.anim.light_on,
            two_back: self.anim.light.two_back,
            previous: self.anim.light.previous,
            next: self.anim.light.next,
            spare: self.anim.light.spare,
            held: self.anim_control.held,
        });
        if let Some(p) = &self.anim_control.preview
            && let Some(stamp) = p.plan.stamps.get(p.step)
            && let Some(frame) = doc.frame(stamp.frame)
        {
            let mut page: IndexedImage = frame.clone();
            let centre = anim_move::centre_of(&page);
            if let Some(c) = anim_move::outline(p.size, p.handle, &stamp.pose, centre) {
                anim_move::draw_outline(&mut page, &c, self.ui.fg);
            }
            self.ui.frame_counter =
                (doc.frame_count() > 1).then_some((stamp.frame + 1, doc.frame_count()));
            self.anim.shown = Some(page);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_ui::{Action, Hit, ReqKey};

    fn frames(app: &mut PainterApp, n: u32) {
        app.engine
            .execute(Command::SetFrameCount { count: n })
            .unwrap();
    }

    fn key(app: &mut PainterApp, k: ReqKey) {
        app.anim_control_event(ReqEvent::Key(k), 0.0);
    }

    fn type_in(app: &mut PainterApp, text: &str) {
        for c in text.chars() {
            key(app, ReqKey::Char(c));
        }
    }

    #[test]
    fn set_range_limits_play_once_and_ping_pong_but_not_play() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        frames(&mut app, 10);
        app.run_action(Action::Command("anim.set_range"), &ctx);
        assert!(app.ui.anim_control.is_some());
        // "Frames": From 3 To 6.
        app.anim_control_event(ReqEvent::Press(10, 31), 0.0);
        app.anim_control_event(ReqEvent::Release, 0.0);
        key(&mut app, ReqKey::Backspace);
        type_in(&mut app, "3");
        key(&mut app, ReqKey::Tab);
        key(&mut app, ReqKey::Backspace);
        key(&mut app, ReqKey::Backspace);
        type_in(&mut app, "6");
        key(&mut app, ReqKey::Enter);
        assert!(app.ui.anim_control.is_none());
        assert_eq!(app.anim_control.range, Some((2, 5)));
        assert_eq!(app.play_frames(PlayMode::Once), (2, 5));
        assert_eq!(app.play_frames(PlayMode::PingPong), (2, 5));
        assert_eq!(app.play_frames(PlayMode::OnceBackward), (2, 5));
        assert_eq!(
            app.play_frames(PlayMode::Loop),
            (0, 9),
            "Play is not affected"
        );
        assert_eq!(app.play_frames(PlayMode::LoopBackward), (0, 9));
        // Play once runs 3..6 at 10 fps and stops on 6.
        app.engine
            .execute(Command::SetFrameRate { fps: 10 })
            .unwrap();
        app.start_playback(PlayMode::Once, 0.0);
        assert_eq!(app.anim.playing.unwrap().shown, 2);
        app.playback_tick(0.35);
        assert_eq!(app.anim.playing.unwrap().shown, 5);
        app.playback_tick(0.45);
        assert!(app.anim.playing.is_none());
        assert_eq!(app.engine.document().current_frame(), 5);
        // A range cut by fewer frames: what is left.
        app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
        app.engine
            .execute(Command::SetFrameCount { count: 4 })
            .unwrap();
        assert_eq!(app.play_frames(PlayMode::Once), (2, 3));
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        assert_eq!(
            app.play_frames(PlayMode::Once),
            (0, 2),
            "one frame left: all"
        );
        // All frames again.
        app.run_action(Action::Command("anim.set_range"), &ctx);
        app.anim_control_event(ReqEvent::Press(10, 19), 0.0);
        app.anim_control_event(ReqEvent::Release, 0.0);
        key(&mut app, ReqKey::Enter);
        assert_eq!(app.anim_control.range, None);
    }

    #[test]
    fn method_is_a_radio_pair_that_changes_nothing() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        let revision = app.engine.document().revision();
        let checked = |app: &PainterApp, id: &'static str| {
            Action::Command(id).checked(&app.ui.menu_context())
        };
        assert_eq!(
            checked(&app, "anim.method_compressed"),
            Some(true),
            "the default"
        );
        assert_eq!(checked(&app, "anim.method_expanded"), Some(false));
        app.run_action(Action::Command("anim.method_expanded"), &ctx);
        assert_eq!(checked(&app, "anim.method_compressed"), Some(false));
        assert_eq!(checked(&app, "anim.method_expanded"), Some(true));
        app.run_action(Action::Command("anim.method_compressed"), &ctx);
        assert!(!app.ui.anim_expanded);
        assert_eq!(app.engine.document().revision(), revision, "no command");
        assert!(!app.engine.can_undo());
    }

    fn move_draw(app: &mut PainterApp, ctx: &egui::Context, setup: impl FnOnce(&mut PainterApp)) {
        app.run_action(Action::Command("anim.move"), ctx);
        setup(app);
        key(app, ReqKey::Enter);
    }

    #[test]
    fn move_draws_the_brush_over_the_frames_from_the_click() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        frames(&mut app, 4);
        // Stamp the 1-pixel brush at (40, 30) in colour 5, then Move X 12 over 4 frames.
        app.ui.fg = 5;
        app.handle_press(Hit::Page { x: 40, y: 30 }, false);
        app.release();
        app.finish_stroke();
        assert_eq!(app.page().get(40, 30), Some(5));
        move_draw(&mut app, &ctx, |app| {
            // Count shows the frame count (4); Dist X 12.
            key(app, ReqKey::Backspace);
            type_in(app, "12");
        });
        assert!(app.ui.anim_control.is_none(), "Draw closes the requester");
        let doc = app.engine.document();
        for (f, x) in [40, 43, 46, 49].into_iter().enumerate() {
            assert_eq!(doc.frame(f).unwrap().get(x, 30), Some(5), "frame {}", f + 1);
        }
        assert_eq!(doc.current_frame(), 0, "cyclic: the frame after the last");
        assert_eq!(app.anim_control.moved_to, Some((52, 30, 0)));
        // One undo step takes the move away from every frame.
        app.undo();
        let doc = app.engine.document();
        assert_eq!(doc.frame(3).unwrap().get(49, 30), Some(0));
        assert_eq!(
            doc.frame(0).unwrap().get(40, 30),
            Some(5),
            "the stamp stays"
        );
        // The settings are kept; the next move continues from the end (no Go Back)…
        app.redo();
        move_draw(&mut app, &ctx, |_| {});
        assert_eq!(app.engine.document().frame(0).unwrap().get(52, 30), Some(5));
        // … unless Go Back.
        app.run_action(Action::Command("anim.move"), &ctx);
        let go_back = app.ui.anim_control.clone();
        let Some(AnimControlRequester::Move(r)) = go_back else {
            panic!("the Move requester");
        };
        assert_eq!(r.size(), (248, 142));
        // Go Back sits at (70, 47).
        app.anim_control_event(ReqEvent::Press(72, 49), 0.0);
        app.anim_control_event(ReqEvent::Release, 0.0);
        assert!(app.ui.anim_control.is_some(), "Go Back keeps it open");
        assert_eq!(app.anim_control.moved_to, None);
        key(&mut app, ReqKey::Escape);
        assert!(app.ui.anim_control.is_none());
    }

    #[test]
    fn a_refused_move_keeps_the_requester_with_the_reason() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        frames(&mut app, 20);
        app.custom_brush = Some(BrushImage::from_image(
            &IndexedImage::new(100, 100, 3),
            None,
        ));
        app.run_action(Action::Command("anim.move"), &ctx);
        // Dist Z -310 and Count 999 with Trails: far too much work for one command.
        key(&mut app, ReqKey::Tab);
        key(&mut app, ReqKey::Tab);
        key(&mut app, ReqKey::Backspace);
        type_in(&mut app, "-310");
        for _ in 0..6 {
            key(&mut app, ReqKey::Tab);
        }
        key(&mut app, ReqKey::Backspace);
        key(&mut app, ReqKey::Backspace);
        type_in(&mut app, "999");
        let revision = app.engine.document().revision();
        // Trails at (66, 112).
        app.anim_control_event(ReqEvent::Press(68, 114), 0.0);
        app.anim_control_event(ReqEvent::Release, 0.0);
        assert!(app.ui.anim_control.is_some(), "still open");
        assert_eq!(app.engine.document().revision(), revision);
        let Some(AnimControlRequester::Move(r)) = &app.ui.anim_control else {
            panic!("the Move requester");
        };
        let p = dpx_ui::Pens::from_palette(app.engine.document().palette());
        let mut quiet = r.as_ref().clone();
        quiet.set_message("");
        assert_ne!(r.render(&p), quiet.render(&p), "the reason is shown");
    }

    #[test]
    fn preview_plays_the_wire_frame_then_returns_to_the_requester() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        frames(&mut app, 3);
        app.engine
            .execute(Command::SetFrameRate { fps: 10 })
            .unwrap();
        app.ui.fg = 2;
        app.note_click(100, 100);
        app.run_action(Action::Command("anim.move"), &ctx);
        type_in(&mut app, "60");
        // Preview at (6, 112).
        app.anim_control_event(ReqEvent::Press(8, 114), 5.0);
        app.anim_control_event(ReqEvent::Release, 5.0);
        assert!(app.anim_control.preview.is_some());
        assert!(app.ui.anim_control.is_none(), "the requester steps aside");
        app.prepare_anim_view();
        let shown = app.anim.shown.take().expect("the preview frame");
        // Dist X 60 (typed after the 0 already there: 060) over 3 cyclic stamps: x 100.
        assert_eq!(
            shown.get(100, 100),
            Some(2),
            "wire frame of the 1-pixel brush"
        );
        assert_eq!(
            app.engine.document().page().get(100, 100),
            Some(0),
            "display only"
        );
        let p = app.anim_control.preview.as_mut().unwrap();
        p.since = 0.0;
        p.step = 2;
        app.prepare_anim_view();
        let shown = app.anim.shown.take().unwrap();
        assert_eq!(shown.get(140, 100), Some(2), "third stamp: 40 to the right");
        assert_eq!(app.ui.frame_counter, Some((3, 3)), "its frame");
    }

    #[test]
    fn the_panel_buttons_step_play_add_delete_and_toggle() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        frames(&mut app, 5);
        app.run_action(Action::Command("anim.panel"), &ctx);
        assert!(app.anim_control.panel);
        app.prepare_anim_view();
        assert!(app.ui.anim_panel.is_some());
        let press = |app: &mut PainterApp, b: PanelButton| {
            app.handle_press(Hit::AnimPanel(PanelHit::Button(b)), false);
        };
        press(&mut app, PanelButton::Last);
        assert_eq!(app.engine.document().current_frame(), 4);
        press(&mut app, PanelButton::Next);
        assert_eq!(app.engine.document().current_frame(), 0, "wraps");
        press(&mut app, PanelButton::Previous);
        press(&mut app, PanelButton::First);
        assert_eq!(app.engine.document().current_frame(), 0);
        app.handle_press(Hit::AnimPanel(PanelHit::Step(1)), false);
        assert_eq!(app.engine.document().current_frame(), 1);
        app.handle_press(Hit::AnimPanel(PanelHit::Step(-1)), false);
        app.handle_press(Hit::AnimPanel(PanelHit::Step(-1)), false);
        assert_eq!(
            app.engine.document().current_frame(),
            0,
            "the slider does not wrap"
        );
        press(&mut app, PanelButton::Add);
        assert_eq!(app.engine.document().frame_count(), 6);
        press(&mut app, PanelButton::Delete);
        assert_eq!(app.engine.document().frame_count(), 5);
        app.undo();
        assert_eq!(app.engine.document().frame_count(), 6, "undoable");
        type IsOn = fn(&PainterApp) -> bool;
        let toggles: [(PanelButton, IsOn); 5] = [
            (PanelButton::LightTable, |a: &PainterApp| a.anim.light_on),
            (PanelButton::TwoBack, |a: &PainterApp| a.anim.light.two_back),
            (PanelButton::LightPrevious, |a: &PainterApp| {
                !a.anim.light.previous
            }),
            (PanelButton::LightNext, |a: &PainterApp| a.anim.light.next),
            (PanelButton::Spare, |a: &PainterApp| a.anim.light.spare),
        ];
        for (b, on) in toggles {
            press(&mut app, b);
            assert!(on(&app), "{b:?}");
        }
        // Play buttons: each starts its playback, Space stops it.
        for (b, mode) in [
            (PanelButton::LoopBackward, PlayMode::LoopBackward),
            (PanelButton::OnceBackward, PlayMode::OnceBackward),
            (PanelButton::OnceForward, PlayMode::Once),
            (PanelButton::LoopForward, PlayMode::Loop),
            (PanelButton::PingPong, PlayMode::PingPong),
        ] {
            press(&mut app, b);
            assert_eq!(app.anim.playing.map(|p| p.mode), Some(mode), "{b:?}");
            app.stop_playback();
        }
        // Go To: the requester first (no Go To yet), then again to the frame it went to.
        app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
        press(&mut app, PanelButton::GoTo);
        assert!(app.ui.anim_requester.is_some());
        app.anim_requester_event(ReqEvent::Key(ReqKey::Backspace));
        app.anim_requester_event(ReqEvent::Key(ReqKey::Char('4')));
        app.anim_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert_eq!(app.engine.document().current_frame(), 3);
        press(&mut app, PanelButton::First);
        press(&mut app, PanelButton::GoTo);
        assert!(app.ui.anim_requester.is_none());
        assert_eq!(
            app.engine.document().current_frame(),
            3,
            "repeat last Go To"
        );
        // Ctrl-click opens the requester again.
        app.anim_control.ctrl = true;
        press(&mut app, PanelButton::GoTo);
        assert!(app.ui.anim_requester.is_some());
    }
}
