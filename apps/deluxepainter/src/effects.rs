//! Palette operations and the Effect menu (ADR-31): Color ▸ Palette ▸ Arrange, Color ▸ Remap,
//! Color Set Load/Save, Effect ▸ AntiAlias / Process / Translucency, Effect ▸ Stencil ▸ Show /
//! Load.. / Save..
//!
//! Every document change goes through a command: Arrange is `ArrangePalette`, Remap
//! `RemapColors` (every frame and layer, ADR-44). The paint effects are carried by the
//! stroke and shape commands (`fx`, see [`dpx_core::PaintFx`]); this module keeps their
//! settings, the title-bar flags and the display-only stencil view.

use std::path::Path;

use dpx_core::{AntiAlias, Command, PaintFx, Palette, ProcessMode, Rgb8, remap};
use dpx_ui::{ArrangeRequester, FileKind, NoteRequester, ToolRequester, TranslucencyRequester};

use crate::{PainterApp, files};

/// Default Translucency: 50 % of the picture stays (manual AGA p.277).
const DEFAULT_PERCENT: u8 = 50;
const STENCIL_FORMAT: &str = "dpx-stencil";

/// Settings of the Effect menu and the bookkeeping Remap and Stencil ▸ Show need.
#[derive(Clone, Debug)]
pub struct Effects {
    pub translucency_on: bool,
    /// Percent of the picture that stays; kept while Translucency is off.
    pub percent: u8,
    pub process_on: bool,
    /// Kept while Process is off.
    pub process_mode: ProcessMode,
    pub antialias: AntiAlias,
    /// Effect ▸ Stencil ▸ Show: the picture is shown dimmed with the protected colours at
    /// full brightness, and painting is off.
    pub stencil_show: bool,
    /// The palette shown instead of the document's while Show is on.
    pub stencil_view: Option<Palette>,
    /// The palette the picture's pixels were last in step with, while the palette has been
    /// changed since (what Color ▸ Remap maps from).
    pub remap_base: Option<Palette>,
    last_palette: Option<Palette>,
    last_revision: u64,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            translucency_on: false,
            percent: DEFAULT_PERCENT,
            process_on: false,
            process_mode: ProcessMode::Tint,
            antialias: AntiAlias::None,
            stencil_show: false,
            stencil_view: None,
            remap_base: None,
            last_palette: None,
            last_revision: 0,
        }
    }
}

impl Effects {
    /// What strokes and shapes carry.
    pub fn fx(&self) -> PaintFx {
        PaintFx {
            translucency: self.translucency_on.then_some(self.percent),
            process: self.process_on.then_some(self.process_mode),
            antialias: self.antialias,
        }
    }
}

/// The palette of the Show view: unprotected colours at half brightness, protected ones as
/// they are.
pub fn stencil_view(palette: &Palette, stencil: &[bool]) -> Palette {
    let colors = palette
        .colors()
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            if stencil.get(i).copied().unwrap_or(false) {
                c
            } else {
                Rgb8::new(c.r / 2, c.g / 2, c.b / 2)
            }
        })
        .collect();
    Palette::new(colors).expect("same number of colours")
}

/// Encodes a stencil table as the text of a stencil file: JSON with the protected colours as
/// palette indices (ADR-31).
pub fn encode_stencil(stencil: &[bool]) -> String {
    let protected: Vec<usize> = stencil
        .iter()
        .enumerate()
        .filter(|&(_, &p)| p)
        .map(|(i, _)| i)
        .collect();
    let file = serde_json::json!({
        "format": STENCIL_FORMAT,
        "version": 1,
        "protected": protected,
    });
    file.to_string() + "\n"
}

