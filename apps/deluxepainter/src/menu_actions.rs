//! Menu commands of ADR-15 and their keys: Picture ▸ Spare (the second page), Picture ▸
//! Delete.. and About.., Color ▸ Cycle, Palette ▸ Restore / Load.. / Save.., Effect ▸
//! Background ▸ Fix / Free.
//!
//! Pixel changes go through document commands (`MergePage`, `SetPage`); this module only
//! keeps the app-side state (the spare page, the fixed background, the cycling clock).

use std::path::Path;
use std::time::Duration;

use dpx_core::{Command, Document, Engine, IndexedImage, Palette, Rgb8, Target};
use dpx_iff::{ColorRange, iff};
use dpx_ui::{FileKind, NoteRequester, RangeData, RangeRequester, RangeResponse, ReqEvent};
use eframe::egui;

use crate::{PainterApp, files, note_requester};

/// Title of the note requester after Palette ▸ Load.
const PALETTE_LOADED: &str = "Palette loaded";

/// The page that is not shown (Picture ▸ Spare) with what belongs to it. Both pages share one
/// palette and screen format; each keeps its own undo history, scroll position, file name,
/// fixed background and BG colour (manual AGA p.69: "BG colours may differ").
pub struct OtherPage {
    pub engine: Engine,
    pub scroll: (i32, i32),
    pub picture_path: Option<std::path::PathBuf>,
    pub background: Option<IndexedImage>,
    pub bg: u8,
}

/// App state of the menu commands in ADR-15.
#[derive(Default)]
pub struct MenuData {
    /// The spare page; `None` until it is first used (a blank page then).
    pub spare: Option<OtherPage>,
    /// Effect ▸ Background ▸ Fix: the fixed picture of the shown page.
    pub background: Option<IndexedImage>,
    /// Palette as of the last picture load; `None` = the start palette.
    pub loaded_palette: Option<Palette>,
    /// Color ▸ Cycle: when cycling was switched on (egui time, seconds).
    pub cycle_start: Option<f64>,
    /// The palette shown while cycling (the document palette stays unchanged).
    pub cycled: Option<Palette>,
    /// Effect menu settings, the Remap base and the stencil view (ADR-31).
    pub effects: crate::effects::Effects,
}

impl MenuData {
    /// The palette to show for document palette `doc`: the cycled one while cycling.
    pub fn shown_palette<'a>(&'a self, doc: &'a Palette) -> &'a Palette {
        if let Some(v) = self
            .effects
            .stencil_view
            .as_ref()
            .filter(|v| v.len() == doc.len())
        {
            return v;
        }
        self.cycled
            .as_ref()
            .filter(|p| p.len() == doc.len())
            .unwrap_or(doc)
    }
}

/// `palette` with every active range of `ranges` rotated by the number of steps its rate
/// gives after `elapsed` seconds (rate 16384 = 60 steps per second). Forward ranges move each
/// colour one entry up, the highest wrapping to the lowest; reverse ranges move down.
pub fn cycled_palette(palette: &Palette, ranges: &[ColorRange], elapsed: f64) -> Palette {
    let mut colors = palette.colors().to_vec();
    for r in ranges {
        let (low, high) = (usize::from(r.low), usize::from(r.high));
        if !r.active() || r.rate == 0 || low >= high || high >= colors.len() {
            continue;
        }
        let len = high - low + 1;
        let steps = (elapsed.max(0.0) * r.steps_per_second()).floor() as u64 % len as u64;
        let range = &mut colors[low..=high];
        if r.reverse() {
            range.rotate_left(steps as usize);
        } else {
            range.rotate_right(steps as usize);
        }
    }
    Palette::new(colors).expect("same number of colours")
}

/// `src` laid over `current`: the first entries come from `src`, the rest stay.
fn fit_colors(src: &[Rgb8], current: &Palette) -> Vec<Rgb8> {
    current
        .colors()
        .iter()
        .enumerate()
        .map(|(i, &c)| src.get(i).copied().unwrap_or(c))
        .collect()
}

