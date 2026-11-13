//! The layer panel in the app (ADR-44, part 2): Prefs ▸ Layers and the key `L` show or hide
//! it; its rows select a layer, show or hide it (the eye box) and rename it (double-click on
//! the name); its buttons add, copy, delete, move and merge layers. Every change is a layer
//! command of the engine (one undo step each; selecting is a setting like Go To Frame), so
//! `U` and Shift-`U` undo and redo them as any other step.
//!
//! Where it sits: in a full-screen or maximized window with room right of the screen it is
//! docked there, in the work area beside the picture (a second texture, drawn right of the
//! screen); otherwise it lies over the screen's right edge, left of the toolbox. While a
//! requester, note, Show Page or playback is open it is hidden (the docked strip stays as an
//! empty work area, so the screen does not jump) and takes no clicks. A click that changes the
//! document ends Stencil ▸ Paint first, as a menu item does. While the panel is off and the
//! picture has layers, the title bar names the active layer.
//!
//! The panel's look and hit areas live in `dpx_ui::layer_panel`; this module keeps it in step
//! with the document, decides where it goes and turns its clicks and keys into commands.

use dpx_core::layer::is_default_stack;
use dpx_core::{Command, Engine, Request};
use dpx_ui::file_requester::DOUBLE_CLICK_MS;
use dpx_ui::layer_panel::{self, BUTTONS, Layout};
use dpx_ui::{
    Hit, LayerButton, LayerHit, LayerPanel, LayerRow, Pens, Pref, RenameStep, ReqKey, UiState,
};
use eframe::egui;

use crate::{PainterApp, View};

/// App state of the layer panel that is not drawn.
#[derive(Default)]
pub struct State {
    /// The last press on a layer's row and when (egui time, ms), for double-clicks.
    last_click: Option<(usize, u64)>,
    /// This pass's time (egui time, ms).
    now_ms: u64,
    /// The pointer is over the panel: the wheel scrolls its list instead of the page.
    over: bool,
    /// Wheel movement over the panel not yet turned into whole rows.
    wheel: f32,
    /// The docked strip as uploaded.
    texture: Option<egui::TextureHandle>,
    /// The docked strip's width (physical pixels) this pass's screen was laid out with.
    laid_out: i32,
    /// The document the panel and the title-bar label were last brought up to date with
    /// ([`document_key`]).
    synced: Option<(u64, u64)>,
}

/// Which document is shown and how far it has changed: its engine (`Engine::id`, another
/// one after Load or a swap of the pages) and its revision (every change, Undo and selecting
/// a layer count).
fn document_key(engine: &Engine) -> (u64, u64) {
    (engine.id(), engine.document().revision())
}

/// The command a panel button runs on a document whose active layer is `active`, if there is
/// one (Dn on the bottom layer has none).
fn button_command(button: LayerButton, active: usize) -> Option<Command> {
    let layer = active as u32;
    Some(match button {
        LayerButton::New => Command::AddLayer { name: None },
        LayerButton::Copy => Command::DuplicateLayer { layer },
        LayerButton::Delete => Command::DeleteLayer { layer },
        LayerButton::Up => Command::MoveLayer {
            from: layer,
            to: layer + 1,
        },
        LayerButton::Down => Command::MoveLayer {
            from: layer,
            to: layer.checked_sub(1)?,
        },
        LayerButton::Merge => Command::MergeLayerDown { layer },
    })
}

/// Where the panel goes on a `w`×`h` UI screen.
fn layout(ui: &UiState, docked: bool, (w, h): (i32, i32)) -> Layout {
    if docked {
        Layout::docked(w, h)
    } else {
        Layout::overlay(
            w,
            h,
            ui.toolbox_visible,
            ui.title_visible,
            ui.anim_panel.is_some(),
        )
    }
}

/// Brings `ui`'s layer panel and title-bar label in step with the document: the panel while
/// Prefs ▸ Layers is on (`docked` right of the screen, `hidden` under a requester), its
/// buttons ghosted where the engine would refuse their command (a dry run); the label while
/// it is off and the picture has more than the one visible "Background" layer.
pub fn sync(engine: &mut Engine, ui: &mut UiState, docked: bool, hidden: bool, screen: (i32, i32)) {
    let doc = engine.document();
    let active = doc.active_layer();
    let layers = doc.layers().to_vec();
    if !ui.prefs.get(Pref::LayerPanel) {
        ui.layer_panel = None;
        ui.layer_label = (!is_default_stack(&layers)).then(|| layers[active].name.clone());
        return;
    }
    ui.layer_label = None;
    let enabled = BUTTONS.map(|b| {
        button_command(b, active).is_some_and(|command| {
            engine
                .execute(Request {
                    command,
                    expected_revision: None,
                    dry_run: true,
                })
                .is_ok()
        })
    });
    let capacity = layout(ui, docked, screen).capacity;
    let rows = layers
        .into_iter()
        .map(|l| LayerRow {
            name: l.name,
            visible: l.visible,
        })
        .collect();
    let panel = ui.layer_panel.get_or_insert_with(LayerPanel::default);
    panel.docked = docked;
    panel.hidden = hidden;
    panel.update(rows, active, enabled, capacity);
}

/// `--layers-panel [docked]` for snapshots: the panel over the screen's right edge, or docked
/// right of it. The title-bar label follows the document as in the window.
pub fn snapshot_ui(args: &[String], engine: &mut Engine, ui: &mut UiState, view: View) {
    let flag = args.iter().position(|a| a == "--layers-panel");
    let docked = flag
        .and_then(|i| args.get(i + 1))
        .is_some_and(|v| v == "docked");
    ui.prefs.set(Pref::LayerPanel, flag.is_some());
    sync(engine, ui, docked, false, view.ui_size());
}

/// A snapshot (`w`×`h` RGBA, each screen pixel `scale`×`fx` by `scale`×`fy` pixels) with the
/// docked panel appended at its right, as the window shows it.
pub fn with_docked_strip(
    (w, h, rgba): (u32, u32, Vec<u8>),
    palette: &dpx_core::Palette,
    ui: &UiState,
    view: View,
    scale: usize,
) -> (u32, u32, Vec<u8>) {
    let Some(panel) = ui.layer_panel.as_ref().filter(|p| p.docked) else {
        return (w, h, rgba);
    };
    let pens = Pens::from_palette(palette);
    let strip = panel.render_strip(&pens, view.ui_size().1);
    let (sx, sy) = (
        scale * (view.ux * view.fx) as usize,
        scale * (view.uy * view.fy) as usize,
    );
    let strip_rgba = dpx_ui::to_rgba(&strip, palette, sx, sy);
    let strip_w = strip.width() as usize * sx;
    debug_assert_eq!(strip.height() as usize * sy, h as usize);
    let (w0, wide) = (w as usize, w as usize + strip_w);
    let mut out = Vec::with_capacity(wide * h as usize * 4);
    for y in 0..h as usize {
        out.extend_from_slice(&rgba[y * w0 * 4..(y + 1) * w0 * 4]);
        out.extend_from_slice(&strip_rgba[y * strip_w * 4..(y + 1) * strip_w * 4]);
    }
    (wide as u32, h, out)
}

impl PainterApp {
    /// The layer panel is on (Prefs ▸ Layers).
    pub(crate) fn layer_panel_on(&self) -> bool {
        self.ui.prefs.get(Pref::LayerPanel)
    }

