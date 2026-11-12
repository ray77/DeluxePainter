//! Effect ▸ Perspective (ADR-37): the custom brush turned in three dimensions with the keypad
//! and seen towards the perspective centre (manual AGA p.126-143, p.278-281, keys p.295).
//!
//! This module keeps the state and wires the input; the projection is
//! [`dpx_core::perspective`]. A click stamps the projected brush as one `Stroke` command, and
//! FillScreen stamps the plane the same way, so both are undoable and the same for the CLI.
//!
//! Keys while perspective mode is on (egui reports the keypad as the main keys, so both act):
//! `7`/`8`, `4`/`5`, `1`/`2` turn about x, y, z by -1/+1 degree (Shift: by the Angle Step),
//! `9`/`6`/`3` reset that axis (Shift: fix it), `0` resets the angles (Shift: everything but
//! the centre), `-` FillScreen, `.` Center, `;`/`'` move along the fixed axis (`:`/`"`/`@`
//! farther; the keys where a US keyboard has `;` and `'` too, whatever they type), `<`/`>`
//! observer distance, `\` angles or position in the title bar, Ctrl fixes the y axis while
//! held, Enter (also outside the mode) switches perspective mode. `-`, `.`, `<` and `>` go by
//! the character typed (Shift-`.` is `>` on a US keyboard, `:` on a German one).

use dpx_core::perspective::{self, Axis3, Orientation, Placement, Vec3, Viewer};
use dpx_core::{BrushImage, Command, Engine, IndexedImage, Rect};
use dpx_ui::{
    Hit, PaintMode, PerspectiveRequester, PerspectiveSettings, Pref, Tool, ToolRequester,
};
use eframe::egui;

use crate::PainterApp;

/// Arms of the small cross hair at the perspective centre.
const CENTER_ARM: i32 = 4;
/// `;` / `'` move the brush this many pixels along its fixed axis; `:` / `"` [`STEP_LARGE`].
const STEP: f64 = 4.0;
const STEP_LARGE: f64 = 32.0;
/// Observer distance limits in pixels (`<` / `>`).
const DISTANCE_RANGE: (f64, f64) = (32.0, 16384.0);
/// The handle stays at least this many pixels in front of the observer.
const MIN_DEPTH: f64 = 8.0;
/// The grid before any brush set it.
const GRID: (u32, u32) = (16, 16);

/// Perspective mode and its settings; kept when the mode ends, so the brush comes back on
/// the same plane (AGA p.141).
#[derive(Clone, Debug)]
pub struct PerspectiveState {
    /// Perspective mode is on (Do, Enter).
    pub active: bool,
    /// Center: the next click on the page places the perspective centre.
    pub placing: bool,
    pub orientation: Orientation,
    /// The Perspective requester's settings.
    pub settings: PerspectiveSettings,
    /// The brush size the grid last followed (AutoGrid).
    grid_brush: Option<(u32, u32)>,
    /// The perspective centre; `None` = the middle of the screen once it is needed.
    pub center: Option<(i32, i32)>,
    /// Observer distance in pixels; `None` = the screen width.
    pub distance: Option<f64>,
    /// The handle's position in space; `None` until the brush is first placed.
    pub position: Option<Vec3>,
    /// The axis the mouse does not move the brush along (Z whenever the mode starts).
    pub fixed: Axis3,
    /// Ctrl is held: the y axis is fixed meanwhile.
    pub ctrl: bool,
    /// A point of the perspective grid (`G` makes it the handle).
    pub grid_origin: Vec3,
    /// The pointer's page pixel while the centre is placed (the large cross hair).
    pub pointer: Option<(i32, i32)>,
    /// The page with brush, wire frame and cross hairs; made just before the screen is
    /// composed and dropped after.
    pub shown: Option<IndexedImage>,
    /// [`crate::tools::ToolState::brush_gen`] when the mode started: a new pick-up or load
    /// ends the mode (a change of the brush in place, such as Arrange, does not).
    picked_gen: u64,
    /// The tool when the mode started: choosing a tool ends the mode.
    tool: (Tool, bool),
}

impl Default for PerspectiveState {
    fn default() -> Self {
        Self {
            active: false,
            placing: false,
            orientation: Orientation::default(),
            settings: PerspectiveSettings::defaults(GRID),
            grid_brush: None,
            center: None,
            distance: None,
            position: None,
            fixed: Axis3::Z,
            ctrl: false,
            grid_origin: [0.5, 0.5, 0.0],
            pointer: None,
            shown: None,
            picked_gen: 0,
            tool: (Tool::ContinuousFreehand, false),
        }
    }
}

impl PerspectiveState {
    /// The pointer belongs to perspective: the mode is on or the centre is being placed.
    pub fn busy(&self) -> bool {
        self.active || self.placing
    }

    /// What the title bar shows at its right: the screen angles, or the handle position.
    fn title(&self) -> String {
        let [x, y, z] = if self.settings.show_position {
            let p = self.position.map(|position| {
                Placement {
                    matrix: self.orientation.matrix(),
                    position,
                }
                .shown_position()
            });
            let (x, y, z) = p.unwrap_or_default();
            [x, y, z]
        } else {
            self.orientation.angles()
        };
        format!("{x:4} {y:4} {z:4}")
    }

    fn angle_type(&self) -> perspective::AngleType {
        if self.settings.brush_axes {
            perspective::AngleType::Brush
        } else {
            perspective::AngleType::Screen
        }
    }
}

/// The keypad digit a key stands for.
fn digit(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    Some(match key {
        Num0 => 0,
        Num1 => 1,
        Num2 => 2,
        Num3 => 3,
        Num4 => 4,
        Num5 => 5,
        Num6 => 6,
        Num7 => 7,
        Num8 => 8,
        Num9 => 9,
        _ => return None,
    })
}

/// The step a key moves the brush along its fixed axis by its place on the keyboard: where a
/// US (and Amiga) keyboard has `;` (towards the observer, negative) and `'` (away), with
/// Shift (`:` and `"`) the large step (AGA p.134-135, p.295). So the steps work on layouts
/// that type other characters there (German Ö and Ä, `@` for Shift-' on a UK keyboard), but
/// not where that key types a letter or another key's character (French M, Dvorak S and -).
fn step_key(key: egui::Key, physical: Option<egui::Key>, shift: bool) -> Option<f64> {
    use egui::Key::{Colon, Quote, Semicolon};
    let physical = physical?;
    let towards = match physical {
        Semicolon => true,
        Quote => false,
        _ => return None,
    };
    // egui names the key by its place when it has no name for the character typed.
    if key != physical && !matches!(key, Semicolon | Colon | Quote) {
        return None;
    }
    let step = if shift { STEP_LARGE } else { STEP };
    Some(if towards { -step } else { step })
}

impl PainterApp {
    /// The observer: the centre (the middle of the screen until placed) and the distance
    /// (also Anim ▸ Move's, ADR-38).
    pub(crate) fn perspective_viewer(&self) -> Viewer {
        let v = self.view();
        let center = self.perspective.center.unwrap_or((
            self.scroll.0 + v.screen_w / 2,
            self.scroll.1 + v.screen_h / 2,
        ));
        let distance = self
            .perspective
            .distance
            .unwrap_or(f64::from(v.screen_w.max(1)));
        Viewer::at_pixel(center, distance)
    }

    fn perspective_placement(&self) -> Option<Placement> {
        Some(Placement {
            matrix: self.perspective.orientation.matrix(),
            position: self.perspective.position?,
        })
    }