/// Reads a stencil file into a 256-entry table.
pub fn decode_stencil(text: &str) -> Result<Vec<bool>, String> {
    let not = || "not a stencil file".to_string();
    let file: serde_json::Value = serde_json::from_str(text).map_err(|_| not())?;
    if file["format"] != STENCIL_FORMAT {
        return Err(not());
    }
    if file["version"] != 1 {
        return Err(format!(
            "stencil file version {} is not known",
            file["version"]
        ));
    }
    let mut table = vec![false; 256];
    for c in file["protected"].as_array().ok_or_else(not)? {
        let c = c.as_u64().filter(|&c| c < 256).ok_or_else(not)?;
        table[c as usize] = true;
    }
    Ok(table)
}

impl PainterApp {
    /// The Color and Effect items of ADR-31.
    pub(crate) fn effect_action(&mut self, id: &str) {
        match id {
            "palette.arrange" => self.open_arrange(),
            "color.remap" => self.remap_picture(),
            // The Color Set is the picture's palette here (at most 256 colours); there is no
            // second 256-slot set behind it, so these are the palette files.
            "color_set.load" => self.open_file_requester(FileKind::LoadPalette),
            "color_set.save" => self.open_file_requester(FileKind::SavePalette),
            "antialias.none" => self.menu_data.effects.antialias = AntiAlias::None,
            "antialias.low" => self.menu_data.effects.antialias = AntiAlias::Low,
            "antialias.high" => self.menu_data.effects.antialias = AntiAlias::High,
            "process.toggle" => self.menu_data.effects.process_on ^= true,
            "process.tint" => self.menu_data.effects.process_mode = ProcessMode::Tint,
            "process.hue" => self.menu_data.effects.process_mode = ProcessMode::Hue,
            "process.value" => self.menu_data.effects.process_mode = ProcessMode::Value,
            "translucency.toggle" => self.menu_data.effects.translucency_on ^= true,
            "translucency.settings" => {
                self.finish_stroke();
                self.ui.tool_requester = Some(ToolRequester::Translucency(
                    TranslucencyRequester::new(self.menu_data.effects.percent),
                ));
            }
            "stencil.show" => self.toggle_stencil_show(),
            "stencil.load" => self.open_file_requester(FileKind::LoadStencil),
            "stencil.save" => {
                if self.stencil.is_some() || self.tools.stencil_off.is_some() {
                    self.open_file_requester(FileKind::SaveStencil);
                } else {
                    self.tell("Save Stencil", "There is no stencil to save");
                }
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn tell(&mut self, title: &str, note: &str) {
        self.ui.note_requester = Some(NoteRequester::new(title, &[note.to_string()]));
    }

    /// Color ▸ Palette ▸ Arrange (`P`).
    fn open_arrange(&mut self) {
        self.finish_stroke();
        self.ui.tool_requester = Some(ToolRequester::Arrange(ArrangeRequester::new(
            self.engine.document().palette().len(),
        )));
    }

    /// The Arrange requester's Ok: one `ArrangePalette` command, and everything that holds
    /// colour numbers follows (FG and BG, the stencil, the brushes, the spare page, the fixed
    /// background), so nothing changes its look.
    pub(crate) fn apply_arrange(&mut self, order: &[u8]) {
        if order == remap::identity(order.len()).as_slice() {
            return;
        }
        self.finish_stroke();
        self.track_palette();
        let base = self.menu_data.effects.remap_base.take();
        let old_palette = self.engine.document().palette().clone();
        if let Err(e) = self.engine.execute(Command::ArrangePalette {
            order: order.to_vec(),
        }) {
            eprintln!("arrange palette: {e}");
            return;
        }
        let lut = remap::inverse(order);
        let n = order.len();
        self.ui.fg = lut[usize::from(self.ui.fg)];
        self.ui.bg = lut[usize::from(self.ui.bg)];
        let arrange_stencil = |s: &mut Vec<bool>| {
            let old = s.clone();
            for (new, &o) in order.iter().enumerate() {
                s[new] = old[usize::from(o)];
            }
        };
        if let Some(s) = &mut self.stencil {
            arrange_stencil(s);
        }
        if let Some(s) = &mut self.tools.stencil_off {
            arrange_stencil(s);
        }
        for b in [&mut self.custom_brush, &mut self.picked_brush]
            .into_iter()
            .flatten()
        {
            for p in &mut b.pixels {
                *p = lut[usize::from(*p)];
            }
            b.transparent = b.transparent.map(|t| lut[usize::from(t)]);
        }
        self.animbrush_arrange(&lut); // ADR-36: the AnimBrush's cels too.
        if let Some(bg) = &mut self.menu_data.background {
            let pixels = remap::reindex(bg.pixels(), order);
            *bg = dpx_core::IndexedImage::from_pixels(bg.width(), bg.height(), pixels)
                .expect("same size");
        }
        if let Some(spare) = &mut self.menu_data.spare
            && spare.engine.document().palette() == &old_palette
            && spare.engine.document().palette().len() == n
        {
            let _ = spare.engine.execute(Command::ArrangePalette {
                order: order.to_vec(),
            });
            spare.bg = lut[usize::from(spare.bg)];
        }
        // The arrangement is a palette change the picture is already in step with; a pending
        // Remap base is arranged along with the pixels.
        self.track_palette();
        self.menu_data.effects.remap_base = base
            .filter(|b| b.len() == n)
            .and_then(|b| Palette::new(remap::arranged_palette(b.colors(), order)).ok());
        self.dirty = true;
    }

    /// Color ▸ Remap: the picture is repainted with the colours of the palette before its
    /// last change, looked up in the palette now (nearest colour); one undoable step.
    fn remap_picture(&mut self) {
        self.finish_stroke();
        let Some(base) = self.menu_data.effects.remap_base.take() else {
            return self.tell(
                "Remap",
                "The palette has not changed since the picture was last in step with it",
            );
        };
        // ADR-44: every frame and layer, one undo step (colour 0 stays transparent above the
        // bottom layer); nothing is run when no pixel would change.
        let doc = self.engine.document();
        let lower = remap::nearest_lut(base.colors(), doc.palette());
        let upper = remap::nearest_lut_upper(base.colors(), doc.palette());
        let changes = (0..doc.layer_count()).any(|l| {
            let lut = if l == 0 { &lower } else { &upper };
            (0..doc.frame_count()).any(|f| {
                doc.cel(f, l)
                    .is_some_and(|c| c.pixels().iter().any(|&p| lut[usize::from(p)] != p))
            })
        });
        if changes {
            self.run(Command::RemapColors {
                from: base.colors().to_vec(),
            });
        }
    }

    /// Keeps track of what Remap maps from: the palette is remembered when it changes by a
    /// single command; a picture change (a stroke, a load, another page) forgets it. Called
    /// every frame.
    pub(crate) fn track_palette(&mut self) {
        let doc = self.engine.document();
        let (palette, revision) = (doc.palette(), doc.revision());
        let e = &mut self.menu_data.effects;
        if let Some(last) = &e.last_palette {
            if last != palette {
                if revision == e.last_revision + 1 {
                    if e.remap_base.is_none() {
                        e.remap_base = Some(last.clone());
                    }
                } else {
                    e.remap_base = None;
                }
            } else if revision != e.last_revision {
                e.remap_base = None;
            }
        }
        if e.last_palette.as_ref() != Some(palette) {
            e.last_palette = Some(palette.clone());
        }
        e.last_revision = revision;
    }

    /// Effect ▸ Stencil ▸ Show: on only while there is a stencil.
    fn toggle_stencil_show(&mut self) {
        let e = &mut self.menu_data.effects;
        e.stencil_show = !e.stencil_show && self.stencil.is_some();
    }

    /// Brings the title-bar flags and the Show view up to date; called every frame.
    pub(crate) fn refresh_effects(&mut self) {
        self.track_palette();
        // ADR-39: the area stencil counts as a stencil for Show and the flag.
        self.refresh_area();
        if self.stencil.is_none() && self.active_area().is_none() {
            self.menu_data.effects.stencil_show = false;
        }
        let stencil_flag = self.stencil_flag();
        let e = &mut self.menu_data.effects;
        let doc_palette = self.engine.document().palette();
        let view = match (&self.stencil, e.stencil_show) {
            (Some(s), true) => Some(stencil_view(doc_palette, s)),
            _ => None,
        };
        let mut flags = Vec::new();
        match e.antialias {
            AntiAlias::High => flags.push("A"),
            AntiAlias::Low => flags.push("a"),
            AntiAlias::None => {}
        }
        if self.menu_data.background.is_some() {
            flags.push("B");
        }
        if e.process_on {
            flags.push("P");
        }
        flags.extend(stencil_flag);
        if e.translucency_on {
            flags.push("T");
        }
        let flags = flags.concat();
        if e.stencil_view != view || self.ui.flags != flags {
            e.stencil_view = view;
            self.ui.flags = flags;
            self.dirty = true;
        }
    }

    /// The Translucency requester's Ok.
    pub(crate) fn set_translucency(&mut self, r: &TranslucencyRequester) {
        self.menu_data.effects.percent = r.percent();
    }

    /// The file requester's answer for Load Stencil and Save Stencil.
    pub(crate) fn stencil_file_accept(&mut self, kind: FileKind, path: &Path, overwrite: bool) {
        match kind {
            FileKind::LoadStencil => self.load_stencil(path),
            _ => self.save_stencil(path, overwrite),
        }
    }

    /// Effect ▸ Stencil ▸ Load..: the file's colours replace the stencil.
    fn load_stencil(&mut self, path: &Path) {
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| decode_stencil(&t));
        let table = match result {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return self.requester_message(e);
            }
        };
        let colors = self.palette().len();
        let mut notes = Vec::new();
        let mut table = table;
        if table[colors.min(256)..].iter().any(|&p| p) {
            table[colors.min(256)..].fill(false);
            notes.push(format!(
                "colours beyond the picture's {colors} were left out of the stencil"
            ));
        }
        self.tools.stencil_off = None;
        self.stencil = table.iter().any(|&p| p).then_some(table);
        self.ui.file_requester = None;
        self.ui.note_requester = crate::note_requester("Stencil loaded", &notes);
    }

    /// Effect ▸ Stencil ▸ Save..: the protected colours as a small text file.
    fn save_stencil(&mut self, path: &Path, overwrite: bool) {
        if path.is_dir() {
            return self.requester_message("That is a drawer".into());
        }
        if path.exists() && !overwrite {
            if let Some(req) = &mut self.ui.file_requester {
                req.ask_overwrite();
            }
            return;
        }
        let Some(stencil) = self.stencil.as_ref().or(self.tools.stencil_off.as_ref()) else {
            return self.requester_message("There is no stencil to save".into());
        };
        let text = encode_stencil(stencil);
        match files::write_atomic(path, text.as_bytes()) {
            Ok(()) => self.ui.file_requester = None,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{BrushImage, DrawMode, IndexedImage};
    use dpx_ui::{Action, Hit, ReqEvent, ReqKey, Tool};
    use eframe::egui;

    fn act(app: &mut PainterApp, id: &'static str) {
        app.run_action(Action::Command(id), &egui::Context::default());
        app.refresh_effects();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-fx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    fn type_into(app: &mut PainterApp, text: &str) {
        for c in text.chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
    }

    /// The shown colour of page pixel (x, y).
    fn rgb(app: &PainterApp, x: i32, y: i32) -> Rgb8 {
        let doc = app.engine.document();
        doc.palette().get(doc.page().get(x, y).unwrap()).unwrap()
    }

    fn drag_swatch(app: &mut PainterApp, from: usize, to: usize) {
        let Some(ToolRequester::Arrange(r)) = &app.ui.tool_requester else {
            panic!("the Arrange requester is open");
        };
        let (a, b) = (r.cell_rect(from), r.cell_rect(to));
        app.tool_requester_event(ReqEvent::Press(a.x + 1, a.y + 1));
        app.tool_requester_event(ReqEvent::Drag(b.x + 1, b.y + 1));
        app.tool_requester_event(ReqEvent::Release);
    }

    fn painted_app() -> PainterApp {
        let mut app = PainterApp::new();
        let mut page = IndexedImage::new(320, 256, 0);
        for (i, c) in [1u8, 5, 9, 20, 31].into_iter().enumerate() {
            page.set(i as i32 * 3, 2, c);
        }
        let doc = dpx_core::Document::from_picture(
            page,
            Palette::dpaint_default_32(),
            app.engine.document().target(),
        );
        app.engine = dpx_core::Engine::new(doc);
        app
    }

    fn row(app: &PainterApp) -> Vec<Rgb8> {
        (0..5).map(|i| rgb(app, i * 3, 2)).collect()
    }

    #[test]
    fn arrange_is_one_undoable_step_and_the_picture_looks_the_same() {
        let mut app = painted_app();
        let before = row(&app);
        app.ui.fg = 5;
        app.ui.bg = 20;
        let mut stencil = vec![false; 256];
        stencil[9] = true;
        app.stencil = Some(stencil);
        act(&mut app, "palette.arrange");
        assert!(matches!(
            app.ui.tool_requester,
            Some(ToolRequester::Arrange(_))
        ));
        drag_swatch(&mut app, 9, 0);
        drag_swatch(&mut app, 31, 2);
        app.tool_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.tool_requester.is_none());
        assert_eq!(app.engine.document().revision(), 1, "one command");
        assert_eq!(before, row(&app), "pixel-exact: every colour is the same");
        // Colour 9 now sits at 0; FG, BG and the stencil followed their colours.
        let orig = Palette::dpaint_default_32();
        assert_eq!(app.palette().get(0), orig.get(9));
        assert_eq!(app.engine.document().page().get(6, 2), Some(0));
        assert_eq!(app.palette().get(app.ui.fg), orig.get(5));
        assert_eq!(app.palette().get(app.ui.bg), orig.get(20));
        let s = app.stencil.as_ref().unwrap();
        assert!(s[0] && s.iter().filter(|&&p| p).count() == 1);
        app.undo();
        assert_eq!(app.palette(), &orig);
        assert_eq!(app.engine.document().page().get(6, 2), Some(9));
    }

    #[test]
    fn arrange_cancel_and_no_change_do_nothing() {
        let mut app = painted_app();
        act(&mut app, "palette.arrange");
        drag_swatch(&mut app, 3, 8);
        app.tool_requester_event(ReqEvent::Key(ReqKey::Escape));
        assert_eq!(app.engine.document().revision(), 0);
        act(&mut app, "palette.arrange");
        app.tool_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert_eq!(app.engine.document().revision(), 0, "identity order");
    }

    #[test]
    fn arrange_keeps_brushes_and_the_spare_page_in_step() {
        let mut app = painted_app();
        let img = IndexedImage::from_pixels(2, 1, vec![5, 9]).unwrap();
        app.custom_brush = Some(BrushImage::from_image(&img, Some(9)));
        act(&mut app, "spare.swap");
        act(&mut app, "spare.swap");
        act(&mut app, "palette.arrange");
        drag_swatch(&mut app, 5, 20);
        app.tool_requester_event(ReqEvent::Key(ReqKey::Enter));
        let b = app.custom_brush.as_ref().unwrap();
        assert_eq!(b.pixels, vec![20, 8], "5 moved to 20, 9 shifted down to 8");
        assert_eq!(b.transparent, Some(8));
        let spare = app.menu_data.spare.as_ref().unwrap();
        assert_eq!(
            spare.engine.document().palette(),
            app.engine.document().palette()
        );
    }

    #[test]
    fn remap_follows_a_palette_change_in_one_undoable_step() {
        let mut app = painted_app();
        app.refresh_effects();
        let expect = row(&app);
        // Exchange colours 5 and 9 in the palette (as the mixer's EX does): the picture's
        // colours change.
        let mut colors = app.palette().colors().to_vec();
        colors.swap(5, 9);
        app.run(Command::SetPalette { colors });
        app.refresh_effects();
        assert_ne!(row(&app), expect);
        act(&mut app, "color.remap");
        assert_eq!(
            row(&app),
            expect,
            "the picture looks as before the palette change"
        );
        assert_eq!(app.engine.document().page().get(3, 2), Some(9));
        assert_eq!(app.engine.document().revision(), 2);
        app.undo();
        assert_eq!(app.engine.document().page().get(3, 2), Some(5));
        // Nothing left to remap: a note, no command.
        app.refresh_effects();
        act(&mut app, "color.remap");
        assert!(app.ui.note_requester.is_some());
    }

    #[test]
    fn painting_after_a_palette_change_forgets_the_remap_base() {
        let mut app = painted_app();
        app.refresh_effects();
        let mut colors = app.palette().colors().to_vec();
        colors.swap(5, 9);
        app.run(Command::SetPalette { colors });
        app.refresh_effects();
        assert!(app.menu_data.effects.remap_base.is_some());
        app.run(Command::Clear { color: 0 });
        app.refresh_effects();
        assert!(app.menu_data.effects.remap_base.is_none());
        let rev = app.engine.document().revision();
        act(&mut app, "color.remap");
        assert_eq!(app.engine.document().revision(), rev);
    }

    #[test]
    fn arrange_does_not_leave_a_remap_base() {
        let mut app = painted_app();
        app.refresh_effects();
        act(&mut app, "palette.arrange");
        drag_swatch(&mut app, 1, 7);
        app.tool_requester_event(ReqEvent::Key(ReqKey::Enter));
        app.refresh_effects();
        assert!(app.menu_data.effects.remap_base.is_none());
    }

    fn paint_dot(app: &mut PainterApp, x: i32, y: i32) {
        app.handle_press(Hit::Page { x, y }, false);
        app.release();
    }

    #[test]
    fn translucency_blends_the_stroke_with_the_page() {
        let mut app = PainterApp::new();
        app.select_tool(Tool::ContinuousFreehand, false);
        app.ui.fg = 3;
        let mut plain = PainterApp::new();
        plain.select_tool(Tool::ContinuousFreehand, false);
        plain.ui.fg = 3;
        paint_dot(&mut plain, 40, 40);
        assert_eq!(plain.engine.document().page().get(40, 40), Some(3));
        act(&mut app, "translucency.toggle");
        assert_eq!(app.ui.flags, "T");
        paint_dot(&mut app, 40, 40);
        // Default 50 %: half of white over black (colour 0).
        let pal = app.palette().clone();
        let white = pal.get(3).unwrap();
        let half = |v: u8| u32::from(v).div_ceil(2) as u8;
        let want = pal.nearest(Rgb8::new(half(white.r), half(white.g), half(white.b)));
        assert_eq!(app.engine.document().page().get(40, 40), Some(want));
        assert_ne!(want, 3);
        assert_ne!(want, 0);
        // Settings: 100 % keeps the page.
        act(&mut app, "translucency.settings");
        let Some(ToolRequester::Translucency(_)) = &app.ui.tool_requester else {
            panic!("settings requester");
        };
        for _ in 0..3 {
            app.tool_requester_event(ReqEvent::Key(ReqKey::Backspace));
        }
        for c in "100".chars() {
            app.tool_requester_event(ReqEvent::Key(ReqKey::Char(c)));
        }
        app.tool_requester_event(ReqEvent::Key(ReqKey::Enter));
        assert_eq!(app.menu_data.effects.percent, 100);
        paint_dot(&mut app, 60, 60);
        assert_eq!(app.engine.document().page().get(60, 60), Some(0));
        // Off again: solid paint.
        act(&mut app, "translucency.toggle");
        assert_eq!(app.ui.flags, "");
        paint_dot(&mut app, 80, 80);
        assert_eq!(app.engine.document().page().get(80, 80), Some(3));
    }

    #[test]
    fn process_and_antialias_set_the_flags_and_reach_the_commands() {
        let mut app = PainterApp::new();
        act(&mut app, "process.toggle");
        act(&mut app, "process.hue");
        act(&mut app, "antialias.high");
        assert_eq!(app.ui.flags, "AP");
        let fx = app.menu_data.effects.fx();
        assert_eq!(fx.process, Some(ProcessMode::Hue));
        assert_eq!(fx.antialias, AntiAlias::High);
        act(&mut app, "antialias.low");
        assert_eq!(app.ui.flags, "aP");
        act(&mut app, "antialias.none");
        act(&mut app, "process.toggle");
        assert_eq!(app.ui.flags, "");
        assert!(app.menu_data.effects.fx().is_off());
        // Process reaches the stroke: on a grey page, Hue with FG red stays grey (no
        // saturation to turn), Tint turns it red-ish.
        let pal = app.palette().clone();
        let grey = pal.nearest(Rgb8::new(0x66, 0x66, 0x66));
        app.run(Command::Clear { color: grey });
        app.select_tool(Tool::ContinuousFreehand, false);
        app.ui.fg = pal.nearest(Rgb8::new(255, 0, 0));
        act(&mut app, "process.toggle");
        act(&mut app, "process.hue");
        paint_dot(&mut app, 30, 30);
        assert_eq!(app.engine.document().page().get(30, 30), Some(grey));
        act(&mut app, "process.tint");
        paint_dot(&mut app, 50, 50);
        let tinted = app.engine.document().page().get(50, 50).unwrap();
        assert_ne!(tinted, grey);
        let c = pal.get(tinted).unwrap();
        assert!(c.r > c.g && c.r > c.b, "reddish: {c}");
    }

    #[test]
    fn antialias_blends_only_the_edge_of_a_custom_brush() {
        let pal = Palette::dpaint_default_32();
        let brush = BrushImage::from_image(&IndexedImage::new(5, 5, 3), Some(0));
        let run = |aa| {
            let mut e = dpx_core::Engine::new(dpx_core::Document::new(16, 16, pal.clone()));
            e.execute(Command::Stroke {
                brush: Vec::new(),
                color: 1,
                points: vec![(8, 8)],
                continuous: false,
                custom: Some(brush.clone()),
                mode: DrawMode::Matte,
                range: None,
                stencil: None,
                right: false,
                fx: PaintFx {
                    antialias: aa,
                    ..PaintFx::default()
                },
            })
            .unwrap();
            e.document().page().clone()
        };
        let none = run(AntiAlias::None);
        assert_eq!(none.get(6, 6), Some(3));
        let low = run(AntiAlias::Low);
        assert_eq!(low.get(8, 8), Some(3), "inner pixel: full colour");
        assert_eq!(low.get(7, 8), Some(3));
        let white = pal.get(3).unwrap();
        let mix = |w: u32| {
            let c = ((u32::from(white.r) * w + 50) / 100) as u8;
            pal.nearest(Rgb8::new(c, c, c))
        };
        assert_eq!(low.get(6, 8), Some(mix(50)), "edge: half the brush colour");
        assert_eq!(low.get(6, 6), Some(mix(50)), "corner");
        let high = run(AntiAlias::High);
        assert_eq!(high.get(8, 8), Some(3));
        assert_eq!(high.get(6, 8), Some(mix(66)));
        assert_eq!(high.get(6, 6), Some(mix(44)));
        assert_eq!(high.get(5, 5), Some(0), "outside the brush");
    }

    #[test]
    fn stencil_show_dims_the_picture_and_disables_painting() {
        let mut app = painted_app();
        act(&mut app, "stencil.show");
        assert!(!app.menu_data.effects.stencil_show, "needs a stencil");
        let mut stencil = vec![false; 256];
        stencil[9] = true;
        app.stencil = Some(stencil);
        act(&mut app, "stencil.show");
        assert_eq!(app.ui.flags, "s");
        let doc = app.engine.document().palette().clone();
        let shown = app.palette().clone();
        assert_eq!(shown.get(9), doc.get(9), "protected colour as it is");
        let c = doc.get(5).unwrap();
        assert_eq!(shown.get(5), Some(Rgb8::new(c.r / 2, c.g / 2, c.b / 2)));
        assert_eq!(app.engine.document().palette(), &doc, "document untouched");
        app.select_tool(Tool::ContinuousFreehand, false);
        paint_dot(&mut app, 100, 100);
        assert_eq!(app.engine.document().revision(), 0, "painting is off");
        act(&mut app, "stencil.show");
        assert_eq!(app.ui.flags, "S");
        assert_eq!(app.palette(), &doc);
        paint_dot(&mut app, 100, 100);
        assert_eq!(app.engine.document().revision(), 1);
        // Show ends when the stencil is freed.
        act(&mut app, "stencil.show");
        act(&mut app, "stencil.free");
        assert!(!app.menu_data.effects.stencil_show);
        assert_eq!(app.palette(), &doc);
    }

    #[test]
    fn stencil_files_round_trip_through_the_file_requester() {
        let dir = temp_dir("stencil");
        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        act(&mut app, "stencil.save");
        assert!(app.ui.file_requester.is_none(), "no stencil: a note only");
        assert!(app.ui.note_requester.is_some());
        app.ui.note_requester = None;
        let mut s = vec![false; 256];
        s[2] = true;
        s[17] = true;
        app.stencil = Some(s.clone());
        act(&mut app, "stencil.save");
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().kind(),
            FileKind::SaveStencil
        );
        type_into(&mut app, "Test.stencil");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none());
        let text = std::fs::read_to_string(dir.join("Test.stencil")).unwrap();
        assert_eq!(decode_stencil(&text).unwrap(), s);

        app.stencil = None;
        act(&mut app, "stencil.load");
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().kind(),
            FileKind::LoadStencil
        );
        type_into(&mut app, "Test.stencil");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none());
        assert_eq!(app.stencil, Some(s));
        app.refresh_effects();
        assert_eq!(app.ui.flags, "S");

