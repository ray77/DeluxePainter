//! DeluxePainter desktop application.
//!
//! `deluxepainter`                       start the GUI
//! `deluxepainter --snapshot out.png`    render the start screen headless to a PNG
//!     [--scale N]                       integer scale (default 2)
//!     [--palette gold16]                use the 16-colour demo palette
//!     [--fg N] [--tool NAME[:filled]]   foreground colour and selected tool
//!     [--menu M[:I[:S]]]                open menu M (0 = Picture), highlight item I / sub item S
//!     [--requester]                     show the Screen Format requester instead
//!     [--file-requester load|save|export] show the Load/Save/Export requester
//!     [--drawer DIR] [--file NAME]      … listing DIR, with NAME in the File field
//!     [--drawer-as TEXT]                … showing TEXT in the Drawer field (doc screenshots)
//!     [--page WxH] [--scroll X,Y]       page size (BG colour 0) and the part of it shown
//!     [--commands FILE.json]            apply a JSON array of commands first (ADR-14)
//!     [--pick X,Y,W,H]                  then pick up that part of the page as custom brush
//!     [--brush-commands FILE.json]      … and apply commands that stamp it (ADR-18)
//!     [--show-page]                     show the Show Page view
//!     [--page-size-requester]           show the Page Size requester
//!     [--palette-requester]             show the Color Mixer (selected colour = `--fg`)
//!     [--range-requester]               show the Range requester (ranges of `--open`, else the defaults)
//!     [--notes]                         show the notes of `--open` in the note requester
//!     [--frames N] [--frame I]          make N animation frames (before `--commands`), then
//!                                       show frame I (from 1) (ADR-28)
//!     [--light-table [LIST]]            Light Table on (previous,next,two_back,spare,dim)
//!     [--anim-requester KIND]           show an Anim requester: count, add, copy, delete,
//!                                       goto or rate
//! `deluxepainter --font-preview out.png` render sample lines of the UI font (3×)
//! `deluxepainter --open pic.iff`        start the GUI (or a snapshot) with a picture
//! `deluxepainter --no-settings`         start the GUI without reading or writing the settings file
//! `deluxepainter info <file>`           describe an IFF ILBM, PNG, JPEG or .dpxproj file
//! `deluxepainter convert <in> <out>`    convert to IFF ILBM, PNG or .dpxproj

mod anim;
mod browse;
mod brush_menu;
mod config;
mod effects;
mod files;
mod menu_actions;
mod settings;
mod tools;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dpx_core::{
    Axis, Command, DisplayMode, Document, Engine, ErrorCode, IndexedImage, Palette, Rect, Target,
};
use dpx_ui::menu::{self, MENUS};
use dpx_ui::requester::{self, ReqEvent, ReqKey, ReqResponse, ScreenFormatRequester};
use dpx_ui::{
    Action, Browse, FileKind, FileRequester, FileResponse, Hit, NoteRequester, NoteResponse,
    PageSizeRequester, PageSizeResponse, PaintMode, Pens, PixelSize, Preset, Screen, ShowPage,
    ShowPageResponse, Tool, UiState, to_rgba,
};
use eframe::egui;

/// Arrow keys scroll the page by this many UI (LoRes) pixels; Alt+arrows by a screen. The
/// manual gives no step for plain arrows (ADR-10).
const SCROLL_STEP: i32 = 16;

/// Initial window size in points: PAL LoRes at 3×.
const START_W: f32 = 960.0;
const START_H: f32 = 768.0;

/// After the user stops resizing the window for this long, it snaps to the nearest pixel size
/// (seconds).
const SNAP_SETTLE: f64 = 0.25;

/// Room kept free on the monitor for the menu bar, the window title and the Dock (points).
const MONITOR_MARGIN: egui::Vec2 = egui::vec2(0.0, 100.0);

/// The window's drawing area in physical pixels and the OS scale (physical pixels per point),
/// as of the last frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Window {
    w: i32,
    h: i32,
    ppp: f32,
}

impl Window {
    /// The drawing area in points.
    fn points(&self) -> egui::Vec2 {
        egui::vec2(self.w as f32, self.h as f32) / self.ppp
    }
}

/// The largest pixel size (half points per UI pixel, 1x–8x) at which a screen of `ui` UI
/// pixels fits into `avail` points.
fn fit_step(avail: egui::Vec2, ui: (i32, i32)) -> u8 {
    let steps = |len: f32, n: i32| (len * 2.0 / n as f32 + 1e-3).floor() as i32;
    steps(avail.x, ui.0)
        .min(steps(avail.y, ui.1))
        .clamp(i32::from(PixelSize::MIN), i32::from(PixelSize::MAX)) as u8
}

/// The pixel size nearest to a window the user sized to `size` points (the smaller of the
/// two axes decides).
fn nearest_step(size: egui::Vec2, ui: (i32, i32)) -> u8 {
    let x = size.x * 2.0 / ui.0 as f32;
    let y = size.y * 2.0 / ui.1 as f32;
    (x.min(y).round() as i32).clamp(i32::from(PixelSize::MIN), i32::from(PixelSize::MAX)) as u8
}

/// The window's drawing area (points) that shows a screen of `ui` UI pixels at `step`.
fn window_size(ui: (i32, i32), step: u8) -> egui::Vec2 {
    egui::vec2(ui.0 as f32, ui.1 as f32) * f32::from(step) / 2.0
}

/// Snapping state of the window (ADR-13).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Snap {
    /// Drawing area (points) in the last frame.
    last: Option<egui::Vec2>,
    /// When the user last changed the window size; cleared when it snapped.
    resized_at: Option<f64>,
    /// The size we asked for and have not seen yet.
    requested: Option<egui::Vec2>,
}

/// How a display mode maps to the window: its screen size, the size of a UI (LoRes) pixel in
/// screen pixels, and the displayed shape of one screen pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct View {
    screen_w: i32,
    screen_h: i32,
    ux: i32,
    uy: i32,
    fx: i32,
    fy: i32,
}

impl View {
    /// A display mode at its standard screen size.
    fn of(mode: DisplayMode) -> Self {
        Self::screen(mode, mode.size())
    }

    /// The document's screen: its mode at its (standard, overscan or custom) size.
    fn of_target(target: Target) -> Self {
        Self::screen(target.mode, target.screen)
    }

    fn screen(mode: DisplayMode, (w, h): (u32, u32)) -> Self {
        let (ux, uy) = mode.ui_scale();
        let (fx, fy) = mode.pixel_shape();
        Self {
            screen_w: w as i32,
            screen_h: h as i32,
            ux: ux as i32,
            uy: uy as i32,
            fx: fx as i32,
            fy: fy as i32,
        }
    }

    /// The screen's size in UI (LoRes) pixels.
    fn ui_size(&self) -> (i32, i32) {
        (self.screen_w / self.ux, self.screen_h / self.uy)
    }

    /// The UI layout screen (always LoRes geometry: 320 wide).
    fn ui_screen(&self) -> Screen {
        Screen::new(self.screen_w / self.ux, self.screen_h / self.uy)
    }

    /// The screen with the part of `page` that starts at `scroll` and the interface on top.
    fn compose(
        &self,
        page: &IndexedImage,
        scroll: (i32, i32),
        palette: &Palette,
        ui: &UiState,
    ) -> IndexedImage {
        let visible = page.crop(Rect::new(scroll.0, scroll.1, self.screen_w, self.screen_h));
        self.ui_screen()
            .render_scaled(&visible, palette, ui, self.ux, self.uy)
    }

    /// Largest scroll position for a page of `page` pixels: the page's right/bottom edge at
    /// the screen's right/bottom edge.
    fn max_scroll(&self, page: &IndexedImage) -> (i32, i32) {
        (
            (page.width() as i32 - self.screen_w).max(0),
            (page.height() as i32 - self.screen_h).max(0),
        )
    }

    /// Screen pixel → UI pixel.
    fn ui_pos(self, (x, y): (i32, i32)) -> (i32, i32) {
        (x.div_euclid(self.ux), y.div_euclid(self.uy))
    }
}

