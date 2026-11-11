//! The remaining Color and Effect items (ADR-39): Color ▸ Palette ▸ Use Brush Palette, Color ▸
//! Recompute, Effect ▸ Stencil ▸ Paint (Ctrl-s) and Effect ▸ Background ▸ Lock FG.
//!
//! Stencil ▸ Paint and Lock FG make an area stencil ([`dpx_core::AreaStencil`]): page pixels
//! protected by position. It and the colour stencil of ADR-20 exclude each other: making one
//! puts the other away (a colour stencil's colours stay in `tools.stencil_off`, as the Make
//! Stencil requester's selection, so Make and Remake start from them again). Painting
//! commands carry the area stencil as [`Command::Stenciled`]; the tools wrap their own
//! commands (so the preview is what the release paints) and [`PainterApp::area_route`] wraps
//! fills and CLR. While Stencil ▸ Paint is on, every painting command goes to the mask
//! instead ([`stencil::mask_command`]), which has its own engine, so Undo steps back through
//! the stencil strokes. Everything that changes pixels lives in `dpx_core`; this module keeps
//! the state and wires the menu, the key and the view.

use dpx_core::brush_palette::{self, Adopted};
use dpx_core::quantize::PaletteDepth;
use dpx_core::stencil::{self, AreaStencil};
use dpx_core::{BrushImage, Chipset, Command, Document, Engine, Palette};
use dpx_iff::ilbm::ColorRange;
use eframe::egui;

use crate::{PainterApp, note_requester};

/// Note titles.
const USE_BRUSH_PALETTE: &str = "Use Brush Palette";
const LOCK_FG: &str = "Lock FG";
const RECOMPUTE: &str = "Recompute";
const SAVE_STENCIL: &str = "Save Stencil";

/// A brush as its file has it (Brush ▸ Load..): what Use Brush Palette takes.
#[derive(Clone, Debug)]
pub struct LoadedBrush {
    /// The brush in the file's colour numbers.
    pub brush: BrushImage,
    /// The file's palette (CMAP).
    pub palette: Palette,
    /// The file's colour ranges (CRNG).
    pub ranges: Vec<ColorRange>,
    /// The custom brush made from it (its colours fitted to the picture palette, ADR-19).
    /// While the picked-up brush is this one, the custom brush comes from the file.
    pub fitted: BrushImage,
}

/// The area stencil with the frame it belongs to: area stencils apply to the picture or
/// frame they were made on only (AGA p.273).
#[derive(Clone, Debug)]
pub struct Area {
    pub stencil: AreaStencil,
    pub frame: usize,
    /// Stencil ▸ On/Off.
    pub on: bool,
}

/// Stencil ▸ Paint in progress.
#[derive(Clone, Debug)]
pub struct Painting {
    /// The mask being painted: a two-colour page with its own undo history.
    pub engine: Engine,
    /// The frame it is painted for.
    pub frame: usize,
}

/// App state of ADR-39.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub area: Option<Area>,
    pub painting: Option<Painting>,
    pub loaded: Option<LoadedBrush>,
}

/// The palette precision of a target: 12-bit on OCS.
fn depth(chipset: Chipset) -> PaletteDepth {
    match chipset {
        Chipset::Ocs => PaletteDepth::Rgb4,
        Chipset::Aga => PaletteDepth::Rgb8,
    }
}

/// Whether `cmd` paints with the left button (adds to a painted stencil): strokes and shapes
/// say so, a fill in the BG colour (and not FG) frees, CLR frees.
fn adds(cmd: &Command, fg: u8, bg: u8) -> bool {
    match cmd {
        Command::Stroke { right, .. } | Command::Shape { right, .. } => !right,
        Command::FloodFill { color, .. } => *color == fg || *color != bg,
        Command::Stenciled { command, .. } => adds(command, fg, bg),
        _ => false,
    }
}

impl PainterApp {
    /// Whether ADR-39 handles menu item `id`: its own four items, and the stencil items that
    /// act on an area stencil when there is one. Any item but Stencil ▸ Paint ends Stencil ▸
    /// Paint first (keeping what was painted), as DeluxePaint may (AGA p.120).
    pub(crate) fn takes_adr39(&mut self, id: &str) -> bool {
        let own = matches!(
            id,
            "palette.use_brush" | "color.recompute" | "stencil.paint" | "background.lock_fg"
        );
        if id != "stencil.paint" && self.ps().painting.is_some() {
            self.leave_stencil_paint();
        }
        own || (self.ps().area.is_some()
            && matches!(
                id,
                "stencil.show"
                    | "stencil.remake"
                    | "stencil.reverse"
                    | "stencil.toggle"
                    | "stencil.free"
                    | "stencil.save"
            ))
    }

    /// Runs an item [`Self::takes_adr39`] took.
    pub(crate) fn adr39_action(&mut self, id: &str) {
        self.finish_stroke();
        match id {
            "palette.use_brush" => self.use_brush_palette(),
            "color.recompute" => self.recompute(),
            "stencil.paint" => self.toggle_stencil_paint(),
            "background.lock_fg" => self.lock_fg(),
            _ => self.area_stencil_item(id),
        }
        // The title flag and the Show view follow in this pass already.
        self.refresh_effects();
        self.dirty = true;
    }

