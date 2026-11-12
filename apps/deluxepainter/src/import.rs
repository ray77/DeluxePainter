//! Picture ▸ Load of PNG and JPEG pictures (ADR-42): the Import Picture requester.
//!
//! When Picture ▸ Load.. picks a PNG (indexed or true colour) or a JPEG, the picture is not
//! opened on a screen of its own (ADR-9) but fitted into the current page: the display mode,
//! chipset, screen and page stay. The requester asks for the position, the palette (new or
//! the current one), the number of colours and the dither, and shows the result as a
//! true-colour thumbnail. IFF ILBM / ANIM and projects load as before, without questions;
//! `--open`, the CLI and MCP keep ADR-9.
//!
//! The fitted picture is made once when the requester opens; every option change only
//! reduces it again (`dpx_core::import::reduce_fitted`), and the thumbnail is made from the
//! page that Ok then takes over unchanged.

use std::path::PathBuf;

use dpx_core::import::{
    self, COLOR_COUNTS, Fitted, ImportOptions, ImportTarget, Imported, PaletteChoice, Position,
};
use dpx_core::quantize::{Dither, PaletteDepth, Reduction};
use dpx_core::{Chipset, Document, IndexedImage, RgbaImage};
use dpx_ui::{ImportInfo, ImportRequester, ImportResponse, ReqEvent, UiState};
use eframe::egui;

use crate::files::{self, Picture};
use crate::{LOADED, PainterApp, View, note_requester};

/// The requester's last choices, kept for the session and in the settings file (ADR-33).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportSettings {
    pub position: Position,
    pub palette: PaletteChoice,
    /// `None` until an import was made: the current document's palette size.
    pub colors: Option<u16>,
    pub dither: Dither,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            position: Position::Centre,
            palette: PaletteChoice::New,
            colors: None,
            dither: Dither::FloydSteinberg,
        }
    }
}

/// Settings-file and `--import` names.
pub fn position_name(p: Position) -> &'static str {
    match p {
        Position::Centre => "centre",
        Position::TopLeft => "top_left",
    }
}

pub fn palette_name(p: PaletteChoice) -> &'static str {
    match p {
        PaletteChoice::New => "new",
        PaletteChoice::Current => "current",
    }
}

pub fn dither_name(d: Dither) -> &'static str {
    match d {
        Dither::None => "none",
        Dither::FloydSteinberg => "floyd_steinberg",
        Dither::Ordered => "ordered",
    }
}

pub fn parse_position(s: &str) -> Option<Position> {
    match s {
        "centre" | "center" => Some(Position::Centre),
        "top_left" => Some(Position::TopLeft),
        _ => None,
    }
}

pub fn parse_palette(s: &str) -> Option<PaletteChoice> {
    match s {
        "new" => Some(PaletteChoice::New),
        "current" => Some(PaletteChoice::Current),
        _ => None,
    }
}

pub fn parse_dither(s: &str) -> Option<Dither> {
    match s {
        "none" => Some(Dither::None),
        "fs" | "floyd_steinberg" => Some(Dither::FloydSteinberg),
        "ordered" => Some(Dither::Ordered),
        _ => None,
    }
}

/// From this many pixels of fitted picture on (bigger than any standard screen, e.g. pages
/// made larger with Page Size), option changes are reduced on a background thread, so the
/// requester never blocks; Ok waits for the result.
pub(crate) const BACKGROUND_PIXELS: u64 = 2_000_000;

/// The picture behind the open requester.
pub(crate) struct Pending {
    path: PathBuf,
    /// Notes from reading the file (shown after Ok).
    notes: Vec<String>,
    /// The picture fitted into the page (made once) and the work option changes reuse;
    /// `None` while a background job has it.
    fitted: Option<Fitted>,
    /// The page made with `made` (the thumbnail shows it; Ok takes it when `made` is the
    /// requester's options).
    result: Imported,
    made: ImportOptions,
    /// A reduction running in the background.
    job: Option<Job>,
}

/// A background reduction (big pages): it gives the fitted picture back with the page.
pub(crate) struct Job {
    options: ImportOptions,
    done: std::sync::mpsc::Receiver<(Fitted, Imported)>,
}

impl Pending {
    /// A reduction is running in the background.
    pub(crate) fn busy(&self) -> bool {
        self.job.is_some()
    }
}

#[derive(Default)]
pub(crate) struct ImportState {
    pub(crate) settings: ImportSettings,
    pub(crate) pending: Option<Pending>,
}

/// Whether Picture ▸ Load asks with the Import requester: PNG pictures (indexed or true
/// colour) and JPEGs. HAM ILBMs are true colour too, but IFF loads without questions.
pub(crate) fn asks(picture: &Picture) -> bool {
    matches!(
        picture,
        Picture::Png { .. }
            | Picture::TrueColor {
                format: "PNG" | "JPEG",
                ..
            }
    )
}

/// The picture as RGBA (indexed PNGs in their palette's colours) and its number of colours
/// (`None` above 256).
fn source(picture: Picture) -> Option<(RgbaImage, Option<usize>)> {
    let image = match picture {
        Picture::Png { image, palette } => import::rgba_of(&image, &palette),
        Picture::TrueColor { image, .. } => image,
        _ => return None,
    };
    let colors = import::count_colors(&image, 256);
    Some((image, colors))
}

/// Notes after Ok: what the file reading noted, plus what the requester did not ask about.
/// Scaling, the colour count and the dither were the user's choices and get no note.
fn notes_after(mut notes: Vec<String>, r: &Imported, palette: PaletteChoice) -> Vec<String> {
    if r.reduction == Reduction::Rounded {
        notes.push("colours rounded to 12-bit OCS values".into());
    }
    if r.transparent_color.is_some() {
        notes.push(match palette {
            PaletteChoice::New => {
                "transparent pixels became colour 0 (pink), the transparent colour".into()
            }
            PaletteChoice::Current => {
                "transparent pixels became colour 0, the transparent colour".into()
            }
        });
    }
    notes
}

/// `doc` as the import's target, with the BG colour `bg`.
fn import_target(doc: &Document, bg: u8) -> ImportTarget<'_> {
    let t = doc.target();
    ImportTarget {
        page: (doc.page().width(), doc.page().height()),
        pixel_shape: t.mode.pixel_shape(),
        depth: match t.chipset {
            Chipset::Ocs => PaletteDepth::Rgb4,
            Chipset::Aga => PaletteDepth::Rgb8,
        },
        palette: doc.palette(),
        bg,
        transparent: files::TRANSPARENT_PINK,
    }
}

/// The requester's default colour count: the document's palette size in whole bitplanes
/// (12 colours take 4 bitplanes: 16), so the default never gives fewer colours.
pub(crate) fn default_colors(palette_len: usize) -> u16 {
    (palette_len.clamp(2, 256) as u16).next_power_of_two()
}