/// A small 16-colour palette (blacks, blues, golds) to demonstrate palette-derived UI pens.
fn gold16() -> Palette {
    Palette::from_rgb4(&[
        0x000, 0x112, 0x124, 0x236, 0x348, 0x46A, 0x320, 0x530, //
        0x750, 0x970, 0xA80, 0xC90, 0xDB4, 0xED6, 0xFE9, 0xFFC, //
    ])
    .expect("16 colours")
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    match (args.get(1).map(String::as_str), &args[args.len().min(2)..]) {
        (Some("info"), [path]) => exit_with(files::info(Path::new(path)).map(|s| print!("{s}"))),
        (Some("convert"), [input, output]) => exit_with(
            files::convert(Path::new(input), Path::new(output))
                .map(|notes| notes.iter().for_each(|n| eprintln!("note: {n}"))),
        ),
        (Some("mcp"), rest) => exit_with(dpx_mcp::run_cli(rest)),
        (Some("info" | "convert"), _) => {
            eprintln!("usage: deluxepainter info <file> | convert <in> <out>");
            std::process::exit(2);
        }
        _ => {}
    }
    let opened = arg_value(&args, "--open").map(|p| match files::load(Path::new(&p)) {
        Ok(loaded) => {
            let (doc, extras, notes) = files::open_picture(loaded.picture, Target::default());
            let (doc, grown) = files::page_at_least_screen(doc);
            let notes: Vec<String> = loaded.notes.into_iter().chain(notes).chain(grown).collect();
            notes.iter().for_each(|n| eprintln!("note: {n}"));
            (PathBuf::from(p), doc, extras, notes)
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    });
    if let Some(path) = arg_value(&args, "--font-preview") {
        if let Err(e) = font_preview(&path) {
            eprintln!("font preview failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        let Some(path) = args.get(i + 1) else {
            eprintln!("usage: deluxepainter --snapshot <out.png> [--scale N] [--palette gold16]");
            std::process::exit(2);
        };
        let scale = arg_value(&args, "--scale")
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        let doc = match &opened {
            Some((_, doc, _, _)) => doc.clone(),
            None => {
                let (w, h) = DisplayMode::PAL_LORES.size();
                Document::new(
                    w,
                    h,
                    match arg_value(&args, "--palette").as_deref() {
                        Some("gold16") => gold16(),
                        _ => Palette::dpaint_default_32(),
                    },
                )
            }
        };
        let mut engine = Engine::new(doc);
        if let Some(spec) = arg_value(&args, "--page") {
            let size = spec
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
            let Some((width, height)) = size else {
                eprintln!("bad --page '{spec}' (expected WxH)");
                std::process::exit(2);
            };
            if let Err(e) = engine.execute(Command::SetPageSize {
                width,
                height,
                fill: 0,
            }) {
                eprintln!("--page {spec}: {}", e.message);
                std::process::exit(2);
            }
        }
        // ADR-28: animation frames before the commands, which may go to frames themselves.
        if let Some(n) = arg_value(&args, "--frames") {
            let count = n.parse().unwrap_or(0);
            if let Err(e) = engine.execute(Command::SetFrameCount { count }) {
                eprintln!("--frames {n}: {}", e.message);
                std::process::exit(2);
            }
        }
        if let Some(file) = arg_value(&args, "--commands")
            && let Err(e) = tools::run_command_file(&mut engine, &file)
        {
            eprintln!("--commands {e}");
            std::process::exit(2);
        }
        // ADR-18: pick up a custom brush, then paint with it.
        let picked = arg_value(&args, "--pick").map(|spec| {
            tools::pick_spec(engine.document().page(), &spec).unwrap_or_else(|e| {
                eprintln!("--pick {e}");
                std::process::exit(2);
            })
        });
        if let Some(file) = arg_value(&args, "--brush-commands") {
            let Some(brush) = &picked else {
                eprintln!("--brush-commands needs --pick");
                std::process::exit(2);
            };
            if let Err(e) = tools::run_brush_command_file(&mut engine, &file, brush) {
                eprintln!("--brush-commands {e}");
                std::process::exit(2);
            }
        }
        // ADR-30: stamp Brush menu transforms of the picked brush.
        if let Some(spec) = arg_value(&args, "--brush-xforms") {
            let Some(brush) = &picked else {
                eprintln!("--brush-xforms needs --pick");
                std::process::exit(2);
            };
            if let Err(e) = tools::run_xform_spec(&mut engine, brush, &spec) {
                eprintln!("--brush-xforms {e}");
                std::process::exit(2);
            }
        }
        if let Some(i) = arg_value(&args, "--frame") {
            let frame = i.parse::<u32>().unwrap_or(0).saturating_sub(1);
            if let Err(e) = engine.execute(Command::GotoFrame { frame }) {
                eprintln!("--frame {i}: {}", e.message);
                std::process::exit(2);
            }
        }
        let doc = engine.document().clone();
        let view = View::of_target(doc.target());
        let scroll = arg_value(&args, "--scroll")
            .and_then(|s| {
                let (x, y) = s.split_once(',')?;
                Some((x.parse().ok()?, y.parse().ok()?))
            })
            .map_or((0, 0), |(x, y): (i32, i32)| {
                let (mx, my) = view.max_scroll(doc.page());
                (x.clamp(0, mx), y.clamp(0, my))
            });
        let mut ui = UiState::default();
        if let Some(fg) = arg_value(&args, "--fg").and_then(|s| s.parse().ok()) {
            ui.fg = fg;
        }
        if picked.is_some() {
            // As after a pick-up in the program.
            ui.mode = PaintMode::Matte;
        }
        if let Some(spec) = arg_value(&args, "--tool") {
            let (name, filled) = spec
                .split_once(':')
                .map_or((spec.as_str(), false), |(n, f)| (n, f == "filled"));
            match dpx_ui::toolbox::TOOL_ROWS
                .iter()
                .flatten()
                .find(|t| t.name().eq_ignore_ascii_case(name))
            {
                Some(&t) => {
                    ui.tool = t;
                    ui.filled = filled;
                }
                None => {
                    eprintln!("unknown tool '{name}'");
                    std::process::exit(2);
                }
            }
        }
        if let Some(spec) = arg_value(&args, "--menu") {
            match parse_menu_spec(&spec) {
                Some((m, item, sub)) => {
                    ui.menu.active = true;
                    ui.menu.menu = Some(m);
                    ui.menu.item = item;
                    ui.menu.sub_item = sub;
                }
                None => {
                    eprintln!("bad --menu '{spec}' (expected M[:I[:S]] with valid indices)");
                    std::process::exit(2);
                }
            }
        }
        if let Some(kind) = arg_value(&args, "--file-requester") {
            let kind = match kind.as_str() {
                "load" => FileKind::LoadPicture,
                "save" => FileKind::SavePicture,
                "export" => FileKind::ExportPicture,
                "load-anim" => FileKind::LoadAnim,
                "save-anim" => FileKind::SaveAnim,
                _ => {
                    eprintln!("--file-requester expects load, save or export");
                    std::process::exit(2);
                }
            };
            let mut req = FileRequester::new(kind);
            let drawer = arg_value(&args, "--drawer").unwrap_or_else(|| ".".into());
            match browse::list_drawer(&drawer) {
                Ok((listed, entries)) => {
                    let shown = arg_value(&args, "--drawer-as").unwrap_or(listed);
                    req.set_listing(&shown, entries);
                }
                Err(e) => {
                    eprintln!("{drawer}: {e}");
                    std::process::exit(1);
                }
            }
            if let Some(name) = arg_value(&args, "--file") {
                req.set_file(&name);
            }
            ui.file_requester = Some(req);
        }
        if args.iter().any(|a| a == "--palette-requester") {
            ui.palette_requester = Some(dpx_ui::PaletteRequester::new(
                doc.palette(),
                ui.fg,
                doc.target().chipset,
            ));
        }
        if args.iter().any(|a| a == "--range-requester") {
            let ranges = match &opened {
                Some((_, _, extras, _)) => extras.ranges.clone(),
                None => files::default_ranges(doc.palette()),
            };
            ui.range_requester = Some(menu_actions::range_requester(doc.palette(), &ranges));
        }
        if let Some(req) = settings::snapshot_requester(&args, &doc) {
            ui.tool_requester = Some(req);
        }
        if args.iter().any(|a| a == "--page-size-requester") {
            ui.page_size_requester = Some(page_size_requester(&doc));
        }
        // ADR-28: frame counter, Anim requesters and the Light Table.
        let count = doc.frame_count();
        ui.frame_counter = (count > 1).then_some((doc.current_frame() + 1, count));
        if let Some(kind) = arg_value(&args, "--anim-requester") {
            match anim::snapshot_requester(&kind, &doc) {
                Some(req) => ui.anim_requester = Some(req),
                None => {
                    eprintln!("--anim-requester expects count, add, copy, delete, goto or rate");
                    std::process::exit(2);
                }
            }
        }
        let doc = if args.iter().any(|a| a == "--light-table") {
            let spec = arg_value(&args, "--light-table").filter(|v| !v.starts_with("--"));
            let light = anim::parse_light_table(spec.as_deref()).unwrap_or_else(|e| {
                eprintln!("--light-table: {e}");
                std::process::exit(2);
            });
            let page = anim::light_table_page(&doc, doc.page(), light, None, ui.bg, light.dim);
            Document::from_picture(page, doc.palette().clone(), doc.target())
        } else {
            doc
        };
        if args.iter().any(|a| a == "--notes")
            && let Some((_, _, _, notes)) = &opened
        {
            ui.note_requester = note_requester(LOADED, notes);
        }
        if args.iter().any(|a| a == "--show-page") {
            let sp = ShowPage::new(
                (doc.page().width(), doc.page().height()),
                (view.screen_w as u32, view.screen_h as u32),
                scroll,
            );
            let screen = sp.render(doc.page(), doc.palette().len());
            let (sx, sy) = (scale * view.fx as usize, scale * view.fy as usize);
            let rgba = to_rgba(&screen, doc.palette(), sx, sy);
            if let Err(e) = write_png(
                path,
                screen.width() * sx as u32,
                screen.height() * sy as u32,
                &rgba,
            ) {
                eprintln!("snapshot failed: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        if args.iter().any(|a| a == "--requester") {
            let page = (doc.page().width(), doc.page().height());
            let req = ScreenFormatRequester::new(doc.target(), page, doc.palette().len());
            let screen = req.render();
            let view = View::of(requester::SCREEN);
            let (sx, sy) = (scale * view.fx as usize, scale * view.fy as usize);
            let rgba = to_rgba(&screen, &requester::palette(), sx, sy);
            if let Err(e) = write_png(
                path,
                screen.width() * sx as u32,
                screen.height() * sy as u32,
                &rgba,
            ) {
                eprintln!("snapshot failed: {e}");
                std::process::exit(1);
            }
            return Ok(());
        }
        if let Err(e) = snapshot(path, scale, &doc, view, scroll, &ui, picked.is_some()) {
            eprintln!("snapshot failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // The settings file (ADR-33). Snapshots and the CLI commands returned above and never
    // touch it; `--no-settings` skips it for the GUI too.
    let (store, settings) = if args.iter().any(|a| a == "--no-settings") {
        (None, config::Config::default())
    } else {
        match config::default_dir() {
            Some(dir) => {
                let (store, c) = config::ConfigStore::open(&dir);
                (Some(store), c)
            }
            None => (None, config::Config::default()),
        }
    };
    let start = if settings.pixel_size == PixelSize::default().0 {
        egui::vec2(START_W, START_H)
    } else {
        window_size(
            View::of_target(Target::default()).ui_size(),
            settings.pixel_size,
        )
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DeluxePainter")
            .with_inner_size(start)
            .with_min_inner_size([320.0, 200.0])
            .with_icon(Arc::new(app_icon())),
        ..Default::default()
    };
    eframe::run_native(
        "DeluxePainter",
        options,
        Box::new(move |cc| {
            // Cmd/Ctrl + `+`/`−`/`0` set DeluxePainter's pixel size instead of egui's zoom.
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            let mut app = PainterApp::new();
            settings.apply(&mut app);
            app.config = store;
            if let Some((path, doc, extras, notes)) = opened {
                app.set_picture(doc, extras, path);
                app.ui.note_requester = note_requester(LOADED, &notes);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Title of the note requester after Picture ▸ Load (and `--open`) and after Save.
const LOADED: &str = "Picture loaded";
const SAVED: &str = "Picture saved";
const EXPORTED: &str = "Picture exported";
const ANIM_SAVED: &str = "Animation saved";

/// Picture ▸ Save.. always writes the lossless project (ADR-27): a picture extension becomes
/// `.dpxproj`, any other name gets `.dpxproj` appended (Amiga names often have no extension).
fn project_path(path: &Path) -> PathBuf {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("dpxproj") => path.to_path_buf(),
        Some("iff" | "ilbm" | "lbm" | "png" | "jpg" | "jpeg" | "anim") => {
            path.with_extension("dpxproj")
        }
        _ => {
            let mut name = path.as_os_str().to_owned();
            name.push(".dpxproj");
            PathBuf::from(name)
        }
    }
}

/// The name Anim ▸ Save.. writes: `.anim` is kept, a picture or project extension becomes
/// `.anim`, any other name gets `.anim` appended (ADR-32).
fn anim_path(path: &Path) -> PathBuf {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("anim") => path.to_path_buf(),
        Some("iff" | "ilbm" | "lbm" | "png" | "jpg" | "jpeg" | "dpxproj") => {
            path.with_extension("anim")
        }
        _ => {
            let mut name = path.as_os_str().to_owned();
            name.push(".anim");
            PathBuf::from(name)
        }
    }
}

/// The name Picture ▸ Export.. suggests: the picture's own name, or for a project the same
/// name as an IFF picture (ADR-27).
fn export_name(path: &Path) -> Option<String> {
    let is_project = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dpxproj"));
    let name = if is_project {
        path.with_extension("iff")
    } else {
        path.to_path_buf()
    };
    name.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// The note requester for `notes`, or `None` when there is nothing to tell (ADR-12).
fn note_requester(title: &str, notes: &[String]) -> Option<NoteRequester> {
    (!notes.is_empty()).then(|| NoteRequester::new(title, notes))
}

/// Picture ▸ Page Size for `doc`: its page size in the fields and the mode's presets.
/// Standard is the screen size (also an overscan or custom screen), as in DeluxePaint.
fn page_size_requester(doc: &Document) -> PageSizeRequester {
    let target = doc.target();
    PageSizeRequester::new(
        (doc.page().width(), doc.page().height()),
        [
            Preset {
                label: "Standard",
                size: target.screen,
            },
            Preset {
                label: "Full Page",
                size: target.full_page_size(),
            },
            Preset {
                label: "Overscan",
                size: target.mode.overscan_size(),
            },
        ],
    )
}

/// Parses `M[:I[:S]]` (menu, item, sub item indices) and checks them against [`MENUS`].
fn parse_menu_spec(spec: &str) -> Option<(usize, Option<usize>, Option<usize>)> {
    let nums: Vec<usize> = spec
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let (&m, rest) = nums.split_first()?;
    let items = MENUS.get(m)?.items;
    let item = match rest.first() {
        Some(&i) if i < items.len() => Some(i),
        Some(_) => return None,
        None => None,
    };
    let sub = match rest.get(1) {
        Some(&s) => match items[item?].entry {
            menu::Entry::Submenu(sub_items) if s < sub_items.len() => Some(s),
            _ => return None,
        },
        None => None,
    };
    (rest.len() <= 2).then_some((m, item, sub))
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Ends a headless command: prints the error and exits non-zero on failure.
fn exit_with(result: Result<(), String>) -> ! {
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Renders `view` of `doc` headless; each screen pixel becomes `scale` × its displayed shape
/// (e.g. HiRes pixels are twice as tall as wide). `custom_brush`: a custom brush is the brush,
/// so no built-in brush is shown selected.
fn snapshot(
    path: &str,
    scale: usize,
    doc: &Document,
    view: View,
    scroll: (i32, i32),
    ui: &UiState,
    custom_brush: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut screen = view.compose(doc.page(), scroll, doc.palette(), ui);
    if custom_brush && ui.toolbox_visible {
        let pens = Pens::from_palette(doc.palette());
        let x0 = view.ui_screen().toolbox_x();
        dpx_ui::toolbox::draw_brush_unselected(&mut screen, x0, ui.brush, &pens, view.ux, view.uy);
    }
    let (sx, sy) = (scale * view.fx as usize, scale * view.fy as usize);
    let rgba = to_rgba(&screen, doc.palette(), sx, sy);
    write_png(
        path,
        screen.width() * sx as u32,
        screen.height() * sy as u32,
        &rgba,
    )
}

fn font_preview(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    const LINES: [&str; 4] = [
        "0123456789  1 4 14 41 2024",
        "DeluxePainter  Color",
        "Picture Brush Mode Anim Color Effect Prefs",
        "The quick brown fox jumps over the lazy dog",
    ];
    let font = dpx_ui::font::Font::builtin();
    let w = LINES.iter().map(|l| font.text_width(l)).max().unwrap_or(0) + 8;
    let h = LINES.len() as i32 * 10 + 6;
    let mut img = IndexedImage::new(w as u32, h as u32, 0);
    for (i, line) in LINES.iter().enumerate() {
        font.draw(&mut img, 4, 4 + i as i32 * 10, line, 1);
    }
    let palette = Palette::from_rgb4(&[0xAAA, 0x000]).expect("2 colours");
    let rgba = to_rgba(&img, &palette, 3, 3);
    write_png(path, w as u32 * 3, h as u32 * 3, &rgba)
}

fn write_png(path: &str, w: u32, h: u32, rgba: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn app_icon() -> egui::IconData {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "../../../assets/app/icons/icon-256.png"
    )));
    let decoded = decoder.read_info().ok().and_then(|mut reader| {
        let mut buf = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        (info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight)
            .then_some((buf, info.width, info.height))
    });
    match decoded {
        Some((rgba, width, height)) => egui::IconData {
            rgba,
            width,
            height,
        },
        None => egui::IconData::default(),
    }
}

struct PainterApp {
    /// The document; every change goes through its command engine.
    engine: Engine,
    ui: UiState,
    texture: Option<egui::TextureHandle>,
    dirty: bool,
    /// The drawing-tool gesture in progress (ADR-14).
    stroke: Option<tools::Stroke>,
    /// Grid, Symmetry, Magnify/Zoom and other tool settings.
    tools: tools::ToolState,
    /// Picture ▸ Screen Format while it is open (shown on its own screen).
    requester: Option<ScreenFormatRequester>,
    /// Requester-screen pointer position while the left button is held.
    req_pointer: Option<(i32, i32)>,
    /// The file the picture was loaded from or last saved to.
    picture_path: Option<PathBuf>,
    /// File details kept from Load to Save (colour ranges, …).
    extras: files::PictureExtras,
    /// Drawer of the last Load/Save, where the file requester opens next time.
    last_drawer: Option<String>,
    /// Mouse-wheel movement not yet turned into whole list rows.
    wheel: f32,
    /// Top-left page pixel shown at the top-left of the screen (pages may be larger).
    scroll: (i32, i32),
    /// Wheel/trackpad movement not yet turned into whole page pixels.
    page_wheel: egui::Vec2,
    /// Page pixel under the pointer (also under the toolbox), for the `n` key.
    pointer_page: Option<(i32, i32)>,
    /// Picture ▸ Show Page while it is shown.
    show_page: Option<ShowPage>,
    /// Spare page, fixed background, colour cycling, palette to restore (ADR-15).
    menu_data: menu_actions::MenuData,
    /// The window as of the last frame; `None` before the first one (and in tests).
    window: Option<Window>,
    /// The window is full screen or maximized: it cannot snap, so the screen is centred at
    /// the largest pixel size that fits.
    free: bool,
    /// The monitor's size in points, if known.
    monitor: Option<egui::Vec2>,
    snap: Snap,
    /// The custom brush (Brush Selector); `None` paints with the toolbox's built-in brush.
    custom_brush: Option<dpx_core::BrushImage>,
    /// The brush as picked up, for Brush ▸ Restore after transforms.
    picked_brush: Option<dpx_core::BrushImage>,
    /// Effect ▸ Stencil: `stencil[c]` protects colour `c` from painting; `None` = off.
    stencil: Option<Vec<bool>>,
    /// Light Table and playback (ADR-28, anim.rs).
    anim: anim::AnimState,
    /// The settings file (ADR-33); `None` in tests, snapshots and with `--no-settings`.
    config: Option<config::ConfigStore>,
}

impl PainterApp {
    fn new() -> Self {
        let (w, h) = DisplayMode::PAL_LORES.size();
        let doc = Document::new(w, h, Palette::dpaint_default_32());
        let extras = files::PictureExtras::for_new_picture(doc.palette());
        Self {
            engine: Engine::new(doc),
            ui: UiState::default(),
            texture: None,
            dirty: true,
            stroke: None,
            tools: tools::ToolState::default(),
            requester: None,
            req_pointer: None,
            picture_path: None,
            extras,
            last_drawer: None,
            wheel: 0.0,
            scroll: (0, 0),
            page_wheel: egui::Vec2::ZERO,
            pointer_page: None,
            show_page: None,
            menu_data: menu_actions::MenuData::default(),
            window: None,
            free: false,
            monitor: None,
            snap: Snap::default(),
            custom_brush: None,
            picked_brush: None,
            stencil: None,
            anim: anim::AnimState::default(),
            config: None,
        }
    }

    /// Replaces the document with a loaded picture (not undoable, like DeluxePaint's Load).
    fn set_picture(&mut self, doc: Document, extras: files::PictureExtras, path: PathBuf) {
        self.menu_data.loaded_palette = Some(doc.palette().clone());
        self.stroke = None;
        self.anim.playing = None;
        self.engine = Engine::new(doc);
        self.extras = extras;
        self.last_drawer = path.parent().map(|p| p.to_string_lossy().into_owned());
        self.picture_path = Some(path);
        self.clamp_colors();
        self.scroll = (0, 0);
        self.dirty = true;
    }

    /// Keeps the shown part inside the page (after loads, page and format changes, undo).
    fn clamp_scroll(&mut self) {
        let (mx, my) = self.view().max_scroll(self.engine.document().page());
        let clamped = (self.scroll.0.clamp(0, mx), self.scroll.1.clamp(0, my));
        if clamped != self.scroll {
            self.scroll = clamped;
            self.dirty = true;
        }
    }

    fn scroll_to(&mut self, x: i32, y: i32) {
        let before = self.scroll;
        self.scroll = (x, y);
        self.clamp_scroll();
        self.dirty |= self.scroll != before;
    }

    /// The `n` key: centres the page pixel under the pointer as far as the page allows.
    fn centre_on(&mut self, (x, y): (i32, i32)) {
        let v = self.view();
        self.scroll_to(x - v.screen_w / 2, y - v.screen_h / 2);
    }

    fn open_page_size(&mut self) {
        self.finish_stroke();
        self.ui.page_size_requester = Some(page_size_requester(self.engine.document()));
        self.dirty = true;
    }

    /// Feeds one event to the Page Size requester; Ok sets the page size (undoable) with the
    /// BG colour in the new area.
    fn page_size_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.page_size_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        match response {
            PageSizeResponse::Continue => {}
            PageSizeResponse::Cancel => self.ui.page_size_requester = None,
            PageSizeResponse::Ok { width, height } => {
                let doc = self.engine.document();
                if (width, height) == (doc.page().width(), doc.page().height()) {
                    self.ui.page_size_requester = None;
                    return;
                }
                let ((min_w, min_h), (max_w, max_h)) = doc.target().page_limits();
                let command = Command::SetPageSize {
                    width,
                    height,
                    fill: self.ui.bg,
                };
                match self.engine.execute(command) {
                    Ok(_) => {
                        self.ui.page_size_requester = None;
                        self.clamp_scroll();
                    }
                    Err(e) => {
                        eprintln!("{e}");
                        if let Some(req) = &mut self.ui.page_size_requester {
                            req.set_message(format!("Size {min_w}x{min_h} to {max_w}x{max_h}"));
                        }
                    }
                }
            }
        }
    }

    fn open_show_page(&mut self) {
        self.finish_stroke();
        let view = self.view();
        let page = self.engine.document().page();
        self.show_page = Some(ShowPage::new(
            (page.width(), page.height()),
            (view.screen_w as u32, view.screen_h as u32),
            self.scroll,
        ));
        self.dirty = true;
    }

    fn show_page_event(&mut self, event: ReqEvent, alt: bool) {
        let Some(sp) = &mut self.show_page else {
            return;
        };
        let response = sp.handle(event, alt);
        self.dirty = true;
        if let ShowPageResponse::Done(view) = response {
            self.show_page = None;
            if let Some((x, y)) = view {
                self.scroll_to(x, y);
            }
        }
    }

    /// Keeps the FG/BG colours inside the palette.
    fn clamp_colors(&mut self) {
        let max = (self.palette().len() - 1) as u8;
        self.ui.fg = self.ui.fg.min(max);
        self.ui.bg = self.ui.bg.min(max);
    }

    fn open_file_requester(&mut self, kind: FileKind) {
        self.finish_stroke();
        let mut req = FileRequester::new(kind);
        let suggested = match (kind, &self.picture_path) {
            (FileKind::SavePicture, Some(p)) => project_path(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned()),
            (FileKind::ExportPicture, Some(p)) => export_name(p),
            (FileKind::SaveAnim, Some(p)) => anim_path(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned()),
            _ => None,
        };
        if let Some(name) = suggested {
            req.set_file(&name);
        }
        let start = self.last_drawer.clone().unwrap_or_else(|| ".".into());
        Self::browse(&mut req, Browse::Drawer(start));
        if req.drawer().is_empty() {
            // The last drawer is gone: fall back to the start folder.
            Self::browse(&mut req, Browse::Drawer(".".into()));
        }
        self.ui.file_requester = Some(req);
        self.dirty = true;
    }

    /// Lists what the file requester asked for.
    fn browse(req: &mut FileRequester, what: Browse) {
        let listing = match what {
            Browse::Drawer(d) => browse::list_drawer(&d),
            Browse::Parent => match browse::parent(req.drawer()) {
                Some(p) => browse::list_drawer(&p),
                None => return,
            },
            Browse::Disks => Ok((String::new(), browse::disks())),
            Browse::Assigns => Ok((String::new(), browse::assigns())),
        };
        match listing {
            Ok((drawer, entries)) => req.set_listing(&drawer, entries),
            Err(e) => req.set_message(e),
        }
    }

    /// Feeds one event to the open file requester and acts on its answer.
    fn file_requester_event(&mut self, event: ReqEvent, now_ms: u64) {
        let Some(req) = &mut self.ui.file_requester else {
            return;
        };
        let kind = req.kind();
        match req.handle(event, now_ms) {
            FileResponse::Continue => {}
            FileResponse::Cancel => self.ui.file_requester = None,
            FileResponse::Browse(what) => Self::browse(req, what),
            FileResponse::Accept {
                drawer,
                file,
                overwrite,
            } => {
                let path = browse::expand_home(&drawer).join(file);
                match kind {
                    FileKind::LoadPicture | FileKind::LoadAnim => self.load_picture(path),
                    FileKind::SaveAnim => self.save_picture(anim_path(&path), overwrite, true),
                    FileKind::SavePicture => {
                        self.save_picture(project_path(&path), overwrite, false)
                    }
                    FileKind::ExportPicture => self.save_picture(path, overwrite, true),
                    FileKind::LoadBrush | FileKind::SaveBrush => {
                        self.brush_file_accept(kind, &path, overwrite)
                    }
                    other => self.menu_file_accept(other, &path, overwrite),
                }
            }
        }
        self.dirty = true;
    }

    fn requester_message(&mut self, text: String) {
        if let Some(req) = &mut self.ui.file_requester {
            req.set_message(text);
        }
    }

    fn load_picture(&mut self, path: PathBuf) {
        match files::load(&path) {
            Ok(loaded) => {
                let current = self.engine.document().target();
                let (doc, extras, notes) = files::open_picture(loaded.picture, current);
                let (doc, grown) = files::page_at_least_screen(doc);
                let notes: Vec<String> =
                    loaded.notes.into_iter().chain(notes).chain(grown).collect();
                for n in &notes {
                    eprintln!("note: {}: {n}", path.display());
                }
                self.set_picture(doc, extras, path);
                self.ui.file_requester = None;
                self.ui.note_requester = note_requester(LOADED, &notes);
            }
            Err(e) => {
                eprintln!("{e}");
                // The requester shows the short first line; the details went to stderr.
                let prefix = format!("{}: ", path.display());
                let text = e.strip_prefix(&prefix).unwrap_or(&e);
                self.requester_message(text.lines().next().unwrap_or("").to_string());
            }
        }
    }

    /// Picture ▸ Save.. (the project, `export` false) and Export.. (IFF ILBM or PNG by name,
    /// `export` true; the picture keeps its name).
    fn save_picture(&mut self, path: PathBuf, overwrite: bool, export: bool) {
        if path.is_dir() {
            return self.requester_message("That is a drawer".into());
        }
        if export && matches!(files::save_format(&path), Ok(files::SaveFormat::Project)) {
            return self.requester_message("Use Save.. for projects".into());
        }
        if path.exists() && !overwrite {
            if let Some(req) = &mut self.ui.file_requester {
                req.ask_overwrite();
            }
            return;
        }
        let result = files::encode_document(self.engine.document(), &self.extras, &path)
            .and_then(|(bytes, notes)| files::write_atomic(&path, &bytes).map(|()| notes));
        match result {
            Ok(notes) => {
                for n in &notes {
                    eprintln!("note: {}: {n}", path.display());
                }
                self.last_drawer = path.parent().map(|p| p.to_string_lossy().into_owned());
                self.ui.file_requester = None;
                self.ui.note_requester = note_requester(
                    match (export, files::save_format(&path)) {
                        (_, Ok(files::SaveFormat::Anim)) => ANIM_SAVED,
                        (true, _) => EXPORTED,
                        _ => SAVED,
                    },
                    &notes,
                );
                if !export {
                    self.picture_path = Some(path);
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e);
            }
        }
    }

    /// The palette shown on screen: the document palette, or its cycled version while
    /// Color ▸ Cycle is on (ADR-15). Commands read `self.engine.document().palette()`.
    fn palette(&self) -> &Palette {
        self.menu_data
            .shown_palette(self.engine.document().palette())
    }

    /// The screen currently shown: the requester screen or the document's display mode.
    fn view(&self) -> View {
        self.layout().0
    }

    /// The screen currently shown and its scale (physical pixels per screen pixel, may be
    /// fractional). The screen is the picture's screen (or the Screen Format requester's own
    /// screen) at the chosen pixel size, or smaller while it does not fit the window (during a
    /// resize, full screen); it is centred in the window (ADR-13).
    fn layout(&self) -> (View, f32) {
        let view = if self.requester.is_some() {
            View::of(requester::SCREEN)
        } else {
            View::of_target(self.engine.document().target())
        };
        let Some(win) = self.window else {
            return (view, 1.0);
        };
        let fit = fit_step(win.points(), view.ui_size());
        let step = if self.free {
            fit
        } else {
            self.ui.pixel_size.0.min(fit)
        };
        let scale = f32::from(step) * win.ppp / 2.0 / (view.fx * view.ux) as f32;
        (view, scale)
    }

    /// The largest pixel size at which the picture's screen fits on the monitor.
    fn largest_step(&self) -> u8 {
        let ui = View::of_target(self.engine.document().target()).ui_size();
        self.monitor
            .map_or(PixelSize::MAX, |m| fit_step(m - MONITOR_MARGIN, ui))
    }

    /// Prefs ▸ Pixel Size and Cmd/Ctrl + `+`, `−`, `0`: the window snaps to the new size.
    fn set_pixel_size(&mut self, size: PixelSize) {
        let size = PixelSize(size.0.clamp(PixelSize::MIN, self.largest_step()));
        if self.ui.pixel_size != size {
            self.ui.pixel_size = size;
            self.clamp_scroll();
            self.dirty = true;
        }
    }

    /// Half a step bigger (`+1`) or smaller (`-1`).
    fn step_pixel_size(&mut self, step: i32) {
        let h = (i32::from(self.ui.pixel_size.0) + step).clamp(0, 255) as u8;
        self.set_pixel_size(PixelSize(h));
    }

    /// The window snaps to the picture's screen at the pixel size (ADR-13). `size` is the
    /// drawing area in points now; after the user resized it and stopped for
    /// [`SNAP_SETTLE`] seconds, the pixel size becomes the nearest step. Returns the size to
    /// ask the window for, if it differs. Full screen, maximized windows and the Screen Format
    /// requester do not snap.
    fn snap_window(&mut self, size: egui::Vec2, now: f64) -> Option<egui::Vec2> {
        let near = |a: egui::Vec2, b: egui::Vec2| (a - b).abs().max_elem() < 1.0;
        let changed = self.snap.last.is_some_and(|l| !near(l, size));
        self.snap.last = Some(size);
        if self.free || self.requester.is_some() {
            self.snap = Snap {
                last: Some(size),
                ..Snap::default()
            };
            return None;
        }
        if changed {
            if !self.snap.requested.is_some_and(|r| near(r, size)) {
                self.snap.resized_at = Some(now);
            }
            self.snap.requested = None;
        }
        let ui = View::of_target(self.engine.document().target()).ui_size();
        if let Some(t) = self.snap.resized_at {
            if now - t < SNAP_SETTLE {
                return None;
            }
            self.snap.resized_at = None;
            self.set_pixel_size(PixelSize(nearest_step(size, ui)));
        }
        let largest = self.largest_step();
        if self.ui.pixel_size.0 > largest {
            self.set_pixel_size(PixelSize(largest));
        }
        let target = window_size(ui, self.ui.pixel_size.0);
        if near(target, size) || self.snap.requested.is_some_and(|r| near(r, target)) {
            return None;
        }
        self.snap.requested = Some(target);
        Some(target)
    }

    fn open_screen_format(&mut self) {
        self.finish_stroke();
        let doc = self.engine.document();
        let page = (doc.page().width(), doc.page().height());
        self.requester = Some(ScreenFormatRequester::new(
            doc.target(),
            page,
            doc.palette().len(),
        ));
        self.dirty = true;
    }

    /// Feeds one event to the open requester and acts on its answer.
    fn requester_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.requester else {
            return;
        };
        match req.handle(event) {
            ReqResponse::Continue => {}
            ReqResponse::Cancel => self.requester = None,
            ReqResponse::Use(command) => match self.engine.execute(command) {
                Ok(_) => {
                    self.requester = None;
                    // Colours beyond the new palette would be invalid.
                    self.clamp_colors();
                    self.clamp_scroll();
                }
                Err(e) => req.set_message(e.message),
            },
        }
        self.dirty = true;
    }

    /// The page as shown: while the screen is composed the Light Table picture or the playing
    /// frame (ADR-28), else the stroke preview while painting, else the document page.
    fn page(&self) -> &IndexedImage {
        if let Some(shown) = &self.anim.shown {
            return shown;
        }
        self.stroke
            .as_ref()
            .map_or(self.engine.document().page(), |s| &s.preview)
    }

    /// Executes a command; a pending stroke is committed first so the order stays intact.
    fn run(&mut self, command: Command) {
        self.finish_stroke();
        let command = self.with_fixed_background(command);
        self.execute(command);
    }

    fn execute(&mut self, command: Command) {
        if let Err(e) = self.engine.execute(command) {
            eprintln!("command rejected: {e}");
        }
        self.dirty = true;
    }

    fn undo(&mut self) {
        self.finish_stroke();
        Self::report(self.engine.undo(), ErrorCode::NothingToUndo);
        self.clamp_colors();
        self.clamp_scroll();
        self.dirty = true;
    }

    fn redo(&mut self) {
        self.finish_stroke();
        Self::report(self.engine.redo(), ErrorCode::NothingToRedo);
        self.clamp_colors();
        self.clamp_scroll();
        self.dirty = true;
    }

    /// Logs unexpected engine errors; an empty undo/redo list is normal.
    fn report<T>(result: Result<T, dpx_core::CommandError>, expected: ErrorCode) {
        if let Err(e) = result
            && e.code != expected
        {
            eprintln!("{e}");
        }
    }

    fn handle_press(&mut self, hit: Hit, right: bool) {
        match hit {
            // DeluxePaint has one undo level; the right button on UNDO is our redo.
            Hit::Tool {
                tool: Tool::Undo, ..
            } if right => self.redo(),
            // Right-click on Zoom shrinks the magnification.
            Hit::Tool {
                tool: Tool::Zoom, ..
            } if right => self.zoom(-1),
            // Right-click on the Brush Selector: last custom brush on/off (ADR-18).
            Hit::Tool {
                tool: Tool::BrushSelect,
                ..
            } if right => self.toggle_custom_brush(),
            // Right-click opens the tool's settings requester (ADR-20); the tool is selected
            // like with a left click, except Grid and Symmetry, which stay as they are.
            Hit::Tool { tool, filled_half } if right && settings::requester_for(tool).is_some() => {
                if !matches!(tool, Tool::Grid | Tool::Symmetry) {
                    self.select_tool(tool, filled_half);
                }
                self.open_tool_requester(tool);
            }
            Hit::Tool { tool, filled_half } => self.select_tool(tool, filled_half),
            Hit::Brush(i) => self.select_builtin_brush(i),
            Hit::PaletteCell(i) => {
                if right {
                    self.ui.bg = i;
                } else {
                    self.ui.fg = i;
                }
                self.dirty = true;
            }
            // Stencil ▸ Show disables painting meanwhile (ADR-31).
            Hit::Page { .. } if self.menu_data.effects.stencil_show => {}
            Hit::Page { x, y } => self.press_page(x, y, right),
            Hit::TitleBar | Hit::Indicator | Hit::ToolboxBackground => {}
        }
    }

    /// Runs a menu action. Commands not implemented yet do nothing.
    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Mode(m) => self.ui.mode = m,
            Action::Pref(p) => self.ui.prefs.toggle(p),
            Action::PixelSize(p) => self.set_pixel_size(p),
            Action::Command(id) => match id {
                "color.ranges" => self.open_ranges(),
                "palette.mixer" => {
                    self.finish_stroke();
                    let doc = self.engine.document();
                    self.ui.palette_requester = Some(dpx_ui::PaletteRequester::new(
                        doc.palette(),
                        self.ui.fg,
                        doc.target().chipset,
                    ));
                }
                "picture.flip_horizontal" => self.run(Command::Flip {
                    axis: Axis::Horizontal,
                }),
                "picture.flip_vertical" => self.run(Command::Flip {
                    axis: Axis::Vertical,
                }),
                // DeluxePaint cannot undo these two; DeluxePainter keeps them undoable.
                "color.bg_to_fg" => self.run(Command::ReplaceColor {
                    from: self.ui.bg,
                    to: self.ui.fg,
                }),
                "color.swap_bg_fg" => self.run(Command::SwapColors {
                    a: self.ui.bg,
                    b: self.ui.fg,
                }),
                "palette.default" if self.palette().len() == 32 => self.run(Command::SetPalette {
                    colors: Palette::dpaint_default_32().colors().to_vec(),
                }),
                "picture.screen_format" => self.open_screen_format(),
                // ADR-20 (settings.rs).
                "stencil.make" | "stencil.remake" | "stencil.reverse" | "stencil.toggle"
                | "stencil.free" => self.stencil_command(id),
                "view.largest_pixel_size" => self.set_pixel_size(PixelSize(self.largest_step())),
                "picture.page_size" => self.open_page_size(),
                "picture.show_page" => self.open_show_page(),
                "picture.load" => self.open_file_requester(FileKind::LoadPicture),
                "picture.save" => self.open_file_requester(FileKind::SavePicture),
                "picture.export" => self.open_file_requester(FileKind::ExportPicture),
                "app.quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                // ADR-15 (menu_actions.rs).
                "spare.swap" => self.swap_spare(),
                "spare.copy" => self.copy_to_spare(),
                "spare.merge_front" => self.merge_spare(true),
                "spare.merge_back" => self.merge_spare(false),
                "spare.delete" => self.delete_page(),
                "picture.delete" => self.open_file_requester(FileKind::DeleteFile),
                "app.about" => self.show_about(),
                "color.cycle" => self.toggle_cycling(ctx.input(|i| i.time)),
                "palette.restore" => self.restore_palette(),
                "palette.load" => self.open_file_requester(FileKind::LoadPalette),
                "palette.save" => self.open_file_requester(FileKind::SavePalette),
                "background.fix" => self.fix_background(),
                "background.free" => self.free_background(),
                // ADR-28 (anim.rs).
                id if id.starts_with("anim.") || id.starts_with("light_table.") => {
                    self.anim_action(id, ctx.input(|i| i.time));
                    ctx.request_repaint();
                }
                // ADR-31 (effects.rs).
                "palette.arrange"
                | "color.remap"
                | "color_set.load"
                | "color_set.save"
                | "stencil.show"
                | "stencil.load"
                | "stencil.save"
                | "antialias.none"
                | "antialias.low"
                | "antialias.high"
                | "process.toggle"
                | "process.tint"
                | "process.hue"
                | "process.value"
                | "translucency.toggle"
                | "translucency.settings" => self.effect_action(id),
                // ADR-19 (brush_menu.rs).
                id if id.starts_with("brush.") => self.brush_action(id),
                _ => {}
            },
        }
        self.dirty = true;
    }
}

impl PainterApp {
    /// Writes the settings file once they have stopped changing (ADR-33).
    fn persist_settings(&mut self, ctx: &egui::Context, now: f64) {
        let Some(mut store) = self.config.take() else {
            return;
        };
        if let Some(wait) = store.update(config::Config::capture(self), now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait + 0.05));
        }
        self.config = Some(store);
    }
}

impl eframe::App for PainterApp {
    fn on_exit(&mut self) {
        if let Some(mut store) = self.config.take() {
            store.flush(config::Config::capture(self));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ui.has_brush = self.custom_brush.is_some();
        self.ui.can_restore = self.picked_brush.is_some();
        self.refresh_effects();
        let ppp = ctx.pixels_per_point();
        let area = ctx.content_rect();
        let window = Window {
            w: (area.width() * ppp).floor() as i32,
            h: (area.height() * ppp).floor() as i32,
            ppp,
        };
        if self.window != Some(window) {
            self.window = Some(window);
            self.dirty = true;
        }
        let (free, monitor, now) = ctx.input(|i| {
            let v = i.viewport();
            (
                v.fullscreen.unwrap_or(false) || v.maximized.unwrap_or(false),
                v.monitor_size,
                i.time,
            )
        });
        self.free = free;
        self.monitor = monitor;
        if let Some(size) = self.snap_window(window.points(), now) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        if self.snap.resized_at.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.clamp_scroll();
        self.persist_settings(&ctx, now);

        // The screen at its pixel size (whole and half steps, ADR-13), centred; each screen
        // pixel is shown fx×fy times `scale` physical pixels (its shape in this mode). When
        // the window has snapped, the screen fills it exactly; otherwise the rest is dark.
        let (view, scale) = self.layout();
        let img_w = ((view.screen_w * view.fx) as f32 * scale).round() as i32;
        let img_h = ((view.screen_h * view.fy) as f32 * scale).round() as i32;
        let origin_px = egui::vec2(
            ((window.w - img_w) / 2) as f32,
            ((window.h - img_h) / 2) as f32,
        );
        let origin = area.min + origin_px / ppp;
        let rect = egui::Rect::from_min_size(origin, egui::vec2(img_w as f32, img_h as f32) / ppp);

        if self.requester.is_some() {
            self.requester_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.ui.tool_requester.is_some() {
            self.tool_requester_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        // ADR-28: playback and the Anim requesters are modal.
        if self.anim.playing.is_some() {
            self.playback_input(&ctx);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.ui.anim_requester.is_some() {
            self.anim_requester_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.palette_input(&ctx, origin, ppp, scale, view) {
            self.palette_upload(&ctx);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.ui.range_requester.is_some() {
            self.range_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.ui.note_requester.is_some() {
            self.note_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.ui.file_requester.is_some() {
            self.file_requester_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.ui.page_size_requester.is_some() {
            self.page_size_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.show_page.is_some() {
            self.show_page_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }

        // Input.
        let (hover, press_l, press_r, down_l, down_r, shift) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.secondary_pressed(),
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.modifiers.shift,
            )
        });
        self.tools.shift = shift;
        let to_screen = |p: egui::Pos2| -> (i32, i32) {
            let v = (p - origin) * ppp;
            (
                (v.x / (scale * view.fx as f32)).floor() as i32,
                (v.y / (scale * view.fy as f32)).floor() as i32,
            )
        };
        // Menus and the toolbox live in UI (LoRes) coordinates.
        let ui_screen = view.ui_screen();
        let (sw, sh) = (ui_screen.width, ui_screen.height);
        if self.ui.menu.active {
            // Menu strip held with the right button: track, multi-select with the left button,
            // select on release.
            if let Some(pos) = hover {
                let (sx, sy) = view.ui_pos(to_screen(pos));
                if self.ui.menu.pointer(sw, sh, sx, sy) {
                    self.dirty = true;
                }
            }
            let menu_ctx = self.ui.menu_context();
            if press_l && let Some(a) = self.ui.menu.hovered_action(&menu_ctx) {
                self.run_action(a, &ctx);
            }
            if !down_r {
                let action = self.ui.menu.hovered_action(&menu_ctx);
                self.ui.menu.close();
                self.dirty = true;
                if let Some(a) = action {
                    self.run_action(a, &ctx);
                }
            }
        } else if let Some(pos) = hover {
            let (sx, sy) = to_screen(pos);
            let (ux, uy) = view.ui_pos((sx, sy));
            // Screen pixels are page pixels, shifted by the scroll position (or magnified,
            // ADR-14; `None` on the Magnify divider). The screen may be larger than the page
            // (ADR-13): there the pointer is off the page.
            let on = self.screen_to_page(view, (sx, sy));
            if on.is_some() {
                self.pointer_page = on;
            }
            let inside = on.filter(|&(px, py)| self.page().bounds().contains(px, py));
            let hit = match ui_screen.hit_test(self.palette().len(), &self.ui, ux, uy) {
                Hit::Page { .. } => inside.map(|(x, y)| Hit::Page { x, y }),
                h => Some(h),
            };
            let on_page = match hit {
                Some(Hit::Page { x, y }) => Some((x, y)),
                _ => None,
            };
            if on_page.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if self.ui.coords != on_page {
                self.ui.coords = on_page;
                self.dirty |= self.ui.prefs.get(dpx_ui::Pref::Coords);
            }
            if press_r && hit == Some(Hit::TitleBar) {
                self.ui.menu.open(sw, sh, ux, uy);
                self.dirty = true;
            } else if press_l || press_r {
                if let Some(hit) = hit {
                    self.handle_press(hit, press_r);
                }
            } else if (down_l || down_r) && rect.contains(pos) {
                if let Some((px, py)) = on {
                    self.handle_drag(px, py);
                }
            } else if let Some(Hit::Page { x, y }) = hit {
                self.handle_hover(x, y);
            } else if !(down_l || down_r) {
                self.hover_brush(None);
            }
        } else {
            self.hover_brush(None);
        }
        if !(down_l || down_r) {
            self.release();
        }
        self.tick(&ctx);
        if self.ui.menu.active {
            // Keyboard shortcuts are ignored while the menu strip is shown.
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        // Tool keys first: while typing text or magnifying they own the keyboard.
        let keys_taken = self.tool_keys(&ctx);
        if !keys_taken {
            self.page_scroll_input(&ctx, ppp, scale, view);
        }
        if self.show_page.is_some() {
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        ctx.input(|i| {
            if i.key_pressed(egui::Key::F10) {
                let show = !(self.ui.toolbox_visible || self.ui.title_visible);
                self.ui.toolbox_visible = show;
                self.ui.title_visible = show;
                self.dirty = true;
            }
            if i.key_pressed(egui::Key::F9) {
                self.ui.title_visible = !self.ui.title_visible;
                self.dirty = true;
            }
            const MODE_KEYS: [(egui::Key, PaintMode); 8] = [
                (egui::Key::F1, PaintMode::Matte),
                (egui::Key::F2, PaintMode::Color),
                (egui::Key::F3, PaintMode::Replace),
                (egui::Key::F4, PaintMode::Smear),
                (egui::Key::F5, PaintMode::Shade),
                (egui::Key::F6, PaintMode::Blend),
                (egui::Key::F7, PaintMode::Cycle),
                (egui::Key::F8, PaintMode::Smooth),
            ];
            for (key, mode) in MODE_KEYS {
                if i.key_pressed(key) {
                    self.ui.mode = mode;
                    self.dirty = true;
                }
            }
        });
        self.window_keys(&ctx);
        self.menu_keys(&ctx);
        self.anim_keys(&ctx);
        // U = undo (as in DeluxePaint), Shift+U = redo (DeluxePainter addition).
        let (undo_key, shift) = ctx.input(|i| (i.key_pressed(egui::Key::U), i.modifiers.shift));
        if undo_key && !self.typing() {
            if shift {
                self.redo();
            } else {
                self.undo();
            }
        }
        self.upload_and_paint(ui, &ctx, rect, area);
    }
}

impl PainterApp {
    /// DeluxePainter window keys (ADR-13): Cmd/Ctrl + `+` / `−` change the pixel size by half a
    /// step, Cmd/Ctrl + `0` picks the largest that fits on the monitor; Ctrl+Cmd+F (macOS) or
    /// F11 switch full screen.
    fn window_keys(&mut self, ctx: &egui::Context) {
        let (bigger, smaller, auto, full) = ctx.input(|i| {
            let cmd = i.modifiers.command && !i.modifiers.alt;
            let mac_full = i.modifiers.mac_cmd && i.modifiers.ctrl && i.key_pressed(egui::Key::F);
            (
                cmd && (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)),
                cmd && i.key_pressed(egui::Key::Minus),
                cmd && i.key_pressed(egui::Key::Num0),
                mac_full || i.key_pressed(egui::Key::F11),
            )
        });
        if bigger {
            self.step_pixel_size(1);
        }
        if smaller {
            self.step_pixel_size(-1);
        }
        if auto {
            self.set_pixel_size(PixelSize(self.largest_step()));
        }
        if full {
            let now = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!now));
        }
    }

    /// Pointer and keys while the Screen Format requester is open (left button only).
    fn requester_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let (hover, press, down) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
            )
        });
        let pos = hover.map(|p| {
            let v = (p - origin) * ppp;
            (
                (v.x / (scale * view.fx as f32)).floor() as i32,
                (v.y / (scale * view.fy as f32)).floor() as i32,
            )
        });
        if press && let Some(p) = pos {
            self.req_pointer = Some(p);
            self.requester_event(ReqEvent::Press(p.0, p.1));
        } else if down && let Some(p) = pos {
            if self.req_pointer != Some(p) {
                self.req_pointer = Some(p);
                self.requester_event(ReqEvent::Drag(p.0, p.1));
            }
        } else if !down && self.req_pointer.take().is_some() {
            self.requester_event(ReqEvent::Release);
        }
        // Letters (U, C, R shortcuts) and digits (custom screen size) arrive as text.
        let keys: Vec<ReqKey> = ctx.input(|i| {
            let mut keys = Vec::new();
            for event in &i.events {
                match event {
                    egui::Event::Text(text) => keys.extend(
                        text.chars()
                            .filter(char::is_ascii_alphanumeric)
                            .map(|c| ReqKey::Char(c.to_ascii_lowercase())),
                    ),
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => keys.extend(match key {
                        egui::Key::ArrowUp => Some(ReqKey::Up),
                        egui::Key::ArrowDown => Some(ReqKey::Down),
                        egui::Key::ArrowLeft => Some(ReqKey::Left),
                        egui::Key::ArrowRight => Some(ReqKey::Right),
                        egui::Key::Home => Some(ReqKey::Home),
                        egui::Key::End => Some(ReqKey::End),
                        egui::Key::Backspace => Some(ReqKey::Backspace),
                        egui::Key::Delete => Some(ReqKey::Delete),
                        egui::Key::Tab => Some(ReqKey::Tab),
                        egui::Key::Enter => Some(ReqKey::Enter),
                        egui::Key::Escape => Some(ReqKey::Escape),
                        _ => None,
                    }),
                    _ => {}
                }
            }
            keys
        });
        for k in keys {
            self.requester_event(ReqEvent::Key(k));
        }
    }

    /// Arrow keys (Alt: by a screen), `n`, Shift+S and the wheel/trackpad on the paint screen.
    fn page_scroll_input(&mut self, ctx: &egui::Context, ppp: f32, scale: f32, view: View) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let step = if mods.alt {
            (view.screen_w, view.screen_h)
        } else {
            (SCROLL_STEP * view.ux, SCROLL_STEP * view.uy)
        };
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } => {
                    let (dx, dy) = match key {
                        egui::Key::ArrowLeft => (-step.0, 0),
                        egui::Key::ArrowRight => (step.0, 0),
                        egui::Key::ArrowUp => (0, -step.1),
                        egui::Key::ArrowDown => (0, step.1),
                        egui::Key::N if !repeat && !modifiers.any() => {
                            if let Some(p) = self.pointer_page {
                                self.centre_on(p);
                            }
                            continue;
                        }
                        egui::Key::S
                            if !repeat
                                && modifiers.shift
                                && !(modifiers.ctrl || modifiers.alt || modifiers.command) =>
                        {
                            self.open_show_page();
                            return;
                        }
                        _ => continue,
                    };
                    self.scroll_to(self.scroll.0 + dx, self.scroll.1 + dy);
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    // DeluxePainter addition: wheel and trackpad scroll the page. Positive
                    // deltas move the content down/right, i.e. show the part above/left.
                    let px = match unit {
                        egui::MouseWheelUnit::Point => egui::vec2(
                            delta.x * ppp / (scale * view.fx as f32),
                            delta.y * ppp / (scale * view.fy as f32),
                        ),
                        egui::MouseWheelUnit::Line => egui::vec2(
                            delta.x * (SCROLL_STEP * view.ux) as f32,
                            delta.y * (SCROLL_STEP * view.uy) as f32,
                        ),
                        egui::MouseWheelUnit::Page => egui::vec2(
                            delta.x * view.screen_w as f32,
                            delta.y * view.screen_h as f32,
                        ),
                    };
                    self.page_wheel -= px;
                    let (dx, dy) = (self.page_wheel.x.trunc(), self.page_wheel.y.trunc());
                    if dx != 0.0 || dy != 0.0 {
                        self.page_wheel -= egui::vec2(dx, dy);
                        self.scroll_to(self.scroll.0 + dx as i32, self.scroll.1 + dy as i32);
                    }
                }
                _ => {}
            }
        }
    }

    /// Pointer and keys in Show Page (screen pixels; any key but the arrows returns).
    fn show_page_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let (hover, press, down, events) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.events.clone(),
            )
        });
        let pos = hover.map(|p| {
            let v = (p - origin) * ppp;
            (
                (v.x / (scale * view.fx as f32)).floor() as i32,
                (v.y / (scale * view.fy as f32)).floor() as i32,
            )
        });
        if press && let Some(p) = pos {
            self.req_pointer = Some(p);
            self.show_page_event(ReqEvent::Press(p.0, p.1), false);
        } else if down && let Some(p) = pos {
            if self.req_pointer != Some(p) {
                self.req_pointer = Some(p);
                self.show_page_event(ReqEvent::Drag(p.0, p.1), false);
            }
        } else if !down && self.req_pointer.take().is_some() {
            self.show_page_event(ReqEvent::Release, false);
        }
        for event in events {
            if self.show_page.is_none() {
                break;
            }
            if let egui::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = event
            {
                let k = match key {
                    egui::Key::ArrowLeft => ReqKey::Left,
                    egui::Key::ArrowRight => ReqKey::Right,
                    egui::Key::ArrowUp => ReqKey::Up,
                    egui::Key::ArrowDown => ReqKey::Down,
                    _ if repeat => continue,
                    egui::Key::Enter => ReqKey::Enter,
                    _ => ReqKey::Escape,
                };
                self.show_page_event(ReqEvent::Key(k), modifiers.alt);
            }
        }
    }

    /// Pointer, keys and wheel for a modal requester drawn on the UI layer with its top-left
    /// at `at` (UI pixels); pointer positions come out requester-local. Also returns the time.
    fn modal_events(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
        at: (i32, i32),
    ) -> (Vec<ReqEvent>, u64) {
        let (rx, ry) = at;
        let mut out = Vec::new();
        let (hover, press, down, time, events) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.time,
                i.events.clone(),
            )
        });
        let now_ms = (time * 1000.0) as u64;
        // Requester coordinates: UI (LoRes) pixels relative to its top-left corner.
        let pos = hover.map(|p| {
            let v = (p - origin) * ppp;
            let screen = (
                (v.x / (scale * view.fx as f32)).floor() as i32,
                (v.y / (scale * view.fy as f32)).floor() as i32,
            );
            let (ux, uy) = view.ui_pos(screen);
            (ux - rx, uy - ry)
        });
        if press && let Some(p) = pos {
            self.req_pointer = Some(p);
            out.push(ReqEvent::Press(p.0, p.1));
        } else if down && let Some(p) = pos {
            if self.req_pointer != Some(p) {
                self.req_pointer = Some(p);
                out.push(ReqEvent::Drag(p.0, p.1));
            }
        } else if !down && self.req_pointer.take().is_some() {
            out.push(ReqEvent::Release);
        }
        for event in events {
            match event {
                egui::Event::Text(text) => {
                    out.extend(text.chars().map(|c| ReqEvent::Key(ReqKey::Char(c))));
                }
                egui::Event::Key {
                    key, pressed: true, ..
                } => {
                    let k = match key {
                        egui::Key::ArrowUp => ReqKey::Up,
                        egui::Key::ArrowDown => ReqKey::Down,
                        egui::Key::ArrowLeft => ReqKey::Left,
                        egui::Key::ArrowRight => ReqKey::Right,
                        egui::Key::Home => ReqKey::Home,
                        egui::Key::End => ReqKey::End,
                        egui::Key::Backspace => ReqKey::Backspace,
                        egui::Key::Delete => ReqKey::Delete,
                        egui::Key::Tab => ReqKey::Tab,
                        egui::Key::Enter => ReqKey::Enter,
                        egui::Key::Escape => ReqKey::Escape,
                        _ => continue,
                    };
                    out.push(ReqEvent::Key(k));
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    // Positive y moves the content down, i.e. scrolls the list up.
                    self.wheel -= match unit {
                        egui::MouseWheelUnit::Point => delta.y / 20.0,
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 8.0,
                    };
                    let rows = self.wheel.trunc();
                    if rows != 0.0 {
                        self.wheel -= rows;
                        out.push(ReqEvent::Scroll(rows as i32));
                    }
                }
                _ => {}
            }
        }
        (out, now_ms)
    }

    /// Pointer, wheel and keys while the file requester is open (it is modal).
    fn file_requester_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let ui_screen = view.ui_screen();
        let at = FileRequester::origin(ui_screen.width, ui_screen.height);
        let (events, now_ms) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.file_requester.is_none() {
                break;
            }
            self.file_requester_event(event, now_ms);
        }
    }

    /// The Color Mixer (Color ▸ Palette ▸ Mixer, ADR-16). While it is closed, the `p` key
    /// opens it when no other requester, view or menu is open; while it is open, it gets the
    /// pointer and keys (it is modal). Returns whether it is or was open this frame, so the
    /// caller only paints.
    fn palette_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) -> bool {
        if self.ui.palette_requester.is_none() {
            let busy = self.ui.note_requester.is_some()
                || self.ui.file_requester.is_some()
                || self.ui.page_size_requester.is_some()
                || self.show_page.is_some()
                || self.ui.menu.active;
            let p = ctx.input(|i| i.key_pressed(egui::Key::P) && i.modifiers.is_none());
            if busy || !p {
                return false;
            }
            // This frame's `p` must not reach the requester (it would close it again).
            self.run_action(Action::Command("palette.mixer"), ctx);
            return true;
        }
        let ui_screen = view.ui_screen();
        let at = dpx_ui::PaletteRequester::origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.palette_requester.is_none() {
                break;
            }
            self.palette_event(event);
        }
        true
    }

    /// Feeds one event to the Color Mixer; Ok sets the edited palette as one undoable
    /// command and makes the selected colour the FG colour, Cancel drops the edits.
    fn palette_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.palette_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        match response {
            dpx_ui::PaletteResponse::Continue => {}
            dpx_ui::PaletteResponse::Cancel => self.ui.palette_requester = None,
            dpx_ui::PaletteResponse::Ok => {
                let Some(req) = self.ui.palette_requester.take() else {
                    return;
                };
                self.ui.fg = req.selected();
                if req.colors() != self.palette().colors() {
                    self.run(Command::SetPalette {
                        colors: req.colors().to_vec(),
                    });
                }
            }
        }
    }

    /// While the Color Mixer is open, uploads the screen in the palette being edited, so
    /// every change shows at once (the document palette changes only on Ok).
    fn palette_upload(&mut self, ctx: &egui::Context) {
        let Some(req) = &self.ui.palette_requester else {
            return;
        };
        if !self.dirty && self.texture.is_some() {
            return;
        }
        let palette = req.palette();
        let screen = self
            .view()
            .compose(self.page(), self.scroll, &palette, &self.ui);
        let rgba = to_rgba(&screen, &palette, 1, 1);
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [screen.width() as usize, screen.height() as usize],
            &rgba,
        );
        match &mut self.texture {
            Some(t) => t.set(image, egui::TextureOptions::NEAREST),
            None => {
                self.texture =
                    Some(ctx.load_texture("screen", image, egui::TextureOptions::NEAREST));
            }
        }
        self.dirty = false;
    }

    /// Pointer and keys while the note requester is shown (it is modal).
    fn note_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let Some(req) = &self.ui.note_requester else {
            return;
        };
        let ui_screen = view.ui_screen();
        let at = req.origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            self.note_event(event);
        }
    }

    /// Feeds one event to the note requester; Ok, Return, Esc or Space close it.
    fn note_event(&mut self, event: ReqEvent) {
        if let Some(req) = &mut self.ui.note_requester {
            if req.handle(event) == NoteResponse::Close {
                self.ui.note_requester = None;
            }
            self.dirty = true;
        }
    }

    /// Pointer and keys while the Page Size requester is open (it is modal).
    fn page_size_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let ui_screen = view.ui_screen();
        let at = PageSizeRequester::origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.page_size_requester.is_none() {
                break;
            }
            self.page_size_event(event);
        }
    }

    fn upload_and_paint(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        rect: egui::Rect,
        area: egui::Rect,
    ) {
        // Upload the screen at native resolution; the GPU scales it with nearest filtering.
        if self.dirty || self.texture.is_none() {
            self.prepare_anim_view();
            let (screen, rgba) = match (&self.requester, &self.show_page) {
                (Some(req), _) => {
                    let screen = req.render();
                    let rgba = to_rgba(&screen, &requester::palette(), 1, 1);
                    (screen, rgba)
                }
                (None, Some(sp)) => {
                    let screen = sp.render(self.page(), self.palette().len());
                    let rgba = to_rgba(&screen, self.palette(), 1, 1);
                    (screen, rgba)
                }
                (None, None) => {
                    let screen = self.compose_screen();
                    let rgba = to_rgba(&screen, self.palette(), 1, 1);
                    (screen, rgba)
                }
            };
            self.anim.shown = None;
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [screen.width() as usize, screen.height() as usize],
                &rgba,
            );
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                None => {
                    self.texture =
                        Some(ctx.load_texture("screen", image, egui::TextureOptions::NEAREST));
                }
            }
            self.dirty = false;
        }

        // Outside the screen (less than one UI pixel at the right and bottom, or the bars
        // around the Screen Format requester): the interface's dark pen.
        let pens = Pens::from_palette(self.palette());
        let dark = self
            .palette()
            .get(pens.dark)
            .map_or(egui::Color32::BLACK, |c| {
                egui::Color32::from_rgb(c.r, c.g, c.b)
            });
        let painter = ui.painter();
        painter.rect_filled(area, 0.0, dark);
        if let Some(t) = &self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(t.id(), rect, uv, egui::Color32::WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one egui pass with the `p` key pressed through the Color Mixer's input.
    fn press_p(app: &mut PainterApp, ctx: &egui::Context) -> bool {
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::P,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        ctx.begin_pass(raw);
        let view = app.view();
        let shown = app.palette_input(ctx, egui::Pos2::ZERO, 1.0, 1.0, view);
        // The font atlas upload is not needed in a test.
        ctx.end_pass().textures_delta.clear();
        shown
    }

    /// Drags the Color Mixer's slider `channel` all the way to the right.
    fn slider_to_max(app: &mut PainterApp, channel: usize) {
        let t = dpx_ui::PaletteRequester::slider_rect(channel);
        // Steps right of the knob, then the knob itself to the far right.
        app.palette_event(ReqEvent::Press(t.x + t.w - 2, t.y + 4));
        app.palette_event(ReqEvent::Release);
        for x in t.x..t.x + t.w {
            app.palette_event(ReqEvent::Press(x, t.y + 4));
            app.palette_event(ReqEvent::Drag(x + 400, t.y + 4));
            app.palette_event(ReqEvent::Release);
        }
    }

    #[test]
    fn color_mixer_edits_live_and_commits_one_undoable_set_palette() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        let before = app.palette().clone();
        app.ui.fg = 0;

        // `p` opens it on the paint screen; the key press itself is not fed to it.
        assert!(press_p(&mut app, &ctx));
        let req = app
            .ui
            .palette_requester
            .as_ref()
            .expect("p opens the mixer");
        assert_eq!(req.selected(), 0, "opens on the FG colour");
        assert!(!app.engine.can_undo());

        // Red of colour 0 to the maximum: the shown palette changes, the document not yet.
        slider_to_max(&mut app, 0);
        let req = app.ui.palette_requester.as_ref().unwrap();
        assert_eq!(req.palette().get(0), Some(dpx_core::Rgb8::new(255, 0, 0)));
        assert_eq!(app.palette(), &before);
        // Pick colour 5 so Ok makes it the FG colour.
        let c = app.ui.palette_requester.as_ref().unwrap().cell_rect(5);
        app.palette_event(ReqEvent::Press(c.x + 2, c.y + 2));
        app.palette_event(ReqEvent::Release);

        // Return = Ok: one undoable SetPalette.
        app.palette_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.palette_requester.is_none());
        assert_eq!(app.palette().get(0), Some(dpx_core::Rgb8::new(255, 0, 0)));
        assert_eq!(&app.palette().colors()[1..], &before.colors()[1..]);
        assert_eq!(app.ui.fg, 5);
        app.undo();
        assert_eq!(app.palette(), &before, "one undo restores the palette");
        assert!(!app.engine.can_undo());

        // Via the menu; Esc cancels and leaves the document as it was.
        app.run_action(Action::Command("palette.mixer"), &ctx);
        assert!(app.ui.palette_requester.is_some());
        slider_to_max(&mut app, 1);
        app.palette_event(ReqEvent::Key(ReqKey::Escape));
        assert!(app.ui.palette_requester.is_none());
        assert_eq!(app.palette(), &before);
        assert!(!app.engine.can_undo(), "Cancel adds no undo step");

        // Ok without changes adds no undo step either.
        app.run_action(Action::Command("palette.mixer"), &ctx);
        app.palette_event(ReqEvent::Key(ReqKey::Enter));
        assert!(!app.engine.can_undo());

        // `p` does nothing while another requester is open.
        app.open_page_size();
        assert!(!press_p(&mut app, &ctx));
        assert!(app.ui.palette_requester.is_none());
    }

    #[test]
    fn menu_spec_parsing() {
        assert_eq!(parse_menu_spec("0"), Some((0, None, None)));
        assert_eq!(parse_menu_spec("0:5:1"), Some((0, Some(5), Some(1))));
        assert_eq!(parse_menu_spec("7"), None);
        assert_eq!(parse_menu_spec("0:0:0"), None, "Load.. has no submenu");
        assert_eq!(parse_menu_spec("0:99"), None);
        assert_eq!(parse_menu_spec("x"), None);
    }

    #[test]
    fn menu_actions_change_the_document_and_ui() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.ui.fg = 5;
        app.handle_press(Hit::Page { x: 0, y: 0 }, false);
        app.finish_stroke();
        assert_eq!(app.page().get(0, 0), Some(5));
        app.run_action(Action::Command("picture.flip_horizontal"), &ctx);
        assert_eq!(app.page().get(319, 0), Some(5));
        app.run_action(Action::Command("picture.flip_vertical"), &ctx);
        assert_eq!(app.page().get(319, 255), Some(5));
        app.select_tool(Tool::Undo, false);
        app.select_tool(Tool::Undo, false);
        assert_eq!(app.page().get(0, 0), Some(5), "flips are undoable");

        app.run_action(Action::Mode(PaintMode::Smear), &ctx);
        assert_eq!(app.ui.mode, PaintMode::Smear);
        app.run_action(Action::Pref(dpx_ui::Pref::Coords), &ctx);
        assert!(app.ui.prefs.get(dpx_ui::Pref::Coords));

        // BG -> FG recolours background pixels with the foreground colour.
        app.run_action(Action::Command("color.bg_to_fg"), &ctx);
        assert_eq!(app.page().get(1, 1), Some(app.ui.fg));
    }

    #[test]
    fn screen_format_requester_changes_the_mode_and_is_undoable() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.ui.fg = 31;
        app.run_action(Action::Command("picture.screen_format"), &ctx);
        assert!(app.requester.is_some());
        assert_eq!(app.view(), View::of(requester::SCREEN));
        // Arrow up from PAL:Low Res selects PAL:High Res Laced; Enter = Use.
        app.requester_event(ReqEvent::Key(ReqKey::Up));
        app.requester_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.requester.is_none());
        let mode = app.engine.document().target().mode;
        assert_eq!(mode.name(), "PAL:High Res Laced");
        assert_eq!(app.page().width(), 640);
        assert_eq!(app.page().height(), 512);
        assert_eq!(app.view().screen_w, 640);
        app.undo();
        assert_eq!(app.engine.document().target().mode, DisplayMode::PAL_LORES);

        // A refused format keeps the requester open with a message.
        app.engine.execute(Command::Clear { color: 31 }).unwrap();
        app.open_screen_format();
        for _ in 0..4 {
            app.requester_event(ReqEvent::Key(ReqKey::Down));
        }
        // Escape cancels.
        app.requester_event(ReqEvent::Key(ReqKey::Escape));
        assert!(app.requester.is_none());
    }

    fn type_into(app: &mut PainterApp, text: &str) {
        for c in text.chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
    }

    #[test]
    fn anim_save_and_load_through_the_requesters() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("dpx-app-anim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();

        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        for frame in 0..3u32 {
            app.engine.execute(Command::GotoFrame { frame }).unwrap();
            let mut page = IndexedImage::new(320, 256, 5 + frame as u8);
            page.set(10 + frame as i32, 20, 31);
            app.engine
                .execute(Command::SetPage {
                    width: 320,
                    height: 256,
                    pixels: page.pixels().to_vec(),
                })
                .unwrap();
        }
        app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
        app.engine
            .execute(Command::SetFrameRate { fps: 12 })
            .unwrap();
        let doc = app.engine.document().clone();

        // Anim > Save.. is "Save Animation"; a name without extension gets .anim.
        app.run_action(Action::Command("anim.save"), &ctx);
        let req = app.ui.file_requester.as_ref().expect("requester open");
        assert_eq!(req.kind(), FileKind::SaveAnim);
        assert_eq!(req.kind().title(), "Save Animation");
        type_into(&mut app, "walk");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "saved");
        let path = dir.join("walk.anim");
        assert!(path.exists());
        assert!(app.ui.note_requester.is_none(), "nothing to report");
        assert_eq!(app.picture_path, None, "Anim > Save keeps the picture name");

        // Anim > Load.. is "Load Animation" and gives the same frames, palette and rate.
        let mut other = PainterApp::new();
        other.last_drawer = Some(dir.to_string_lossy().into_owned());
        other.run_action(Action::Command("anim.load"), &ctx);
        let req = other.ui.file_requester.as_ref().unwrap();
        assert_eq!(req.kind().title(), "Load Animation");
        type_into(&mut other, "walk.anim");
        other.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(other.ui.file_requester.is_none(), "loaded");
        let back = other.engine.document();
        assert_eq!(back.frame_count(), 3);
        assert!(back.frames().eq(doc.frames()), "identical frames");
        assert_eq!(back.palette(), doc.palette());
        assert_eq!((back.fps(), back.current_frame()), (12, 0));
        assert_eq!(other.picture_path.as_deref(), Some(path.as_path()));
        // The suggested name for the next save is the picture's, as .anim.
        other.run_action(Action::Command("anim.save"), &ctx);
        assert_eq!(
            other.ui.file_requester.as_ref().unwrap().file(),
            "walk.anim"
        );
        other.file_requester_event(ReqEvent::Key(ReqKey::Escape), 0);

        // Picture > Load.. opens an ANIM too; Export with a .anim name writes one, and a
        // single frame gives a one-frame animation.
        let mut third = PainterApp::new();
        third.last_drawer = Some(dir.to_string_lossy().into_owned());
        third.run_action(Action::Command("picture.load"), &ctx);
        type_into(&mut third, "walk.anim");
        third.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert_eq!(third.engine.document().frame_count(), 3);
        let mut single = PainterApp::new();
        single.last_drawer = Some(dir.to_string_lossy().into_owned());
        single.run_action(Action::Command("picture.export"), &ctx);
        type_into(&mut single, "one.anim");
        single.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(single.ui.file_requester.is_none());
        let one = files::load(&dir.join("one.anim")).unwrap();
        let (one, _, _) = files::open_picture(one.picture, Target::default());
        assert_eq!(one.frame_count(), 1);
        assert_eq!(one.page(), single.engine.document().page());

        assert_eq!(anim_path(Path::new("Venus")), Path::new("Venus.anim"));
        assert_eq!(anim_path(Path::new("a/V.iff")), Path::new("a/V.anim"));
        assert_eq!(
            project_path(Path::new("a/V.anim")),
            Path::new("a/V.dpxproj")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_and_save_through_the_file_requester() {
        use dpx_core::{Resolution, Video};
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("dpx-app-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        // A PAL HiRes picture with one painted pixel.
        let hi = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        let mut img = IndexedImage::new(640, 256, 0);
        img.set(600, 200, 9);
        let mut src = dpx_iff::Ilbm::from_indexed(img.clone(), Palette::dpaint_default_32());
        src.camg = Some(hi.mode_id());
        let bytes = dpx_iff::ilbm::write(&src, dpx_iff::Compression::ByteRun1).unwrap();
        std::fs::write(dir.join("Castle.iff"), bytes).unwrap();

        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.run_action(Action::Command("picture.load"), &ctx);
        let req = app.ui.file_requester.as_ref().expect("requester open");
        assert_eq!(req.drawer(), dir.to_string_lossy());
        // A missing file keeps the requester open with a message.
        type_into(&mut app, "Missing.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_some());
        for _ in 0.."Missing.iff".len() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Backspace), 0);
        }
        type_into(&mut app, "Castle.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "loaded and closed");
        assert_eq!(app.engine.document().target().mode, hi);
        assert_eq!(app.page(), &img);
        assert_eq!(app.view().screen_w, 640);

        // Save writes the project and offers the picture's name as one (ADR-27).
        app.run_action(Action::Command("picture.save"), &ctx);
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().file(),
            "Castle.dpxproj"
        );
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "saved without asking");
        let project = files::load(&dir.join("Castle.dpxproj")).unwrap().picture;
        assert_eq!(project.indexed().unwrap().0, &img);
        assert_eq!(
            app.picture_path.as_deref(),
            Some(dir.join("Castle.dpxproj").as_path())
        );
        // A name without extension gets .dpxproj; Export refuses a project name.
        assert_eq!(project_path(Path::new("Venus")), Path::new("Venus.dpxproj"));
        assert_eq!(project_path(Path::new("a/V.LBM")), Path::new("a/V.dpxproj"));
        app.run_action(Action::Command("picture.export"), &ctx);
        assert_eq!(app.ui.file_requester.as_ref().unwrap().file(), "Castle.iff");
        for _ in 0.."Castle.iff".len() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Backspace), 0);
        }
        type_into(&mut app, "Castle.dpxproj");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(
            app.ui.file_requester.is_some(),
            "Export does not write projects"
        );
        app.file_requester_event(ReqEvent::Key(ReqKey::Escape), 0);

        // Export offers the IFF name; replacing it needs a "yes"; the picture keeps its name.
        app.run_action(Action::Command("picture.export"), &ctx);
        assert_eq!(app.ui.file_requester.as_ref().unwrap().file(), "Castle.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_some(), "asks before replacing");
        app.file_requester_event(ReqEvent::Key(ReqKey::Char('y')), 0);
        assert!(app.ui.file_requester.is_none());
        let back = files::load(&dir.join("Castle.iff")).unwrap().picture;
        assert_eq!(back.indexed().unwrap().0, &img);
        assert_eq!(back.camg(), Some(0x0002_9000), "AGA PAL HiRes mode id");
        assert_eq!(
            app.picture_path.as_deref(),
            Some(dir.join("Castle.dpxproj").as_path())
        );

        // A new name with .png writes PNG; JPEG is refused with a message.
        app.run_action(Action::Command("picture.export"), &ctx);
        for _ in 0.."Castle.iff".len() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Backspace), 0);
        }
        type_into(&mut app, "Castle.jpg");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_some());
        for _ in 0..3 {
            app.file_requester_event(ReqEvent::Key(ReqKey::Backspace), 0);
        }
        type_into(&mut app, "png");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none());
        assert_eq!(
            files::load(&dir.join("Castle.png"))
                .unwrap()
                .picture
                .indexed()
                .unwrap()
                .0,
            &img
        );
        assert!(!dir.join("Castle.jpg").exists());

        // Cancel closes without changes.
        app.run_action(Action::Command("picture.load"), &ctx);
        app.file_requester_event(ReqEvent::Key(ReqKey::Escape), 0);
        assert!(app.ui.file_requester.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pixel_size_steps_fit_and_window_sizes() {
        let v = egui::vec2;
        let lores = (320, 256);
        // The largest whole or half step that fits: 960x768 is 3x, 1120x896 3.5x.
        assert_eq!(fit_step(v(960.0, 768.0), lores), 6);
        assert_eq!(fit_step(v(1119.0, 900.0), lores), 6);
        assert_eq!(fit_step(v(1120.0, 896.0), lores), 7);
        assert_eq!(
            fit_step(v(2560.0, 1340.0), lores),
            10,
            "5x on a 1440-line monitor"
        );
        assert_eq!(fit_step(v(100.0, 100.0), lores), 2, "never below 1x");
        assert_eq!(fit_step(v(9999.0, 9999.0), lores), 16, "never above 8x");
        // After a resize the nearest step wins; the smaller axis decides.
        assert_eq!(nearest_step(v(1100.0, 850.0), lores), 7);
        assert_eq!(nearest_step(v(1000.0, 790.0), lores), 6);
        assert_eq!(nearest_step(v(2000.0, 790.0), lores), 6);
        // The window for a step; overscan screens get half-point sizes at half steps.
        assert_eq!(window_size(lores, 7), v(1120.0, 896.0));
        assert_eq!(window_size((368, 283), 5), v(920.0, 707.5));
        assert_eq!(View::of(DisplayMode::PAL_LORES).ui_size(), lores);
    }

    #[test]
    fn the_window_snaps_to_the_screen_at_the_pixel_size() {
        use dpx_core::{Resolution, Video};
        let v = egui::vec2;
        let mut app = PainterApp::new();
        let plain = |w: f32, h: f32| Window {
            w: w as i32,
            h: h as i32,
            ppp: 1.0,
        };
        // The start window is 3x: nothing to do.
        app.monitor = Some(v(2560.0, 1440.0));
        assert_eq!(app.ui.pixel_size, PixelSize(6));
        assert_eq!(app.snap_window(v(960.0, 768.0), 0.0), None);
        // The user drags to 1100x850: wait until the size settles, then snap to 3.5x.
        assert_eq!(app.snap_window(v(1100.0, 850.0), 1.0), None);
        assert_eq!(app.snap_window(v(1100.0, 850.0), 1.1), None);
        assert_eq!(
            app.snap_window(v(1100.0, 850.0), 1.3),
            Some(v(1120.0, 896.0))
        );
        assert_eq!(app.ui.pixel_size, PixelSize(7));
        // While the window has not followed yet, the screen is centred at the step that fits.
        app.window = Some(plain(1100.0, 850.0));
        assert_eq!(app.layout(), (View::of(DisplayMode::PAL_LORES), 3.0));
        assert_eq!(app.snap_window(v(1100.0, 850.0), 1.4), None, "asked once");
        // Our own size arrives: no new resize; the screen fills the window at 3.5x.
        assert_eq!(app.snap_window(v(1120.0, 896.0), 1.5), None);
        assert_eq!(app.snap.resized_at, None);
        app.window = Some(plain(1120.0, 896.0));
        assert_eq!(app.layout().1, 3.5);
        // Keys: half a step; Cmd+0 = the largest that fits on the monitor (5x on 2560x1440).
        app.step_pixel_size(1);
        assert_eq!(app.ui.pixel_size, PixelSize(8));
        assert_eq!(
            app.snap_window(v(1120.0, 896.0), 2.0),
            Some(v(1280.0, 1024.0))
        );
        app.run_action(
            Action::Command("view.largest_pixel_size"),
            &egui::Context::default(),
        );
        assert_eq!(app.ui.pixel_size, PixelSize(10));
        app.run_action(Action::PixelSize(PixelSize(16)), &egui::Context::default());
        assert_eq!(app.ui.pixel_size, PixelSize(10), "limited by the monitor");
        app.step_pixel_size(-9);
        assert_eq!(app.ui.pixel_size, PixelSize(2), "1x is the smallest");
        // A new screen (HiRes overscan) keeps the pixel size and resizes the window.
        app.set_pixel_size(PixelSize(6));
        let hi = Target::new(
            dpx_core::Chipset::Aga,
            DisplayMode::new(Video::Pal, Resolution::Hi, false),
        )
        .with_screen(736, 283);
        app.run(Command::SetFormat {
            mode: hi.mode,
            colors: 32,
            width: 736,
            height: 283,
            retain: false,
            screen: Some(hi.screen),
        });
        assert_eq!(app.engine.document().target().screen, (736, 283));
        let got = app.snap_window(v(1280.0, 1024.0), 3.0);
        assert_eq!(got, Some(v(1104.0, 849.0)), "368x283 UI pixels at 3x");
        // Full screen: no snapping; the screen is centred at the largest step that fits.
        app.free = true;
        app.window = Some(plain(2560.0, 1440.0));
        assert_eq!(app.snap_window(v(2560.0, 1440.0), 4.0), None);
        let (view, scale) = app.layout();
        assert_eq!(view.ui_size(), (368, 283));
        assert_eq!(scale, 2.5, "5x for UI pixels of two HiRes pixels");
    }

    #[test]
    fn load_and_save_notes_show_in_a_requester_that_ok_closes() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("dpx-app-notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        // An ILBM with a chunk DeluxePainter does not keep, and a plain one.
        let img = IndexedImage::new(320, 256, 0);
        let mut src = dpx_iff::Ilbm::from_indexed(img.clone(), Palette::dpaint_default_32());
        src.other_chunks.push(dpx_iff::ilbm::RawChunk {
            id: *b"DPPS",
            data: vec![0; 10],
        });
        let write = |name: &str, i: &dpx_iff::Ilbm| {
            let bytes = dpx_iff::ilbm::write(i, dpx_iff::Compression::ByteRun1).unwrap();
            std::fs::write(dir.join(name), bytes).unwrap();
        };
        write("Notes.iff", &src);
        src.other_chunks.clear();
        write("Plain.iff", &src);

        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.run_action(Action::Command("picture.load"), &ctx);
        type_into(&mut app, "Notes.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "loaded and closed");
        let req = app.ui.note_requester.as_ref().expect("notes shown");
        // Drawn on top of the screen; the Ok button closes it.
        let ui_screen = app.view().ui_screen();
        let (ox, oy) = req.origin(ui_screen.width, ui_screen.height);
        let with = app
            .view()
            .compose(app.page(), app.scroll, app.palette(), &app.ui);
        let ok = (ox + 128, oy + req.height() - 10);
        app.note_event(ReqEvent::Press(ok.0 - ox, ok.1 - oy));
        app.note_event(ReqEvent::Release);
        assert!(app.ui.note_requester.is_none(), "Ok closes");
        let without = app
            .view()
            .compose(app.page(), app.scroll, app.palette(), &app.ui);
        assert_ne!(with, without);

        // A picture without notes opens none.
        app.run_action(Action::Command("picture.load"), &ctx);
        type_into(&mut app, "Plain.iff");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.note_requester.is_none());

        // Exporting PNG tells what PNG cannot store (the display mode); Return closes.
        app.run_action(Action::Command("picture.export"), &ctx);
        for _ in 0.."Plain.iff".len() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Backspace), 0);
        }
        type_into(&mut app, "Plain.png");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none());
        assert!(dir.join("Plain.png").exists());
        assert!(app.ui.note_requester.is_some(), "PNG notes shown");
        app.note_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.note_requester.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_big_true_colour_png_loads_scaled_to_the_current_screen() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("dpx-app-tc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 700x600 fits no screen format (at most 512 lines): two colour halves.
        let (w, h) = (700u32, 600u32);
        let data: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                if i % w < w / 2 {
                    [200, 40, 40]
                } else {
                    [40, 40, 200]
                }
            })
            .collect();
        let mut png_bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png_bytes, w, h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().unwrap().write_image_data(&data).unwrap();
        }
        std::fs::write(dir.join("Photo.png"), png_bytes).unwrap();

        let mut app = PainterApp::new();
        let before = app.engine.document().target();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.run_action(Action::Command("picture.load"), &ctx);
        type_into(&mut app, "Photo.png");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "loaded and closed");
        assert_eq!(app.engine.document().target(), before, "kept the screen");
        // 320x256 LoRes: 7:6 shown on square pixels → 299x256, on a screen-sized page.
        assert_eq!((app.page().width(), app.page().height()), (320, 256));
        let at = |x: i32| app.palette().get(app.page().get(x, 100).unwrap()).unwrap();
        assert_eq!(at(10), dpx_core::Rgb8::new(200, 40, 40));
        assert_eq!(at(290), dpx_core::Rgb8::new(40, 40, 200));
        assert_eq!(
            app.page().get(310, 100),
            Some(0),
            "rest of the page: colour 0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stroke_is_previewed_then_committed_as_one_undo_step() {
        let mut app = PainterApp::new();
        app.ui.tool = Tool::ContinuousFreehand;
        app.ui.fg = 2;
        app.handle_press(Hit::Page { x: 10, y: 10 }, false);
        app.handle_drag(20, 14);
        app.handle_drag(30, 10);
        // Shown while painting, but not yet in the document.
        assert_eq!(app.page().get(20, 14), Some(2));
        assert_eq!(app.engine.document().page().get(20, 14), Some(0));
        let preview = app.page().clone();
        app.finish_stroke();
        assert_eq!(app.engine.document().page(), &preview);
        assert_eq!(app.engine.document().revision(), 1);

        app.undo();
        assert_eq!(
            app.page().get(20, 14),
            Some(0),
            "whole stroke undone at once"
        );
        app.redo();
        assert_eq!(app.page(), &preview);

        // Right-click on UNDO redoes, left-click undoes.
        app.handle_press(
            Hit::Tool {
                tool: Tool::Undo,
                filled_half: false,
            },
            false,
        );
        assert_eq!(app.page().get(20, 14), Some(0));
        app.handle_press(
            Hit::Tool {
                tool: Tool::Undo,
                filled_half: false,
            },
            true,
        );
        assert_eq!(app.page(), &preview);
    }

    #[test]
    fn page_size_requester_sets_an_undoable_bigger_page_and_scrolling_follows() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.ui.bg = 3;
        app.handle_press(Hit::Page { x: 5, y: 5 }, false);
        app.finish_stroke();
        app.run_action(Action::Command("picture.page_size"), &ctx);
        assert!(app.ui.page_size_requester.is_some());
        let key = |app: &mut PainterApp, k| app.page_size_event(ReqEvent::Key(k));
        // Too small: refused with the limits, requester stays open.
        for _ in 0..3 {
            key(&mut app, ReqKey::Backspace);
        }
        for c in "100".chars() {
            key(&mut app, ReqKey::Char(c));
        }
        key(&mut app, ReqKey::Enter);
        assert!(app.ui.page_size_requester.is_some());
        assert_eq!(app.page().width(), 320);
        // 640 x 512.
        for _ in 0..3 {
            key(&mut app, ReqKey::Backspace);
        }
        for c in "640".chars() {
            key(&mut app, ReqKey::Char(c));
        }
        key(&mut app, ReqKey::Tab);
        for _ in 0..3 {
            key(&mut app, ReqKey::Backspace);
        }
        for c in "512".chars() {
            key(&mut app, ReqKey::Char(c));
        }
        key(&mut app, ReqKey::Enter);
        assert!(app.ui.page_size_requester.is_none());
        assert_eq!((app.page().width(), app.page().height()), (640, 512));
        assert_eq!(app.page().get(5, 5), Some(app.ui.fg), "picture kept");
        assert_eq!(
            app.page().get(600, 500),
            Some(3),
            "new area in the BG colour"
        );

        // Scrolling: clamped to the page; the screen shows the scrolled part.
        app.scroll_to(1000, -10);
        assert_eq!(app.scroll, (320, 0));
        app.centre_on((330, 400));
        assert_eq!(app.scroll, (170, 256), "as far as the page allows");
        app.scroll_to(0, 0);
        let screen = app
            .view()
            .compose(app.page(), (0, 0), app.palette(), &app.ui);
        assert_eq!(screen.get(5, 100), Some(0));
        let mut marked = app.page().clone();
        marked.set(325, 300, 9);
        let screen = app
            .view()
            .compose(&marked, (320, 200), app.palette(), &app.ui);
        assert_eq!(
            screen.get(5, 100),
            Some(9),
            "page pixel (325,300) at screen (5,100)"
        );

        // Undo restores the screen-sized page and clamps the scroll position.
        app.scroll_to(300, 200);
        app.undo();
        assert_eq!((app.page().width(), app.page().height()), (320, 256));
        assert_eq!(app.scroll, (0, 0));
    }

    #[test]
    fn show_page_moves_the_view() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run(Command::SetPageSize {
            width: 640,
            height: 512,
            fill: 0,
        });
        app.run_action(Action::Command("picture.show_page"), &ctx);
        let sp = app.show_page.as_ref().expect("Show Page open");
        assert_eq!(sp.factor(), (2, 2));
        // Grab the rectangle (160x128 at 0,0), drag it 40 right and 20 down, release.
        app.show_page_event(ReqEvent::Press(10, 10), false);
        app.show_page_event(ReqEvent::Drag(50, 30), false);
        app.show_page_event(ReqEvent::Release, false);
        assert!(app.show_page.is_none());
        assert_eq!(app.scroll, (80, 40));
        // Any other key returns without moving.
        app.open_show_page();
        app.show_page_event(ReqEvent::Key(ReqKey::Right), true);
        app.show_page_event(ReqEvent::Key(ReqKey::Escape), false);
        assert_eq!(app.scroll, (80, 40));
    }

    #[test]
    fn a_small_picture_opens_on_a_screen_sized_page() {
        let small = Document::from_picture(
            IndexedImage::new(64, 48, 2),
            Palette::dpaint_default_32(),
            Target::default(),
        );
        let (doc, note) = files::page_at_least_screen(small);
        assert_eq!((doc.page().width(), doc.page().height()), (320, 256));
        assert_eq!(doc.page().get(63, 47), Some(2));
        assert_eq!(doc.page().get(64, 0), Some(0));
        assert!(note.unwrap().contains("64x48"));
        let (same, note) = files::page_at_least_screen(doc.clone());
        assert_eq!(same, doc);
        assert!(note.is_none());
    }

    #[test]
    fn screen_format_sets_an_overscan_screen() {
        use dpx_ui::requester::ScreenChoice;
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run_action(Action::Command("picture.screen_format"), &ctx);
        // The Screen Size cycle gadget (requester screen pixels): Standard → Overscan.
        app.requester_event(ReqEvent::Press(60, 118));
        app.requester_event(ReqEvent::Release);
        let req = app.requester.as_ref().unwrap();
        assert_eq!(req.screen_choice(), ScreenChoice::Overscan);
        app.requester_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.requester.is_none());
        let target = app.engine.document().target();
        assert_eq!(target.screen, (368, 283));
        assert_eq!((app.page().width(), app.page().height()), (368, 283));
        let view = app.view();
        assert_eq!((view.screen_w, view.screen_h), (368, 283));
        // The Page Size requester's Standard is the screen size now.
        app.run_action(Action::Command("picture.page_size"), &ctx);
        app.page_size_event(ReqEvent::Key(ReqKey::Escape));
        app.undo();
        assert_eq!(app.engine.document().target(), Target::default());
        assert_eq!(app.view(), View::of(DisplayMode::PAL_LORES));
    }
}