    fn ps(&self) -> &State {
        &self.menu_data.palette_stencil
    }

    fn ps_mut(&mut self) -> &mut State {
        &mut self.menu_data.palette_stencil
    }

    fn note(&mut self, title: &str, text: String) {
        self.ui.note_requester = note_requester(title, &[text]);
    }

    // ---- Use Brush Palette ---------------------------------------------------------------

    /// Brush ▸ Load.. read `brush` from a file with `palette` and `ranges`; `fitted` is the
    /// custom brush made from it.
    pub(crate) fn remember_loaded_brush(
        &mut self,
        brush: BrushImage,
        palette: Palette,
        ranges: Vec<ColorRange>,
        fitted: BrushImage,
    ) {
        self.ps_mut().loaded = Some(LoadedBrush {
            brush,
            palette,
            ranges,
            fitted,
        });
    }

    /// Color ▸ Palette ▸ Use Brush Palette (AGA p.267-268): the picture palette becomes the
    /// custom brush's own palette, with the brush file's colour ranges; a brush palette with
    /// more colours than the picture is recomputed with the picture's number. One undoable
    /// palette change (the picture keeps its colour numbers, so Color ▸ Remap can fit it).
    fn use_brush_palette(&mut self) {
        let Some(custom) = self.custom_brush.clone() else {
            return;
        };
        let doc = self.engine.document();
        let current = doc.palette().clone();
        let n = current.len();
        let depth = depth(doc.target().chipset);
        // What the custom brush's colour numbers mean now.
        let meant = self
            .tools
            .brush_palette
            .clone()
            .unwrap_or_else(|| current.clone());
        let loaded = self
            .ps()
            .loaded
            .clone()
            .filter(|l| self.picked_brush.as_ref() == Some(&l.fitted));
        let (own_brush, own, ranges) = match &loaded {
            // The loaded brush as it is: exactly the file's colour numbers.
            Some(l) if custom == l.fitted => (l.brush.clone(), l.palette.clone(), Some(&l.ranges)),
            // Changed since: its colours, in the file palette's nearest numbers.
            Some(l) => (
                brush_palette::fit_brush(&custom, &meant, l.palette.colors()),
                l.palette.clone(),
                Some(&l.ranges),
            ),
            // Picked up: the picture palette of the moment.
            None => (custom.clone(), meant, None),
        };
        let Adopted {
            colors,
            brush,
            reduced,
        } = brush_palette::use_brush_palette(&own_brush, own.colors(), &current, depth);
        let mut notes = Vec::new();
        if reduced {
            notes.push(format!(
                "the brush palette has {} colours; it was recomputed with the picture's {n}",
                own.len()
            ));
        }
        if let Some(ranges) = ranges.filter(|r| !r.is_empty()) {
            if reduced {
                notes.push("the brush's colour ranges were not used".into());
            } else {
                let kept: Vec<ColorRange> = ranges
                    .iter()
                    .filter(|r| usize::from(r.high) < n)
                    .copied()
                    .collect();
                if kept.len() < ranges.len() {
                    notes.push(format!(
                        "{} brush colour ranges lie beyond the picture's {n} colours",
                        ranges.len() - kept.len()
                    ));
                }
                self.extras.ranges = kept;
            }
        }
        if colors != current.colors() {
            self.run(Command::SetPalette { colors });
        }
        if self.picked_brush.as_ref() == Some(&custom) {
            self.picked_brush = Some(brush.clone());
            if let Some(l) = &mut self.ps_mut().loaded
                && loaded.is_some()
            {
                l.fitted = brush.clone();
            }
        }
        self.custom_brush = Some(brush);
        self.tools.brush_palette = Some(self.engine.document().palette().clone());
        self.ui.note_requester = note_requester(USE_BRUSH_PALETTE, &notes);
    }

    // ---- Recompute ------------------------------------------------------------------------

    /// Color ▸ Recompute: HAM pictures only (AGA p.270). Documents are never HAM yet (HAM
    /// pictures are reduced to a palette on load, ADR-26), so the item is ghosted; the HAM
    /// work itself is `dpx_formats::ham::recompute`.
    fn recompute(&mut self) {
        self.note(RECOMPUTE, "Recompute works on HAM pictures only".into());
    }

    // ---- Area stencils --------------------------------------------------------------------

    /// The area stencil painting honours now: on, made for the current frame, of the page's
    /// size, and not being painted.
    pub(crate) fn active_area(&self) -> Option<&AreaStencil> {
        let st = self.ps();
        let doc = self.engine.document();
        st.area
            .as_ref()
            .filter(|a| {
                st.painting.is_none()
                    && a.on
                    && a.frame == doc.current_frame()
                    && a.stencil.fits(doc.page())
            })
            .map(|a| &a.stencil)
    }

    /// Stencil ▸ Paint is on.
    pub(crate) fn stencil_painting(&self) -> bool {
        self.ps().painting.is_some()
    }