/// Runs `draw` (marks drawn over the composed screen, such as the toolbox's toggles) without
/// touching the Import requester: it covers the toolbox column, and the marks belong under it.
pub(crate) fn under_import_requester(
    screen: &mut IndexedImage,
    ui: &UiState,
    (ux, uy): (i32, i32),
    draw: impl FnOnce(&mut IndexedImage),
) {
    let kept = ui.import_requester.as_ref().map(|req| {
        let (ox, oy) = req.origin();
        let (w, h) = req.size();
        let rect = dpx_core::Rect::new(ox * ux, oy * uy, w * ux, h * uy);
        (rect, screen.crop(rect))
    });
    draw(screen);
    if let Some((rect, img)) = kept {
        screen.blit(&img, rect.x, rect.y);
    }
}

impl PainterApp {
    /// Opens the Import requester for a loaded PNG or JPEG (the file requester closes).
    pub(crate) fn open_import(&mut self, path: PathBuf, picture: Picture, notes: Vec<String>) {
        if let Some((image, colors)) = source(picture) {
            self.open_import_image(path, image, colors, notes);
        }
    }

    /// Opens the Import requester for `image`, fitted into the current page.
    pub(crate) fn open_import_image(
        &mut self,
        path: PathBuf,
        image: RgbaImage,
        colors: Option<usize>,
        notes: Vec<String>,
    ) {
        // No stroke, hover brush or menu strip under the requester.
        self.finish_stroke();
        self.ui.menu.close();
        let doc = self.engine.document();
        let target = doc.target();
        let page = (doc.page().width(), doc.page().height());
        let shape = target.mode.pixel_shape();
        let mut fitted = Fitted::new(&image, page, shape);
        let max = target.mode.max_colors(target.chipset).unwrap_or(16);
        let current = doc.palette().len().min(256) as u16;
        let s = self.import.settings;
        let options = ImportOptions {
            position: s.position,
            palette: s.palette,
            colors: s
                .colors
                .unwrap_or_else(|| default_colors(doc.palette().len())),
            dither: s.dither,
        };
        let info = ImportInfo {
            name: path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            size: (image.width(), image.height()),
            colors,
            fitted: (fitted.image().width(), fitted.image().height()),
            page,
        };
        let screen = View::of_target(target).ui_size();
        let req = ImportRequester::new(info, options, max, current, screen, shape);
        let made = req.options();
        let result = fitted.reduce(&made, &import_target(doc, self.ui.bg));
        self.import.pending = Some(Pending {
            path,
            notes,
            fitted: Some(fitted),
            result,
            made,
            job: None,
        });
        self.ui.import_requester = Some(req);
        self.ui.file_requester = None;
        self.refresh_import_thumbnail();
        self.dirty = true;
    }

    /// The thumbnail from the page Ok would give.
    fn refresh_import_thumbnail(&mut self) {
        let (Some(req), Some(p)) = (&mut self.ui.import_requester, &self.import.pending) else {
            return;
        };
        let (w, h) = req.thumb_size();
        req.set_thumbnail(import::thumbnail(&p.result.page, &p.result.palette, w, h));
    }

    /// Pointer and keys while the Import requester is open (it is modal).
    pub(crate) fn import_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        // A background result that has arrived.
        self.import_poll(false);
        if self.import.pending.as_ref().is_some_and(Pending::busy) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let Some(req) = &self.ui.import_requester else {
            return;
        };
        let at = req.origin();
        // A held Return (the one that picked the file) does not answer Ok.
        let (events, _) = self.modal_events_with(ctx, origin, ppp, scale, view, at, false);
        for event in events {
            if self.ui.import_requester.is_none() {
                break;
            }
            self.import_event(event);
        }
    }

    /// Feeds one event to the Import requester: a changed option reduces the fitted picture
    /// again, Cancel keeps the old picture, Ok replaces it.
    pub(crate) fn import_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.import_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        match response {
            ImportResponse::Continue => {}
            ImportResponse::Changed => self.import_recompute(),
            ImportResponse::Cancel => {
                self.ui.import_requester = None;
                self.import.pending = None;
            }
            ImportResponse::Ok => self.import_ok(),
        }
    }

    /// Makes the page for the requester's options: at once, or for a big picture on a
    /// background thread (while one runs, the next starts when it ends, with the options
    /// wanted then).
    fn import_recompute(&mut self) {
        let (Some(req), Some(p)) = (&self.ui.import_requester, &mut self.import.pending) else {
            return;
        };
        let wanted = req.options();
        if p.job.is_some() || p.made == wanted {
            return;
        }
        let Some(mut fitted) = p.fitted.take() else {
            return;
        };
        let doc = self.engine.document();
        let pixels = u64::from(fitted.image().width()) * u64::from(fitted.image().height());
        if pixels < BACKGROUND_PIXELS {
            p.result = fitted.reduce(&wanted, &import_target(doc, self.ui.bg));
            p.made = wanted;
            p.fitted = Some(fitted);
            self.refresh_import_thumbnail();
            return;
        }
        let t = import_target(doc, self.ui.bg);
        let (page, pixel_shape, depth, bg, transparent) =
            (t.page, t.pixel_shape, t.depth, t.bg, t.transparent);
        let palette = doc.palette().clone();
        let (tx, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let t = ImportTarget {
                page,
                pixel_shape,
                depth,
                palette: &palette,
                bg,
                transparent,
            };
            let result = fitted.reduce(&wanted, &t);
            // The requester may be gone (Cancel): then nobody waits.
            let _ = tx.send((fitted, result));
        });
        p.job = Some(Job {
            options: wanted,
            done,
        });
    }

    /// Takes a finished background result (with `wait`, waits for it) and starts the next
    /// one if the options changed meanwhile.
    pub(crate) fn import_poll(&mut self, wait: bool) {
        let Some(p) = &mut self.import.pending else {
            return;
        };
        let Some(job) = &p.job else {
            return;
        };
        let got = if wait {
            job.done
                .recv()
                .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)
        } else {
            job.done.try_recv()
        };
        match got {
            Ok((fitted, result)) => {
                p.made = job.options;
                p.fitted = Some(fitted);
                p.result = result;
                p.job = None;
                self.dirty = true;
                self.refresh_import_thumbnail();
                self.import_recompute();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                // The reduction failed: nothing can be shown or taken over.
                eprintln!("import: the colour reduction failed");
                self.ui.import_requester = None;
                self.import.pending = None;
            }
        }
    }

    /// Waits until the page for the requester's options is made.
    fn import_settle(&mut self) {
        while self.import.pending.as_ref().is_some_and(Pending::busy) {
            self.import_poll(true);
        }
        self.import_recompute();
        while self.import.pending.as_ref().is_some_and(Pending::busy) {
            self.import_poll(true);
        }
    }

    /// Ok: the page shown in the thumbnail becomes the picture, like Load (not undoable,
    /// ADR-8). Mode, chipset, screen and page size stay; the choices are remembered.
    fn import_ok(&mut self) {
        // The page for exactly these options (a background job may still run).
        self.import_settle();
        let (Some(req), Some(p)) = (self.ui.import_requester.take(), self.import.pending.take())
        else {
            return;
        };
        let opts = req.options();
        debug_assert_eq!(
            p.made, opts,
            "Ok takes the page made with the requester's options"
        );
        self.import.settings = ImportSettings {
            position: opts.position,
            palette: opts.palette,
            colors: Some(opts.colors),
            dither: opts.dither,
        };
        // The current palette keeps its colour ranges; a new palette has none (ADR-9).
        let ranges = match opts.palette {
            PaletteChoice::Current => self.extras.ranges.clone(),
            PaletteChoice::New => Vec::new(),
        };
        let extras = files::PictureExtras {
            ranges,
            grab: None,
            transparent_color: p.result.transparent_color.map(u16::from),
        };
        let notes = notes_after(p.notes, &p.result, opts.palette);
        for n in &notes {
            eprintln!("note: {}: {n}", p.path.display());
        }
        let target = self.engine.document().target();
        let doc = Document::from_picture(p.result.page, p.result.palette, target);
        self.set_picture(doc, extras, p.path);
        self.ui.note_requester = note_requester(LOADED, &notes);
    }

    /// Paints the thumbnail into the RGBA screen (`frame_w` pixels wide, one pixel per screen
    /// pixel) unless the quit question covers it.
    pub(crate) fn import_overlay(&self, rgba: &mut [u8], frame_w: usize) {
        if self.ui.choice_requester.is_some() {
            return;
        }
        if let Some(req) = &self.ui.import_requester {
            let v = self.view();
            req.overlay(rgba, frame_w, v.ux as usize, v.uy as usize);
        }
    }
}