    /// Width (physical pixels) of the docked panel right of the screen at `scale`; 0 while it
    /// is not docked. It docks in a full-screen or maximized window (the screen is centred
    /// there, ADR-13) when screen and panel fit side by side at the screen's pixel size; the
    /// Screen Format requester's own screen has none.
    pub(crate) fn layer_strip_px(&self, view: View, scale: f32) -> i32 {
        let Some(win) = self.window else {
            return 0;
        };
        if !self.layer_panel_on() || !self.free || self.requester.is_some() {
            return 0;
        }
        // ADR-47: the Modern window has no strip beside the screen (its layer column comes
        // with a later slice; until then the panel lies over the picture as in a window).
        if self.is_modern() {
            return 0;
        }
        let img_w = ((view.screen_w * view.fx) as f32 * scale).round() as i32;
        let strip =
            ((layer_panel::geom::PANEL_W * view.ux * view.fx) as f32 * scale).round() as i32;
        if img_w + strip <= win.w { strip } else { 0 }
    }

    /// [`Self::layer_strip_px`] for this pass's layout (`ui()` centres screen and strip with
    /// it); remembered, so a change later in the pass asks for another one.
    pub(crate) fn lay_out_layer_strip(&mut self, view: View, scale: f32) -> i32 {
        self.layers.laid_out = self.layer_strip_px(view, scale);
        self.layers.laid_out
    }

    /// Once per pass, first: the time for double-clicks, and a held button goes up with the
    /// mouse button.
    pub(crate) fn layers_pass(&mut self, ctx: &egui::Context) {
        let (time, down) = ctx.input(|i| (i.time, i.pointer.any_down()));
        self.layers.now_ms = (time * 1000.0) as u64;
        if !down
            && let Some(panel) = &mut self.ui.layer_panel
            && panel.held.take().is_some()
        {
            self.dirty = true;
        }
    }

    /// Before the screen is drawn (wave-8 integration): a document that changed since the
    /// panel and the title-bar label were last brought up to date asks for a redraw in this
    /// pass, whoever changed it: a layer tool or command of an MCP client (ADR-43), a
    /// generated picture placed as a layer (ADR-45), Undo, a new picture or the other page
    /// swapped in (another engine). Paths that set `dirty` themselves need nothing more.
    pub(crate) fn follow_document(&mut self) {
        if self.layers.synced != Some(document_key(&self.engine)) {
            self.dirty = true;
        }
    }

    /// Before the screen is composed: the panel and the title-bar label follow the document.
    pub(crate) fn sync_layer_panel(&mut self) {
        let (view, scale) = self.layout();
        let docked = self.layer_strip_px(view, scale) > 0;
        let hidden = self.modal_open();
        sync(
            &mut self.engine,
            &mut self.ui,
            docked,
            hidden,
            view.ui_size(),
        );
        self.layers.synced = Some(document_key(&self.engine));
    }

    /// Takes the name being edited, as a click elsewhere does: an MCP change runs after it,
    /// so the rename goes to the layer it was opened on (the change may renumber layers).
    pub(crate) fn finish_layer_rename(&mut self) {
        if let Some((layer, name)) = self
            .ui
            .layer_panel
            .as_mut()
            .and_then(LayerPanel::finish_rename)
        {
            self.rename_layer(layer, name);
            self.dirty = true;
        }
    }

    /// The pointer is over `hit` (the hover branch of `ui()`).
    pub(crate) fn layer_hover(&mut self, hit: Option<Hit>) {
        self.layers.over = matches!(hit, Some(Hit::LayerPanel(_)));
    }

    /// A press on the panel. The left button acts; the right one does nothing there. A
    /// press that can change the document (a button that works, an eye box, a row) ends
    /// Stencil ▸ Paint first, as a menu item does, so the panel's command goes to the picture
    /// and `U` undoes it, not a stroke of the mask.
    pub(crate) fn layer_panel_press(&mut self, hit: LayerHit, right: bool) {
        if right {
            return;
        }
        let Some(panel) = &self.ui.layer_panel else {
            return;
        };
        let acts = match hit {
            LayerHit::Button(b) => panel.enabled(b),
            LayerHit::Eye(_) | LayerHit::Name(_) => true,
            _ => false,
        };
        if acts && self.stencil_painting() {
            self.leave_stencil_paint();
        }
        let Some(panel) = &self.ui.layer_panel else {
            return;
        };
        let doc = self.engine.document();
        let active = doc.active_layer();
        match hit {
            LayerHit::Button(b) => {
                if !panel.enabled(b) {
                    return;
                }
                let command = button_command(b, active);
                if let Some(panel) = &mut self.ui.layer_panel {
                    panel.held = Some(b);
                }
                if let Some(command) = command {
                    self.run(command);
                }
            }
            LayerHit::Eye(layer) => {
                let visible = doc.layers().get(layer).is_some_and(|l| l.visible);
                self.run(Command::SetLayerVisible {
                    layer: layer as u32,
                    visible: !visible,
                });
            }
            LayerHit::Name(layer) => {
                let now = self.layers.now_ms;
                let double = matches!(self.layers.last_click,
                    Some((l, t)) if l == layer && now.saturating_sub(t) <= DOUBLE_CLICK_MS);
                self.layers.last_click = (!double).then_some((layer, now));
                if layer != active {
                    self.run(Command::SelectLayer {
                        layer: layer as u32,
                    });
                }
                if double && let Some(panel) = &mut self.ui.layer_panel {
                    panel.begin_rename(layer);
                }
            }
            LayerHit::ScrollUp | LayerHit::ScrollDown => {
                let step = if hit == LayerHit::ScrollUp { -1 } else { 1 };
                self.scroll_layer_list(step);
            }
            LayerHit::Field | LayerHit::Background => {}
        }
        self.dirty = true;
    }

    fn scroll_layer_list(&mut self, rows: i32) {
        let screen = self.view().ui_size();
        let Some(panel) = &self.ui.layer_panel else {
            return;
        };
        let capacity = layout(&self.ui, panel.docked, screen).capacity;
        if let Some(panel) = &mut self.ui.layer_panel {
            panel.scroll_by(rows, capacity);
        }
        self.dirty = true;
    }

    /// The wheel while the pointer is over the panel scrolls its list (one row per notch);
    /// returns whether it took the wheel event (the page does not scroll then).
    pub(crate) fn layer_wheel(&mut self, unit: egui::MouseWheelUnit, delta: egui::Vec2) -> bool {
        if !self.layers.over || self.ui.layer_panel.is_none() {
            return false;
        }
        // Positive y moves the content down, i.e. shows the rows above.
        self.layers.wheel -= match unit {
            egui::MouseWheelUnit::Point => delta.y / 20.0,
            egui::MouseWheelUnit::Line => delta.y,
            egui::MouseWheelUnit::Page => delta.y * 8.0,
        };
        let rows = self.layers.wheel.trunc();
        if rows != 0.0 {
            self.layers.wheel -= rows;
            self.scroll_layer_list(rows as i32);
        }
        true
    }

    /// The layer being renamed.
    pub(crate) fn renaming_layer(&self) -> Option<usize> {
        self.ui.layer_panel.as_ref().and_then(LayerPanel::renaming)
    }

    /// A press while a name is being edited: anywhere but the gadget it takes the name (as
    /// Return does) and does nothing else. Returns whether it took the press.
    pub(crate) fn layer_rename_press(&mut self, hit: Option<Hit>) -> bool {
        if self.renaming_layer().is_none() {
            return false;
        }
        if hit != Some(Hit::LayerPanel(LayerHit::Field))
            && let Some((layer, name)) = self
                .ui
                .layer_panel
                .as_mut()
                .and_then(LayerPanel::finish_rename)
        {
            self.rename_layer(layer, name);
        }
        self.dirty = true;
        true
    }