    /// Makes `stencil` the area stencil of the current frame; the colour stencil steps aside
    /// (its colours stay the Make Stencil selection). An empty mask means no stencil.
    fn set_area(&mut self, stencil: AreaStencil) {
        if let Some(s) = self.stencil.take() {
            self.tools.stencil_off = Some(s);
        }
        let frame = self.engine.document().current_frame();
        self.ps_mut().area = (!stencil.is_empty()).then_some(Area {
            stencil,
            frame,
            on: true,
        });
    }

    /// Effect ▸ Background ▸ Lock FG (AGA p.275, tutorial p.119): the pixels painted since
    /// Background ▸ Fix, whatever their colour, become an area stencil.
    fn lock_fg(&mut self) {
        let Some(bg) = self.menu_data.background.clone() else {
            return self.note(
                LOCK_FG,
                "Lock FG needs a fixed background: Effect ▸ Background ▸ Fix first".into(),
            );
        };
        match AreaStencil::from_changes(self.engine.document().page(), &bg) {
            None => self.note(
                LOCK_FG,
                "The page size changed since Background ▸ Fix".into(),
            ),
            Some(a) if a.is_empty() => self.note(
                LOCK_FG,
                "Nothing has been painted since Background ▸ Fix".into(),
            ),
            Some(a) => {
                self.menu_data.effects.stencil_show = false;
                self.set_area(a);
            }
        }
    }

    /// Stencil items while there is an area stencil: Show, Remake, Reverse, On/Off, Free and
    /// Save.. act on it.
    fn area_stencil_item(&mut self, id: &str) {
        match id {
            "stencil.show" => {
                let on = self.active_area().is_some();
                let e = &mut self.menu_data.effects;
                e.stencil_show = !e.stencil_show && on;
            }
            // Remake makes a colour stencil: the painted one is discarded (AGA p.273).
            "stencil.remake" => {
                self.ps_mut().area = None;
                self.stencil_command("stencil.remake");
            }
            "stencil.reverse" => {
                if let Some(a) = &mut self.ps_mut().area {
                    a.stencil = a.stencil.reversed();
                }
            }
            "stencil.toggle" => {
                if let Some(a) = &mut self.ps_mut().area {
                    a.on = !a.on;
                }
            }
            "stencil.free" => {
                self.ps_mut().area = None;
                self.stencil = None;
                self.tools.stencil_off = None;
            }
            "stencil.save" => self.note(
                SAVE_STENCIL,
                "A painted stencil cannot be saved yet, only one made from colours".into(),
            ),
            _ => {}
        }
    }

    /// Effect ▸ Stencil ▸ Paint (Ctrl-s): on, the drawing tools paint the stencil (left adds,
    /// right frees), starting from the area stencil or from the colours of the colour stencil
    /// (AGA p.120); chosen again, the painted mask becomes the stencil.
    pub(crate) fn toggle_stencil_paint(&mut self) {
        self.finish_stroke();
        if self.ps().painting.is_some() {
            self.leave_stencil_paint();
            return;
        }
        let doc = self.engine.document();
        let (page, frame) = (doc.page(), doc.current_frame());
        // The colours of the colour stencil, on or off (the Make Stencil selection).
        let colors = self.stencil.as_ref().or(self.tools.stencil_off.as_ref());
        let start = match (&self.ps().area, colors) {
            (Some(a), _) if a.frame == frame && a.stencil.fits(page) => a.stencil.clone(),
            (_, Some(colors)) => AreaStencil::from_colors(page, colors),
            _ => AreaStencil::free(page),
        };
        let mask = Document::from_picture(start.to_image(), stencil::mask_palette(), doc.target());
        self.menu_data.effects.stencil_show = false;
        self.ps_mut().painting = Some(Painting {
            engine: Engine::new(mask),
            frame,
        });
        self.dirty = true;
    }

    /// Ends Stencil ▸ Paint: the mask becomes the area stencil of the frame it was painted
    /// for.
    fn leave_stencil_paint(&mut self) {
        self.finish_stroke();
        let Some(p) = self.ps_mut().painting.take() else {
            return;
        };
        let mask = AreaStencil::from_image(p.engine.document().page());
        self.set_area(mask);
        if let Some(a) = &mut self.ps_mut().area {
            a.frame = p.frame;
        }
        self.dirty = true;
    }

    /// Every command on its way to the engine: while Stencil ▸ Paint is on, a painting
    /// command paints the mask instead (`None`: nothing left for the picture); otherwise the
    /// area stencil joins strokes, shapes, fills and CLR.
    pub(crate) fn area_route(&mut self, command: Command) -> Option<Command> {
        let (fg, bg) = (self.ui.fg, self.ui.bg);
        if let Some(p) = &mut self.menu_data.palette_stencil.painting {
            if let Some(mask) = stencil::mask_command(&command, adds(&command, fg, bg)) {
                if let Err(e) = p.engine.execute(mask) {
                    eprintln!("stencil paint: {e}");
                }
                self.dirty = true;
                return None;
            }
            return Some(command);
        }
        Some(self.with_area(command, false))
    }