/// A DeluxePaint palette file: an ILBM FORM with BMHD and CMAP (plus the CRNG colour ranges
/// that fit the palette) and no BODY.
pub fn encode_palette(palette: &Palette, ranges: &[ColorRange], target: Target) -> Vec<u8> {
    let (w, h) = target.screen;
    let (xa, ya) = target.mode.aspect();
    let mut bmhd = Vec::with_capacity(20);
    bmhd.extend_from_slice(&(w as u16).to_be_bytes());
    bmhd.extend_from_slice(&(h as u16).to_be_bytes());
    bmhd.extend_from_slice(&[0, 0, 0, 0]); // x, y
    bmhd.push(dpx_iff::ilbm::planes_for(palette.len()));
    bmhd.extend_from_slice(&[0, 0, 0]); // no masking, no compression, pad
    bmhd.extend_from_slice(&0u16.to_be_bytes()); // transparent colour
    bmhd.extend_from_slice(&[xa, ya]);
    bmhd.extend_from_slice(&(w as i16).to_be_bytes());
    bmhd.extend_from_slice(&(h as i16).to_be_bytes());
    let mut form = iff::FormWriter::new(*b"ILBM");
    form.chunk(*b"BMHD", &bmhd);
    let cmap: Vec<u8> = palette
        .colors()
        .iter()
        .flat_map(|c| [c.r, c.g, c.b])
        .collect();
    form.chunk(*b"CMAP", &cmap);
    for r in ranges
        .iter()
        .filter(|r| usize::from(r.high) < palette.len())
    {
        let mut d = vec![0, 0];
        d.extend_from_slice(&r.rate.to_be_bytes());
        d.extend_from_slice(&r.flags.to_be_bytes());
        d.extend_from_slice(&[r.low, r.high]);
        form.chunk(*b"CRNG", &d);
    }
    form.finish()
}

/// Reads the colours (CMAP) and colour ranges (CRNG) of any ILBM: a palette file or a picture.
/// CMAPs of 4-bit programs (all low nibbles zero) are scaled up like the picture loader does.
pub fn decode_palette(bytes: &[u8]) -> Result<(Vec<Rgb8>, Vec<ColorRange>), String> {
    let (form_type, chunks) = iff::parse_form(bytes).map_err(|e| e.to_string())?;
    if &form_type != b"ILBM" {
        return Err(format!(
            "not an ILBM palette (FORM {})",
            iff::id_str(form_type)
        ));
    }
    let planes = chunks
        .iter()
        .find(|c| &c.id == b"BMHD" && c.data.len() >= 20)
        .map(|c| c.data[8]);
    let cmap = chunks
        .iter()
        .find(|c| &c.id == b"CMAP")
        .map(|c| c.data)
        .ok_or("no colours (CMAP) in this file")?;
    let mut n = (cmap.len() / 3).min(256);
    if let Some(p @ 1..=8) = planes {
        n = n.min(1 << p);
    }
    if n == 0 {
        return Err("the colour map (CMAP) is empty".into());
    }
    let bytes = &cmap[..n * 3];
    let fix = bytes.iter().all(|b| b & 0x0F == 0);
    let v = |b: u8| if fix { b | (b >> 4) } else { b };
    let colors = bytes
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| Rgb8::new(v(c[0]), v(c[1]), v(c[2])))
        .collect();
    let ranges = chunks
        .iter()
        .filter(|c| &c.id == b"CRNG" && c.data.len() >= 8)
        .map(|c| ColorRange {
            rate: u16::from_be_bytes([c.data[2], c.data[3]]),
            flags: u16::from_be_bytes([c.data[4], c.data[5]]),
            low: c.data[6],
            high: c.data[7],
        })
        .collect();
    Ok((colors, ranges))
}