    /// While a name is being edited, every key of this pass goes to the gadget (Return takes
    /// the name, Esc keeps the old one) and no other key handler sees it. Returns whether the
    /// gadget took the keys.
    pub(crate) fn layer_rename_keys(&mut self, ctx: &egui::Context) -> bool {
        if self.renaming_layer().is_none() {
            return false;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let keys: Vec<ReqKey> = match event {
                egui::Event::Text(text) => text.chars().map(ReqKey::Char).collect(),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !(modifiers.command || modifiers.ctrl) => match key {
                    egui::Key::ArrowLeft => vec![ReqKey::Left],
                    egui::Key::ArrowRight => vec![ReqKey::Right],
                    egui::Key::Home => vec![ReqKey::Home],
                    egui::Key::End => vec![ReqKey::End],
                    egui::Key::Backspace => vec![ReqKey::Backspace],
                    egui::Key::Delete => vec![ReqKey::Delete],
                    egui::Key::Enter => vec![ReqKey::Enter],
                    egui::Key::Escape => vec![ReqKey::Escape],
                    _ => continue,
                },
                _ => continue,
            };
            for key in keys {
                let Some(panel) = &mut self.ui.layer_panel else {
                    return false;
                };
                match panel.rename_key(key) {
                    RenameStep::Editing | RenameStep::Cancelled => {}
                    RenameStep::Done(layer, name) => self.rename_layer(layer, name),
                }
                self.dirty = true;
                if self.renaming_layer().is_none() {
                    // The rest of this pass's keys are paint-screen keys again next pass.
                    return true;
                }
            }
        }
        true
    }

    /// Renames layer `layer` to `name`, unless the name is empty or unchanged (no undo step
    /// then).
    fn rename_layer(&mut self, layer: usize, name: String) {
        let doc = self.engine.document();
        let unchanged = doc.layers().get(layer).is_none_or(|l| l.name == name);
        if unchanged || name.trim().is_empty() {
            return;
        }
        self.run(Command::RenameLayer {
            layer: layer as u32,
            name,
        });
    }