    /// For `run`: CLR on a fixed background (ADR-15) restores it, sparing the area stencil
    /// too (tutorial AGA p.119); during Stencil ▸ Paint CLR stays CLR (it clears the mask).
    pub(crate) fn with_background_and_area(&self, command: Command) -> Command {
        if self.stencil_painting() {
            return command;
        }
        let clear = matches!(command, Command::Clear { .. });
        let command = self.with_fixed_background(command);
        self.with_area(command, clear)
    }

    /// `command` under the active area stencil, if it paints (a SetPage only with
    /// `set_page`).
    fn with_area(&self, command: Command, set_page: bool) -> Command {
        let Some(area) = self.active_area() else {
            return command;
        };
        let wrap = match command {
            Command::Stroke { .. }
            | Command::Shape { .. }
            | Command::FloodFill { .. }
            | Command::Clear { .. } => true,
            Command::SetPage { .. } => set_page,
            _ => false,
        };
        if wrap {
            Command::Stenciled {
                area: area.clone(),
                command: Box::new(command),
            }
        } else {
            command
        }
    }

    /// Undo (`redo`: Redo) while Stencil ▸ Paint is on steps through the stencil strokes;
    /// `false` when it is off.
    pub(crate) fn stencil_paint_undo(&mut self, redo: bool) -> bool {
        let Some(p) = &mut self.menu_data.palette_stencil.painting else {
            return false;
        };
        let _ = if redo {
            p.engine.redo()
        } else {
            p.engine.undo()
        };
        self.dirty = true;
        true
    }

    /// Once per pass: a colour stencil made since (Make, Remake, Load) discards the area
    /// stencil (AGA p.273); a page of another size ends it, and ends Stencil ▸ Paint.
    pub(crate) fn refresh_area(&mut self) {
        let page = self.engine.document().page();
        let st = &self.menu_data.palette_stencil;
        let drop_area = st
            .area
            .as_ref()
            .is_some_and(|a| self.stencil.is_some() || !a.stencil.fits(page));
        let drop_paint = st.painting.as_ref().is_some_and(|p| {
            let mask = p.engine.document().page();
            (mask.width(), mask.height()) != (page.width(), page.height())
        });
        if drop_paint {
            self.menu_data.palette_stencil.painting = None;
            self.dirty = true;
        }
        if drop_area {
            self.menu_data.palette_stencil.area = None;
            self.dirty = true;
        }
    }

    /// The title bar's stencil flag (AGA p.120, p.271): `s` while Show or Stencil ▸ Paint is
    /// on, `S` while a stencil applies, nothing otherwise.
    pub(crate) fn stencil_flag(&self) -> Option<&'static str> {
        let painting = self.stencil_painting();
        if self.menu_data.effects.stencil_show || painting {
            Some("s")
        } else if self.stencil.is_some() || self.active_area().is_some() {
            Some("S")
        } else {
            None
        }
    }

    /// Ctrl-s: Effect ▸ Stencil ▸ Paint (manual keyboard table, AGA p.294).
    pub(crate) fn palette_stencil_keys(&mut self, ctx: &egui::Context) {
        let pressed = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::S,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if modifiers.ctrl && !(modifiers.alt || modifiers.shift || modifiers.mac_cmd)
                )
            })
        });
        if pressed && !self.modal_open() {
            self.toggle_stencil_paint();
            self.refresh_effects();
        }
    }

    /// Before the screen is composed: during Stencil ▸ Paint the picture dimmed with the
    /// mask (and the gesture in progress) at full brightness; during Show the same for the
    /// area stencil. Display only.
    pub(crate) fn prepare_stencil_view(&mut self) {
        let doc = self.engine.document();
        let (mut base, mask) = if let Some(p) = &self.ps().painting {
            let mut mask = p.engine.document().page().clone();
            let pal = p.engine.document().palette();
            let mut base = doc.page();
            if let Some(s) = &self.stroke {
                match s.preview_command() {
                    Some(cmd) => {
                        let add = adds(&cmd, self.ui.fg, self.ui.bg);
                        if let Some(m) = stencil::mask_command(&cmd, add)
                            && m.check_paint(&mask, pal).is_ok()
                        {
                            let before = mask.clone();
                            m.paint(&mut mask, &before, pal);
                        }
                    }
                    // The Brush Selector's frame: shown as the tools draw it.
                    None => base = &s.preview,
                }
            }
            (base.clone(), AreaStencil::from_image(&mask))
        } else if self.menu_data.effects.stencil_show
            && let Some(a) = self.active_area()
        {
            (doc.page().clone(), a.clone())
        } else {
            return;
        };
        base = stencil::view(&base, &mask, doc.palette());
        self.anim.shown = Some(base);
    }
}