impl PainterApp {
    /// Keys of the ADR-15 menu items on the paint screen, and the colour-cycling clock: `j`
    /// Spare ▸ Swap, Ctrl+`j` Copy to Spare, `k` CLR, Tab Cycle, `?` About.
    pub(crate) fn menu_keys(&mut self, ctx: &egui::Context) {
        let (now, events) = ctx.input(|i| (i.time, i.events.clone()));
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => {
                    let plain = !modifiers.any();
                    let ctrl_only = modifiers.ctrl && !(modifiers.alt || modifiers.shift);
                    match key {
                        egui::Key::J if plain => self.swap_spare(),
                        egui::Key::J if ctrl_only => self.copy_to_spare(),
                        egui::Key::K if plain => self.run(Command::Clear { color: self.ui.bg }),
                        egui::Key::Tab if plain => self.toggle_cycling(now),
                        _ => {}
                    }
                }
                egui::Event::Text(t) if t == "?" => self.show_about(),
                _ => {}
            }
        }
        if self.cycle_tick(now) {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// CLR on a fixed background restores it instead of filling the page with BG
    /// (manual AGA p.256). Other commands pass unchanged.
    pub(crate) fn with_fixed_background(&self, command: Command) -> Command {
        let page = self.engine.document().page();
        match (&command, &self.menu_data.background) {
            (Command::Clear { .. }, Some(bg))
                if (bg.width(), bg.height()) == (page.width(), page.height()) =>
            {
                Command::SetPage {
                    width: bg.width(),
                    height: bg.height(),
                    pixels: bg.pixels().to_vec(),
                }
            }
            _ => command,
        }
    }

    /// A blank spare page (colour 0) of the shown page's size and format.
    fn blank_page(&self) -> OtherPage {
        let doc = self.engine.document();
        let (w, h) = (doc.page().width(), doc.page().height());
        OtherPage {
            engine: Engine::new(Document::from_picture(
                IndexedImage::new(w, h, 0),
                doc.palette().clone(),
                doc.target(),
            )),
            scroll: (0, 0),
            picture_path: None,
            background: None,
            bg: self.ui.bg,
        }
    }

    /// The spare page made to match the shown page: a spare of another screen format is
    /// dropped (DeluxePaint discards the spare on a format change, AGA p.241), one with an
    /// older palette takes the current palette (its undo history starts over then).
    fn take_spare(&mut self) -> OtherPage {
        let doc = self.engine.document();
        let (target, palette) = (doc.target(), doc.palette().clone());
        match self.menu_data.spare.take() {
            Some(s) if s.engine.document().target() == target => {
                let spare = s.engine.document();
                if spare.palette() == &palette {
                    s
                } else if spare.palette().len() == palette.len() {
                    let page = spare.page().clone();
                    OtherPage {
                        engine: Engine::new(Document::from_picture(page, palette, target)),
                        ..s
                    }
                } else {
                    self.blank_page()
                }
            }
            _ => self.blank_page(),
        }
    }

    /// Picture ▸ Spare ▸ Swap (`j`): shows the other page.
    pub(crate) fn swap_spare(&mut self) {
        self.finish_stroke();
        let mut other = self.take_spare();
        std::mem::swap(&mut self.engine, &mut other.engine);
        std::mem::swap(&mut self.scroll, &mut other.scroll);
        std::mem::swap(&mut self.picture_path, &mut other.picture_path);
        std::mem::swap(&mut self.menu_data.background, &mut other.background);
        std::mem::swap(&mut self.ui.bg, &mut other.bg);
        self.menu_data.spare = Some(other);
        self.clamp_colors();
        self.clamp_scroll();
        self.dirty = true;
    }

    /// Picture ▸ Spare ▸ Copy to Spare (Ctrl+`j`): the spare becomes a copy of the shown page
    /// (one undo step on the spare page).
    pub(crate) fn copy_to_spare(&mut self) {
        self.finish_stroke();
        let page = self.engine.document().page().clone();
        let mut other = self.take_spare();
        let command = Command::SetPage {
            width: page.width(),
            height: page.height(),
            pixels: page.pixels().to_vec(),
        };
        if let Err(e) = other.engine.execute(command) {
            eprintln!("copy to spare: {e}");
            other.engine = Engine::new(self.engine.document().clone());
        }
        self.menu_data.spare = Some(other);
    }

    /// Picture ▸ Spare ▸ Merge in Front / Back (undoable). Colour 0 is transparent: in front,
    /// the spare's colour-0 pixels; in back, the shown page's (ADR-15).
    pub(crate) fn merge_spare(&mut self, front: bool) {
        if self.menu_data.spare.is_none() {
            return; // A blank spare changes nothing.
        }
        let other = self.take_spare();
        let page = other.engine.document().page();
        let command = Command::MergePage {
            width: page.width(),
            height: page.height(),
            pixels: page.pixels().to_vec(),
            front,
            transparent: 0,
        };
        self.menu_data.spare = Some(other);
        self.run(command);
    }

    /// Picture ▸ Spare ▸ Delete This Page: the shown page is replaced by the spare (one undo
    /// step brings it back) and the spare is freed.
    pub(crate) fn delete_page(&mut self) {
        self.finish_stroke();
        let other = self.take_spare();
        let page = other.engine.document().page();
        let command = Command::SetPage {
            width: page.width(),
            height: page.height(),
            pixels: page.pixels().to_vec(),
        };
        match self.engine.execute(command) {
            Ok(_) => {
                self.scroll = other.scroll;
                self.picture_path = other.picture_path;
                self.menu_data.background = other.background;
                self.ui.bg = other.bg;
                self.clamp_colors();
                self.clamp_scroll();
            }
            Err(e) => {
                eprintln!("delete this page: {e}");
                self.menu_data.spare = Some(other);
            }
        }
        self.dirty = true;
    }

    /// Picture ▸ About..: our own short text in the note requester.
    pub(crate) fn show_about(&mut self) {
        let doc = self.engine.document();
        let (w, h) = (doc.page().width(), doc.page().height());
        let notes = [
            format!(
                "DeluxePainter {}: a pixel-art editor in the spirit of DeluxePaint IV AGA",
                env!("CARGO_PKG_VERSION")
            ),
            "Written in Rust; source-available under the PolyForm Noncommercial License 1.0.0"
                .to_string(),
            "Not affiliated with Electronic Arts".to_string(),
            format!(
                "Page {w}x{h}, {} colours, {}",
                doc.palette().len(),
                doc.target().mode.name()
            ),
        ];
        self.ui.note_requester = Some(NoteRequester::new("About DeluxePainter", &notes));
        self.dirty = true;
    }

    /// Color ▸ Cycle (Tab): switches colour cycling on or off.
    pub(crate) fn toggle_cycling(&mut self, now: f64) {
        if self.menu_data.cycle_start.take().is_none() {
            self.menu_data.cycle_start = Some(now);
        }
        self.menu_data.cycled = None;
        self.cycle_tick(now);
        self.dirty = true;
    }

    /// Advances colour cycling to `now`; returns whether cycling is on.
    fn cycle_tick(&mut self, now: f64) -> bool {
        let Some(start) = self.menu_data.cycle_start else {
            return false;
        };
        let shown = cycled_palette(
            self.engine.document().palette(),
            &self.extras.ranges,
            now - start,
        );
        if self.menu_data.cycled.as_ref() != Some(&shown) {
            self.menu_data.cycled = Some(shown);
            self.dirty = true;
        }
        true
    }

    /// Palette ▸ Restore Palette: back to the palette of the last loaded picture (the start
    /// palette before any load); undoable.
    pub(crate) fn restore_palette(&mut self) {
        let base = self
            .menu_data
            .loaded_palette
            .clone()
            .unwrap_or_else(Palette::dpaint_default_32);
        let current = self.engine.document().palette();
        let colors = fit_colors(base.colors(), current);
        if colors != current.colors() {
            self.run(Command::SetPalette { colors });
        }
    }

    /// Effect ▸ Background ▸ Fix: CLR goes back to the page as it is now.
    pub(crate) fn fix_background(&mut self) {
        self.finish_stroke();
        self.menu_data.background = Some(self.engine.document().page().clone());
    }

    /// Effect ▸ Background ▸ Free: CLR fills with the BG colour again.
    pub(crate) fn free_background(&mut self) {
        self.menu_data.background = None;
    }

    /// The file requester's answer for Delete File, Load Palette and Save Palette.
    pub(crate) fn menu_file_accept(&mut self, kind: FileKind, path: &Path, overwrite: bool) {
        match kind {
            FileKind::DeleteFile => self.delete_file(path, overwrite),
            FileKind::LoadPalette => self.load_palette(path),
            FileKind::SavePalette => self.save_palette(path, overwrite),
            FileKind::LoadStencil | FileKind::SaveStencil => {
                self.stencil_file_accept(kind, path, overwrite)
            }
            FileKind::LoadPicture | FileKind::SavePicture | FileKind::ExportPicture => {}
            FileKind::LoadBrush | FileKind::SaveBrush => {
                self.brush_file_accept(kind, path, overwrite)
            }
        }
    }

    /// Picture ▸ Delete..: removes the file once the requester's Yes confirmed it.
    fn delete_file(&mut self, path: &Path, confirmed: bool) {
        if !confirmed {
            return;
        }
        if path.is_dir() {
            return self.requester_message("That is a drawer".into());
        }
        match std::fs::remove_file(path) {
            Ok(()) => self.ui.file_requester = None,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e.to_string());
            }
        }
    }

    /// Palette ▸ Load..: the colours of a palette file (or of any ILBM picture) replace the
    /// palette from colour 0 on (undoable); its colour ranges replace the current ones.
    fn load_palette(&mut self, path: &Path) {
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| decode_palette(&b));
        let (colors, ranges) = match result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                return self.requester_message(e);
            }
        };
        let current = self.engine.document().palette();
        let (have, want) = (colors.len(), current.len());
        let mut notes = Vec::new();
        if have > want {
            notes.push(format!(
                "the file has {have} colours; the first {want} were loaded (the picture has {want})"
            ));
        } else if have < want {
            notes.push(format!(
                "the file has {have} colours; colours {have}-{} are unchanged",
                want - 1
            ));
        }
        let new = fit_colors(&colors, current);
        if new != current.colors() {
            self.run(Command::SetPalette { colors: new });
        }
        if !ranges.is_empty() {
            self.extras.ranges = ranges;
        }
        self.ui.file_requester = None;
        self.ui.note_requester = note_requester(PALETTE_LOADED, &notes);
        self.dirty = true;
    }

    /// Palette ▸ Save..: writes the palette and colour ranges as an ILBM without BODY.
    fn save_palette(&mut self, path: &Path, overwrite: bool) {
        if path.is_dir() {
            return self.requester_message("That is a drawer".into());
        }
        if path.exists() && !overwrite {
            if let Some(req) = &mut self.ui.file_requester {
                req.ask_overwrite();
            }
            return;
        }
        let doc = self.engine.document();
        let bytes = encode_palette(doc.palette(), &self.extras.ranges, doc.target());
        match files::write_atomic(path, &bytes) {
            Ok(()) => self.ui.file_requester = None,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e);
            }
        }
    }
}