/// `--import 'position=centre|top_left;palette=new|current;colors=N;dither=none|fs|ordered'`:
/// any of the keys, in any order. `max` is the screen's most colours.
fn parse_spec(spec: &str, max: u16) -> Result<ImportSettings, String> {
    let mut s = ImportSettings::default();
    for part in spec.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("'{part}' is not key=value"))?;
        let bad = || format!("bad {key} '{value}'");
        match key.trim() {
            "position" => s.position = parse_position(value).ok_or_else(bad)?,
            "palette" => s.palette = parse_palette(value).ok_or_else(bad)?,
            "dither" => s.dither = parse_dither(value).ok_or_else(bad)?,
            "colors" | "colours" => {
                let n: u16 = value.parse().map_err(|_| bad())?;
                if !COLOR_COUNTS.contains(&n) || n > max {
                    return Err(format!(
                        "colors must be 2, 4, 8 … up to {max} on this screen, not {value}"
                    ));
                }
                s.colors = Some(n);
            }
            _ => return Err(format!("unknown key '{key}'")),
        }
    }
    Ok(s)
}

/// Snapshot options (ADR-42): `--import-requester FILE` shows the Import requester for a PNG
/// or JPEG over the current screen, with its thumbnail; `--import SPEC` sets its options;
/// `--import-apply` shows the page after Ok instead. Returns the document to show. Errors
/// carry the exit code: 1 for a file that cannot be read, 2 for bad options.
pub(crate) fn snapshot(
    args: &[String],
    doc: &Document,
    ui: &mut UiState,
) -> Result<Document, (i32, String)> {
    let file = crate::arg_value(args, "--import-requester");
    let spec = crate::arg_value(args, "--import");
    let apply = args.iter().any(|a| a == "--import-apply");
    let Some(file) = file else {
        if spec.is_some() || apply {
            return Err((
                2,
                "--import and --import-apply need --import-requester".into(),
            ));
        }
        return Ok(doc.clone());
    };
    let loaded = files::load(std::path::Path::new(&file)).map_err(|e| (1, e))?;
    if !asks(&loaded.picture) {
        return Err((
            2,
            format!("--import-requester {file}: not a PNG or JPEG picture"),
        ));
    }
    let target = doc.target();
    let max = target.mode.max_colors(target.chipset).unwrap_or(16);
    let mut app = PainterApp::new();
    app.engine = dpx_core::Engine::new(doc.clone());
    app.ui = ui.clone();
    if let Some(spec) = spec {
        app.import.settings = parse_spec(&spec, max).map_err(|e| (2, format!("--import: {e}")))?;
    }
    app.open_import(PathBuf::from(&file), loaded.picture, loaded.notes);
    if apply {
        app.import_event(ReqEvent::Key(dpx_ui::ReqKey::Enter));
        // The notes are not part of the page snapshot.
        app.ui.note_requester = None;
    }
    *ui = app.ui.clone();
    Ok(app.engine.document().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{key_event, ui_pass, ui_point};
    use dpx_core::{Command, DisplayMode, IndexedImage, Palette, Rgb8, Rgba8, Target};
    use dpx_ui::{ImportControl, ReqKey};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    /// A photo-like RGB picture with thousands of colours.
    fn photo(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h, Rgba8::default());
        for y in 0..h {
            for x in 0..w {
                img.set(
                    x,
                    y,
                    Rgba8::opaque(
                        (x * 255 / (w - 1)) as u8,
                        (y * 255 / (h - 1)) as u8,
                        ((x * 7 + y * 3) % 256) as u8,
                    ),
                );
            }
        }
        img
    }

    fn write_png_rgb(path: &std::path::Path, img: &RgbaImage) {
        let data: Vec<u8> = img.pixels().iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, img.width(), img.height());
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&data).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// A 4×3 indexed PNG with three colours.
    fn write_pixel_art(path: &std::path::Path) -> (IndexedImage, Palette) {
        let palette = Palette::new(vec![
            Rgb8::new(255, 0, 0),
            Rgb8::new(0, 128, 0),
            Rgb8::new(0, 0, 255),
            Rgb8::new(255, 255, 0),
        ])
        .unwrap();
        let px = vec![0, 1, 2, 1, 1, 2, 0, 2, 2, 0, 1, 0];
        let image = IndexedImage::from_pixels(4, 3, px).unwrap();
        std::fs::write(path, dpx_formats::write_png(&image, &palette).unwrap()).unwrap();
        (image, palette)
    }

    /// Picture ▸ Load.. with `name` typed into the file requester and Return.
    fn load_through_the_requester(
        app: &mut PainterApp,
        ctx: &egui::Context,
        dir: &std::path::Path,
        name: &str,
    ) {
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.run_action(dpx_ui::Action::Command("picture.load"), ctx);
        for c in name.chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
    }

    /// A click on a requester control through `ui()` with real pointer events.
    fn click(app: &mut PainterApp, ctx: &egui::Context, t: f64, c: ImportControl) {
        let req = app.ui.import_requester.as_ref().expect("requester open");
        let (ox, oy) = req.origin();
        let r = req.controls().into_iter().find(|&(k, _)| k == c).unwrap().1;
        let pos = ui_point(app, (ox + r.x + r.w / 2, oy + r.y + r.h / 2));
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        ui_pass(
            app,
            ctx,
            t,
            vec![egui::Event::PointerMoved(pos), button(true)],
        );
        ui_pass(app, ctx, t + 0.01, vec![button(false)]);
    }

    /// A `ui()` pass that redraws; returns the uploaded screen image.
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
        let (w, h) = (app.view().screen_w as usize, app.view().screen_h as usize);
        let screen = out
            .textures_delta
            .set
            .values()
            .flat_map(|deltas| deltas.iter())
            .find_map(|delta| match &delta.image {
                egui::ImageData::Color(img) if img.size == [w, h] => Some((**img).clone()),
                _ => None,
            });
        out.textures_delta.clear();
        screen.expect("the screen was uploaded")
    }

    fn shown(app: &PainterApp, x: u32, y: u32) -> Rgb8 {
        let doc = app.engine.document();
        doc.palette()
            .get(doc.page().get(x as i32, y as i32).unwrap())
            .unwrap()
    }

    #[test]
    fn a_png_opens_the_import_requester_an_ilbm_does_not_and_cancel_keeps_the_picture() {
        let ctx = egui::Context::default();
        let dir = temp_dir("open");
        write_png_rgb(&dir.join("Photo.png"), &photo(200, 100));
        let src = dpx_iff::Ilbm::from_indexed(
            IndexedImage::new(320, 256, 3),
            Palette::dpaint_default_32(),
        );
        std::fs::write(
            dir.join("Pic.iff"),
            dpx_iff::ilbm::write(&src, dpx_iff::Compression::ByteRun1).unwrap(),
        )
        .unwrap();
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 5 });
        let before = app.engine.document().clone();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        assert!(app.ui.file_requester.is_none(), "the file requester closed");
        let req = app
            .ui
            .import_requester
            .as_ref()
            .expect("Import requester open");
        assert!(app.modal_open());
        assert_eq!(req.info().name, "Photo.png");
        assert_eq!(req.info().size, (200, 100));
        assert_eq!(req.info().colors, None, "true colour");
        assert_eq!(req.info().fitted, (320, 160));
        assert_eq!(req.info().page, (320, 256));
        assert_eq!(req.options().colors, 32, "the document's palette size");
        assert_eq!(app.engine.document(), &before, "nothing changed yet");
        // Esc cancels through the real ui loop; the old picture stays.
        ui_pass(
            &mut app,
            &ctx,
            0.2,
            vec![key_event(egui::Key::Escape, true)],
        );
        assert!(app.ui.import_requester.is_none());
        assert!(app.import.pending.is_none());
        assert!(app.ui.file_requester.is_none());
        assert_eq!(app.engine.document(), &before);
        assert_eq!(app.engine.document().revision(), before.revision());
        // Cancel with the button does the same.
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        click(&mut app, &ctx, 0.3, ImportControl::Cancel);
        assert!(app.ui.import_requester.is_none());
        assert_eq!(app.engine.document(), &before);
        // An ILBM loads without questions.
        load_through_the_requester(&mut app, &ctx, &dir, "Pic.iff");
        assert!(app.ui.import_requester.is_none());
        assert_eq!(app.engine.document().page().get(0, 0), Some(3));
        // Anim ▸ Load.. does not ask either (a PNG opens as before, on its own screen).
        app.run_action(dpx_ui::Action::Command("anim.load"), &ctx);
        for c in "Photo.png".chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.import_requester.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ok_with_each_option_keeps_mode_screen_and_page() {
        use dpx_core::{Resolution, Video};
        let ctx = egui::Context::default();
        let dir = temp_dir("options");
        let pic = photo(300, 100);
        write_png_rgb(&dir.join("Wide.png"), &pic);
        // A HiRes PAL OCS screen with a page larger than the screen.
        let hi = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        let palette = Palette::new(
            (0..16u8)
                .map(|i| Rgb8::new(i * 16, 255 - i * 16, i * 8))
                .collect(),
        )
        .unwrap();
        let start = Document::from_picture(
            IndexedImage::new(640, 300, 0),
            palette.clone(),
            Target::new(Chipset::Ocs, hi),
        );
        let fresh = |app: &mut PainterApp| {
            app.set_picture(
                start.clone(),
                files::PictureExtras::default(),
                dir.join("start.iff"),
            );
            app.ui.bg = 4;
        };
        let mut app = PainterApp::new();
        fresh(&mut app);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let bg = palette.get(4).unwrap();

        // New palette, 8 colours, Floyd-Steinberg, centred.
        load_through_the_requester(&mut app, &ctx, &dir, "Wide.png");
        let req = app.ui.import_requester.as_ref().unwrap();
        assert_eq!(req.options().colors, 16, "OCS HiRes: 16 colours at most");
        // 300×100 square pixels on 1×2 HiRes pixels: 640 wide, 640·100/300/2 ≈ 107 high.
        assert_eq!(req.info().fitted, (640, 107));
        click(&mut app, &ctx, 0.1, ImportControl::Fewer);
        assert_eq!(
            app.ui.import_requester.as_ref().unwrap().options().colors,
            8
        );
        ui_pass(&mut app, &ctx, 0.2, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.ui.import_requester.is_none(), "Ok closes");
        let doc = app.engine.document();
        assert_eq!(
            doc.target(),
            start.target(),
            "mode, chipset and screen kept"
        );
        assert_eq!(
            (doc.page().width(), doc.page().height()),
            (640, 300),
            "page kept"
        );
        assert_eq!(doc.palette().len(), 8);
        for &c in doc.palette().colors() {
            assert_eq!(Rgb8::from_rgb4(c.to_rgb4()), c, "12-bit OCS entries");
        }
        let top = (300 - 107) / 2;
        for y in [0, top - 1, top + 107, 299] {
            for x in [0, 320, 639] {
                assert_eq!(
                    shown(&app, x, y),
                    Rgb8::from_rgb4(bg.to_rgb4()),
                    "margin {x},{y}"
                );
            }
        }
        assert!(!app.engine.can_undo(), "Load is not undoable (ADR-8)");

        // Top left, Current palette, ordered dither: the palette stays exactly.
        fresh(&mut app);
        load_through_the_requester(&mut app, &ctx, &dir, "Wide.png");
        click(
            &mut app,
            &ctx,
            1.0,
            ImportControl::Position(Position::TopLeft),
        );
        click(
            &mut app,
            &ctx,
            1.1,
            ImportControl::Palette(PaletteChoice::Current),
        );
        click(&mut app, &ctx, 1.2, ImportControl::Dither(Dither::Ordered));
        click(&mut app, &ctx, 1.3, ImportControl::Ok);
        let doc = app.engine.document();
        assert_eq!(doc.palette(), &palette, "the current palette, identical");
        assert_eq!(doc.target(), start.target());
        for y in [107, 200, 299] {
            assert_eq!(doc.page().get(10, y), Some(4), "BG index below the picture");
        }
        assert_ne!(doc.page().get(10, 50), Some(4));

        // No dither, centred again, New with 4 colours: exactly 4 entries.
        fresh(&mut app);
        load_through_the_requester(&mut app, &ctx, &dir, "Wide.png");
        let r = app.ui.import_requester.as_ref().unwrap();
        assert_eq!(r.options().position, Position::TopLeft, "remembered");
        assert_eq!(r.options().palette, PaletteChoice::Current, "remembered");
        click(
            &mut app,
            &ctx,
            2.0,
            ImportControl::Position(Position::Centre),
        );
        click(
            &mut app,
            &ctx,
            2.1,
            ImportControl::Palette(PaletteChoice::New),
        );
        click(&mut app, &ctx, 2.2, ImportControl::Fewer);
        click(&mut app, &ctx, 2.3, ImportControl::Dither(Dither::None));
        click(&mut app, &ctx, 2.4, ImportControl::Ok);
        let doc = app.engine.document();
        assert_eq!(doc.palette().len(), 4);
        assert_eq!(shown(&app, 5, 5), Rgb8::from_rgb4(bg.to_rgb4()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pixel_art_is_enlarged_crisp_and_exact() {
        let ctx = egui::Context::default();
        let dir = temp_dir("art");
        let (image, palette) = write_pixel_art(&dir.join("Art.png"));
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Art.png");
        let req = app.ui.import_requester.as_ref().unwrap();
        assert_eq!(req.info().colors, Some(3));
        // 4×3 → 320×240 on the 320×256 page: 80×80 blocks.
        assert_eq!(req.info().fitted, (320, 240));
        ui_pass(&mut app, &ctx, 0.1, vec![key_event(egui::Key::Enter, true)]);
        assert!(
            app.ui.note_requester.is_none(),
            "no notes for the user's own choices"
        );
        for y in 0..240 {
            for x in (0..320).step_by(7) {
                let want = palette.get(image.get(x / 80, y / 80).unwrap()).unwrap();
                assert_eq!(shown(&app, x as u32, y as u32 + 8), want, "{x},{y}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_uploaded_screen_shows_the_result_thumbnail_in_true_colour() {
        let ctx = egui::Context::default();
        let dir = temp_dir("thumb");
        write_png_rgb(&dir.join("Photo.png"), &photo(400, 300));
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        // 256 colours on AGA: a palette the screen's 32-colour palette cannot show.
        for i in 0..3 {
            click(
                &mut app,
                &ctx,
                0.1 + f64::from(i) * 0.1,
                ImportControl::More,
            );
        }
        assert_eq!(
            app.ui.import_requester.as_ref().unwrap().options().colors,
            256
        );
        let image = screen_pass(&mut app, &ctx, 1.0);
        let req = app.ui.import_requester.clone().unwrap();
        let rect = req.thumb_rect();
        let (tw, th) = req.thumb_size();
        let screen_palette = app.palette().clone();
        // Around the box: the indexed screen (a face gap, then the dark frame).
        let pens = dpx_ui::Pens::from_palette(&screen_palette);
        let at = |d: usize| {
            let p = image.pixels[(rect.y as usize - d) * image.size[0] + rect.x as usize - d];
            Rgb8::new(p.r(), p.g(), p.b())
        };
        assert_eq!(Some(at(1)), screen_palette.get(pens.face));
        assert_eq!(Some(at(2)), screen_palette.get(pens.dark));
        // Ok, then the final page made small the same way.
        ui_pass(&mut app, &ctx, 1.1, vec![key_event(egui::Key::Enter, true)]);
        let doc = app.engine.document();
        assert_eq!(doc.palette().len(), 256);
        let want = import::thumbnail(doc.page(), doc.palette(), tw, th);
        let mut true_colour = 0;
        for y in 0..th {
            for x in 0..tw {
                let got = image.pixels
                    [(rect.y as usize + y as usize) * image.size[0] + rect.x as usize + x as usize];
                let w = want.get(x, y).unwrap();
                assert_eq!(
                    (got.r(), got.g(), got.b()),
                    (w.r, w.g, w.b),
                    "thumbnail pixel {x},{y}"
                );
                if !screen_palette.colors().contains(&Rgb8::new(w.r, w.g, w.b)) {
                    true_colour += 1;
                }
            }
        }
        // Most thumbnail colours are not in the screen palette at all.
        assert!(true_colour > (tw * th) as usize / 2, "{true_colour}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_do_not_reach_the_paint_screen_and_cycling_goes_on() {
        let ctx = egui::Context::default();
        let dir = temp_dir("keys");
        write_png_rgb(&dir.join("Photo.png"), &photo(100, 80));
        let mut app = PainterApp::new();
        // A range that cycles, switched on.
        app.extras.ranges = vec![dpx_iff::ColorRange {
            rate: 16384,
            flags: 1,
            low: 8,
            high: 15,
        }];
        app.run(Command::Clear { color: 2 });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        app.toggle_cycling(0.0);
        ui_pass(&mut app, &ctx, 0.05, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        assert!(app.ui.import_requester.is_some());
        let revision = app.engine.document().revision();
        // `u` (undo), `p` (Color Mixer), F10 (hide the toolbox), `4` (play), digits.
        let mut keys = vec![
            key_event(egui::Key::U, true),
            egui::Event::Text("u".into()),
            key_event(egui::Key::P, true),
            egui::Event::Text("p".into()),
            key_event(egui::Key::F10, true),
            key_event(egui::Key::Num4, true),
            egui::Event::Text("4".into()),
        ];
        keys.push(key_event(egui::Key::B, true));
        ui_pass(&mut app, &ctx, 0.1, keys);
        assert!(app.ui.import_requester.is_some(), "still open");
        assert!(app.ui.palette_requester.is_none(), "no Color Mixer");
        assert!(app.ui.toolbox_visible, "F10 did not reach the screen");
        assert!(app.anim.playing.is_none());
        assert_eq!(app.engine.document().revision(), revision, "no undo");
        assert_eq!(app.engine.document().page().get(0, 0), Some(2));
        // The cycling clock keeps running under the requester.
        let mut shown = std::collections::HashSet::new();
        let mut t = 0.2;
        while t < 1.5 {
            ui_pass(&mut app, &ctx, t, vec![]);
            shown.insert(app.palette().colors()[8..16].to_vec());
            t += 1.0 / 30.0;
        }
        assert!(
            shown.len() > 3,
            "cycling continued: {} palettes",
            shown.len()
        );
        assert!(app.ui.import_requester.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hover_brush_is_not_drawn_under_the_requester() {
        let ctx = egui::Context::default();
        let dir = temp_dir("hover");
        write_png_rgb(&dir.join("Photo.png"), &photo(100, 80));
        let mut app = PainterApp::new();
        app.custom_brush = Some(dpx_core::BrushImage {
            width: 3,
            height: 3,
            pixels: vec![7; 9],
            transparent: None,
            handle: (1, 1),
        });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let pos = ui_point(&app, (50, 50));
        ui_pass(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(pos)]);
        assert!(app.stroke.is_some(), "the brush follows the pointer");
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.open_file_requester(dpx_ui::FileKind::LoadPicture);
        for c in "Photo.png".chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.import_requester.is_some());
        // Moving over the page while the requester is open draws no brush.
        ui_pass(&mut app, &ctx, 0.2, vec![egui::Event::PointerMoved(pos)]);
        let elsewhere = ui_point(&app, (60, 60));
        ui_pass(
            &mut app,
            &ctx,
            0.3,
            vec![egui::Event::PointerMoved(elsewhere)],
        );
        assert!(app.stroke.is_none());
        assert_eq!(&*app.page(), app.engine.document().page());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_are_remembered_in_the_settings_file() {
        let ctx = egui::Context::default();
        let dir = temp_dir("settings");
        let config_dir = dir.join("config");
        write_png_rgb(&dir.join("Photo.png"), &photo(100, 80));
        let mut app = PainterApp::new();
        let (store, c) = crate::config::ConfigStore::open(&config_dir);
        c.apply(&mut app);
        app.config = Some(store);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        click(
            &mut app,
            &ctx,
            0.1,
            ImportControl::Position(Position::TopLeft),
        );
        click(&mut app, &ctx, 0.2, ImportControl::More);
        click(&mut app, &ctx, 0.3, ImportControl::Dither(Dither::Ordered));
        click(&mut app, &ctx, 0.4, ImportControl::Ok);
        // The settings file is written once they stop changing for a second.
        ui_pass(&mut app, &ctx, 0.5, vec![]);
        ui_pass(&mut app, &ctx, 2.0, vec![]);
        let text = std::fs::read_to_string(config_dir.join("settings.json")).unwrap();
        assert!(text.contains("\"import\""), "{text}");
        let mut next = PainterApp::new();
        let (_, c) = crate::config::ConfigStore::open(&config_dir);
        c.apply(&mut next);
        assert_eq!(
            next.import.settings,
            ImportSettings {
                position: Position::TopLeft,
                palette: PaletteChoice::New,
                colors: Some(64),
                dither: Dither::Ordered,
            }
        );
        let ctx = egui::Context::default();
        ui_pass(&mut next, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut next, &ctx, &dir, "Photo.png");
        let r = next.ui.import_requester.as_ref().unwrap();
        assert_eq!(r.options().position, Position::TopLeft);
        assert_eq!(r.options().colors, 64);
        assert_eq!(r.options().dither, Dither::Ordered);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transparent_pixels_and_notes_after_ok() {
        let ctx = egui::Context::default();
        let dir = temp_dir("alpha");
        let mut img = photo(64, 64);
        for y in 0..64 {
            for x in 0..8 {
                img.set(x, y, Rgba8::new(0, 0, 0, 0));
            }
        }
        let data: Vec<u8> = img
            .pixels()
            .iter()
            .flat_map(|p| [p.r, p.g, p.b, p.a])
            .collect();
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, 64, 64);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&data).unwrap();
        }
        std::fs::write(dir.join("Alpha.png"), bytes).unwrap();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Alpha.png");
        ui_pass(&mut app, &ctx, 0.1, vec![key_event(egui::Key::Enter, true)]);
        assert_eq!(app.extras.transparent_color, Some(0));
        assert_eq!(
            app.engine.document().palette().get(0),
            Some(files::TRANSPARENT_PINK)
        );
        let note = app
            .ui
            .note_requester
            .as_ref()
            .expect("a note about transparency");
        assert_eq!(note.title(), LOADED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_spec_parses_and_refuses_bad_values() {
        let s = parse_spec("position=top_left;palette=current;colors=8;dither=fs", 32).unwrap();
        assert_eq!(
            s,
            ImportSettings {
                position: Position::TopLeft,
                palette: PaletteChoice::Current,
                colors: Some(8),
                dither: Dither::FloydSteinberg,
            }
        );
        assert_eq!(parse_spec("", 32).unwrap(), ImportSettings::default());
        for bad in [
            "colors=12",
            "colors=64",
            "dither=floyd",
            "position=middle",
            "size=3",
            "colors",
        ] {
            assert!(parse_spec(bad, 32).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_held_return_does_not_answer_the_import_requester() {
        let ctx = egui::Context::default();
        let dir = temp_dir("repeat");
        write_png_rgb(&dir.join("Photo.png"), &photo(100, 80));
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 5 });
        let before = app.engine.document().clone();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.run_action(dpx_ui::Action::Command("picture.load"), &ctx);
        ui_pass(
            &mut app,
            &ctx,
            0.1,
            vec![egui::Event::Text("Photo.png".into())],
        );
        ui_pass(&mut app, &ctx, 0.2, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.ui.file_requester.is_none());
        assert!(
            app.ui.import_requester.is_some(),
            "the Import requester asks"
        );
        // Return is still held: egui marks the next press a repeat, as the OS's auto-repeat.
        ui_pass(&mut app, &ctx, 0.7, vec![key_event(egui::Key::Enter, true)]);
        let repeat = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::NONE,
        };
        ui_pass(&mut app, &ctx, 0.8, vec![repeat]);
        assert!(app.ui.import_requester.is_some(), "a held Return is no Ok");
        assert_eq!(app.engine.document(), &before);
        // Released and pressed again: Ok.
        ui_pass(
            &mut app,
            &ctx,
            0.9,
            vec![key_event(egui::Key::Enter, false)],
        );
        ui_pass(&mut app, &ctx, 1.0, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.ui.import_requester.is_none());
        assert_ne!(app.engine.document(), &before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_default_count_holds_the_whole_palette() {
        let ctx = egui::Context::default();
        let dir = temp_dir("count");
        write_png_rgb(&dir.join("Photo.png"), &photo(100, 80));
        let mut app = PainterApp::new();
        let twelve = Palette::new(
            (0..12u8)
                .map(|i| Rgb8::new(i * 20, 100, 255 - i * 20))
                .collect(),
        )
        .unwrap();
        let target = app.engine.document().target();
        app.set_picture(
            Document::from_picture(IndexedImage::new(320, 256, 0), twelve, target),
            files::PictureExtras::default(),
            dir.join("twelve.png"),
        );
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        let req = app.ui.import_requester.as_ref().unwrap();
        assert_eq!(req.options().colors, 16, "12 colours take 4 bitplanes");
        assert_eq!(req.shown_colors(), 16);
        click(
            &mut app,
            &ctx,
            0.1,
            ImportControl::Palette(PaletteChoice::Current),
        );
        let req = app.ui.import_requester.as_ref().unwrap();
        assert_eq!(
            req.shown_colors(),
            12,
            "Current shows the palette's own size"
        );
        assert_eq!(default_colors(32), 32);
        assert_eq!(default_colors(2), 2);
        assert_eq!(default_colors(17), 32);
        assert_eq!(default_colors(256), 256);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An indexed PNG with exactly the colour count, smaller than the page.
    fn write_many_colours(path: &std::path::Path, w: u32, h: u32) -> (IndexedImage, Palette) {
        let palette = Palette::new(
            (0..32u32)
                .map(|i| {
                    Rgb8::new(
                        (40 + i * 6) as u8,
                        (255 - i * 5) as u8,
                        ((i * 37) % 200 + 30) as u8,
                    )
                })
                .collect(),
        )
        .unwrap();
        let mut image = IndexedImage::new(w, h, 0);
        for y in 0..h {
            for x in 0..w {
                image.set(x as i32, y as i32, ((x / 3 + y * 7) % 32) as u8);
            }
        }
        std::fs::write(path, dpx_formats::write_png(&image, &palette).unwrap()).unwrap();
        (image, palette)
    }

    #[test]
    fn a_32_colour_png_stays_exact_on_the_default_page() {
        let ctx = egui::Context::default();
        let dir = temp_dir("exact");
        // 320×200 on the 320×256 page (56 rows of margin), and 40×30 enlarged 8×.
        for (name, (w, h), zoom) in [("Wide.png", (320, 200), 1), ("Art.png", (40, 30), 8)] {
            let (image, palette) = write_many_colours(&dir.join(name), w, h);
            let mut app = PainterApp::new();
            ui_pass(&mut app, &ctx, 0.0, vec![]);
            load_through_the_requester(&mut app, &ctx, &dir, name);
            let req = app.ui.import_requester.as_ref().unwrap();
            assert_eq!(req.info().colors, Some(32));
            assert_eq!(req.options().colors, 32);
            assert_eq!(req.options().dither, Dither::FloydSteinberg);
            ui_pass(
                &mut app,
                &ctx,
                0.1,
                vec![
                    key_event(egui::Key::Enter, true),
                    key_event(egui::Key::Enter, false),
                ],
            );
            assert!(app.ui.import_requester.is_none());
            let top = (256 - h * zoom) / 2;
            let mut wrong = 0;
            for y in 0..h * zoom {
                for x in 0..w * zoom {
                    let want =
                        palette.get(image.get((x / zoom) as i32, (y / zoom) as i32).unwrap());
                    if Some(shown(&app, x, top + y)) != want {
                        wrong += 1;
                    }
                }
            }
            assert_eq!(wrong, 0, "{name}: every pixel its own colour");
            assert_eq!(app.engine.document().palette().len(), 32);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toolbox_marks_stay_under_the_requester() {
        let ctx = egui::Context::default();
        let dir = temp_dir("marks");
        write_png_rgb(&dir.join("Photo.png"), &photo(100, 80));
        // NTSC LoRes (320×200) and the default PAL screen.
        for ntsc in [true, false] {
            let mut app = PainterApp::new();
            if ntsc {
                let mode = DisplayMode::new(dpx_core::Video::Ntsc, dpx_core::Resolution::Lo, false);
                let target = Target::new(Chipset::Aga, mode);
                app.set_picture(
                    Document::from_picture(
                        IndexedImage::new(320, 200, 0),
                        Palette::dpaint_default_32(),
                        target,
                    ),
                    files::PictureExtras::default(),
                    dir.join("ntsc.iff"),
                );
            }
            ui_pass(&mut app, &ctx, 0.0, vec![]);
            load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
            let plain = app.compose_screen();
            // A custom brush (no built-in brush selected) and Symmetry on.
            app.custom_brush = Some(dpx_core::BrushImage {
                width: 1,
                height: 1,
                pixels: vec![1],
                transparent: None,
                handle: (0, 0),
            });
            app.tools.symmetry = true;
            app.tools.grid = true;
            let marked = app.compose_screen();
            let req = app.ui.import_requester.as_ref().unwrap();
            let (ox, oy) = req.origin();
            let (w, h) = req.size();
            for y in oy..oy + h {
                for x in ox..ox + w {
                    assert_eq!(marked.get(x, y), plain.get(x, y), "ntsc {ntsc}: {x},{y}");
                }
            }
            // The snapshot path keeps the custom brush's mark under it too.
            let view = app.view();
            let doc = app.engine.document();
            let (_, _, a) = crate::snapshot_rgba(1, doc, view, (0, 0), &app.ui, false);
            let (fw, _, b) = crate::snapshot_rgba(1, doc, view, (0, 0), &app.ui, true);
            for y in oy..oy + h {
                for x in ox..ox + w {
                    let o = (y as usize * fw as usize + x as usize) * 4;
                    assert_eq!(a[o..o + 4], b[o..o + 4], "snapshot, ntsc {ntsc}: {x},{y}");
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshots_show_the_result_thumbnail_in_true_colour() {
        use dpx_core::{Resolution, Video};
        let dir = temp_dir("snapshot");
        let file = dir.join("Photo.png");
        write_png_rgb(&file, &photo(400, 300));
        let file = file.to_string_lossy().into_owned();
        // The default LoRes screen and a HiRes one (2×1 screen pixels per UI pixel).
        let hires = Target::new(
            Chipset::Aga,
            DisplayMode::new(Video::Pal, Resolution::Hi, false),
        );
        let docs = [
            PainterApp::new().engine.document().clone(),
            Document::from_picture(
                IndexedImage::new(640, 256, 0),
                Palette::dpaint_default_32(),
                hires,
            ),
        ];
        for doc in docs {
            let view = View::of_target(doc.target());
            let args = |apply: bool| {
                let mut a: Vec<String> = ["--import-requester", &file, "--import", "colors=256"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                if apply {
                    a.push("--import-apply".into());
                }
                a
            };
            let mut ui = UiState::default();
            let shown = snapshot(&args(false), &doc, &mut ui).unwrap();
            assert_eq!(shown, doc, "the page is unchanged under the requester");
            let mut after = UiState::default();
            let result = snapshot(&args(true), &doc, &mut after).unwrap();
            assert!(after.import_requester.is_none());
            let req = ui.import_requester.clone().expect("the requester is shown");
            let (tw, th) = req.thumb_size();
            let want = import::thumbnail(result.page(), result.palette(), tw, th);
            for scale in [1usize, 2] {
                let (fw, _, rgba) = crate::snapshot_rgba(scale, &shown, view, (0, 0), &ui, false);
                let (sx, sy) = (
                    view.ux as usize * scale * view.fx as usize,
                    view.uy as usize * scale * view.fy as usize,
                );
                let r = req.thumb_rect();
                let mut true_colour = 0;
                for y in 0..th as usize * sy {
                    for x in 0..tw as usize * sx {
                        let fx = r.x as usize * sx + x;
                        let fy = r.y as usize * sy + y;
                        let o = (fy * fw as usize + fx) * 4;
                        let w = want.get((x / sx) as u32, (y / sy) as u32).unwrap();
                        assert_eq!(rgba[o..o + 3], [w.r, w.g, w.b], "scale {scale}: {x},{y}");
                        if !doc.palette().colors().contains(&Rgb8::new(w.r, w.g, w.b)) {
                            true_colour += 1;
                        }
                    }
                }
                assert!(true_colour > (tw * th) as usize * sx * sy / 2);
                // Around the box: the indexed frame (face gap, then the dark frame).
                let pens = dpx_ui::Pens::from_palette(doc.palette());
                let px = |ux: i32, uy: i32| {
                    let o = ((uy as usize * sy) * fw as usize + ux as usize * sx) * 4;
                    Rgb8::new(rgba[o], rgba[o + 1], rgba[o + 2])
                };
                assert_eq!(Some(px(r.x - 1, r.y - 1)), doc.palette().get(pens.face));
                assert_eq!(Some(px(r.x - 2, r.y - 2)), doc.palette().get(pens.dark));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Presses and releases a requester control without the ui loop.
    fn press(app: &mut PainterApp, c: ImportControl) {
        let req = app.ui.import_requester.as_ref().expect("requester open");
        let r = req.controls().into_iter().find(|&(k, _)| k == c).unwrap().1;
        app.import_event(ReqEvent::Press(r.x + r.w / 2, r.y + r.h / 2));
        app.import_event(ReqEvent::Release);
    }

    #[test]
    fn big_pages_reduce_in_the_background_and_ok_waits_for_the_page() {
        let ctx = egui::Context::default();
        let dir = temp_dir("background");
        write_png_rgb(&dir.join("Photo.png"), &photo(400, 200));
        let mut app = PainterApp::new();
        // A 2048×1024 page (Page Size): the picture is fitted to 2048×1024, 2.1 megapixels.
        let target = app.engine.document().target();
        app.set_picture(
            Document::from_picture(
                IndexedImage::new(2048, 1024, 0),
                Palette::dpaint_default_32(),
                target,
            ),
            files::PictureExtras::default(),
            dir.join("big.iff"),
        );
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        let p = app.import.pending.as_ref().unwrap();
        assert!(!p.busy(), "the first page is made when the requester opens");
        let image = p.fitted.as_ref().unwrap().image().clone();
        assert!(u64::from(image.width()) * u64::from(image.height()) >= BACKGROUND_PIXELS);
        let first = app
            .ui
            .import_requester
            .as_ref()
            .unwrap()
            .thumbnail()
            .cloned();
        // A change starts a background job; the old thumbnail stays until it ends.
        press(&mut app, ImportControl::Dither(Dither::Ordered));
        assert!(app.import.pending.as_ref().unwrap().busy());
        assert_eq!(
            app.ui
                .import_requester
                .as_ref()
                .unwrap()
                .thumbnail()
                .cloned(),
            first
        );
        // A second change while it runs; the requester still answers at once.
        press(&mut app, ImportControl::Fewer);
        let wanted = app.ui.import_requester.as_ref().unwrap().options();
        assert_eq!((wanted.dither, wanted.colors), (Dither::Ordered, 16));
        // The ui loop collects the results and starts the next job itself.
        let mut t = 0.1;
        for _ in 0..4000 {
            if !app.import.pending.as_ref().unwrap().busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
            ui_pass(&mut app, &ctx, t, vec![]);
            t += 0.002;
        }
        let start = app.engine.document().clone();
        let tg = import_target(&start, app.ui.bg);
        let p = app.import.pending.as_ref().unwrap();
        assert!(!p.busy());
        assert_eq!(p.made, wanted);
        let want = import::reduce_fitted(&image, &wanted, &tg);
        assert_eq!(p.result, want, "the page for the last options");
        let req = app.ui.import_requester.as_ref().unwrap();
        let (tw, th) = req.thumb_size();
        assert_eq!(
            req.thumbnail(),
            Some(&import::thumbnail(&want.page, &want.palette, tw, th))
        );
        // Ok right after a change waits for that change's page.
        press(&mut app, ImportControl::Position(Position::TopLeft));
        press(&mut app, ImportControl::Dither(Dither::None));
        let last = app.ui.import_requester.as_ref().unwrap().options();
        let want = import::reduce_fitted(&image, &last, &tg);
        ui_pass(&mut app, &ctx, t, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.ui.import_requester.is_none());
        let doc = app.engine.document();
        assert_eq!(doc.page(), &want.page);
        assert_eq!(doc.palette(), &want.palette);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn option_changes_reuse_palettes_and_the_mapped_picture() {
        let ctx = egui::Context::default();
        let dir = temp_dir("reuse");
        write_png_rgb(&dir.join("Photo.png"), &photo(200, 160));
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        load_through_the_requester(&mut app, &ctx, &dir, "Photo.png");
        let made = |app: &PainterApp| {
            let f = app
                .import
                .pending
                .as_ref()
                .unwrap()
                .fitted
                .as_ref()
                .unwrap();
            (f.palettes_made(), f.mappings_made())
        };
        assert_eq!(made(&app), (1, 1));
        // A position change only places the same picture again.
        press(&mut app, ImportControl::Position(Position::TopLeft));
        assert_eq!(made(&app), (1, 1));
        // A dither change maps again with the same palette.
        press(&mut app, ImportControl::Dither(Dither::Ordered));
        assert_eq!(made(&app), (1, 2));
        // A new count builds a palette; going back reuses the first.
        press(&mut app, ImportControl::Fewer);
        assert_eq!(made(&app), (2, 3));
        press(&mut app, ImportControl::More);
        assert_eq!(made(&app), (2, 4));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Timings for ADR-42 (`cargo test [--release] -p deluxepainter -- --ignored --nocapture
    /// import_timing`): opening the requester (fit, first reduction, thumbnail) and one option
    /// change at a time, on AGA documents with 256 colours (the slowest count, the default
    /// there) on standard screens and on larger pages made with Page Size.
    #[test]
    #[ignore]
    fn import_timing() {
        use dpx_core::{Resolution, Video};
        let palette = Palette::new(
            (0..=255u8)
                .map(|i| Rgb8::new(i, i.wrapping_mul(3), 255 - i))
                .collect(),
        )
        .unwrap();
        let pages = [
            ("LoRes 320x256", Resolution::Lo, false, (320, 256)),
            ("HiRes laced 640x512", Resolution::Hi, true, (640, 512)),
            (
                "SuperHiRes laced 1280x512",
                Resolution::SuperHi,
                true,
                (1280, 512),
            ),
            ("LoRes, page 1024x1024", Resolution::Lo, false, (1024, 1024)),
            ("LoRes, page 4096x4096", Resolution::Lo, false, (4096, 4096)),
        ];
        let filter = std::env::var("IMPORT_TIMING").unwrap_or_default();
        for (w, h) in [(4032u32, 3024u32), (1280, 1024)] {
            let img = photo(w, h);
            for (name, res, laced, page) in pages {
                if !filter.is_empty() && !name.contains(&filter) {
                    continue;
                }
                let mut app = PainterApp::new();
                let target = Target::new(Chipset::Aga, DisplayMode::new(Video::Pal, res, laced));
                app.set_picture(
                    Document::from_picture(
                        IndexedImage::new(page.0, page.1, 0),
                        palette.clone(),
                        target,
                    ),
                    files::PictureExtras::default(),
                    PathBuf::from("start.iff"),
                );
                let t0 = std::time::Instant::now();
                let colors = import::count_colors(&img, 256);
                app.open_import_image(PathBuf::from("timing.png"), img.clone(), colors, Vec::new());
                let open = t0.elapsed().as_secs_f64();
                assert_eq!(
                    app.ui.import_requester.as_ref().unwrap().options().colors,
                    256
                );
                let mut times = Vec::new();
                // Per change: how long the click blocks, and when the page is ready
                // (later only on big pages, where a background job makes it).
                let mut time = |app: &mut PainterApp, label: &str, c: ImportControl| {
                    let t = std::time::Instant::now();
                    press(app, c);
                    let block = t.elapsed().as_secs_f64();
                    app.import_settle();
                    let ready = t.elapsed().as_secs_f64();
                    times.push(if ready - block > 0.001 {
                        format!("{label} {block:.3}/{ready:.3}")
                    } else {
                        format!("{label} {block:.3}")
                    });
                };
                time(
                    &mut app,
                    "position",
                    ImportControl::Position(Position::TopLeft),
                );
                time(
                    &mut app,
                    "fs->ordered",
                    ImportControl::Dither(Dither::Ordered),
                );
                time(
                    &mut app,
                    "ordered->none",
                    ImportControl::Dither(Dither::None),
                );
                time(
                    &mut app,
                    "none->fs",
                    ImportControl::Dither(Dither::FloydSteinberg),
                );
                time(&mut app, "256->128", ImportControl::Fewer);
                time(&mut app, "128->256", ImportControl::More);
                time(
                    &mut app,
                    "new->current",
                    ImportControl::Palette(PaletteChoice::Current),
                );
                time(
                    &mut app,
                    "current->new",
                    ImportControl::Palette(PaletteChoice::New),
                );
                let t = std::time::Instant::now();
                app.import_event(ReqEvent::Key(ReqKey::Enter));
                let ok = t.elapsed().as_secs_f64();
                println!(
                    "{w}x{h} on {name}: open {open:.3}; {}; ok {ok:.3}",
                    times.join(", ")
                );
            }
        }
    }
}