    /// The paint-screen key of the panel: `L` (Shift+L) shows or hides it, as Prefs ▸ Layers.
    pub(crate) fn layer_keys(&mut self, ctx: &egui::Context) {
        if self.typing() || self.modal_open() {
            return;
        }
        let pressed = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key {
                    key: egui::Key::L,
                    pressed: true,
                    repeat: false,
                    modifiers: m,
                    ..
                } if m.shift && !(m.ctrl || m.alt || m.command || m.mac_cmd))
            })
        });
        if pressed {
            self.ui.prefs.toggle(Pref::LayerPanel);
            self.dirty = true;
        }
    }

    /// Uploads the docked strip (with the screen, when the screen changed); drops it while
    /// the panel is not docked.
    pub(crate) fn upload_layer_strip(&mut self, ctx: &egui::Context) {
        let Some(panel) = self.ui.layer_panel.as_ref().filter(|p| p.docked) else {
            self.layers.texture = None;
            return;
        };
        let palette = self.palette();
        let strip = panel.render_strip(&Pens::from_palette(palette), self.view().ui_size().1);
        let rgba = dpx_ui::to_rgba(&strip, palette, 1, 1);
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [strip.width() as usize, strip.height() as usize],
            &rgba,
        );
        match &mut self.layers.texture {
            Some(t) => t.set(image, egui::TextureOptions::NEAREST),
            None => {
                self.layers.texture =
                    Some(ctx.load_texture("layers", image, egui::TextureOptions::NEAREST));
            }
        }
    }

    /// Paints the docked strip right of the screen drawn at `rect` (points), as many UI
    /// pixels wide as the panel, at the screen's pixel size. When the panel docked or went in
    /// this pass (a menu item, a key, a requester's screen), the screen was laid out without
    /// it: nothing is painted and another pass lays both out.
    pub(crate) fn paint_layer_strip(&self, painter: &egui::Painter, rect: egui::Rect) {
        let (view, scale) = self.layout();
        if self.layer_strip_px(view, scale) != self.layers.laid_out {
            painter.ctx().request_repaint();
            return;
        }
        let (Some(t), Some(_), true) = (
            &self.layers.texture,
            self.ui.layer_panel.as_ref().filter(|p| p.docked),
            self.layers.laid_out > 0,
        ) else {
            return;
        };
        let per_ui = rect.width() / self.view().ui_size().0 as f32;
        let strip = egui::Rect::from_min_size(
            rect.right_top(),
            egui::vec2(layer_panel::geom::PANEL_W as f32 * per_ui, rect.height()),
        );
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(t.id(), strip, uv, egui::Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{click_ui, key_event, pick, pointer_button, ui_pass, ui_point};
    use dpx_core::Shape;
    use dpx_ui::menu::{self, Action, MENUS};
    use dpx_ui::{Tool, layer_panel::Layout};
    use egui::PointerButton::Primary;

    fn rect(from: (i32, i32), to: (i32, i32), color: u8) -> Command {
        Command::Shape {
            shape: Shape::Rect { from, to },
            brush: vec![],
            color,
            filled: true,
            symmetry: None,
            custom: None,
            mode: Default::default(),
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    /// The demo of the snapshot: Background in blue (6) with Hill, a green (12) rectangle at
    /// the bottom, above it, and Sky, a white (1) square, on top; Hill active. One pass has
    /// laid the screen out.
    fn three_layers(ctx: &egui::Context) -> PainterApp {
        let mut app = PainterApp::new();
        let e = &mut app.engine;
        e.execute(Command::Clear { color: 6 }).unwrap();
        e.execute(Command::AddLayer {
            name: Some("Hill".into()),
        })
        .unwrap();
        e.execute(rect((0, 150), (199, 199), 12)).unwrap();
        e.execute(Command::AddLayer {
            name: Some("Sky".into()),
        })
        .unwrap();
        e.execute(rect((20, 20), (39, 39), 1)).unwrap();
        e.execute(Command::SelectLayer { layer: 1 }).unwrap();
        app.dirty = true;
        ui_pass(&mut app, ctx, 0.0, vec![]);
        app
    }

    fn names(app: &PainterApp) -> Vec<String> {
        let doc = app.engine.document();
        doc.layers().iter().map(|l| l.name.clone()).collect()
    }

    /// The overlay's layout on the current screen.
    fn panel_layout(app: &PainterApp) -> Layout {
        let (w, h) = app.view().ui_size();
        Layout::of(&app.ui, w, h).expect("the panel shows")
    }

    fn panel(app: &PainterApp) -> &LayerPanel {
        app.ui.layer_panel.as_ref().expect("the panel")
    }

    /// The UI pixel in the middle of `r`.
    fn mid(r: dpx_core::Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    /// Clicks the panel's `button` at `t`.
    fn press(app: &mut PainterApp, ctx: &egui::Context, t: f64, button: LayerButton) {
        let at = mid(panel_layout(app).button(button));
        click_ui(app, ctx, t, at, Primary);
    }

    /// Clicks the row of `layer`: its name, or its eye box with `eye`.
    fn row(app: &mut PainterApp, ctx: &egui::Context, t: f64, layer: usize, eye: bool) {
        let l = panel_layout(app);
        let slot = panel(app)
            .slot_of(layer, l.capacity)
            .expect("the row shows");
        let at = if eye {
            mid(l.eye(slot))
        } else {
            mid(l.name(slot))
        };
        click_ui(app, ctx, t, at, Primary);
    }

    fn shift_l() -> Vec<egui::Event> {
        let event = |pressed| egui::Event::Key {
            key: egui::Key::L,
            physical_key: Some(egui::Key::L),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        };
        vec![event(true), egui::Event::Text("L".into()), event(false)]
    }

    fn undo(app: &mut PainterApp, ctx: &egui::Context, t: f64) {
        ui_pass(app, ctx, t, vec![key_event(egui::Key::U, true)]);
    }

    /// Shift+U (redo), Shift held as egui-winit reports it; let go in the next pass.
    fn redo(app: &mut PainterApp, ctx: &egui::Context, t: f64) {
        let shift = egui::Modifiers::SHIFT;
        let key = egui::Event::Key {
            key: egui::Key::U,
            physical_key: Some(egui::Key::U),
            pressed: true,
            repeat: false,
            modifiers: shift,
        };
        let events = vec![
            egui::Event::ModifiersChanged(shift),
            key,
            egui::Event::Text("U".into()),
        ];
        ui_pass(app, ctx, t, events);
        let up = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
        ui_pass(app, ctx, t + 0.01, vec![up]);
    }

    /// The app with the panel on (the key `L`).
    fn with_panel(ctx: &egui::Context) -> PainterApp {
        let mut app = three_layers(ctx);
        ui_pass(&mut app, ctx, 0.1, shift_l());
        assert!(app.layer_panel_on());
        app
    }

    /// UI pixels over the Prefs title and over Prefs ▸ Layers.
    fn layers_item_path(app: &PainterApp) -> [(i32, i32); 2] {
        let (w, h) = app.view().ui_size();
        let wanted = Action::Pref(Pref::LayerPanel);
        let m = MENUS.iter().position(|m| m.title == "Prefs").unwrap();
        let item = MENUS[m]
            .items
            .iter()
            .position(|it| matches!(it.entry, menu::Entry::Action(a) if a == wanted))
            .expect("Prefs ▸ Layers");
        let title_x = (0..w)
            .find(|&x| {
                let mut state = dpx_ui::MenuState::default();
                state.open(w, h, x, 1);
                state.menu == Some(m)
            })
            .unwrap();
        let mbox = menu::menu_box(m, w, h);
        [(title_x, 1), (mbox.x + mbox.w / 2, mbox.item_top(item) + 1)]
    }

    #[test]
    fn the_key_and_prefs_layers_show_and_hide_the_panel() {
        let ctx = egui::Context::default();
        let mut app = three_layers(&ctx);
        assert!(app.ui.layer_panel.is_none());
        // Off: the title bar names the active layer.
        assert_eq!(app.ui.layer_label.as_deref(), Some("Hill"));
        let ctx_menu = app.ui.menu_context();
        assert_eq!(
            Action::Pref(Pref::LayerPanel).checked(&ctx_menu),
            Some(false)
        );
        ui_pass(&mut app, &ctx, 0.1, shift_l());
        let p = panel(&app);
        assert!(!p.docked && !p.hidden);
        assert_eq!(p.active, 1);
        assert_eq!(p.rows.len(), 3);
        assert_eq!(
            app.ui.layer_label, None,
            "the panel shows the layer instead"
        );
        assert_eq!(
            Action::Pref(Pref::LayerPanel).checked(&app.ui.menu_context()),
            Some(true)
        );
        // Drawn over the screen's right edge, left of the toolbox.
        let l = panel_layout(&app);
        assert_eq!(l.panel.x + l.panel.w, 320 - 24);
        let screen = app.compose_screen();
        let pens = Pens::from_palette(app.palette());
        assert_eq!(screen.get(l.panel.x, 100), Some(pens.dark), "its left edge");
        // Prefs ▸ Layers hides it again, through the real menu.
        let path = layers_item_path(&app);
        pick(&mut app, &ctx, 0.3, &path);
        assert!(!app.layer_panel_on());
        assert!(app.ui.layer_panel.is_none());
        assert_eq!(app.ui.layer_label.as_deref(), Some("Hill"));
        // Toggling is no undo step: U undoes the last layer command (Select is none either:
        // the Sky square goes).
        undo(&mut app, &ctx, 0.5);
        assert!(!app.layer_panel_on());
        assert!(
            app.engine
                .document()
                .layer(2)
                .unwrap()
                .pixels()
                .iter()
                .all(|&p| p == 0)
        );
        // A single default layer: no label.
        let mut plain = PainterApp::new();
        ui_pass(&mut plain, &egui::Context::default(), 0.0, vec![]);
        assert_eq!(plain.ui.layer_label, None);
    }

    #[test]
    fn the_settings_file_keeps_whether_the_panel_is_shown() {
        let ctx = egui::Context::default();
        let app = with_panel(&ctx);
        let json = crate::config::Config::capture(&app).to_json();
        assert_eq!(json["prefs"]["layer_panel"], serde_json::Value::Bool(true));
        let mut fresh = PainterApp::new();
        crate::config::Config::from_json(&json)
            .expect("a settings file")
            .apply(&mut fresh);
        assert!(fresh.layer_panel_on());
        // Older settings files have no such key: off.
        let old = serde_json::json!({"prefs": {"coords": true}});
        let mut fresh = PainterApp::new();
        crate::config::Config::from_json(&old)
            .unwrap()
            .apply(&mut fresh);
        assert!(!fresh.layer_panel_on());
    }

    #[test]
    fn clicking_a_row_selects_and_painting_lands_on_the_active_layer() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        let revision = app.engine.document().revision();
        row(&mut app, &ctx, 0.2, 2, false);
        assert_eq!(app.engine.document().active_layer(), 2, "Sky");
        assert_eq!(panel(&app).active, 2);
        // One setting (the revision counts it, ADR-28), no undo step: U undoes the last step
        // before the click, Sky's square, and Shift-U brings it back.
        assert_eq!(app.engine.document().revision(), revision + 1);
        undo(&mut app, &ctx, 0.25);
        assert_eq!(app.engine.document().layer(2).unwrap().get(30, 30), Some(0));
        redo(&mut app, &ctx, 0.3);
        assert_eq!(app.engine.document().layer(2).unwrap().get(30, 30), Some(1));
        assert_eq!(app.engine.document().active_layer(), 2);
        // Paint on Sky with the freehand tool: only Sky changes.
        app.select_tool(Tool::ContinuousFreehand, false);
        app.ui.fg = 3;
        let (a, b) = (ui_point(&app, (50, 60)), ui_point(&app, (90, 60)));
        ui_pass(&mut app, &ctx, 0.4, vec![egui::Event::PointerMoved(a)]);
        ui_pass(&mut app, &ctx, 0.42, vec![pointer_button(a, Primary, true)]);
        ui_pass(&mut app, &ctx, 0.44, vec![egui::Event::PointerMoved(b)]);
        ui_pass(
            &mut app,
            &ctx,
            0.46,
            vec![pointer_button(b, Primary, false)],
        );
        let doc = app.engine.document();
        assert_eq!(doc.layer(2).unwrap().get(70, 60), Some(3), "on Sky");
        assert_eq!(doc.layer(1).unwrap().get(70, 60), Some(0), "Hill untouched");
        assert_eq!(
            doc.layer(0).unwrap().get(70, 60),
            Some(6),
            "Background untouched"
        );
        assert_eq!(app.compose_screen().get(70, 60), Some(3));
        // Undo takes the stroke back and stays on Sky; select is no step of its own.
        undo(&mut app, &ctx, 0.6);
        let doc = app.engine.document();
        assert_eq!(doc.layer(2).unwrap().get(70, 60), Some(0));
        assert_eq!(doc.active_layer(), 2);
        redo(&mut app, &ctx, 0.7);
        assert_eq!(app.engine.document().layer(2).unwrap().get(70, 60), Some(3));
    }

    #[test]
    fn clicks_on_the_panel_never_paint_the_picture() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        app.select_tool(Tool::ContinuousFreehand, false);
        app.ui.fg = 3;
        let before: Vec<_> = app.engine.document().cels().cloned().collect();
        let l = panel_layout(&app);
        let r5 = l.row(5);
        for (i, at) in [
            (l.panel.x + 40, l.panel.y + 4),
            (r5.x + 30, r5.y + 4),
            (l.panel.x + 1, l.buttons_y),
        ]
        .into_iter()
        .enumerate()
        {
            let pos = ui_point(&app, at);
            let t = 0.2 + i as f64 * 0.1;
            ui_pass(&mut app, &ctx, t, vec![egui::Event::PointerMoved(pos)]);
            ui_pass(
                &mut app,
                &ctx,
                t + 0.02,
                vec![pointer_button(pos, Primary, true)],
            );
            ui_pass(
                &mut app,
                &ctx,
                t + 0.04,
                vec![pointer_button(pos, Primary, false)],
            );
        }
        let after: Vec<_> = app.engine.document().cels().cloned().collect();
        assert_eq!(
            before, after,
            "the panel's gaps, title and empty rows paint nothing"
        );
        assert!(app.stroke.is_none());
    }

    #[test]
    fn new_copy_and_delete_with_undo_and_redo() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        press(&mut app, &ctx, 0.2, LayerButton::New);
        assert_eq!(names(&app), ["Background", "Hill", "Layer 1", "Sky"]);
        assert_eq!(
            app.engine.document().active_layer(),
            2,
            "above Hill, active"
        );
        assert_eq!(panel(&app).held, None, "the button is up again");
        undo(&mut app, &ctx, 0.3);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        assert_eq!(app.engine.document().active_layer(), 1);
        redo(&mut app, &ctx, 0.4);
        assert_eq!(names(&app), ["Background", "Hill", "Layer 1", "Sky"]);
        undo(&mut app, &ctx, 0.45);
        // Copy: Hill's rectangle on a copy above it.
        press(&mut app, &ctx, 0.5, LayerButton::Copy);
        assert_eq!(names(&app), ["Background", "Hill", "Hill copy", "Sky"]);
        let doc = app.engine.document();
        assert_eq!(doc.active_layer(), 2);
        assert_eq!(doc.layer(2).unwrap().get(10, 160), Some(12));
        undo(&mut app, &ctx, 0.6);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        redo(&mut app, &ctx, 0.7);
        assert_eq!(names(&app).len(), 4);
        // Del: the copy goes, the layer below becomes active.
        press(&mut app, &ctx, 0.8, LayerButton::Delete);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        assert_eq!(app.engine.document().active_layer(), 1);
        // Delete Hill too: its rectangle goes from the screen; undo brings it back.
        press(&mut app, &ctx, 0.9, LayerButton::Delete);
        assert_eq!(names(&app), ["Background", "Sky"]);
        assert_eq!(app.compose_screen().get(10, 160), Some(6));
        undo(&mut app, &ctx, 1.0);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        assert_eq!(app.compose_screen().get(10, 160), Some(12));
        redo(&mut app, &ctx, 1.1);
        assert_eq!(names(&app), ["Background", "Sky"]);
    }

    #[test]
    fn up_down_and_ghosted_buttons() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        // Hill (1) up: above Sky, still active.
        press(&mut app, &ctx, 0.2, LayerButton::Up);
        assert_eq!(names(&app), ["Background", "Sky", "Hill"]);
        assert_eq!(app.engine.document().active_layer(), 2);
        // The top layer cannot go up: ghosted, and a click does nothing.
        assert!(!panel(&app).enabled(LayerButton::Up));
        press(&mut app, &ctx, 0.3, LayerButton::Up);
        assert_eq!(names(&app), ["Background", "Sky", "Hill"]);
        undo(&mut app, &ctx, 0.4);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        assert_eq!(app.engine.document().active_layer(), 1);
        redo(&mut app, &ctx, 0.5);
        assert_eq!(names(&app), ["Background", "Sky", "Hill"]);
        undo(&mut app, &ctx, 0.55);
        // Dn to the bottom: Hill becomes the opaque bottom layer.
        press(&mut app, &ctx, 0.6, LayerButton::Down);
        assert_eq!(names(&app), ["Hill", "Background", "Sky"]);
        assert_eq!(app.engine.document().active_layer(), 0);
        let p = panel(&app);
        assert!(!p.enabled(LayerButton::Down) && !p.enabled(LayerButton::Merge));
        assert!(p.enabled(LayerButton::Up) && p.enabled(LayerButton::Delete));
        undo(&mut app, &ctx, 0.7);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        // One layer: Del, Merge and the moves are ghosted; New and Copy work.
        let (mut one, ctx) = (PainterApp::new(), egui::Context::default());
        ui_pass(&mut one, &ctx, 0.0, shift_l());
        let p = panel(&one);
        for b in [
            LayerButton::Delete,
            LayerButton::Merge,
            LayerButton::Up,
            LayerButton::Down,
        ] {
            assert!(!p.enabled(b), "{b:?}");
        }
        assert!(p.enabled(LayerButton::New) && p.enabled(LayerButton::Copy));
        press(&mut one, &ctx, 0.2, LayerButton::Delete);
        assert_eq!(one.engine.document().layer_count(), 1);
        assert!(!one.engine.can_undo());
    }

    #[test]
    fn the_eye_hides_a_layer_from_the_screen_undoably() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        assert_eq!(app.compose_screen().get(30, 30), Some(1), "Sky's square");
        row(&mut app, &ctx, 0.2, 2, true);
        assert!(!app.engine.document().layers()[2].visible);
        assert!(!panel(&app).rows[2].visible);
        assert_eq!(
            app.engine.document().active_layer(),
            1,
            "the eye selects nothing"
        );
        assert_eq!(
            app.compose_screen().get(30, 30),
            Some(6),
            "hidden: not shown"
        );
        undo(&mut app, &ctx, 0.3);
        assert_eq!(app.compose_screen().get(30, 30), Some(1));
        redo(&mut app, &ctx, 0.4);
        assert_eq!(app.compose_screen().get(30, 30), Some(6));
        // And shown again with the eye.
        row(&mut app, &ctx, 0.5, 2, true);
        assert_eq!(app.compose_screen().get(30, 30), Some(1));
    }

    #[test]
    fn merge_down_with_undo_and_redo() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        row(&mut app, &ctx, 0.2, 2, false);
        press(&mut app, &ctx, 0.3, LayerButton::Merge);
        assert_eq!(names(&app), ["Background", "Hill"]);
        let doc = app.engine.document();
        assert_eq!(doc.active_layer(), 1);
        let hill = doc.layer(1).unwrap();
        assert_eq!(hill.get(30, 30), Some(1), "Sky's square merged into Hill");
        assert_eq!(hill.get(10, 160), Some(12));
        assert_eq!(hill.get(100, 100), Some(0), "still transparent between");
        undo(&mut app, &ctx, 0.4);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        assert_eq!(app.engine.document().layer(1).unwrap().get(30, 30), Some(0));
        redo(&mut app, &ctx, 0.5);
        assert_eq!(names(&app), ["Background", "Hill"]);
    }

    #[test]
    fn a_double_click_renames_return_takes_esc_keeps() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        // Two clicks on Sky's name within the double-click time: the gadget opens.
        row(&mut app, &ctx, 0.2, 2, false);
        assert_eq!(app.renaming_layer(), None, "one click selects");
        row(&mut app, &ctx, 0.3, 2, false);
        assert_eq!(app.renaming_layer(), Some(2));
        // Keys go to the gadget, not to the tools ('r' would pick the rectangle, 'u' undo).
        let tool = app.ui.tool;
        let mut keys = vec![];
        for _ in 0..3 {
            keys.push(key_event(egui::Key::Backspace, true));
        }
        keys.push(egui::Event::Text("Sun ru".into()));
        ui_pass(&mut app, &ctx, 0.4, keys);
        assert_eq!(app.ui.tool, tool);
        assert_eq!(
            panel(&app).rename_text().as_deref(),
            Some("Sun ru"),
            "typed"
        );
        ui_pass(
            &mut app,
            &ctx,
            0.5,
            vec![key_event(egui::Key::Backspace, true)],
        );
        ui_pass(
            &mut app,
            &ctx,
            0.6,
            vec![key_event(egui::Key::Backspace, true)],
        );
        ui_pass(
            &mut app,
            &ctx,
            0.7,
            vec![key_event(egui::Key::Backspace, true)],
        );
        ui_pass(&mut app, &ctx, 0.8, vec![key_event(egui::Key::Enter, true)]);
        assert_eq!(app.renaming_layer(), None);
        assert_eq!(names(&app), ["Background", "Hill", "Sun"]);
        undo(&mut app, &ctx, 0.9);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        redo(&mut app, &ctx, 1.0);
        assert_eq!(names(&app), ["Background", "Hill", "Sun"]);
        // Esc keeps the name; a slow second click is no double-click.
        row(&mut app, &ctx, 2.0, 1, false);
        row(&mut app, &ctx, 3.0, 1, false);
        assert_eq!(app.renaming_layer(), None);
        row(&mut app, &ctx, 3.2, 1, false);
        assert_eq!(app.renaming_layer(), Some(1));
        ui_pass(&mut app, &ctx, 3.3, vec![egui::Event::Text("x".into())]);
        ui_pass(
            &mut app,
            &ctx,
            3.4,
            vec![key_event(egui::Key::Escape, true)],
        );
        assert_eq!(app.renaming_layer(), None);
        assert_eq!(names(&app), ["Background", "Hill", "Sun"]);
        // A click elsewhere takes the name (and does nothing else: no stroke on the page).
        row(&mut app, &ctx, 5.0, 0, false);
        row(&mut app, &ctx, 5.2, 0, false);
        assert_eq!(app.renaming_layer(), Some(0));
        ui_pass(&mut app, &ctx, 5.3, vec![egui::Event::Text("!".into())]);
        app.select_tool(Tool::ContinuousFreehand, false);
        let cels: Vec<_> = app.engine.document().cels().cloned().collect();
        click_ui(&mut app, &ctx, 5.4, (100, 100), Primary);
        assert_eq!(app.renaming_layer(), None);
        assert_eq!(names(&app), ["Background!", "Hill", "Sun"]);
        let after: Vec<_> = app.engine.document().cels().cloned().collect();
        assert_eq!(cels, after, "the click only ended the rename");
        // Return on the untouched name, and on a cleared field, rename nothing: no undo step,
        // so U then undoes the rename to "Background!".
        let revision = app.engine.document().revision();
        row(&mut app, &ctx, 6.0, 1, false);
        row(&mut app, &ctx, 6.2, 1, false);
        assert_eq!(app.renaming_layer(), Some(1));
        ui_pass(&mut app, &ctx, 6.3, vec![key_event(egui::Key::Enter, true)]);
        assert_eq!(app.renaming_layer(), None);
        row(&mut app, &ctx, 7.0, 1, false);
        row(&mut app, &ctx, 7.2, 1, false);
        let clear = (0..4)
            .map(|_| key_event(egui::Key::Backspace, true))
            .collect();
        ui_pass(&mut app, &ctx, 7.3, clear);
        assert_eq!(panel(&app).rename_text().as_deref(), Some(""));
        ui_pass(&mut app, &ctx, 7.4, vec![key_event(egui::Key::Enter, true)]);
        assert_eq!(names(&app), ["Background!", "Hill", "Sun"]);
        // The clicks selected Hill (a setting: the revision counts it, the undo list not).
        assert_eq!(app.engine.document().revision(), revision + 1);
        undo(&mut app, &ctx, 7.6);
        assert_eq!(names(&app), ["Background", "Hill", "Sun"]);
    }

    #[test]
    fn opening_and_closing_the_gadget_keeps_a_name_the_font_cannot_show() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        // The core (and MCP, and a loaded project) take any name.
        app.run(Command::RenameLayer {
            layer: 2,
            name: "Caf\u{e9} \u{2601}".into(),
        });
        app.dirty = true;
        ui_pass(&mut app, &ctx, 0.1, vec![]);
        let revision = app.engine.document().revision();
        // Double-click, Return without typing: the name and the undo list stay.
        row(&mut app, &ctx, 0.2, 2, false);
        row(&mut app, &ctx, 0.3, 2, false);
        assert_eq!(panel(&app).rename_text().as_deref(), Some("Caf "));
        ui_pass(&mut app, &ctx, 0.4, vec![key_event(egui::Key::Enter, true)]);
        assert_eq!(app.renaming_layer(), None);
        assert_eq!(names(&app), ["Background", "Hill", "Caf\u{e9} \u{2601}"]);
        // Double-click, then a click elsewhere: the same.
        row(&mut app, &ctx, 1.0, 2, false);
        row(&mut app, &ctx, 1.1, 2, false);
        assert_eq!(app.renaming_layer(), Some(2));
        click_ui(&mut app, &ctx, 1.2, (100, 100), Primary);
        assert_eq!(app.renaming_layer(), None);
        assert_eq!(names(&app)[2], "Caf\u{e9} \u{2601}");
        // The first click selected Sky: one setting, no undo step; U undoes the rename.
        assert_eq!(app.engine.document().revision(), revision + 1);
        undo(&mut app, &ctx, 1.5);
        assert_eq!(names(&app)[2], "Sky");
    }

    #[test]
    fn colour_0_erases_on_an_upper_layer_through_the_panel() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        // Hill is active: erase part of its rectangle with colour 0 (CLR-like filled box).
        app.select_tool(Tool::Rectangle, true);
        app.ui.fg = 0;
        let (a, b) = (ui_point(&app, (10, 160)), ui_point(&app, (30, 180)));
        ui_pass(&mut app, &ctx, 0.2, vec![egui::Event::PointerMoved(a)]);
        ui_pass(&mut app, &ctx, 0.22, vec![pointer_button(a, Primary, true)]);
        ui_pass(&mut app, &ctx, 0.24, vec![egui::Event::PointerMoved(b)]);
        ui_pass(
            &mut app,
            &ctx,
            0.26,
            vec![pointer_button(b, Primary, false)],
        );
        let screen = app.compose_screen();
        assert_eq!(screen.get(20, 170), Some(6), "the Background shows through");
        assert_eq!(screen.get(50, 170), Some(12), "the rest of the hill");
        assert_eq!(
            app.engine.document().layer(1).unwrap().get(20, 170),
            Some(0)
        );
        // On the bottom layer colour 0 is a colour: select Background and paint it there.
        row(&mut app, &ctx, 0.4, 0, false);
        let (a, b) = (ui_point(&app, (100, 20)), ui_point(&app, (110, 30)));
        ui_pass(&mut app, &ctx, 0.5, vec![egui::Event::PointerMoved(a)]);
        ui_pass(&mut app, &ctx, 0.52, vec![pointer_button(a, Primary, true)]);
        ui_pass(&mut app, &ctx, 0.54, vec![egui::Event::PointerMoved(b)]);
        ui_pass(
            &mut app,
            &ctx,
            0.56,
            vec![pointer_button(b, Primary, false)],
        );
        assert_eq!(
            app.compose_screen().get(105, 25),
            Some(0),
            "opaque colour 0"
        );
    }

    #[test]
    fn the_panel_hides_under_requesters_and_the_wheel_scrolls_its_list() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        app.open_page_size();
        ui_pass(&mut app, &ctx, 0.2, vec![]);
        assert!(panel(&app).hidden);
        let (w, h) = app.view().ui_size();
        assert!(Layout::of(&app.ui, w, h).is_none());
        let screen = app.compose_screen();
        let pens = Pens::from_palette(app.palette());
        let l = Layout::overlay(w, h, true, true, false);
        assert_eq!(
            screen.get(l.panel.x, 15),
            Some(6),
            "not drawn: the picture shows"
        );
        assert_ne!(pens.dark, 6);
        app.ui.page_size_requester = None;
        app.dirty = true;
        ui_pass(&mut app, &ctx, 0.3, vec![]);
        assert!(!panel(&app).hidden);
        // Many layers on a page larger than the screen: the wheel over the panel scrolls the
        // list, not the page.
        app.engine
            .execute(Command::SetPageSize {
                width: 640,
                height: 512,
                fill: 6,
            })
            .unwrap();
        let view = app.view();
        assert_ne!(view.max_scroll(app.engine.document().page()), (0, 0));
        for _ in 0..20 {
            app.engine
                .execute(Command::AddLayer { name: None })
                .unwrap();
        }
        app.engine
            .execute(Command::SelectLayer { layer: 22 })
            .unwrap();
        app.dirty = true;
        ui_pass(&mut app, &ctx, 0.4, vec![]);
        assert_eq!(panel(&app).scroll, 0);
        let l = panel_layout(&app);
        let over = ui_point(&app, (l.panel.x + 30, 100));
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -2.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        };
        ui_pass(&mut app, &ctx, 0.5, vec![egui::Event::PointerMoved(over)]);
        ui_pass(&mut app, &ctx, 0.6, vec![wheel.clone()]);
        assert_eq!(panel(&app).scroll, 2);
        assert_eq!(app.scroll, (0, 0), "the page stays");
        // The arrow scrolls back up.
        click_ui(&mut app, &ctx, 0.7, mid(l.scroll_up()), Primary);
        assert_eq!(panel(&app).scroll, 1);
        // The same wheel over the picture scrolls the page (and not the list).
        let picture = ui_point(&app, (100, 100));
        ui_pass(
            &mut app,
            &ctx,
            0.8,
            vec![egui::Event::PointerMoved(picture)],
        );
        ui_pass(&mut app, &ctx, 0.9, vec![wheel]);
        assert_ne!(app.scroll, (0, 0), "the page scrolls");
        assert_eq!(panel(&app).scroll, 1);
    }

    #[test]
    fn a_panel_command_ends_stencil_paint_so_u_undoes_it() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        app.toggle_stencil_paint();
        assert!(app.stencil_painting());
        // Del on the panel: the layer goes from the picture, Stencil ▸ Paint ends (as a menu
        // item ends it), and U brings the layer back instead of undoing a mask stroke.
        press(&mut app, &ctx, 0.2, LayerButton::Delete);
        assert_eq!(names(&app), ["Background", "Sky"]);
        assert!(!app.stencil_painting());
        undo(&mut app, &ctx, 0.3);
        assert_eq!(names(&app), ["Background", "Hill", "Sky"]);
        // The eye and the rows end it too; the scroll arrows and a ghosted button do not.
        app.toggle_stencil_paint();
        row(&mut app, &ctx, 0.5, 2, true);
        assert!(!app.stencil_painting());
        assert!(!app.engine.document().layers()[2].visible);
        app.toggle_stencil_paint();
        row(&mut app, &ctx, 0.7, 0, false);
        assert!(!app.stencil_painting());
        assert_eq!(app.engine.document().active_layer(), 0);
        app.toggle_stencil_paint();
        assert!(!panel(&app).enabled(LayerButton::Down));
        press(&mut app, &ctx, 0.9, LayerButton::Down);
        assert!(app.stencil_painting(), "a ghosted button does nothing");
    }

    #[test]
    fn the_docked_title_row_belongs_to_the_panel() {
        let ctx = egui::Context::default();
        let mut app = three_layers(&ctx);
        for _ in 0..22 {
            app.engine
                .execute(Command::AddLayer { name: None })
                .unwrap();
        }
        app.ui.prefs.set(Pref::LayerPanel, true);
        app.dirty = true;
        full_pass(&mut app, &ctx, 0.1, vec![]);
        full_pass(&mut app, &ctx, 0.2, vec![]);
        assert!(panel(&app).docked);
        let l = panel_layout(&app);
        assert!(l.capacity < 25 && panel(&app).more_below(l.capacity));
        let scroll = panel(&app).scroll;
        // The down arrow in the title row, in line with the screen's title bar.
        let down = mid(l.scroll_down());
        assert!(down.1 < 10);
        let at = ui_point(&app, down);
        full_pass(&mut app, &ctx, 0.3, vec![egui::Event::PointerMoved(at)]);
        full_pass(
            &mut app,
            &ctx,
            0.32,
            vec![pointer_button(at, Primary, true)],
        );
        full_pass(
            &mut app,
            &ctx,
            0.34,
            vec![pointer_button(at, Primary, false)],
        );
        assert_eq!(panel(&app).scroll, scroll + 1, "the arrow scrolls the list");
        // The right button on its "Layers" label opens no menu strip.
        let label = ui_point(&app, (l.panel.x + 10, 5));
        let right = egui::PointerButton::Secondary;
        full_pass(&mut app, &ctx, 0.4, vec![egui::Event::PointerMoved(label)]);
        full_pass(
            &mut app,
            &ctx,
            0.42,
            vec![pointer_button(label, right, true)],
        );
        assert!(!app.ui.menu.active, "no menu over the panel");
        full_pass(
            &mut app,
            &ctx,
            0.44,
            vec![pointer_button(label, right, false)],
        );
        // The wheel over the title row scrolls the list as over the rest of the panel.
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        };
        full_pass(&mut app, &ctx, 0.5, vec![wheel]);
        assert_eq!(panel(&app).scroll, scroll);
        // Over the screen's own title bar the right button still opens the menus.
        let title = ui_point(&app, (100, 5));
        full_pass(&mut app, &ctx, 0.6, vec![egui::Event::PointerMoved(title)]);
        full_pass(
            &mut app,
            &ctx,
            0.62,
            vec![pointer_button(title, right, true)],
        );
        assert!(app.ui.menu.active, "the screen's title bar");
    }

    #[test]
    fn panel_buttons_show_their_tooltips() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        let l = panel_layout(&app);
        let at = ui_point(&app, mid(l.button(LayerButton::Merge)));
        ui_pass(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at)]);
        ui_pass(&mut app, &ctx, 1.6, vec![]);
        let tip = app.ui.tooltip.as_ref().expect("a tooltip");
        assert_eq!(tip.text, "Merge Down");
        let (w, h) = app.view().ui_size();
        let r = tip.layout(w, h, &app.ui).unwrap();
        assert_eq!(r.x + r.w, l.panel.x - 2, "left of the panel");
    }

    /// One pass of a full-screen 1600×1024 window (1 pixel per point): the screen fits at
    /// 4x by its height and the docked panel beside it.
    fn full_pass(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        events: Vec<egui::Event>,
    ) -> std::time::Duration {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1024.0),
            )),
            time: Some(t),
            events,
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .fullscreen = Some(true);
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
        out.textures_delta.clear();
        out.viewport_output[&egui::ViewportId::ROOT].repaint_delay
    }

    #[test]
    fn in_a_full_screen_window_the_panel_docks_right_of_the_screen() {
        let ctx = egui::Context::default();
        let mut app = three_layers(&ctx);
        full_pass(&mut app, &ctx, 0.1, vec![]);
        assert!(app.free);
        // `L` docks it: that pass laid the screen out alone, so it asks for the next one at
        // once, which centres screen and panel together.
        let delay = full_pass(&mut app, &ctx, 0.15, shift_l());
        assert_eq!(delay, std::time::Duration::ZERO);
        assert_eq!(app.layers.laid_out, 0);
        full_pass(&mut app, &ctx, 0.2, vec![]);
        assert_eq!(app.layers.laid_out, 320);
        let (view, scale) = app.layout();
        let per_ui = scale * (view.fx * view.ux) as f32;
        assert_eq!(per_ui, 4.0, "4x: 1024 rows high");
        assert!(panel(&app).docked);
        // Screen and panel side by side fill the 1600 points: 1280 + 320.
        assert_eq!(app.layer_strip_px(view, scale), 320);
        assert_eq!(ui_point(&app, (0, 0)), egui::pos2(2.0, 2.0));
        assert!(app.layers.texture.is_some(), "the strip is uploaded");
        // The screen itself is unchanged: no panel over the picture.
        let pens = Pens::from_palette(app.palette());
        assert_ne!(app.compose_screen().get(216, 100), Some(pens.dark));
        // A click on the strip's New button: a layer above Hill.
        let l = panel_layout(&app);
        assert_eq!(l.panel.x, 320);
        let at = ui_point(&app, mid(l.button(LayerButton::New)));
        full_pass(&mut app, &ctx, 0.3, vec![egui::Event::PointerMoved(at)]);
        full_pass(
            &mut app,
            &ctx,
            0.32,
            vec![pointer_button(at, Primary, true)],
        );
        full_pass(
            &mut app,
            &ctx,
            0.34,
            vec![pointer_button(at, Primary, false)],
        );
        assert_eq!(names(&app), ["Background", "Hill", "Layer 1", "Sky"]);
        // A requester hides it; the strip stays as an empty work area.
        app.open_page_size();
        full_pass(&mut app, &ctx, 0.4, vec![]);
        let p = panel(&app);
        assert!(p.docked && p.hidden);
        assert_eq!(
            app.layer_strip_px(view, scale),
            320,
            "the screen does not move"
        );
        // Off: the screen is centred alone again.
        app.ui.page_size_requester = None;
        app.ui.prefs.set(Pref::LayerPanel, false);
        app.dirty = true;
        full_pass(&mut app, &ctx, 0.5, vec![]);
        assert_eq!(app.layer_strip_px(view, scale), 0);
        assert!(app.layers.texture.is_none());
        assert_eq!(
            ui_point(&app, (0, 0)),
            egui::pos2(160.0 + 2.0, 2.0),
            "centred alone"
        );
        // A window too narrow for both (the 1280-point test window, not full screen): over
        // the screen.
        app.ui.prefs.set(Pref::LayerPanel, true);
        app.dirty = true;
        ui_pass(&mut app, &ctx, 0.6, vec![]);
        assert!(!panel(&app).docked);
    }

    #[test]
    fn snapshots_show_the_panel_over_or_beside_the_screen() {
        let ctx = egui::Context::default();
        let mut app = three_layers(&ctx);
        let view = app.view();
        let mut ui = UiState::default();
        let args: Vec<String> = ["--layers-panel"].map(String::from).to_vec();
        snapshot_ui(&args, &mut app.engine, &mut ui, view);
        let p = ui.layer_panel.as_ref().unwrap();
        assert!(!p.docked && p.rows.len() == 3 && p.active == 1);
        let doc = app.engine.document().clone();
        let shot = crate::snapshot_rgba(1, &doc, view, (0, 0), &ui, false);
        let wide = with_docked_strip(shot.clone(), doc.palette(), &ui, view, 1);
        assert_eq!(wide.0, shot.0, "the overlay adds nothing");
        let args: Vec<String> = ["--layers-panel", "docked"].map(String::from).to_vec();
        snapshot_ui(&args, &mut app.engine, &mut ui, view);
        assert!(ui.layer_panel.as_ref().unwrap().docked);
        let shot = crate::snapshot_rgba(2, &doc, view, (0, 0), &ui, false);
        let (w, h, rgba) = with_docked_strip(shot.clone(), doc.palette(), &ui, view, 2);
        let per_ui = 2 * (view.ux * view.fx) as u32;
        assert_eq!((w, h), (shot.0 + 80 * per_ui, shot.1));
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // No flag: no panel, but the label of the active layer.
        snapshot_ui(&[], &mut app.engine, &mut ui, view);
        assert!(ui.layer_panel.is_none());
        assert_eq!(ui.layer_label.as_deref(), Some("Hill"));
    }

    /// Wave-8 integration: a change that reaches the document without setting the redraw
    /// flag (as an MCP job or a generated picture placed as a layer may) still brings the
    /// panel and the title-bar label up to date in the next pass, before it is drawn.
    #[test]
    fn the_panel_and_the_label_follow_changes_made_elsewhere() {
        let ctx = egui::Context::default();
        let mut app = with_panel(&ctx);
        assert_eq!(panel(&app).rows.len(), 3);
        // Around every path that sets `dirty`: straight into the engine.
        app.engine
            .execute(Command::AddLayer {
                name: Some("Moon".into()),
            })
            .unwrap();
        app.dirty = false;
        ui_pass(&mut app, &ctx, 0.2, vec![]);
        let p = panel(&app);
        assert_eq!(p.rows.len(), 4);
        assert_eq!((p.active, p.rows[2].name.as_str()), (2, "Moon"));
        assert!(!app.dirty, "drawn in that pass");
        // A pass without a change redraws nothing.
        ui_pass(&mut app, &ctx, 0.3, vec![]);
        assert!(!app.dirty);
        // Undo outside the panel, too.
        app.engine.undo().unwrap();
        ui_pass(&mut app, &ctx, 0.4, vec![]);
        assert_eq!(panel(&app).rows.len(), 3);
        // With the panel off, the title-bar label follows a selection made elsewhere.
        ui_pass(&mut app, &ctx, 0.5, shift_l());
        assert_eq!(app.ui.layer_label.as_deref(), Some("Hill"));
        app.engine
            .execute(Command::SelectLayer { layer: 2 })
            .unwrap();
        ui_pass(&mut app, &ctx, 0.6, vec![]);
        assert_eq!(app.ui.layer_label.as_deref(), Some("Sky"));
        // Another document at the same revision (a picture loaded, the spare page swapped
        // in): another engine counts too.
        let mut other = Engine::new(app.engine.document().clone());
        other
            .execute(Command::RenameLayer {
                layer: 2,
                name: "Stars".into(),
            })
            .unwrap();
        app.engine
            .execute(Command::SetLayerVisible {
                layer: 0,
                visible: true,
            })
            .unwrap();
        ui_pass(&mut app, &ctx, 0.7, vec![]);
        assert_eq!(
            other.document().revision(),
            app.engine.document().revision()
        );
        app.engine = other;
        ui_pass(&mut app, &ctx, 0.8, vec![]);
        assert_eq!(app.ui.layer_label.as_deref(), Some("Stars"));
    }
}