/// The Range requester over `palette` for the CRNG `ranges` (file order = range numbers).
pub fn range_requester(palette: &Palette, ranges: &[ColorRange]) -> RangeRequester {
    let data: Vec<RangeData> = ranges
        .iter()
        .map(|r| RangeData {
            rate: r.rate,
            flags: r.flags,
            low: r.low,
            high: r.high,
        })
        .collect();
    RangeRequester::new(palette, &data, usize::MAX)
}

impl PainterApp {
    /// Color ▸ Ranges..: opens the Range requester on a copy of the picture's ranges.
    pub(crate) fn open_ranges(&mut self) {
        self.finish_stroke();
        let req = range_requester(self.engine.document().palette(), &self.extras.ranges);
        self.ui.range_requester = Some(req);
        self.dirty = true;
    }

    /// Pointer and keys while the Range requester is open (it is modal).
    pub(crate) fn range_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: crate::View,
    ) {
        let ui_screen = view.ui_screen();
        let at = RangeRequester::origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.range_requester.is_none() {
                break;
            }
            self.range_event(event);
        }
    }

    /// Feeds one event to the Range requester. Ok stores the edited ranges as the picture's
    /// colour ranges (saved as CRNG chunks; they are file data, not part of the undo list);
    /// Cancel drops the edits.
    pub(crate) fn range_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.range_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        match response {
            RangeResponse::Continue => {}
            RangeResponse::Cancel => self.ui.range_requester = None,
            RangeResponse::Ok => {
                let Some(req) = self.ui.range_requester.take() else {
                    return;
                };
                self.extras.ranges = req
                    .ranges()
                    .iter()
                    .map(|r| ColorRange {
                        rate: r.rate,
                        flags: r.flags,
                        low: r.low,
                        high: r.high,
                    })
                    .collect();
                // The cycling clock restarts from the new ranges.
                self.menu_data.cycled = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_ui::{Action, ReqEvent, ReqKey, Tool};

    fn ctx() -> egui::Context {
        egui::Context::default()
    }

    fn act(app: &mut PainterApp, id: &'static str) {
        app.run_action(Action::Command(id), &ctx());
    }

    fn dot(app: &mut PainterApp, x: i32, y: i32, color: u8) {
        app.run(Command::Stroke {
            brush: vec![(0, 0)],
            color,
            points: vec![(x, y)],
            continuous: false,
            custom: None,
            mode: dpx_core::DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        });
    }

    /// One frame with these key events at time `t` through [`PainterApp::menu_keys`].
    fn keys(app: &mut PainterApp, t: f64, events: Vec<egui::Event>) {
        let ctx = ctx();
        ctx.begin_pass(egui::RawInput {
            events,
            time: Some(t),
            ..Default::default()
        });
        app.menu_keys(&ctx);
        // The pass uploads egui's font texture; nothing renders it here.
        ctx.end_pass().textures_delta.clear();
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-menu-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    fn type_into(app: &mut PainterApp, text: &str) {
        for c in text.chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
    }

    #[test]
    fn spare_page_swap_keeps_each_page_and_its_undo() {
        let mut app = PainterApp::new();
        dot(&mut app, 1, 1, 5);
        app.ui.bg = 2;
        // `j`: the spare starts blank; the BG colour belongs to the page.
        keys(
            &mut app,
            0.0,
            vec![key(egui::Key::J, egui::Modifiers::NONE)],
        );
        assert_eq!(app.page().get(1, 1), Some(0));
        dot(&mut app, 2, 2, 7);
        app.ui.bg = 4;
        act(&mut app, "spare.swap");
        assert_eq!(app.page().get(1, 1), Some(5));
        assert_eq!(app.page().get(2, 2), Some(0));
        assert_eq!(app.ui.bg, 2);
        // Undo works on the shown page's own history.
        app.undo();
        assert_eq!(app.page().get(1, 1), Some(0));
        app.redo();
        act(&mut app, "spare.swap");
        assert_eq!(app.page().get(2, 2), Some(7));
        assert_eq!(app.ui.bg, 4);
        app.undo();
        assert_eq!(app.page().get(2, 2), Some(0), "the spare's stroke undone");
    }

    #[test]
    fn merge_in_front_and_back_with_colour_0_transparent_and_undo() {
        let mut app = PainterApp::new();
        // Shown page: colour 3 except (2,2) = 0. Spare: (1,1) = 5, (2,2) = 6, rest 0.
        app.run(Command::Clear { color: 3 });
        dot(&mut app, 2, 2, 0);
        act(&mut app, "spare.merge_front");
        assert_eq!(app.engine.document().revision(), 2, "no spare yet: nothing");
        act(&mut app, "spare.swap");
        dot(&mut app, 1, 1, 5);
        dot(&mut app, 2, 2, 6);
        act(&mut app, "spare.swap");
        act(&mut app, "spare.merge_front");
        assert_eq!(app.page().get(1, 1), Some(5));
        assert_eq!(app.page().get(2, 2), Some(6));
        assert_eq!(
            app.page().get(9, 9),
            Some(3),
            "spare colour 0 is transparent"
        );
        app.undo();
        assert_eq!(app.page().get(1, 1), Some(3));
        act(&mut app, "spare.merge_back");
        assert_eq!(app.page().get(1, 1), Some(3), "shown page on top");
        assert_eq!(
            app.page().get(2, 2),
            Some(6),
            "its colour 0 shows the spare"
        );
        app.undo();
        assert_eq!(app.page().get(2, 2), Some(0));
        // Copy to Spare (Ctrl+j), then swap: the same picture.
        let shown = app.page().clone();
        keys(
            &mut app,
            0.0,
            vec![key(egui::Key::J, egui::Modifiers::CTRL)],
        );
        act(&mut app, "spare.swap");
        assert_eq!(app.page(), &shown);
    }

    #[test]
    fn delete_this_page_shows_the_spare_and_is_undoable() {
        let mut app = PainterApp::new();
        dot(&mut app, 1, 1, 5);
        act(&mut app, "spare.swap");
        dot(&mut app, 2, 2, 7);
        act(&mut app, "spare.swap");
        act(&mut app, "spare.delete");
        assert!(app.menu_data.spare.is_none());
        assert_eq!(app.page().get(2, 2), Some(7));
        assert_eq!(app.page().get(1, 1), Some(0));
        app.undo();
        assert_eq!(app.page().get(1, 1), Some(5), "the deleted page comes back");
    }

    #[test]
    fn a_spare_of_another_format_is_dropped() {
        use dpx_core::{Resolution, Video};
        let mut app = PainterApp::new();
        act(&mut app, "spare.swap");
        dot(&mut app, 1, 1, 5);
        act(&mut app, "spare.swap");
        app.run(Command::SetFormat {
            mode: dpx_core::DisplayMode::new(Video::Pal, Resolution::Hi, false),
            colors: 32,
            width: 640,
            height: 256,
            retain: false,
            screen: None,
        });
        act(&mut app, "spare.swap");
        assert_eq!(app.page().width(), 640);
        assert_eq!(app.page().get(1, 1), Some(0), "a blank HiRes spare");
    }

    #[test]
    fn fixed_background_is_what_clr_restores() {
        let mut app = PainterApp::new();
        app.ui.bg = 2;
        dot(&mut app, 1, 1, 5);
        act(&mut app, "background.fix");
        dot(&mut app, 3, 3, 6);
        // CLR in the toolbox.
        app.select_tool(Tool::Clear, false);
        assert_eq!(app.page().get(1, 1), Some(5), "the fixed picture stays");
        assert_eq!(app.page().get(3, 3), Some(0), "new paint is erased");
        app.undo();
        assert_eq!(app.page().get(3, 3), Some(6));
        // `k` does the same.
        keys(
            &mut app,
            0.0,
            vec![key(egui::Key::K, egui::Modifiers::NONE)],
        );
        assert_eq!(app.page().get(3, 3), Some(0));
        // Free: CLR fills with BG again.
        act(&mut app, "background.free");
        keys(
            &mut app,
            0.0,
            vec![key(egui::Key::K, egui::Modifiers::NONE)],
        );
        assert_eq!(app.page().get(1, 1), Some(2));
    }

    #[test]
    fn palette_save_and_load_round_trip_through_the_file_requester() {
        let dir = temp_dir("palette");
        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        let mut colors = app.palette().colors().to_vec();
        colors[1] = Rgb8::new(12, 34, 56);
        colors[31] = Rgb8::new(250, 1, 2);
        app.run(Command::SetPalette {
            colors: colors.clone(),
        });
        let range = ColorRange {
            rate: 8192,
            flags: 1,
            low: 4,
            high: 9,
        };
        app.extras.ranges = vec![range];
        act(&mut app, "palette.save");
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().kind(),
            FileKind::SavePalette
        );
        type_into(&mut app, "Warm.pal");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "saved and closed");
        let bytes = std::fs::read(dir.join("Warm.pal")).unwrap();
        let (_, chunks) = iff::parse_form(&bytes).unwrap();
        let ids: Vec<_> = chunks.iter().map(|c| iff::id_str(c.id)).collect();
        assert_eq!(ids, ["BMHD", "CMAP", "CRNG"], "no BODY");

        // Change palette and ranges, then load the file back (undoable).
        act(&mut app, "palette.default");
        app.extras.ranges.clear();
        assert_ne!(app.palette().colors(), &colors[..]);
        act(&mut app, "palette.load");
        type_into(&mut app, "Warm.pal");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none());
        assert!(
            app.ui.note_requester.is_none(),
            "32 colours: nothing to note"
        );
        assert_eq!(app.palette().colors(), &colors[..]);
        assert_eq!(app.extras.ranges, vec![range]);
        app.undo();
        assert_eq!(app.palette(), &Palette::dpaint_default_32());

        // The CMAP of a full picture loads too; a 4-colour file changes colours 0-3 only.
        let mut pic = dpx_iff::Ilbm::from_indexed(
            IndexedImage::new(8, 8, 0),
            Palette::from_rgb4(&[0x000, 0xF00, 0x0F0, 0x00F]).unwrap(),
        );
        pic.planes = 2;
        let pic = dpx_iff::ilbm::write(&pic, dpx_iff::Compression::ByteRun1).unwrap();
        std::fs::write(dir.join("Four.iff"), pic).unwrap();
        act(&mut app, "palette.load");
        type_into(&mut app, "Four.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert_eq!(app.palette().get(1), Some(Rgb8::new(255, 0, 0)));
        assert_eq!(app.palette().get(4), Palette::dpaint_default_32().get(4));
        assert!(app.ui.note_requester.is_some(), "tells what stayed");
        // Not a palette: the requester stays open with a message.
        app.ui.note_requester = None;
        std::fs::write(dir.join("Text.txt"), b"hello").unwrap();
        act(&mut app, "palette.load");
        type_into(&mut app, "Text.txt");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_palette_goes_back_to_the_loaded_picture() {
        let dir = temp_dir("restore");
        let pal = Palette::from_rgb4(&[0x000, 0x111, 0x222, 0x333]).unwrap();
        let src = dpx_iff::Ilbm::from_indexed(IndexedImage::new(320, 256, 0), pal.clone());
        let bytes = dpx_iff::ilbm::write(&src, dpx_iff::Compression::ByteRun1).unwrap();
        std::fs::write(dir.join("Grey.iff"), bytes).unwrap();
        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        act(&mut app, "picture.load");
        type_into(&mut app, "Grey.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert_eq!(app.palette(), &pal);
        app.run(Command::SetPalette {
            colors: vec![Rgb8::new(9, 9, 9); 4],
        });
        act(&mut app, "palette.restore");
        assert_eq!(app.palette(), &pal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_removes_the_file_only_after_yes() {
        let dir = temp_dir("delete");
        std::fs::write(dir.join("Old.iff"), b"x").unwrap();
        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        act(&mut app, "picture.delete");
        let req = app.ui.file_requester.as_ref().unwrap();
        assert_eq!(req.kind(), FileKind::DeleteFile);
        type_into(&mut app, "Old.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(dir.join("Old.iff").exists(), "asks first");
        app.file_requester_event(ReqEvent::Key(ReqKey::Char('n')), 0);
        assert!(dir.join("Old.iff").exists());
        assert!(app.ui.file_requester.is_some());
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        app.file_requester_event(ReqEvent::Key(ReqKey::Char('y')), 0);
        assert!(!dir.join("Old.iff").exists(), "deleted after Yes");
        assert!(app.ui.file_requester.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cycling_rotates_the_shown_palette_not_the_document() {
        let mut app = PainterApp::new();
        dot(&mut app, 1, 1, 3);
        let doc = app.engine.document().palette().clone();
        let page = app.page().clone();
        // Colours 2-4 at 60 steps per second, 10-12 reverse at 30 per second.
        app.extras.ranges = vec![
            ColorRange {
                rate: 16384,
                flags: 1,
                low: 2,
                high: 4,
            },
            ColorRange {
                rate: 8192,
                flags: 3,
                low: 10,
                high: 12,
            },
        ];
        // Tab at t = 10 s; at t = 10 the palette is unchanged.
        keys(
            &mut app,
            10.0,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert!(app.menu_data.cycle_start.is_some());
        assert_eq!(app.palette(), &doc);
        // One step later: colour 3 shows the old colour 2, colour 2 the old colour 4.
        keys(&mut app, 10.0 + 1.5 / 60.0, vec![]);
        let shown = app.palette().clone();
        assert_eq!(shown.get(3), doc.get(2));
        assert_eq!(shown.get(2), doc.get(4));
        assert_eq!(shown.get(10), doc.get(10), "30 per second: no step yet");
        keys(&mut app, 10.0 + 2.5 / 60.0, vec![]);
        assert_eq!(app.palette().get(10), doc.get(11), "reverse moves down");
        assert_eq!(app.palette().get(2), doc.get(3), "two steps");
        // The document, its pixels and the other colours do not change.
        assert_eq!(app.engine.document().palette(), &doc);
        assert_eq!(app.page(), &page);
        assert_eq!(app.palette().get(5), doc.get(5));
        // Tab again: the document palette is shown.
        keys(
            &mut app,
            11.0,
            vec![key(egui::Key::Tab, egui::Modifiers::NONE)],
        );
        assert_eq!(app.palette(), &doc);
        // The menu item toggles as well; inactive ranges never move.
        app.extras.ranges[0].flags = 0;
        act(&mut app, "color.cycle");
        keys(&mut app, 99.0, vec![]);
        assert_eq!(app.palette().get(3), doc.get(3));
    }

    #[test]
    fn about_shows_our_own_text() {
        let mut app = PainterApp::new();
        act(&mut app, "app.about");
        assert!(app.ui.note_requester.is_some());
        app.note_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.note_requester.is_none());
        keys(&mut app, 0.0, vec![egui::Event::Text("?".into())]);
        assert!(app.ui.note_requester.is_some());
    }

    #[test]
    fn palette_files_decode_what_they_encode() {
        let pal = Palette::dpaint_default_32();
        let bytes = encode_palette(&pal, &[], Target::default());
        let (colors, ranges) = decode_palette(&bytes).unwrap();
        assert_eq!(colors, pal.colors());
        assert!(ranges.is_empty());
        // 4-bit CMAP values (low nibble zero) are scaled up: 0xF0 -> 0xFF.
        let mut form = iff::FormWriter::new(*b"ILBM");
        form.chunk(*b"CMAP", &[0xF0, 0x80, 0x00]);
        let (colors, _) = decode_palette(&form.finish()).unwrap();
        assert_eq!(colors, [Rgb8::new(0xFF, 0x88, 0)]);
        assert!(decode_palette(b"FORM\0\0\0\x04ILBM").is_err(), "no CMAP");
    }

    fn press_release(app: &mut PainterApp, r: dpx_core::Rect) {
        app.range_event(ReqEvent::Press(r.x + r.w / 2, r.y + r.h / 2));
        app.range_event(ReqEvent::Release);
    }

    #[test]
    fn new_picture_has_inactive_default_ranges() {
        let app = PainterApp::new();
        let r = &app.extras.ranges;
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].low, r[0].high), (20, 27));
        assert_eq!((r[1].low, r[1].high), (8, 15));
        assert!(r.iter().all(|r| !r.active() && r.rate == 4096));
        // Tab cycling with only inactive ranges leaves the palette alone.
        let doc = app.engine.document().palette();
        assert_eq!(&cycled_palette(doc, r, 3.0), doc);
        // Palettes below 32 colours get none.
        let small = Palette::new(vec![Rgb8::BLACK; 16]).unwrap();
        assert!(files::default_ranges(&small).is_empty());
    }

    #[test]
    fn ranges_requester_edits_cycles_and_saves_as_crng() {
        let mut app = PainterApp::new();
        act(&mut app, "color.ranges");
        let req = app.ui.range_requester.as_ref().expect("requester open");
        // Opens on the first defined default range.
        assert_eq!(req.selected(), 0);
        // Switch range 1 on, make it reverse, set range 3 to colours 2..=6.
        press_release(&mut app, RangeRequester::active_rect());
        press_release(&mut app, RangeRequester::direction_rect());
        press_release(&mut app, RangeRequester::select_rect(2));
        let cell = |app: &PainterApp, i| app.ui.range_requester.as_ref().unwrap().cell_rect(i);
        let (a, b) = (cell(&app, 2), cell(&app, 6));
        press_release(&mut app, a);
        press_release(&mut app, b);
        // Cancel first: nothing changes.
        let before = app.extras.ranges.clone();
        app.range_event(ReqEvent::Key(ReqKey::Escape));
        assert!(app.ui.range_requester.is_none());
        assert_eq!(app.extras.ranges, before);

        // Again, this time Ok.
        act(&mut app, "color.ranges");
        press_release(&mut app, RangeRequester::active_rect());
        press_release(&mut app, RangeRequester::direction_rect());
        press_release(&mut app, RangeRequester::select_rect(2));
        let (a, b) = (cell(&app, 2), cell(&app, 6));
        press_release(&mut app, a);
        press_release(&mut app, b);
        app.range_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.range_requester.is_none());
        let r = app.extras.ranges.clone();
        assert_eq!(r.len(), 3);
        assert!(r[0].active() && r[0].reverse() && (r[0].low, r[0].high) == (20, 27));
        assert_eq!(r[1], before[1]);
        assert!(r[2].active() && !r[2].reverse() && (r[2].low, r[2].high) == (2, 6));

        // Cycling reads the new ranges: range 1 now moves.
        let doc = app.engine.document().palette().clone();
        assert_ne!(cycled_palette(&doc, &r, 1.0), doc);

        // Save as ILBM and load again: the ranges come back as CRNG chunks.
        let dir = std::env::temp_dir().join(format!("dpx-ranges-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ranges.iff");
        let (bytes, _) = files::encode_document(app.engine.document(), &app.extras, &path).unwrap();
        files::write_atomic(&path, &bytes).unwrap();
        let files::Picture::Ilbm(back) = files::load(&path).unwrap().picture else {
            panic!("expected ILBM");
        };
        assert_eq!(back.ranges, r);
        let (_, extras, _) = files::open_picture(files::Picture::Ilbm(back), Target::default());
        assert_eq!(extras.ranges, r);
        std::fs::remove_dir_all(&dir).ok();
    }
}