        // Not a stencil file: the requester stays open with a message.
        std::fs::write(dir.join("Bad.stencil"), "hello").unwrap();
        act(&mut app, "stencil.load");
        type_into(&mut app, "Bad.stencil");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stencil_file_text_is_checked() {
        assert!(decode_stencil("{\"format\":\"x\",\"version\":1,\"protected\":[]}").is_err());
        assert!(
            decode_stencil("{\"format\":\"dpx-stencil\",\"version\":2,\"protected\":[]}").is_err()
        );
        assert!(
            decode_stencil("{\"format\":\"dpx-stencil\",\"version\":1,\"protected\":[300]}")
                .is_err()
        );
        let t = decode_stencil(&encode_stencil(&[false, true, false, true])).unwrap();
        assert!(t[1] && t[3] && !t[0] && !t[2] && t.len() == 256);
    }

    #[test]
    fn color_set_items_are_the_palette_files() {
        let mut app = PainterApp::new();
        act(&mut app, "color_set.load");
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().kind(),
            FileKind::LoadPalette
        );
        app.ui.file_requester = None;
        act(&mut app, "color_set.save");
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().kind(),
            FileKind::SavePalette
        );
    }

    #[test]
    fn effect_items_are_enabled() {
        let ctx = PainterApp::new().ui.menu_context();
        for id in [
            "palette.arrange",
            "color.remap",
            "stencil.show",
            "process.toggle",
        ] {
            assert!(Action::Command(id).enabled(&ctx), "{id}");
        }
    }
}