/// `--stencil-paint X,Y,W,H` for snapshots: the page as Stencil ▸ Paint shows it with that
/// rectangle painted into the stencil, and the title flag `s`.
pub fn snapshot_view(
    spec: &str,
    doc: &Document,
    ui: &mut dpx_ui::UiState,
) -> Result<Document, String> {
    let v: Vec<i32> = spec
        .split(',')
        .map(|n| n.trim().parse::<i32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{spec}: expected X,Y,W,H"))?;
    let [x, y, w, h] = v[..] else {
        return Err(format!("{spec}: expected X,Y,W,H"));
    };
    let page = doc.page();
    let mut mask = stencil::AreaStencil::free(page).to_image();
    mask.fill_rect(x, y, w, h, stencil::PROTECTED);
    let shown = stencil::view(page, &AreaStencil::from_image(&mask), doc.palette());
    ui.flags = "s".into();
    Ok(Document::from_picture(
        shown,
        doc.palette().clone(),
        doc.target(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{key_event, menu_path, ui_pass, ui_point};
    use dpx_core::{DrawMode, Rgb8, Shape};
    use dpx_ui::{Action, FileKind, Hit, ReqEvent, ReqKey, Tool};

    fn act(app: &mut PainterApp, id: &'static str) {
        app.run_action(Action::Command(id), &egui::Context::default());
        app.refresh_effects();
    }

    fn button(pos: egui::Pos2, right: bool, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: if right {
                egui::PointerButton::Secondary
            } else {
                egui::PointerButton::Primary
            },
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Picks `id` from the menu with the right button through `ui()` (press on the title,
    /// move over the submenu item, release), from time `t`.
    fn pick_menu(app: &mut PainterApp, ctx: &egui::Context, id: &'static str, t: f64) {
        let path = menu_path(app, id).map(|p| ui_point(app, p));
        ui_pass(
            app,
            ctx,
            t,
            vec![
                egui::Event::PointerMoved(path[0]),
                button(path[0], true, true),
            ],
        );
        assert!(app.ui.menu.active, "menu strip for {id}");
        ui_pass(app, ctx, t + 0.01, vec![egui::Event::PointerMoved(path[1])]);
        ui_pass(app, ctx, t + 0.02, vec![egui::Event::PointerMoved(path[2])]);
        ui_pass(app, ctx, t + 0.03, vec![button(path[2], true, false)]);
        assert!(!app.ui.menu.active);
    }

    /// A left (or right) click on page pixel `p` through `ui()`.
    fn click_page(app: &mut PainterApp, ctx: &egui::Context, p: (i32, i32), right: bool, t: f64) {
        let pos = ui_point(app, p);
        ui_pass(
            app,
            ctx,
            t,
            vec![egui::Event::PointerMoved(pos), button(pos, right, true)],
        );
        ui_pass(app, ctx, t + 0.01, vec![button(pos, right, false)]);
    }

    /// A `ui()` pass that redraws the screen; returns the image uploaded for it (the default
    /// LoRes screen: one screen pixel per page pixel).
    fn screen_pass(app: &mut PainterApp, ctx: &egui::Context, t: f64) -> egui::ColorImage {
        app.dirty = true;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 1024.0),
            )),
            time: Some(t),
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| eframe::App::ui(app, ui, &mut frame));
        let screen = out
            .textures_delta
            .set
            .values()
            .flat_map(|deltas| deltas.iter())
            .find_map(|delta| match &delta.image {
                egui::ImageData::Color(img) if img.size == [320, 256] => Some((**img).clone()),
                _ => None,
            });
        out.textures_delta.clear();
        screen.expect("the screen was uploaded")
    }

    fn ctrl_s(pressed: bool) -> egui::Event {
        egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }
    }

    /// A 3×3 square brush (built-in brush 4 is 4×4; a custom brush keeps the test exact).
    fn square_brush(color: u8) -> BrushImage {
        BrushImage {
            width: 3,
            height: 3,
            pixels: vec![color; 9],
            handle: (1, 1),
            transparent: None,
        }
    }

    fn page_px(app: &PainterApp, x: i32, y: i32) -> u8 {
        app.engine.document().page().get(x, y).unwrap()
    }

    #[test]
    fn menu_items_are_enabled_where_they_apply() {
        let mut app = PainterApp::new();
        let ctx = app.ui.menu_context();
        for id in ["stencil.paint", "background.lock_fg"] {
            assert!(Action::Command(id).enabled(&ctx), "{id}");
        }
        // Use Brush Palette needs a custom brush, like the Brush menu's colour items.
        assert!(!Action::Command("palette.use_brush").enabled(&ctx));
        app.custom_brush = Some(square_brush(3));
        app.ui.has_brush = true;
        assert!(Action::Command("palette.use_brush").enabled(&app.ui.menu_context()));
        // Recompute only on HAM screens, which documents never are yet; Print stays out.
        assert!(!Action::Command("color.recompute").enabled(&app.ui.menu_context()));
        let ham = dpx_ui::menu::MenuContext {
            ham: true,
            ..app.ui.menu_context()
        };
        assert!(Action::Command("color.recompute").enabled(&ham));
        assert!(!Action::Command("picture.print").enabled(&ham));
        act(&mut app, "color.recompute");
        assert!(app.ui.note_requester.is_some(), "never silent");
    }

    #[test]
    fn ctrl_s_paints_a_stencil_through_the_real_ui_loop() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 3 });
        app.custom_brush = Some(square_brush(9));
        app.ui.tool = Tool::DottedFreehand;
        app.ui.mode = dpx_ui::PaintMode::Matte;
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // Ctrl-s: Stencil Paint on; the title shows "s".
        ui_pass(&mut app, &ctx, 0.1, vec![ctrl_s(true)]);
        ui_pass(&mut app, &ctx, 0.11, vec![ctrl_s(false)]);
        assert!(app.stencil_painting());
        assert_eq!(app.ui.flags, "s");
        // A left click adds the brush's shape to the stencil, the picture stays.
        click_page(&mut app, &ctx, (40, 40), false, 0.2);
        assert_eq!(page_px(&app, 40, 40), 3, "the picture is not painted");
        // The uploaded screen shows the picture dimmed, the stencil at full brightness.
        let pal = app.engine.document().palette().clone();
        let dim = stencil::half_bright_table(&pal)[3];
        assert_ne!(dim, 3);
        let screen = screen_pass(&mut app, &ctx, 0.25);
        let rgb = |c: u8| {
            let c = pal.get(c).unwrap();
            egui::Color32::from_rgb(c.r, c.g, c.b)
        };
        let at = |x: usize, y: usize| screen.pixels[y * screen.size[0] + x];
        assert_eq!(at(40, 40), rgb(3));
        assert_eq!(at(41, 41), rgb(3));
        assert_eq!(at(60, 60), rgb(dim));
        // A right click frees part of it again; `u` undoes that.
        click_page(&mut app, &ctx, (42, 42), true, 0.3);
        let mask = |app: &PainterApp| {
            let p = app.ps().painting.as_ref().expect("painting");
            AreaStencil::from_image(p.engine.document().page())
        };
        assert!(!mask(&app).protects(41, 41));
        ui_pass(&mut app, &ctx, 0.4, vec![key_event(egui::Key::U, true)]);
        assert!(
            mask(&app).protects(41, 41),
            "undo steps back in the stencil"
        );
        assert_eq!(page_px(&app, 40, 40), 3, "and not in the picture");
        // Ctrl-s again: the mask becomes the stencil, flag "S".
        ui_pass(&mut app, &ctx, 0.5, vec![ctrl_s(true)]);
        ui_pass(&mut app, &ctx, 0.51, vec![ctrl_s(false)]);
        assert!(!app.stencil_painting());
        assert_eq!(app.ui.flags, "S");
        assert_eq!(app.active_area().map(|a| a.count()), Some(9));
        // Painting now goes around the stencilled square: brush 9 over it changes only the
        // free pixels.
        app.custom_brush = Some(BrushImage {
            width: 7,
            height: 1,
            pixels: vec![9; 7],
            handle: (3, 0),
            transparent: None,
        });
        click_page(&mut app, &ctx, (40, 40), false, 0.6);
        let row: Vec<u8> = (36..45).map(|x| page_px(&app, x, 40)).collect();
        assert_eq!(row, [3, 9, 9, 3, 3, 3, 9, 9, 3]);
    }

    #[test]
    fn the_hover_preview_goes_behind_the_stencil() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.custom_brush = Some(square_brush(9));
        app.ui.tool = Tool::DottedFreehand;
        app.ui.mode = dpx_ui::PaintMode::Matte;
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let mut mask = vec![false; 320 * 256];
        mask[100 * 320 + 100] = true;
        app.set_area(AreaStencil::from_mask(320, 256, mask).unwrap());
        app.refresh_effects();
        let at = ui_point(&app, (100, 100));
        ui_pass(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(at)]);
        let preview = &app.stroke.as_ref().expect("hover").preview;
        assert_eq!(preview.get(100, 100), Some(0), "behind the stencil");
        assert_eq!(preview.get(101, 100), Some(9));
        // The engine paints exactly what was previewed.
        let shown = preview.clone();
        click_page(&mut app, &ctx, (100, 100), false, 0.2);
        assert_eq!(app.engine.document().page().get(100, 100), Some(0));
        assert_eq!(app.engine.document().page(), &shown);
    }

    #[test]
    fn stencil_paint_from_the_menu_starts_from_the_colour_stencil() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.engine
            .execute(Command::Shape {
                shape: Shape::Rect {
                    from: (10, 10),
                    to: (19, 19),
                },
                brush: Vec::new(),
                color: 5,
                filled: true,
                symmetry: None,
                custom: None,
                mode: DrawMode::Matte,
                range: None,
                stencil: None,
                right: false,
                fx: Default::default(),
            })
            .unwrap();
        let mut colors = vec![false; 256];
        colors[5] = true;
        app.stencil = Some(colors.clone());
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pick_menu(&mut app, &ctx, "stencil.paint", 0.1);
        assert!(app.stencil_painting());
        // Choosing it again from the menu ends it: an area stencil of colour 5's square.
        pick_menu(&mut app, &ctx, "stencil.paint", 0.2);
        assert!(!app.stencil_painting());
        let area = app.active_area().expect("area stencil").clone();
        assert_eq!(area.count(), 100);
        assert!(area.protects(10, 10) && !area.protects(20, 20));
        // The colour stencil stepped aside; its colours stay for Make and Remake.
        assert!(app.stencil.is_none());
        assert_eq!(app.tools.stencil_off, Some(colors));
        // Remake makes the colour stencil again and discards the painted one.
        act(&mut app, "stencil.remake");
        assert!(app.stencil.is_some());
        assert!(app.ps().area.is_none());
    }

    #[test]
    fn another_menu_item_ends_stencil_paint_and_keeps_the_mask() {
        let mut app = PainterApp::new();
        act(&mut app, "stencil.paint");
        app.custom_brush = Some(square_brush(2));
        app.ui.tool = Tool::DottedFreehand;
        app.handle_press(Hit::Page { x: 5, y: 5 }, false);
        app.release();
        // CLR (`K` is CLR too) during Stencil Paint clears the mask, not the picture.
        app.run(Command::Clear { color: 4 });
        assert_eq!(page_px(&app, 0, 0), 0);
        app.handle_press(Hit::Page { x: 5, y: 5 }, false);
        app.release();
        act(&mut app, "picture.flip_horizontal");
        assert!(!app.stencil_painting());
        assert_eq!(app.ps().area.as_ref().unwrap().stencil.count(), 9);
        // The flip itself ran on the picture.
        assert_eq!(app.engine.document().revision(), 1);
    }

    #[test]
    fn stencil_items_act_on_the_area_stencil() {
        let mut app = PainterApp::new();
        app.set_area(
            AreaStencil::from_mask(320, 256, {
                let mut m = vec![false; 320 * 256];
                m[0] = true;
                m
            })
            .unwrap(),
        );
        act(&mut app, "stencil.reverse");
        let a = app.active_area().unwrap().clone();
        assert!(!a.protects(0, 0) && a.protects(1, 0));
        act(&mut app, "stencil.show");
        assert!(app.menu_data.effects.stencil_show);
        assert_eq!(app.ui.flags, "s");
        // Show: painting is off, and the view dims the free pixels.
        app.prepare_stencil_view();
        assert!(app.anim.shown.take().is_some());
        app.handle_press(Hit::Page { x: 0, y: 0 }, false);
        app.release();
        assert_eq!(app.engine.document().revision(), 0);
        act(&mut app, "stencil.show");
        act(&mut app, "stencil.toggle");
        assert!(app.active_area().is_none());
        assert_eq!(app.ui.flags, "");
        act(&mut app, "stencil.toggle");
        assert!(app.active_area().is_some());
        act(&mut app, "stencil.save");
        assert!(app.ui.note_requester.take().is_some());
        assert!(app.ui.file_requester.is_none());
        act(&mut app, "stencil.free");
        assert!(app.ps().area.is_none());
        assert_eq!(app.ui.flags, "");
        // A colour stencil made later discards an area stencil.
        app.set_area(AreaStencil::from_mask(320, 256, vec![true; 320 * 256]).unwrap());
        app.stencil = Some(vec![true; 256]);
        app.refresh_effects();
        assert!(app.ps().area.is_none());
    }

    #[test]
    fn area_stencils_apply_to_their_frame_and_page_size_only() {
        let mut app = PainterApp::new();
        app.engine
            .execute(Command::SetFrameCount { count: 2 })
            .unwrap();
        app.set_area(AreaStencil::from_mask(320, 256, vec![true; 320 * 256]).unwrap());
        app.run(Command::Clear { color: 7 });
        assert_eq!(page_px(&app, 0, 0), 0, "protected");
        app.engine.execute(Command::GotoFrame { frame: 1 }).unwrap();
        app.refresh_effects();
        app.run(Command::Clear { color: 7 });
        assert_eq!(page_px(&app, 0, 0), 7, "another frame");
        assert!(app.ps().area.is_some(), "kept for its own frame");
        app.run(Command::SetPageSize {
            width: 400,
            height: 300,
            fill: 0,
        });
        app.refresh_effects();
        assert!(app.ps().area.is_none(), "another page size ends it");
    }

    #[test]
    fn lock_fg_protects_what_was_painted_since_fix_and_clr_spares_it() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 2 });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pick_menu(&mut app, &ctx, "background.lock_fg", 0.1);
        assert!(
            app.ui.note_requester.take().is_some(),
            "needs a fixed background"
        );
        act(&mut app, "background.fix");
        pick_menu(&mut app, &ctx, "background.lock_fg", 0.2);
        assert!(
            app.ui.note_requester.take().is_some(),
            "nothing painted yet"
        );
        // Paint a square in colour 6: the changed pixels count, whatever their colour.
        app.custom_brush = Some(square_brush(6));
        app.ui.tool = Tool::DottedFreehand;
        app.ui.mode = dpx_ui::PaintMode::Matte;
        click_page(&mut app, &ctx, (50, 50), false, 0.3);
        pick_menu(&mut app, &ctx, "background.lock_fg", 0.4);
        assert!(app.ui.note_requester.is_none());
        assert_eq!(app.active_area().map(|a| a.count()), Some(9));
        assert_eq!(app.ui.flags, "BS");
        // Painting goes behind the locked foreground ...
        app.custom_brush = Some(square_brush(8));
        click_page(&mut app, &ctx, (52, 52), false, 0.5);
        assert_eq!(page_px(&app, 51, 51), 6);
        assert_eq!(page_px(&app, 53, 53), 8);
        // ... and CLR restores the fixed background around it (tutorial AGA p.119).
        app.run(Command::Clear { color: 0 });
        assert_eq!(page_px(&app, 51, 51), 6);
        assert_eq!(page_px(&app, 53, 53), 2);
        // One undo step each.
        app.undo();
        assert_eq!(page_px(&app, 53, 53), 8);
    }

    /// A brush file in `dir` with `palette` and `ranges`, loaded through the file requester.
    fn load_brush_file(app: &mut PainterApp, name: &str, palette: &Palette, ranges: &[ColorRange]) {
        let dir = std::env::temp_dir().join(format!("dpx-adr39-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let brush = BrushImage {
            width: 3,
            height: 1,
            pixels: vec![0, 4, 5],
            handle: (1, 0),
            transparent: Some(0),
        };
        let bytes = crate::brush_menu::encode_brush(&brush, palette, ranges).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        std::fs::write(dir.join("b.brush"), bytes).unwrap();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.open_file_requester(FileKind::LoadBrush);
        for c in "b.brush".chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "loaded");
        app.ui.note_requester = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn use_brush_palette_adopts_the_brush_file_palette_and_ranges_undoably() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        let mut colors = Palette::dpaint_default_32().colors().to_vec();
        colors[4] = Rgb8::new(10, 200, 30);
        colors[5] = Rgb8::new(250, 120, 0);
        let palette = Palette::new(colors.clone()).unwrap();
        let range = ColorRange {
            rate: 4096,
            flags: 1,
            low: 4,
            high: 5,
        };
        load_brush_file(&mut app, "use", &palette, &[range]);
        let fitted = app.custom_brush.clone().unwrap();
        assert_ne!(fitted.pixels, [0, 4, 5], "fitted to the picture palette");
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pick_menu(&mut app, &ctx, "palette.use_brush", 0.1);
        assert_eq!(app.engine.document().palette().colors(), colors.as_slice());
        assert_eq!(app.custom_brush.as_ref().unwrap().pixels, [0, 4, 5]);
        assert_eq!(app.extras.ranges, vec![range]);
        assert!(app.ui.note_requester.is_none());
        // Color ▸ Remap can fit the picture afterwards (the palette changed by one command).
        assert_eq!(
            app.menu_data.effects.remap_base.as_ref(),
            Some(&Palette::dpaint_default_32())
        );
        // Restore (`B`) gives the brush in the new palette too; a second use changes nothing.
        assert_eq!(app.picked_brush, app.custom_brush);
        let revision = app.engine.document().revision();
        act(&mut app, "palette.use_brush");
        assert_eq!(app.engine.document().revision(), revision);
        // The palette change is one undo step.
        app.undo();
        assert_eq!(
            app.engine.document().palette(),
            &Palette::dpaint_default_32()
        );
    }

    #[test]
    fn use_brush_palette_recomputes_a_bigger_palette() {
        let mut app = PainterApp::new();
        // A 4-colour picture and a brush file with 32 colours using colours 4 and 5.
        app.engine
            .execute(Command::SetFormat {
                mode: dpx_core::DisplayMode::PAL_LORES,
                colors: 4,
                width: 320,
                height: 256,
                retain: false,
                screen: None,
            })
            .unwrap();
        let mut colors = Palette::dpaint_default_32().colors().to_vec();
        colors[4] = Rgb8::new(10, 200, 30);
        colors[5] = Rgb8::new(250, 120, 0);
        load_brush_file(&mut app, "big", &Palette::new(colors.clone()).unwrap(), &[]);
        act(&mut app, "palette.use_brush");
        let pal = app.engine.document().palette().clone();
        assert_eq!(pal.len(), 4);
        let b = app.custom_brush.clone().unwrap();
        assert_eq!(b.transparent, Some(b.pixels[0]));
        let shown = |i: usize| pal.get(b.pixels[i]).unwrap();
        assert!(shown(1).distance_sq(colors[4]) < 3 * 40 * 40);
        assert!(shown(2).distance_sq(colors[5]) < 3 * 40 * 40);
        let note = app.ui.note_requester.as_ref().expect("a note");
        assert_eq!(note.title(), USE_BRUSH_PALETTE);
    }

    #[test]
    fn use_brush_palette_on_a_picked_up_brush_brings_back_the_palette_of_the_pick_up() {
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 7 });
        app.ui.tool = Tool::BrushSelect;
        app.handle_press(Hit::Page { x: 0, y: 0 }, false);
        app.handle_drag(3, 3);
        app.release();
        let picked = app.custom_brush.clone().expect("picked up");
        let old = app.engine.document().palette().clone();
        let mut colors = old.colors().to_vec();
        colors[7] = Rgb8::new(1, 2, 3);
        app.run(Command::SetPalette { colors });
        act(&mut app, "palette.use_brush");
        assert_eq!(app.engine.document().palette(), &old);
        assert_eq!(app.custom_brush, Some(picked));
    }
}