    /// Replace mode paints the brush's transparent pixels in BG (with the right button the
    /// whole brush): the projection does that inside the brush and the stamp is Matte, so
    /// nothing outside the brush is painted.
    fn perspective_replace(&self, right: bool) -> Option<perspective::Replace<'_>> {
        (self.ui.mode == PaintMode::Replace).then(|| perspective::Replace {
            bg: self.ui.bg,
            right,
            page: self.engine.document().page(),
        })
    }

    /// The Effect ▸ Perspective items.
    pub(crate) fn perspective_action(&mut self, id: &str) {
        match id {
            "perspective.do" => self.enter_perspective(),
            "perspective.fill_screen" => self.perspective_fill_screen(),
            "perspective.reset" => self.reset_perspective(),
            "perspective.center" => self.place_perspective_center(),
            "perspective.settings" => self.open_perspective_requester(),
            _ => {}
        }
        self.dirty = true;
    }

    /// AutoGrid (on by default): the grid follows the brush size whenever the brush has
    /// another size than last time; also the first time a brush is used.
    fn follow_brush_grid(&mut self) {
        let Some(size) = self.custom_brush.as_ref().map(|b| (b.width, b.height)) else {
            return;
        };
        let p = &mut self.perspective;
        let auto = self.ui.prefs.get(Pref::AutoGrid) || p.grid_brush.is_none();
        if auto && p.grid_brush != Some(size) {
            p.settings.grid = PerspectiveSettings::defaults(size).grid;
            p.grid_brush = Some(size);
        }
    }

    /// Perspective ▸ Do (Enter): the brush comes back on the plane it was on, under the
    /// pointer (AGA p.127, p.141). Needs a custom brush.
    fn enter_perspective(&mut self) {
        if self.custom_brush.is_none() || self.perspective.active {
            return;
        }
        self.finish_stroke();
        self.follow_brush_grid();
        let viewer_center = self.perspective_viewer().center;
        let p = &mut self.perspective;
        p.active = true;
        p.placing = false;
        p.fixed = Axis3::Z;
        p.picked_gen = self.tools.brush_gen;
        p.tool = (self.ui.tool, self.ui.filled);
        p.center.get_or_insert((
            viewer_center.0.floor() as i32,
            viewer_center.1.floor() as i32,
        ));
        if p.position.is_none() {
            let at = self.ui.coords.unwrap_or(p.center.unwrap_or_default());
            p.position = Some(Placement::at_pixel(at).position);
        } else if let Some((x, y)) = self.ui.coords {
            self.perspective_move(x, y);
        }
        self.dirty = true;
    }

    fn leave_perspective(&mut self) {
        let p = &mut self.perspective;
        p.active = false;
        p.shown = None;
        self.dirty = true;
    }

    /// Perspective ▸ Reset (Shift + keypad 0): not turned, back on the screen plane, the
    /// requester's defaults and the standard observer distance; the centre stays (AGA p.278).
    fn reset_perspective(&mut self) {
        let size = self
            .custom_brush
            .as_ref()
            .map(|b| (b.width, b.height))
            .unwrap_or(GRID);
        let p = &mut self.perspective;
        p.orientation = Orientation::default();
        p.settings = PerspectiveSettings::defaults(size);
        p.grid_brush = self.custom_brush.as_ref().map(|_| size);
        p.distance = None;
        p.fixed = Axis3::Z;
        if let Some(pos) = &mut p.position {
            pos[2] = 0.0;
        }
        self.dirty = true;
    }

    /// Perspective ▸ Center (keypad `.`): the next click places the centre; meanwhile a large
    /// cross hair follows the pointer (AGA p.278-279).
    fn place_perspective_center(&mut self) {
        self.finish_stroke();
        self.perspective.placing = true;
        self.perspective.pointer = self.ui.coords;
        self.dirty = true;
    }

    /// Perspective ▸ Settings.. (right-click on Grid while the mode is on).
    pub(crate) fn open_perspective_requester(&mut self) {
        self.finish_stroke();
        self.follow_brush_grid();
        let size = self.custom_brush.as_ref().map(|b| (b.width, b.height));
        self.ui.tool_requester = Some(ToolRequester::Perspective(PerspectiveRequester::new(
            self.perspective.settings,
            size,
        )));
        self.dirty = true;
    }

    /// The Perspective requester's Ok. Going back to Type Screen keeps the shown angles.
    pub(crate) fn apply_perspective_settings(&mut self, r: &PerspectiveRequester) {
        let new = r.settings();
        let p = &mut self.perspective;
        if p.settings.brush_axes && !new.brush_axes {
            p.orientation.snap_to_angles();
        }
        p.settings = new;
        self.dirty = true;
    }

    /// Once per pass: Ctrl, and the end of the mode when its brush or tool is gone.
    pub(crate) fn refresh_perspective(&mut self, ctx: &egui::Context) {
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        self.perspective.ctrl = ctrl;
        self.check_perspective();
    }

    /// Perspective mode ends when its brush goes, another brush is picked up or loaded, or
    /// another tool is chosen; the title bar shows its angles (or position) meanwhile.
    fn check_perspective(&mut self) {
        if self.perspective.active
            && (self.custom_brush.is_none()
                || self.tools.brush_gen != self.perspective.picked_gen
                || (self.ui.tool, self.ui.filled) != self.perspective.tool)
        {
            self.leave_perspective();
        }
        let title = self.perspective.active.then(|| self.perspective.title());
        if self.ui.perspective != title {
            self.ui.perspective = title;
            self.dirty = true;
        }
    }

    /// A press while perspective has the pointer; returns whether it took it. In the mode a
    /// click on the page stamps the brush, a right-click on Grid opens the requester and a
    /// click on a tool or built-in brush ends the mode (AGA p.278-279). The toolbox's action
    /// buttons (UNDO, CLR, Grid, Symmetry, Magnify, Zoom) do their job and keep it: they choose
    /// no tool, and the tutorial clicks CLR in the mode and goes on turning (AGA p.131).
    pub(crate) fn perspective_press(&mut self, hit: Hit, right: bool) -> bool {
        // An armed Brush menu gesture (Handle ▸ Place, Stretch, Rotate, Shear, Bend) takes the
        // page click and ends as outside the mode; the mode goes on with its brush.
        if self.xform_armed() && matches!(hit, Hit::Page { .. }) {
            return false;
        }
        if self.perspective.placing {
            self.perspective.placing = false;
            self.perspective.pointer = None;
            self.dirty = true;
            if let Hit::Page { x, y } = hit {
                self.perspective.center = Some((x, y));
                return true;
            }
        }
        if !self.perspective.active {
            return false;
        }
        match hit {
            Hit::Page { x, y } => {
                self.perspective_move(x, y);
                self.perspective_stamp(right);
                true
            }
            Hit::Tool {
                tool: Tool::Grid, ..
            } if right => {
                self.open_perspective_requester();
                true
            }
            Hit::Tool {
                tool:
                    Tool::Undo | Tool::Clear | Tool::Grid | Tool::Symmetry | Tool::Magnify | Tool::Zoom,
                ..
            } => false,
            Hit::Tool { .. } | Hit::Brush(_) => {
                self.leave_perspective();
                false
            }
            _ => false,
        }
    }

    /// The pointer is over page pixel (`x`, `y`): the brush slides on its plane to it (along
    /// the two axes that are not fixed, on the perspective grid while Grid is on), or the
    /// large cross hair follows it while the centre is placed.
    pub(crate) fn perspective_move(&mut self, x: i32, y: i32) {
        if self.perspective.placing {
            if self.perspective.pointer != Some((x, y)) {
                self.perspective.pointer = Some((x, y));
                self.dirty = true;
            }
            return;
        }
        if !self.perspective.active {
            return;
        }
        let viewer = self.perspective_viewer();
        let Some(place) = self.perspective_placement() else {
            self.perspective.position = Some(Placement::at_pixel((x, y)).position);
            self.dirty = true;
            return;
        };
        let fixed = if self.perspective.ctrl {
            Axis3::Y
        } else {
            self.perspective.fixed
        };
        let Some(mut p) = perspective::slide(&place, &viewer, fixed, (x, y)) else {
            return;
        };
        if self.tools.grid {
            let s = &self.perspective.settings;
            let grid = s.grid.map(|g| g.max(1) as u32);
            p = perspective::snap(
                p,
                self.perspective.grid_origin,
                &place.matrix,
                grid,
                Some(fixed),
            );
        }
        self.set_perspective_position(p);
    }

    fn set_perspective_position(&mut self, mut p: Vec3) {
        let viewer = self.perspective_viewer();
        let near = MIN_DEPTH - viewer.distance;
        let far = (perspective::FAR - 1.0) * viewer.distance;
        p[2] = p[2].clamp(near, far);
        if p[2].abs() < 1e-9 {
            p[2] = 0.0;
        }
        if self.perspective.position != Some(p) {
            self.perspective.position = Some(p);
            self.dirty = true;
        }
    }

    /// A click in perspective mode: the turned brush as one stamp (FG, BG with the right
    /// button), in the Mode menu's mode. Stencil ▸ Show keeps painting off. The next Move
    /// starts from the stamped brush (ADR-38), and an AnimBrush goes on to its next cel
    /// (ADR-36).
    fn perspective_stamp(&mut self, right: bool) {
        if self.menu_data.effects.stencil_show {
            return;
        }
        let Some(place) = self.perspective_placement() else {
            return;
        };
        if self.custom_brush.is_none() {
            return;
        }
        // Move ▸ starts from this stamp's exact pose (ADR-38).
        self.note_stamp(place.position, place.matrix.0);
        let animated = self.animbrush_cels().is_some();
        let Some(brush) = &self.custom_brush else {
            return;
        };
        let doc = self.engine.document();
        let replace = self.perspective_replace(right);
        let matte = replace.is_some();
        let projected = perspective::project_brush(
            brush,
            &place,
            &self.perspective_viewer(),
            doc.page().bounds(),
            doc.palette().len(),
            replace,
        );
        if let Some((img, at)) = projected {
            let cmd = self.brush_stamp(img, at, right, matte);
            self.run_stamp(cmd, animated);
        }
    }

    /// Runs a perspective stamp or FillScreen; with an AnimBrush whose cels fit the palette
    /// the next cel becomes the brush when it painted (the picture or the Stencil ▸ Paint
    /// mask), one cel per click, as a dotted click outside the mode (ADR-36).
    fn run_stamp(&mut self, cmd: Command, animated: bool) {
        let revisions = |app: &Self| {
            (
                app.engine.document().revision(),
                app.stencil_mask_revision(),
            )
        };
        let before = revisions(self);
        self.run(cmd);
        if animated && revisions(self) != before {
            self.advance_anim_brush(1);
        }
    }

    /// Perspective ▸ FillScreen (keypad `-`): the screen covered with the brush plane, copies
    /// Grid X by Grid Y apart, as one stamp (AGA p.137, p.278).
    fn perspective_fill_screen(&mut self) {
        if self.custom_brush.is_none() || self.menu_data.effects.stencil_show {
            return;
        }
        self.follow_brush_grid();
        if self.perspective.position.is_none() {
            let at = self.ui.coords.unwrap_or_else(|| {
                let c = self.perspective_viewer().center;
                (c.0.floor() as i32, c.1.floor() as i32)
            });
            self.perspective.position = Some(Placement::at_pixel(at).position);
        }
        let (Some(brush), Some(place)) = (&self.custom_brush, self.perspective_placement()) else {
            return;
        };
        let animated = self.animbrush_cels().is_some();
        let v = self.view();
        let doc = self.engine.document();
        let screen = Rect::new(self.scroll.0, self.scroll.1, v.screen_w, v.screen_h)
            .intersect(doc.page().bounds());
        let [gx, gy, _] = self.perspective.settings.grid;
        let replace = self.perspective_replace(false);
        let matte = replace.is_some();
        let plane = perspective::fill_plane(
            brush,
            &place,
            &self.perspective_viewer(),
            (gx.max(1) as u32, gy.max(1) as u32),
            screen,
            doc.palette().len(),
            replace,
        );
        if let Some((img, at)) = plane {
            let cmd = self.brush_stamp(img, at, false, matte);
            self.run_stamp(cmd, animated);
        }
    }

    /// Turns the brush about `axis` (Type Screen: the screen's axis, Brush: its own).
    fn perspective_rotate(&mut self, axis: Axis3, degrees: i32) {
        let kind = self.perspective.angle_type();
        self.perspective.orientation.rotate(axis, degrees, kind);
        self.dirty = true;
    }

    /// `;` / `'`: the brush moves along its fixed axis (on the grid's Z steps while Grid is
    /// on), towards the observer for negative `steps`.
    fn perspective_push(&mut self, step: f64) {
        let Some(place) = self.perspective_placement() else {
            return;
        };
        let axis = if self.perspective.ctrl {
            Axis3::Y
        } else {
            self.perspective.fixed
        };
        let step = if self.tools.grid {
            step.signum() * f64::from(self.perspective.settings.grid[axis.index()].max(1))
        } else {
            step
        };
        let dir = place.matrix.axis(axis);
        let p = place.position;
        self.set_perspective_position([
            p[0] + dir[0] * step,
            p[1] + dir[1] * step,
            p[2] + dir[2] * step,
        ]);
    }

    /// One key of the keypad table (AGA p.280-281, p.295) or a step key ([`step_key`]);
    /// returns whether it was used.
    fn perspective_key(
        &mut self,
        key: egui::Key,
        digit_key: Option<u8>,
        step: Option<f64>,
        shift: bool,
    ) -> bool {
        if key == egui::Key::Escape && self.perspective.placing {
            self.perspective.placing = false;
            self.perspective.pointer = None;
            self.dirty = true;
            return true;
        }
        if key == egui::Key::Enter {
            let idle = !self.gesture_in_progress();
            if self.perspective.active {
                self.leave_perspective();
                return true;
            }
            if self.custom_brush.is_some() && idle {
                self.enter_perspective();
                return true;
            }
            return false;
        }
        if !self.perspective.active {
            return false;
        }
        if let Some(step) = step {
            self.perspective_push(step);
            self.dirty = true;
            return true;
        }
        let angle = if shift {
            self.perspective.settings.angle_step
        } else {
            1
        };
        match digit_key {
            Some(7) => self.perspective_rotate(Axis3::X, -angle),
            Some(8) => self.perspective_rotate(Axis3::X, angle),
            Some(4) => self.perspective_rotate(Axis3::Y, -angle),
            Some(5) => self.perspective_rotate(Axis3::Y, angle),
            Some(1) => self.perspective_rotate(Axis3::Z, -angle),
            Some(2) => self.perspective_rotate(Axis3::Z, angle),
            Some(d @ (9 | 6 | 3)) => {
                let axis = match d {
                    9 => Axis3::X,
                    6 => Axis3::Y,
                    _ => Axis3::Z,
                };
                if shift {
                    self.perspective.fixed = axis;
                } else {
                    self.perspective.orientation.reset_axis(axis);
                }
            }
            Some(0) if shift => self.reset_perspective(),
            Some(0) => {
                // Angles back to 0 and the z axis fixed; distances stay (AGA p.281).
                self.perspective.orientation = Orientation::default();
                self.perspective.fixed = Axis3::Z;
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// One typed character in perspective mode; returns whether it was used. `-` and `.` (the
    /// keypad's FillScreen and Center) go by the character, not the key: Shift-`.` types `>`
    /// on a US keyboard and `:` on a German one, and those are keys of their own. `held`: the
    /// key is held down and repeats; only the steps and the distance go on then.
    fn perspective_char(&mut self, c: char, held: bool) -> bool {
        match c {
            '-' | '.' | '\\' if held => {}
            '-' => self.perspective_fill_screen(),
            '.' => self.place_perspective_center(),
            ';' => self.perspective_push(-STEP),
            ':' => self.perspective_push(-STEP_LARGE),
            '\'' => self.perspective_push(STEP),
            // The manual calls the large step away `@` once (AGA p.135).
            '"' | '@' => self.perspective_push(STEP_LARGE),
            '<' | '>' => {
                let d = self.perspective_viewer().distance;
                let d = if c == '>' { d * 1.25 } else { d * 0.8 };
                self.perspective.distance = Some(d.clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1));
            }
            '\\' => {
                let s = &mut self.perspective.settings;
                s.show_position = !s.show_position;
            }
            'G' => {
                // Also toggles the grid (tool keys): the handle becomes a grid point.
                if let Some(p) = self.perspective.position {
                    self.perspective.grid_origin = p;
                }
                return false;
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// The perspective keys of this pass, before the tool and animation keys, which then do
    /// not see them (the digits are animation keys outside perspective mode).
    pub(crate) fn perspective_keys(&mut self, ctx: &egui::Context) {
        if self.typing() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let mut taken = Vec::new();
        // The text a used key types (e.g. `"` for Shift-2 on a German keyboard) is no
        // command of its own; the text of a held, repeating key repeats only some.
        let mut its_text = false;
        let mut held = false;
        for (n, event) in events.iter().enumerate() {
            let used = match event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat,
                    modifiers,
                } => {
                    its_text = false;
                    held = *repeat;
                    if modifiers.ctrl || modifiers.alt || modifiers.command || modifiers.mac_cmd {
                        false
                    } else {
                        // Shift changes what the main digit keys type: go by the key's place.
                        let d = physical_key.and_then(digit).or_else(|| digit(*key));
                        let step = step_key(*key, *physical_key, modifiers.shift);
                        // Held down, the turn and step keys go on.
                        let repeats = matches!(d, Some(1 | 2 | 4 | 5 | 7 | 8)) || step.is_some();
                        let used = (!*repeat || repeats)
                            && self.perspective_key(*key, d, step, modifiers.shift);
                        its_text = used && !matches!(key, egui::Key::Enter | egui::Key::Escape);
                        used
                    }
                }
                egui::Event::Text(t) => {
                    let mut chars = t.chars();
                    let held = std::mem::take(&mut held);
                    if std::mem::take(&mut its_text) {
                        true
                    } else if self.perspective.active {
                        match (chars.next(), chars.next()) {
                            (Some(c), None) => self.perspective_char(c, held),
                            _ => false,
                        }
                    } else {
                        false
                    }
                }
                _ => false,
            };
            if used {
                taken.push(n);
            }
        }
        if !taken.is_empty() {
            ctx.input_mut(|i| {
                let mut n = 0;
                i.events.retain(|_| {
                    let keep = !taken.contains(&n);
                    n += 1;
                    keep
                });
            });
            self.dirty = true;
        }
    }

    /// Just before the screen is composed: the page with the brush (only while it lies
    /// unturned on the screen plane, AGA p.128), its wire frame, the centre's cross hair and,
    /// while the centre is placed, the large cross hair; the marks in complementary colours.
    /// The brush goes behind the area stencil as the click paints it (ADR-39); during
    /// Stencil ▸ Paint the stencil view shows it in the mask instead and draws the marks on
    /// top of that view (`prepare_stencil_view`).
    pub(crate) fn prepare_perspective_view(&mut self) {
        // Changes later in this pass (a menu item, a tool key) show at once.
        self.check_perspective();
        if !self.perspective_shown() {
            self.perspective.shown = None;
            return;
        }
        let doc = self.engine.document();
        let (page, palette) = (doc.page(), doc.palette());
        let mut stamped = page.clone();
        if !self.stencil_painting()
            && let Some(cmd) = self.perspective_preview_stamp()
        {
            let cmd = self.with_area_stencil(cmd);
            if cmd.check_paint(page, palette).is_ok() {
                cmd.paint(&mut stamped, page, palette);
            }
        }
        // ADR-44: the stamp goes into the active layer; the frame shows all visible layers
        // and the marks lie on top of them.
        let mut shown = doc.composite_with(&stamped).into_owned();
        self.draw_perspective_marks(&mut shown);
        self.perspective.shown = Some(shown);
    }

    /// Perspective shows itself on the screen: the mode is on or the centre is being placed,
    /// and no Brush menu gesture shows its own preview (the stroke) meanwhile.
    pub(crate) fn perspective_shown(&self) -> bool {
        self.perspective.busy() && !self.xform_armed()
    }

    /// The stamp a click paints now while the brush lies unturned on the screen plane (only
    /// then its image is shown, AGA p.128), without the area stencil; `None` otherwise.
    pub(crate) fn perspective_preview_stamp(&self) -> Option<Command> {
        if !self.perspective_shown() || !self.perspective.active {
            return None;
        }
        let (Some(brush), Some(place)) = (&self.custom_brush, self.perspective_placement()) else {
            return None;
        };
        if !place.on_screen_plane() {
            return None;
        }
        let doc = self.engine.document();
        let replace = self.perspective_replace(false);
        let matte = replace.is_some();
        let viewer = self.perspective_viewer();
        let clip = doc.page().bounds();
        let (img, at) =
            perspective::project_brush(brush, &place, &viewer, clip, doc.palette().len(), replace)?;
        Some(self.brush_stamp(img, at, false, matte))
    }

    /// Draws the wire frame, the centre's cross hair and, while the centre is placed, the
    /// large cross hair on `img` (the page as shown), in the complementary colours of what
    /// `img` shows there; nothing while perspective does not show itself.
    pub(crate) fn draw_perspective_marks(&self, img: &mut IndexedImage) {
        if !self.perspective_shown() {
            return;
        }
        let viewer = self.perspective_viewer();
        let doc = self.engine.document();
        let clip = doc.page().bounds();
        let mut marks = Vec::new();
        if self.perspective.active
            && let (Some(brush), Some(place)) = (&self.custom_brush, self.perspective_placement())
        {
            let size = (brush.width, brush.height);
            marks.extend(perspective::wireframe(
                size,
                brush.handle,
                &place,
                &viewer,
                clip,
            ));
        }
        let center = (
            viewer.center.0.floor() as i32,
            viewer.center.1.floor() as i32,
        );
        marks.extend(perspective::cross(center, CENTER_ARM, clip));
        if self.perspective.placing
            && let Some(at) = self.perspective.pointer
        {
            let arm = clip.w.max(clip.h);
            marks.extend(perspective::cross(at, arm, clip));
        }
        marks.sort_unstable();
        marks.dedup();
        let top = (doc.palette().len().clamp(1, 256) - 1) as u8;
        for (x, y) in marks {
            if let Some(c) = img.get(x, y) {
                img.set(x, y, top.saturating_sub(c));
            }
        }
    }
}

/// `--perspective 'center=X,Y;distance=D;fill:AX,AY,AZ@X,Y[,Z];stamp:AX,AY,AZ@X,Y[,Z]'`
/// (with `--pick`): FillScreen and stamps of the picked brush turned by the screen angles
/// with its handle at page pixel (X, Y) and depth Z, in Matte; the centre defaults to the
/// middle of the `screen`, the distance to its width.
pub fn run_spec(
    engine: &mut Engine,
    brush: &BrushImage,
    spec: &str,
    screen: (i32, i32),
) -> Result<(), String> {
    let mut center = (screen.0 / 2, screen.1 / 2);
    let mut distance = f64::from(screen.0.max(1));
    let grid = (brush.width, brush.height);
    for entry in spec.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let bad = || format!("'{entry}': expected center=X,Y, distance=D or OP:AX,AY,AZ@X,Y[,Z]");
        let nums = |s: &str| -> Result<Vec<i32>, String> {
            s.split(',')
                .map(|n| n.trim().parse::<i32>().map_err(|_| bad()))
                .collect()
        };
        if let Some(v) = entry.strip_prefix("center=") {
            let &[x, y] = nums(v)?.as_slice() else {
                return Err(bad());
            };
            center = (x, y);
            continue;
        }
        if let Some(v) = entry.strip_prefix("distance=") {
            distance = f64::from(nums(v)?.first().copied().ok_or_else(bad)?.max(1));
            continue;
        }
        let (op, rest) = entry.split_once(':').ok_or_else(bad)?;
        let (angles, at) = rest.split_once('@').ok_or_else(bad)?;
        let &[ax, ay, az] = nums(angles)?.as_slice() else {
            return Err(bad());
        };
        let at = nums(at)?;
        let (x, y, z) = match at[..] {
            [x, y] => (x, y, 0),
            [x, y, z] => (x, y, z),
            _ => return Err(bad()),
        };
        let mut place = Placement::at_pixel((x, y));
        place.matrix = Orientation::from_angles([ax, ay, az]).matrix();
        place.position[2] = f64::from(z);
        let viewer = Viewer::at_pixel(center, distance);
        let doc = engine.document();
        let (bounds, colors) = (doc.page().bounds(), doc.palette().len());
        let projected = match op {
            "stamp" => perspective::project_brush(brush, &place, &viewer, bounds, colors, None),
            "fill" => {
                let area = Rect::new(0, 0, screen.0, screen.1).intersect(bounds);
                perspective::fill_plane(brush, &place, &viewer, grid, area, colors, None)
            }
            _ => return Err(format!("'{op}': unknown perspective operation")),
        };
        let Some((img, at)) = projected else {
            continue;
        };
        engine
            .execute(Command::Stroke {
                brush: Vec::new(),
                color: 0,
                points: vec![at],
                continuous: false,
                custom: Some(img),
                mode: dpx_core::DrawMode::Matte,
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            })
            .map_err(|e| format!("'{entry}': {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{key_event, menu_path, ui_pass, ui_point};

    /// A 6×4 brush: colour 0 (transparent) at its top-left, colours 5..=7 elsewhere; the
    /// handle at (3, 2).
    fn brush() -> BrushImage {
        BrushImage {
            width: 6,
            height: 4,
            pixels: (0..24u8)
                .map(|i| if i == 0 { 0 } else { 5 + i % 3 })
                .collect(),
            handle: (3, 2),
            transparent: Some(0),
        }
    }

    fn setup(with_brush: bool) -> (PainterApp, egui::Context) {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        if with_brush {
            app.custom_brush = Some(brush());
            app.picked_brush = Some(brush());
            app.ui.mode = PaintMode::Matte;
        }
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        (app, ctx)
    }

    /// A key as the window reports it: the key, its place on the keyboard, the text it types.
    fn typed(key: egui::Key, physical: egui::Key, shift: bool, text: &str) -> Vec<egui::Event> {
        let mut v = vec![egui::Event::Key {
            key,
            physical_key: Some(physical),
            pressed: true,
            repeat: false,
            modifiers: if shift {
                egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::NONE
            },
        }];
        if !text.is_empty() {
            v.push(egui::Event::Text(text.into()));
        }
        v.push(egui::Event::Key {
            key,
            physical_key: Some(physical),
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        v
    }

    /// A key pressed and let go in one pass.
    fn tap(key: egui::Key) -> Vec<egui::Event> {
        vec![key_event(key, true), key_event(key, false)]
    }

    /// A keypad digit (the window reports it as the main digit key).
    fn pad(n: u8, shift: bool) -> Vec<egui::Event> {
        let key = [
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
        ][usize::from(n)];
        typed(key, key, shift, &n.to_string())
    }

    fn text(t: &str) -> Vec<egui::Event> {
        vec![egui::Event::Text(t.into())]
    }

    fn moved(app: &PainterApp, at: (i32, i32)) -> egui::Event {
        egui::Event::PointerMoved(ui_point(app, at))
    }

    fn button(app: &PainterApp, at: (i32, i32), b: egui::PointerButton, down: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: ui_point(app, at),
            button: b,
            pressed: down,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A click with `b` at UI pixel `at` (the page pixel on a LoRes screen without scrolling):
    /// press in one pass, release in the next.
    fn click(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        at: (i32, i32),
        b: egui::PointerButton,
    ) {
        let down = vec![moved(app, at), button(app, at, b, true)];
        ui_pass(app, ctx, t, down);
        let up = vec![button(app, at, b, false)];
        ui_pass(app, ctx, t + 0.01, up);
    }

    /// A UI pixel on `tool`'s toolbox button (its upper-left part: the outline half of a
    /// split icon).
    fn tool_at(app: &PainterApp, tool: Tool) -> (i32, i32) {
        let screen = app.view().ui_screen();
        let mut hits = Vec::new();
        for y in 0..screen.height {
            for x in screen.toolbox_x()..screen.width {
                if let Hit::Tool { tool: t, .. } = screen.hit_test(32, &app.ui, x, y)
                    && t == tool
                {
                    hits.push((x, y));
                }
            }
        }
        hits[hits.len() / 4]
    }

    /// Effect ▸ Perspective ▸ `id` picked from the menu with the right mouse button.
    fn menu(app: &mut PainterApp, ctx: &egui::Context, t: f64, id: &'static str) {
        use egui::PointerButton::Secondary;
        let [title, item, sub] = menu_path(app, id);
        let open = vec![moved(app, title), button(app, title, Secondary, true)];
        ui_pass(app, ctx, t, open);
        assert!(app.ui.menu.active);
        let steps = [moved(app, item), moved(app, sub)];
        for (i, step) in steps.into_iter().enumerate() {
            ui_pass(app, ctx, t + 0.01 * (i + 1) as f64, vec![step]);
        }
        let up = vec![button(app, sub, Secondary, false)];
        ui_pass(app, ctx, t + 0.04, up);
        assert!(!app.ui.menu.active);
    }

    fn painted(app: &PainterApp, at: (i32, i32)) -> Option<u8> {
        app.engine.document().page().get(at.0, at.1)
    }

    /// Moves the pointer to page pixel `at` and presses Enter (perspective mode on).
    fn enter_at(app: &mut PainterApp, ctx: &egui::Context, t: f64, at: (i32, i32)) {
        let pointer = vec![moved(app, at)];
        ui_pass(app, ctx, t, pointer);
        ui_pass(app, ctx, t + 0.01, tap(egui::Key::Enter));
        assert!(app.perspective.active);
    }

    #[test]
    fn do_from_the_menu_and_the_keypad_turns_the_brush() {
        let (mut app, ctx) = setup(true);
        // Three frames: the digits must not step through them meanwhile.
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
        let pointer = vec![moved(&app, (100, 100))];
        ui_pass(&mut app, &ctx, 0.1, pointer);
        menu(&mut app, &ctx, 0.2, "perspective.do");
        assert!(app.perspective.active);
        assert_eq!(app.ui.perspective.as_deref(), Some("   0    0    0"));
        ui_pass(&mut app, &ctx, 0.3, pad(2, false));
        ui_pass(&mut app, &ctx, 0.4, pad(2, false));
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 2]);
        assert_eq!(app.ui.perspective.as_deref(), Some("   0    0    2"));
        // Shift turns by the Angle Step (90); 4 and 5 turn about y, 7 and 8 about x.
        ui_pass(&mut app, &ctx, 0.5, pad(7, true));
        ui_pass(&mut app, &ctx, 0.6, pad(5, false));
        assert_eq!(app.perspective.orientation.angles(), [-90, 1, 2]);
        assert_eq!(app.engine.document().current_frame(), 0, "no frame steps");
        assert!(app.anim.playing.is_none() && app.ui.anim_requester.is_none());
        // 9 resets x, 6 y; Shift-9 fixes x.
        ui_pass(&mut app, &ctx, 0.7, pad(9, false));
        ui_pass(&mut app, &ctx, 0.8, pad(6, false));
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 2]);
        ui_pass(&mut app, &ctx, 0.9, pad(9, true));
        assert_eq!(app.perspective.fixed, Axis3::X);
        // 1 turns back about z, 3 resets z; 0 resets the angles and fixes z again.
        ui_pass(&mut app, &ctx, 1.0, pad(1, false));
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 1]);
        ui_pass(&mut app, &ctx, 1.1, pad(3, false));
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 0]);
        ui_pass(&mut app, &ctx, 1.2, pad(8, false));
        ui_pass(&mut app, &ctx, 1.3, pad(0, false));
        assert!(app.perspective.orientation.is_identity());
        assert_eq!(app.perspective.fixed, Axis3::Z);
        // Enter leaves the mode; then the digits step frames again.
        ui_pass(&mut app, &ctx, 1.4, tap(egui::Key::Enter));
        assert!(!app.perspective.active);
        assert_eq!(app.ui.perspective, None);
        ui_pass(&mut app, &ctx, 1.5, pad(2, false));
        assert_eq!(app.engine.document().current_frame(), 1);
    }

    #[test]
    fn shifted_digits_turn_whatever_they_type() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        // German keyboard: Shift-2 types `"`; the key turns, the text does not move the brush.
        let german = typed(egui::Key::Num2, egui::Key::Num2, true, "\"");
        ui_pass(&mut app, &ctx, 0.3, german);
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 90]);
        assert_eq!(app.perspective.position.unwrap()[2], 0.0);
        // US keyboard: Shift-' types `"` with the quote key: the brush moves away (32).
        let us = typed(egui::Key::Quote, egui::Key::Quote, true, "\"");
        ui_pass(&mut app, &ctx, 0.4, us);
        assert_eq!(app.perspective.position.unwrap()[2], STEP_LARGE);
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 90]);
        // `;` back 4 at a time, `'` away; `:` back by the large step.
        ui_pass(&mut app, &ctx, 0.5, text(";"));
        ui_pass(&mut app, &ctx, 0.6, text(";"));
        ui_pass(&mut app, &ctx, 0.7, text("'"));
        ui_pass(&mut app, &ctx, 0.8, text(":"));
        assert_eq!(app.perspective.position.unwrap()[2], -STEP);
        ui_pass(&mut app, &ctx, 0.85, text("'"));
        assert_eq!(app.perspective.position.unwrap()[2], 0.0);
        // `>` / `<`: the observer moves away and closer (no Magnify zoom meanwhile); `\`
        // shows the position instead of the angles.
        let zoom = app.tools.zoom;
        ui_pass(&mut app, &ctx, 0.9, text(">"));
        assert_eq!(app.perspective.distance, Some(400.0));
        ui_pass(&mut app, &ctx, 1.0, text("<"));
        assert_eq!(app.perspective.distance, Some(320.0));
        assert_eq!(app.tools.zoom, zoom, "no zoom change");
        ui_pass(&mut app, &ctx, 1.1, text("\\"));
        assert_eq!(app.ui.perspective.as_deref(), Some(" 100  100    0"));
        ui_pass(&mut app, &ctx, 1.2, text("'"));
        assert_eq!(app.ui.perspective.as_deref(), Some(" 100  100    4"));
    }

    #[test]
    fn shift_period_is_the_observer_and_the_period_alone_the_centre() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        // US and UK keyboards: egui has no key for `>`, so Shift-. comes as the period key
        // with Shift, then the text `>`: the observer moves away, no Center.
        let us = typed(egui::Key::Period, egui::Key::Period, true, ">");
        ui_pass(&mut app, &ctx, 0.3, us);
        assert_eq!(app.perspective.distance, Some(400.0));
        assert!(!app.perspective.placing);
        let us = typed(egui::Key::Comma, egui::Key::Comma, true, "<");
        ui_pass(&mut app, &ctx, 0.4, us);
        assert_eq!(app.perspective.distance, Some(320.0));
        // German keyboard: Shift-. types `:`, the large step towards the observer.
        let german = typed(egui::Key::Colon, egui::Key::Period, true, ":");
        ui_pass(&mut app, &ctx, 0.5, german);
        assert_eq!(app.perspective.position.unwrap()[2], -STEP_LARGE);
        assert!(!app.perspective.placing);
        // A held `-` fills the screen once, not with every repeat (egui marks the second
        // press of a key that is still down as a repeat).
        let revision = app.engine.document().revision();
        let mut minus = typed(egui::Key::Minus, egui::Key::Minus, false, "-");
        let release = minus.pop().unwrap();
        ui_pass(&mut app, &ctx, 0.6, minus.clone());
        assert_eq!(app.engine.document().revision(), revision + 1);
        ui_pass(&mut app, &ctx, 0.65, minus);
        ui_pass(&mut app, &ctx, 0.7, vec![release]);
        assert_eq!(app.engine.document().revision(), revision + 1);
        // The period itself places the centre; so does the keypad's with Shift (the keypad
        // period has no key place in egui).
        let period = typed(egui::Key::Period, egui::Key::Period, false, ".");
        ui_pass(&mut app, &ctx, 0.8, period);
        assert!(app.perspective.placing);
        ui_pass(&mut app, &ctx, 0.9, tap(egui::Key::Escape));
        assert!(!app.perspective.placing && app.perspective.active);
        let keypad = vec![
            egui::Event::Key {
                key: egui::Key::Period,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
            },
            egui::Event::Text(".".into()),
        ];
        ui_pass(&mut app, &ctx, 1.0, keypad);
        assert!(app.perspective.placing);
    }

    #[test]
    fn the_step_keys_go_by_their_place_on_the_keyboard_too() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        use egui::Key::{Colon, M, Quote, Semicolon};
        let z = |app: &PainterApp| app.perspective.position.unwrap()[2];
        // German keyboard: Ä and Ö sit where a US keyboard has ' and ;.
        let steps = [
            (Quote, Quote, false, "ä", STEP),
            (Quote, Quote, true, "Ä", STEP + STEP_LARGE),
            (Semicolon, Semicolon, false, "ö", STEP_LARGE),
            (Semicolon, Semicolon, true, "Ö", 0.0),
            // UK keyboard: Shift-' types `@`.
            (Quote, Quote, true, "@", STEP_LARGE),
            // US keyboard: Shift-; is `:` (egui knows that key): one large step, not two.
            (Colon, Semicolon, true, ":", 0.0),
            // French keyboard: M sits there; it stays M (Magnify), no step.
            (M, Semicolon, false, "m", 0.0),
        ];
        for (i, (key, place, shift, text, want)) in steps.into_iter().enumerate() {
            let t = 0.3 + 0.1 * i as f64;
            ui_pass(&mut app, &ctx, t, typed(key, place, shift, text));
            assert_eq!(z(&app), want, "{text}");
        }
        assert!(app.tools.magnify.is_some(), "M is Magnify");
        // `@` typed another way (Option-L on a German Mac) is the large step away too.
        ui_pass(&mut app, &ctx, 1.1, text("@"));
        assert_eq!(z(&app), STEP_LARGE);
    }

    #[test]
    fn undo_clr_and_the_toggle_buttons_keep_the_mode() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(true);
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
        enter_at(&mut app, &ctx, 0.1, (60, 60));
        click(&mut app, &ctx, 0.3, (80, 70), Primary);
        assert_eq!(painted(&app, (80, 70)), brush().get(3, 2));
        // UNDO takes the stamp back and the mode stays (as Shift-K's CLR keeps it).
        let undo = tool_at(&app, Tool::Undo);
        click(&mut app, &ctx, 0.4, undo, Primary);
        assert_eq!(painted(&app, (80, 70)), Some(0));
        assert!(app.perspective.active, "UNDO");
        // CLR, as in the tutorial (AGA p.131), then the keypad turns on.
        click(&mut app, &ctx, 0.5, (80, 70), Primary);
        let clr = tool_at(&app, Tool::Clear);
        click(&mut app, &ctx, 0.6, clr, Primary);
        assert_eq!(painted(&app, (80, 70)), Some(0));
        assert!(app.perspective.active, "CLR");
        // Grid and Symmetry switch on and off.
        let grid = tool_at(&app, Tool::Grid);
        click(&mut app, &ctx, 0.7, grid, Primary);
        let sym = tool_at(&app, Tool::Symmetry);
        click(&mut app, &ctx, 0.8, sym, Primary);
        assert!(app.tools.grid && app.tools.symmetry);
        assert!(app.perspective.active, "Grid, Symmetry");
        ui_pass(&mut app, &ctx, 0.9, pad(2, true));
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 90]);
        assert_eq!(app.engine.document().current_frame(), 0, "no frame step");
        // A drawing tool still ends it.
        let rect = tool_at(&app, Tool::Rectangle);
        click(&mut app, &ctx, 1.0, rect, Primary);
        assert!(!app.perspective.active);
    }

    #[test]
    fn brush_menu_gestures_work_in_the_mode() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        let revision = app.engine.document().revision();
        // Brush ▸ Handle ▸ Place (the tutorial changes the handle in the mode, AGA p.142).
        menu(&mut app, &ctx, 0.3, "brush.handle_place");
        assert!(app.xform_armed() && app.perspective.active);
        let m = vec![moved(&app, (100, 100))];
        ui_pass(&mut app, &ctx, 0.4, m);
        // The gesture shows its own preview: the brush flat, its handle at the pointer.
        app.prepare_perspective_view();
        assert!(app.perspective.shown.is_none());
        assert_eq!(app.compose_screen().get(101, 101), brush().get(4, 3));
        // Press, drag two left and one up, let go: the handle moves to (1, 1); no stamp.
        let down = vec![button(&app, (100, 100), Primary, true)];
        ui_pass(&mut app, &ctx, 0.5, down);
        let drag = vec![moved(&app, (98, 99))];
        ui_pass(&mut app, &ctx, 0.6, drag);
        let up = vec![button(&app, (98, 99), Primary, false)];
        ui_pass(&mut app, &ctx, 0.7, up);
        assert!(!app.xform_armed());
        assert_eq!(app.engine.document().revision(), revision, "no stamp");
        assert_eq!(app.custom_brush.as_ref().unwrap().handle, (1, 1));
        assert!(app.perspective.active);
        // The mode goes on with the new handle.
        click(&mut app, &ctx, 0.8, (150, 120), Primary);
        assert_eq!(app.engine.document().revision(), revision + 1);
        assert_eq!(painted(&app, (150, 120)), brush().get(1, 1));
        assert_eq!(painted(&app, (152, 122)), brush().get(3, 3));
    }

    #[test]
    fn replace_on_a_four_colour_screen_paints_bg_inside_the_brush() {
        use egui::PointerButton::{Primary, Secondary};
        let (mut app, ctx) = setup(false);
        let rgb = [0x000, 0xF00, 0x0F0, 0xFFF];
        let palette = dpx_core::Palette::from_rgb4(&rgb).unwrap();
        app.engine = Engine::new(dpx_core::Document::new(320, 256, palette));
        app.engine.execute(Command::Clear { color: 3 }).unwrap();
        // Every colour in the brush, 0 transparent; BG 0: no colour is free.
        let b = BrushImage {
            width: 4,
            height: 2,
            pixels: vec![0, 1, 2, 3, 3, 2, 1, 0],
            handle: (1, 1),
            transparent: Some(0),
        };
        app.custom_brush = Some(b.clone());
        app.picked_brush = Some(b);
        app.ui.mode = PaintMode::Replace;
        (app.ui.fg, app.ui.bg) = (1, 0);
        ui_pass(&mut app, &ctx, 0.05, vec![]);
        enter_at(&mut app, &ctx, 0.1, (60, 60));
        click(&mut app, &ctx, 0.3, (80, 70), Primary);
        // The brush's transparent corners in BG, its colours as they are, around it nothing.
        assert_eq!(painted(&app, (79, 69)), Some(0));
        assert_eq!(painted(&app, (82, 70)), Some(0));
        assert_eq!(painted(&app, (80, 69)), Some(1));
        assert_eq!(painted(&app, (79, 70)), Some(3));
        assert_eq!(painted(&app, (83, 70)), Some(3), "outside");
        // The right button: the whole brush in BG.
        click(&mut app, &ctx, 0.4, (180, 70), Secondary);
        assert!((179..183).all(|x| (69..71).all(|y| painted(&app, (x, y)) == Some(0))));
        assert_eq!(painted(&app, (183, 70)), Some(3), "outside");
        // Turned by 45 degrees: only the turned brush, not the box around it.
        app.perspective.orientation = Orientation::from_angles([0, 0, 45]);
        click(&mut app, &ctx, 0.5, (180, 150), Secondary);
        let bg: Vec<_> = (170..190)
            .flat_map(|x| (140..160).map(move |y| (x, y)))
            .filter(|&p| painted(&app, p) == Some(0))
            .collect();
        let (x0, x1) = (bg.iter().map(|p| p.0).min(), bg.iter().map(|p| p.0).max());
        let (y0, y1) = (bg.iter().map(|p| p.1).min(), bg.iter().map(|p| p.1).max());
        let (x0, x1, y0, y1) = (x0.unwrap(), x1.unwrap(), y0.unwrap(), y1.unwrap());
        assert!(bg.len() >= 8, "{bg:?}");
        for corner in [(x0, y0), (x1, y0), (x0, y1)] {
            assert_eq!(
                painted(&app, corner),
                Some(3),
                "a corner of the box: {corner:?}"
            );
        }
    }

    #[test]
    fn a_click_stamps_one_undoable_command_and_a_tool_ends_the_mode() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (60, 60));
        let revision = app.engine.document().revision();
        click(&mut app, &ctx, 0.3, (80, 70), Primary);
        assert_eq!(app.engine.document().revision(), revision + 1);
        let b = brush();
        // Unturned: exactly the brush around the click (handle at (3, 2)).
        assert_eq!(painted(&app, (80, 70)), b.get(3, 2));
        assert_eq!(painted(&app, (78, 69)), b.get(1, 1));
        assert_eq!(painted(&app, (77, 68)), Some(0), "its transparent corner");
        // U undoes the stamp.
        ui_pass(&mut app, &ctx, 0.4, tap(egui::Key::U));
        assert_eq!(painted(&app, (80, 70)), Some(0));
        // A quarter turn about z (Shift-2): pixel (u, v) from the handle lands at (-v, u).
        ui_pass(&mut app, &ctx, 0.5, pad(2, true));
        click(&mut app, &ctx, 0.6, (150, 120), Primary);
        assert_eq!(painted(&app, (150 + 1, 120 - 2)), b.get(1, 1));
        assert_eq!(painted(&app, (150 - 1, 120 + 2)), b.get(5, 3));
        assert!(app.perspective.active);
        // A click on a tool ends the mode and picks the tool.
        let line = tool_at(&app, Tool::Line);
        click(&mut app, &ctx, 0.7, line, Primary);
        assert!(!app.perspective.active);
        assert_eq!(app.ui.tool, Tool::Line);
        // The turn stays for next time.
        assert_eq!(app.perspective.orientation.angles(), [0, 0, 90]);
    }

    #[test]
    fn right_click_on_grid_opens_the_perspective_requester_in_the_mode() {
        use egui::PointerButton::{Primary, Secondary};
        let (mut app, ctx) = setup(true);
        let grid = tool_at(&app, Tool::Grid);
        // Outside the mode: the Gridding requester, as before.
        click(&mut app, &ctx, 0.1, grid, Secondary);
        assert!(matches!(
            app.ui.tool_requester,
            Some(ToolRequester::Grid(_))
        ));
        ui_pass(&mut app, &ctx, 0.2, tap(egui::Key::Escape));
        assert!(app.ui.tool_requester.is_none());
        enter_at(&mut app, &ctx, 0.3, (60, 60));
        click(&mut app, &ctx, 0.5, grid, Secondary);
        let Some(ToolRequester::Perspective(r)) = &app.ui.tool_requester else {
            panic!("the Perspective requester");
        };
        // AutoGrid: the grid is the brush, Z = Y.
        assert_eq!(r.settings().grid, [6, 4, 4]);
        assert!(app.perspective.active, "still in the mode");
        assert!(!app.tools.grid, "Grid not toggled");
        // Angle Step 45 (Tab to the fourth field, type), Type Brush, Ok with Return.
        let mut keys = vec![];
        for _ in 0..3 {
            keys.push(key_event(egui::Key::Tab, true));
        }
        for _ in 0..3 {
            keys.push(key_event(egui::Key::Backspace, true));
        }
        keys.push(egui::Event::Text("45".into()));
        ui_pass(&mut app, &ctx, 0.6, keys);
        let (ox, oy) = {
            let req = app.ui.tool_requester.as_ref().unwrap();
            let s = app.view().ui_screen();
            req.origin(s.width, s.height)
        };
        click(&mut app, &ctx, 0.7, (ox + 132, oy + 78), Primary);
        ui_pass(&mut app, &ctx, 0.8, tap(egui::Key::Enter));
        assert!(app.ui.tool_requester.is_none());
        assert_eq!(app.perspective.settings.angle_step, 45);
        assert!(app.perspective.settings.brush_axes);
        ui_pass(&mut app, &ctx, 0.9, pad(8, true));
        assert_eq!(app.perspective.orientation.angles(), [45, 0, 0]);
        // Settings.. from the menu opens it too; Cancel keeps the settings.
        menu(&mut app, &ctx, 1.0, "perspective.settings");
        assert!(matches!(
            app.ui.tool_requester,
            Some(ToolRequester::Perspective(_))
        ));
        ui_pass(&mut app, &ctx, 1.1, tap(egui::Key::Escape));
        assert!(app.ui.tool_requester.is_none());
        assert_eq!(app.perspective.settings.angle_step, 45);
    }

    #[test]
    fn fill_screen_reset_and_center() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (148, 240));
        // Flat on the floor (Shift-7), then keypad `-`: the floor up to the horizon at the
        // centre's height (the middle of the screen, y 128), one undo step.
        ui_pass(&mut app, &ctx, 0.3, pad(7, true));
        let revision = app.engine.document().revision();
        let minus = typed(egui::Key::Minus, egui::Key::Minus, false, "-");
        ui_pass(&mut app, &ctx, 0.4, minus);
        assert_eq!(app.engine.document().revision(), revision + 1);
        let row = |app: &PainterApp, y: i32| (0..320).any(|x| painted(app, (x, y)) != Some(0));
        assert!(row(&app, 200) && row(&app, 140));
        assert!(!row(&app, 128) && !row(&app, 60));
        assert!(app.perspective.active, "the mode stays");
        ui_pass(&mut app, &ctx, 0.5, tap(egui::Key::U));
        assert!(!row(&app, 200));
        // Keypad `.`: a large cross hair follows the pointer; a click places the centre.
        let period = typed(egui::Key::Period, egui::Key::Period, false, ".");
        ui_pass(&mut app, &ctx, 0.6, period);
        assert!(app.perspective.placing);
        let m = vec![moved(&app, (100, 50))];
        ui_pass(&mut app, &ctx, 0.7, m);
        assert_eq!(app.perspective.pointer, Some((100, 50)));
        app.prepare_perspective_view();
        let shown = app.compose_screen();
        assert_eq!(shown.get(10, 50), Some(31), "cross hair across the page");
        assert_eq!(shown.get(100, 200), Some(31));
        let at = app.perspective.position;
        click(&mut app, &ctx, 0.8, (100, 50), Primary);
        assert_eq!(app.perspective.center, Some((100, 50)));
        assert!(!app.perspective.placing);
        assert!(!row(&app, 200), "no stamp");
        assert_eq!(app.perspective.position, at, "the brush stayed");
        // Escape drops a placing; the menu's Center works outside the mode too.
        ui_pass(&mut app, &ctx, 0.9, tap(egui::Key::Enter));
        menu(&mut app, &ctx, 1.0, "perspective.center");
        assert!(app.perspective.placing && !app.perspective.active);
        ui_pass(&mut app, &ctx, 1.1, tap(egui::Key::Escape));
        assert!(!app.perspective.placing);
        // Reset from the menu: angles, settings and distance go back; the centre stays.
        app.perspective.distance = Some(500.0);
        app.perspective.settings.angle_step = 10;
        menu(&mut app, &ctx, 1.2, "perspective.reset");
        assert!(app.perspective.orientation.is_identity());
        assert_eq!(
            app.perspective.settings,
            PerspectiveSettings::defaults((6, 4))
        );
        assert_eq!(app.perspective.distance, None);
        assert_eq!(app.perspective.center, Some((100, 50)));
        // FillScreen from the menu, outside the mode: the brush flat on the screen tiles it.
        menu(&mut app, &ctx, 1.3, "perspective.fill_screen");
        assert!(row(&app, 10) && row(&app, 250));
    }

    #[test]
    fn shift_0_resets_like_the_menu_but_0_keeps_the_distance() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        ui_pass(&mut app, &ctx, 0.3, text(">"));
        ui_pass(&mut app, &ctx, 0.4, text("'"));
        ui_pass(&mut app, &ctx, 0.5, pad(4, false));
        ui_pass(&mut app, &ctx, 0.6, pad(0, false));
        assert!(app.perspective.orientation.is_identity());
        assert_eq!(app.perspective.distance, Some(400.0));
        assert_eq!(app.perspective.position.unwrap()[2], STEP);
        ui_pass(&mut app, &ctx, 0.7, pad(0, true));
        assert_eq!(app.perspective.distance, None);
        assert_eq!(app.perspective.position.unwrap()[2], 0.0);
    }

    #[test]
    fn the_brush_shows_unturned_and_its_wire_frame_always() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        app.prepare_perspective_view();
        let shown = app.compose_screen();
        let b = brush();
        // The brush (u, v = 1, 1 from the handle), the frame one pixel outside, the cross
        // through the handle, the centre's cross hair (all complementary: 31 - colour).
        assert_eq!(shown.get(101, 101), b.get(4, 3));
        assert_eq!(shown.get(100 - 4, 100), Some(31), "frame");
        assert_eq!(shown.get(100, 100 - 3), Some(31), "frame");
        assert_eq!(shown.get(100, 99), Some(31 - b.get(3, 1).unwrap()), "cross");
        assert_eq!(shown.get(160 + 4, 128), Some(31), "centre");
        assert_eq!(painted(&app, (101, 101)), Some(0), "only shown");
        // Turned by a degree: only the wire frame.
        ui_pass(&mut app, &ctx, 0.3, pad(2, false));
        app.prepare_perspective_view();
        let shown = app.compose_screen();
        assert_eq!(shown.get(101, 101), Some(0));
        assert!(
            (96..=104).any(|y| shown.get(96, y) == Some(31)),
            "the frame turned with it"
        );
        // A real pass that draws composes the view and then drops it.
        app.dirty = true;
        ui_pass(&mut app, &ctx, 0.4, vec![]);
        assert!(app.perspective.shown.is_none());
    }

    // ---- Interactions with area stencils, Stencil ▸ Paint, Arrange, Use Brush Palette and
    // Move (review of the wave-7 merge).

    /// An area stencil protecting page pixel (`x`, `y`) on frame 1.
    fn protect(app: &mut PainterApp, x: usize, y: usize) {
        let mut bits = vec![false; 320 * 256];
        bits[y * 320 + x] = true;
        app.menu_data.palette_stencil.area = Some(crate::palette_stencil::Area {
            stencil: dpx_core::AreaStencil::from_mask(320, 256, bits).unwrap(),
            frame: 0,
            on: true,
        });
    }

    #[test]
    fn the_brush_preview_goes_behind_an_area_stencil_as_the_click_does() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(true);
        protect(&mut app, 101, 101);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        app.prepare_perspective_view();
        let shown = app.compose_screen();
        let b = brush();
        assert_eq!(
            shown.get(101, 101),
            Some(0),
            "spared, as the click spares it"
        );
        assert_eq!(shown.get(102, 101), b.get(5, 3));
        click(&mut app, &ctx, 0.2, (100, 100), Primary);
        // Inside the brush (off the cross through its handle) the click paints exactly what
        // was shown.
        for y in [98, 99, 101] {
            for x in [97, 98, 99, 101, 102] {
                assert_eq!(painted(&app, (x, y)), shown.get(x, y), "({x}, {y})");
            }
        }
    }

    #[test]
    fn stencil_paint_in_the_mode_shows_the_brush_in_the_mask_and_the_marks_on_top() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(true);
        app.run(Command::Clear { color: 3 });
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        let ctrl_s = |pressed| egui::Event::Key {
            key: egui::Key::S,
            physical_key: Some(egui::Key::S),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        };
        ui_pass(&mut app, &ctx, 0.2, vec![ctrl_s(true), ctrl_s(false)]);
        assert!(app.stencil_painting() && app.perspective.active);
        let dim = dpx_core::stencil::half_bright_table(app.palette())[3];
        assert_ne!(dim, 3);
        app.prepare_perspective_view();
        app.prepare_anim_view();
        app.prepare_stencil_view();
        let shown = app.compose_screen();
        // The dimmed picture, the brush where a click stamps it into the mask (bright), and
        // the wire frame and the centre's cross hair on top (complementary colours).
        assert_eq!(shown.get(120, 120), Some(dim));
        assert_eq!(
            shown.get(101, 101),
            Some(3),
            "the brush's shape in the mask"
        );
        assert_eq!(shown.get(100 - 4, 100), Some(31 - dim), "frame");
        assert_eq!(shown.get(160 + 4, 128), Some(31 - dim), "centre");
        // The click stamps into the mask; the picture stays.
        let revision = app.engine.document().revision();
        click(&mut app, &ctx, 0.3, (100, 100), Primary);
        assert_eq!(app.engine.document().revision(), revision);
        let p = app.menu_data.palette_stencil.painting.as_ref().unwrap();
        let mask = dpx_core::AreaStencil::from_image(p.engine.document().page());
        assert!(
            mask.protects(101, 101) && !mask.protects(97, 98),
            "its shape"
        );
    }

    #[test]
    fn arrange_and_use_brush_palette_keep_perspective_mode() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        // Arrange renumbers the brushes in place: no brush was picked up or loaded.
        let order: Vec<u8> = (0..32u8).rev().collect();
        app.apply_arrange(&order);
        ui_pass(&mut app, &ctx, 0.2, vec![]);
        assert!(app.perspective.active, "after Arrange");
        // Use Brush Palette gives a loaded brush the file's colour numbers in place.
        let custom = app.custom_brush.clone().unwrap();
        let mut own = custom.clone();
        for p in &mut own.pixels {
            if Some(*p) != own.transparent {
                *p = 1;
            }
        }
        app.menu_data.palette_stencil.loaded = Some(crate::palette_stencil::LoadedBrush {
            brush: own.clone(),
            palette: app.palette().clone(),
            ranges: Vec::new(),
            fitted: custom,
        });
        app.run_action(dpx_ui::Action::Command("palette.use_brush"), &ctx);
        assert_eq!(app.custom_brush.as_ref(), Some(&own));
        assert_eq!(app.picked_brush.as_ref(), Some(&own));
        ui_pass(&mut app, &ctx, 0.3, vec![]);
        assert!(app.perspective.active, "after Use Brush Palette");
    }

    #[test]
    fn a_move_after_a_perspective_stamp_starts_from_the_turned_brush() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 60));
        // Turned a quarter about Z and a degree about X, and pushed 8 pixels away: the
        // Moving Ground recipe (AGA p.192) turns, stamps, undoes and moves.
        ui_pass(&mut app, &ctx, 0.2, pad(2, true));
        ui_pass(&mut app, &ctx, 0.3, pad(7, false));
        ui_pass(&mut app, &ctx, 0.4, text("'"));
        ui_pass(&mut app, &ctx, 0.45, text("'"));
        assert_eq!(app.perspective.orientation.angles(), [-1, 0, 90]);
        click(&mut app, &ctx, 0.5, (100, 60), Primary);
        let stamped = app.engine.document().page().clone();
        assert!(stamped.pixels().iter().any(|&c| c != 0));
        ui_pass(&mut app, &ctx, 0.6, tap(egui::Key::U));
        assert!(painted(&app, (100, 60)) == Some(0));
        // A move of no distance, one stamp: exactly the stamp again.
        app.anim_control.settings = Some(dpx_core::anim_move::MovePath {
            count: 1,
            ..Default::default()
        });
        let shift_m = |pressed| egui::Event::Key {
            key: egui::Key::M,
            physical_key: Some(egui::Key::M),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        };
        ui_pass(&mut app, &ctx, 0.7, vec![shift_m(true), shift_m(false)]);
        assert!(app.ui.anim_control.is_some(), "the Move requester");
        ui_pass(&mut app, &ctx, 0.8, tap(egui::Key::Enter));
        assert!(app.ui.anim_control.is_none(), "drawn");
        assert_eq!(app.engine.document().page(), &stamped);
    }

    #[test]
    fn the_mouse_slides_the_brush_on_its_plane_and_ctrl_fixes_y() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (160, 200));
        let m = vec![moved(&app, (150, 210))];
        ui_pass(&mut app, &ctx, 0.3, m);
        assert_eq!(app.perspective.position, Some([150.5, 210.5, 0.0]));
        // Ctrl held: the mouse moving up moves the brush away along z, on its height.
        let ctrl = vec![
            egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
            moved(&app, (150, 180)),
        ];
        ui_pass(&mut app, &ctx, 0.4, ctrl);
        let p = app.perspective.position.unwrap();
        assert_eq!(p[1], 210.5);
        assert!(p[2] > 50.0, "{p:?}");
        let up = vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
        ui_pass(&mut app, &ctx, 0.45, up);
        assert!(!app.perspective.ctrl);
        // With Grid on (`g`), the brush steps by the grid (the brush size) along its axes.
        ui_pass(&mut app, &ctx, 0.5, text("g"));
        assert!(app.tools.grid && app.perspective.active);
        let m = vec![moved(&app, (150, 210))];
        ui_pass(&mut app, &ctx, 0.6, m);
        let p = app.perspective.position.unwrap();
        assert_eq!(
            ((p[0] - 0.5) % 6.0, (p[1] - 0.5) % 4.0),
            (0.0, 0.0),
            "{p:?}"
        );
    }

    #[test]
    fn enter_needs_a_brush_and_waits_for_text() {
        use egui::PointerButton::Primary;
        let (mut app, ctx) = setup(false);
        ui_pass(&mut app, &ctx, 0.1, tap(egui::Key::Enter));
        assert!(!app.perspective.active, "no custom brush");
        // Do and FillScreen are ghosted without one (the DeluxePaint way).
        let c = app.ui.menu_context();
        assert!(!dpx_ui::Action::Command("perspective.do").enabled(&c));
        assert!(!dpx_ui::Action::Command("perspective.fill_screen").enabled(&c));
        assert!(dpx_ui::Action::Command("perspective.center").enabled(&c));
        app.run_action(dpx_ui::Action::Command("perspective.do"), &ctx);
        assert!(!app.perspective.active);
        // Typing text: Return is a new line.
        app.custom_brush = Some(brush());
        ui_pass(&mut app, &ctx, 0.2, text("t"));
        click(&mut app, &ctx, 0.3, (50, 50), Primary);
        assert!(app.typing());
        ui_pass(&mut app, &ctx, 0.4, tap(egui::Key::Enter));
        assert!(!app.perspective.active && app.typing());
    }

    #[test]
    fn a_new_brush_or_no_brush_ends_the_mode() {
        let (mut app, ctx) = setup(true);
        enter_at(&mut app, &ctx, 0.1, (100, 100));
        // A handle change keeps it (the tutorial uses Alt-x in the mode, AGA p.142) ...
        app.custom_brush = Some(dpx_core::brush_ops::handle_next_corner(&brush()));
        ui_pass(&mut app, &ctx, 0.3, vec![]);
        assert!(app.perspective.active);
        // ... a new pick-up or load ends it (AGA p.141), and so does no brush.
        let mut other = brush();
        other.pixels[5] = 9;
        app.picked_brush = Some(other);
        app.tools.brush_gen += 1;
        ui_pass(&mut app, &ctx, 0.4, vec![]);
        assert!(!app.perspective.active);
        ui_pass(&mut app, &ctx, 0.5, tap(egui::Key::Enter));
        assert!(app.perspective.active);
        app.custom_brush = None;
        ui_pass(&mut app, &ctx, 0.6, vec![]);
        assert!(!app.perspective.active);
    }

    #[test]
    fn spec_stamps_and_fills() {
        let mut engine = Engine::new(dpx_core::Document::new(
            320,
            256,
            dpx_core::Palette::dpaint_default_32(),
        ));
        let spec = "center=160,100;fill:-90,0,0@160,250;stamp:0,0,90@40,40;stamp:0,0,0@20,20,320";
        run_spec(&mut engine, &brush(), spec, (320, 256)).unwrap();
        let page = engine.document().page();
        assert!((0..320).any(|x| page.get(x, 200) != Some(0)));
        assert!(
            (60..320).all(|x| page.get(x, 90) == Some(0)),
            "above the horizon"
        );
        assert_eq!(page.get(41, 38), brush().get(1, 1), "turned stamp");
        assert!(run_spec(&mut engine, &brush(), "spin:1,2,3@4,5", (320, 256)).is_err());
        assert!(run_spec(&mut engine, &brush(), "stamp:1,2@4,5", (320, 256)).is_err());
    }
}
