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
//!     [--animbrush-requester KIND]      show an AnimBrush requester: pick, make or settings
//!                                       (ADR-36)
//!     [--anim-panel]                    show the Animation Control Panel (ADR-38)
//!     [--anim-control range|move]       show the Set Range or Move requester (ADR-38)
//!     [--stencil-paint X,Y,W,H]         Stencil ▸ Paint with that rectangle painted (ADR-39)
//!     [--hover X,Y]                     the tooltip a pointer resting on UI pixel X,Y shows
//!                                       (ADR-41)
//!     [--import-requester FILE]         the Import requester for a PNG or JPEG (ADR-42)
//!     [--import SPEC] [--import-apply]  … with position=centre|top_left;palette=new|current;
//!                                       colors=N;dither=none|fs|ordered, or the page after Ok
//!     [--chipset ocs|aga]               the picture's chipset (before `--page`; ADR-46)
//!     [--preset ID]                     a screen preset, e.g. pixel_art_320x240 (ADR-46)
//!     [--requester-keys K,K..]          keys for `--requester` first: up, down, tab, enter,
//!                                       or a letter (h Chipset, p Preset) (ADR-46)
//!     [--export-note NAME]              the note Picture > Export.. shows for NAME (ADR-46)
//!     [--load-palette FILE]             Palette > Load.. of FILE and the note it shows (ADR-46)
//!     [--layers-panel [docked]]         show the layer panel (ADR-44) over the screen's right
//!                                       edge, or docked in the work area right of the screen
//!     [--mcp-calls FILE.json]           run a JSON array of {"tool", "arguments"} MCP tool
//!                                       calls on the GUI workspace first (ADR-43)
//!     [--mcp-question NAME]             the question before an MCP client replaces file NAME
//!     [--mcp-gen-question picture|spare] the question before an MCP client's generated
//!                                       picture replaces the picture or the spare page's (ADR-45)
//!     [--mcp-prefs STATE]               Prefs ▸ AI & MCP: off, ready, connected (with the
//!                                       --mcp-calls in its activity), fallback, error,
//!                                       code-confirm, code-replace, code-missing,
//!                                       desktop-missing or disconnect (ADR-43), generator
//!                                       (OpenAI set up) or imagegen-setup (Set up..; ADR-45)
//!     [--ui modern]                     the Modern skin's main window (ADR-47) instead: the
//!                                       picture in a vector interface, with
//!     [--theme dark|light]              … its colour scheme (default dark),
//!     [--size WxH]                      … the window's size in points (default 1280x800),
//!     [--device-scale 1|2]              … physical pixels per point (default 1) and
//!     [--pointer X,Y]                   … a pointer at window point X,Y
//!     [--tooltip]                       … that has rested there: the tooltip of the dock
//!                                       button under it (`--menu` draws the Modern cards)
//! `deluxepainter --font-preview out.png` render sample lines of the UI font (3×)
//! `deluxepainter --open pic.iff`        start the GUI (or a snapshot) with a picture
//! `deluxepainter --no-settings`         start the GUI without reading or writing the settings file
//! `deluxepainter --mcp`                 start the GUI with its MCP server on for this session
//!                                       (Streamable HTTP on 127.0.0.1, ADR-43); ends at once
//!                                       while another DeluxePainter runs
//! `deluxepainter info <file>`           describe an IFF ILBM, PNG, JPEG or .dpxproj file
//! `deluxepainter convert <in> <out>`    convert to IFF ILBM, PNG or .dpxproj
//! `deluxepainter mcp [--allow-dir DIR]...` headless MCP server on stdio (ADR-21, ADR-24)
//! `deluxepainter mcp --http [--port N] [--allow-dir DIR]...` headless MCP server on
//!                                       http://127.0.0.1:N/mcp; port and bearer token in the
//!                                       endpoint file of the config directory (ADR-43)
//! `deluxepainter mcp-bridge`            stdio MCP bridge to the running app (ADR-43)

mod anim;
mod anim_control;
mod animbrush;
mod autosave;
mod browse;
mod brush_menu;
// The Classic guard of ADR-47.
#[cfg(test)]
mod classic_pin_tests;
// ADR-44: the composite on screen, in exports and in saved projects.
#[cfg(test)]
mod composite_tests;
mod config;
mod connect;
mod effects;
mod files;
mod generate;
mod import;
// The layer panel (ADR-44).
mod layers;
mod mcp;
mod mcp_prefs;
mod menu_actions;
// The Modern skin's app side (ADR-47).
mod modern;
mod palette_stencil;
mod perspective;
mod recovery;
// Chipset, presets and hardware rules in the app (ADR-46).
mod screen_format;
mod settings;
mod tools;
mod tooltip;

use std::borrow::Cow;
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

/// The smallest window of the Classic skin, in points (the Modern skin has its own, larger
/// minimum while it is shown, ADR-47).
const CLASSIC_MIN_WINDOW: [f32; 2] = [320.0, 200.0];

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
        (Some("mcp-bridge"), rest) => exit_with(dpx_mcp::bridge::run_cli(rest)),
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
        // ADR-47: the Modern skin's main window instead of the classic screen.
        let modern = modern::snapshot_options(&args).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        });
        if modern.is_some()
            && let Some(flag) = ["--show-page", "--requester"]
                .into_iter()
                .find(|f| args.iter().any(|a| a == f))
        {
            eprintln!("{flag} is a full-screen view the Modern skin does not host yet");
            std::process::exit(2);
        }
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
        // ADR-46: the picture's chipset and a screen preset.
        if let Err(e) = screen_format::snapshot_format(&args, &mut engine) {
            eprintln!("{e}");
            std::process::exit(2);
        }
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
        // ADR-37: perspective stamps and FillScreen of the picked brush.
        if let Some(spec) = arg_value(&args, "--perspective") {
            let Some(brush) = &picked else {
                eprintln!("--perspective needs --pick");
                std::process::exit(2);
            };
            let screen = View::of_target(engine.document().target());
            let size = (screen.screen_w, screen.screen_h);
            if let Err(e) = perspective::run_spec(&mut engine, brush, &spec, size) {
                eprintln!("--perspective {e}");
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
        if let Some(req) = animbrush::snapshot_requester(&args, &doc) {
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
        // ADR-38: the Control Panel, Set Range and Move.
        if let Err(e) = anim_control::snapshot_ui(&args, &doc, &mut ui) {
            eprintln!("{e}");
            std::process::exit(2);
        }
        let doc = if args.iter().any(|a| a == "--light-table") {
            let spec = arg_value(&args, "--light-table").filter(|v| !v.starts_with("--"));
            let light = anim::parse_light_table(spec.as_deref()).unwrap_or_else(|e| {
                eprintln!("--light-table: {e}");
                std::process::exit(2);
            });
            // ADR-44: the frame as it shows, all visible layers.
            let shown = doc.composite();
            let page = anim::light_table_page(&doc, &shown, light, None, ui.bg, light.dim);
            Document::from_picture(page, doc.palette().clone(), doc.target())
        } else {
            doc
        };
        // ADR-39: the Stencil ▸ Paint view.
        let doc = match arg_value(&args, "--stencil-paint") {
            Some(spec) => {
                palette_stencil::snapshot_view(&spec, &doc, &mut ui).unwrap_or_else(|e| {
                    eprintln!("--stencil-paint: {e}");
                    std::process::exit(2);
                })
            }
            None => doc,
        };
        // ADR-42: the Import requester for a PNG or JPEG, or its result after Ok.
        let doc = import::snapshot(&args, &doc, &mut ui).unwrap_or_else(|(code, e)| {
            eprintln!("{e}");
            std::process::exit(code);
        });
        // ADR-45: Picture > Generate.. and the Image Generator setup (no network, no keys).
        let doc = generate::snapshot(&args, &doc, &mut ui).unwrap_or_else(|(code, e)| {
            eprintln!("{e}");
            std::process::exit(code);
        });
        // ADR-43: MCP tool calls on the GUI workspace, and Prefs ▸ AI & MCP showing them.
        let doc = match mcp_prefs::snapshot(&args, &doc, &mut ui) {
            Ok(changed) => changed,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        };
        // Apply or a call may have made a picture of another screen or display mode.
        let view = View::of_target(doc.target());
        if args.iter().any(|a| a == "--notes")
            && let Some((_, _, _, notes)) = &opened
        {
            ui.note_requester = note_requester(LOADED, notes);
        }
        // ADR-43: the question before an MCP client replaces a file.
        if let Some(name) = arg_value(&args, "--mcp-question") {
            let q = dpx_mcp::tools::overwrite_question(Some("claude-code"), &name);
            ui.choice_requester = Some(mcp::question_requester(&q));
        }
        if args.iter().any(|a| a == "--recover-requester") {
            ui.choice_requester = Some(autosave::recover_requester(
                "Castle",
                recovery::format_utc(recovery::unix_now()),
            ));
        }
        // ADR-44: the layer panel (`--layers-panel`) or the title bar's layer name, of the
        // document shown (ADR-45: Generate's Apply or MCP calls may have made another one).
        if engine.document() != &doc {
            engine = Engine::new(doc.clone());
        }
        layers::snapshot_ui(&args, &mut engine, &mut ui, view);
        // ADR-41: the tooltip of a resting pointer.
        if let Err(e) =
            tooltip::snapshot_hover(&args, doc.palette().len(), &view.ui_screen(), &mut ui)
        {
            eprintln!("{e}");
            std::process::exit(2);
        }
        if args.iter().any(|a| a == "--show-page") {
            let sp = ShowPage::new(
                (doc.page().width(), doc.page().height()),
                (view.screen_w as u32, view.screen_h as u32),
                scroll,
            );
            let screen = sp.render(&doc.composite(), doc.palette().len());
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
        // ADR-46: Palette ▸ Load.. of a file on the picture, with the note it shows.
        let doc = screen_format::snapshot_palette_load(&args, &doc, &mut ui).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        });
        // ADR-46: the note Picture ▸ Export.. would show under a name (nothing is written).
        if let Some(name) = arg_value(&args, "--export-note") {
            let extras = opened.as_ref().map(|o| o.2.clone()).unwrap_or_default();
            match screen_format::snapshot_export_note(&name, &doc, &extras) {
                Ok(note) => ui.note_requester = note,
                Err(e) => {
                    eprintln!("--export-note {name}: {e}");
                    std::process::exit(2);
                }
            }
        }
        if args.iter().any(|a| a == "--requester") {
            // ADR-46: `--requester-keys` drives it first.
            let req = screen_format::snapshot_requester(&args, &doc).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(2);
            });
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
        let written = match &modern {
            Some(options) => {
                modern::snapshot(path, options, &doc, view, scroll, &ui, picked.is_some())
            }
            None => snapshot(path, scale, &doc, view, scroll, &ui, picked.is_some()),
        };
        if let Err(e) = written {
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
    let mcp_flag = args.iter().any(|a| a == "--mcp");
    // ADR-43: the first GUI holds `mcp-app.lock`, so the bridge never starts a second one;
    // a `--mcp` start (the bridge's) while another GUI runs ends here.
    let instance = match mcp::claim_instance(config::default_dir().as_deref(), mcp_flag) {
        Ok(claim) => claim,
        Err(e) => {
            eprintln!("{e}");
            return Ok(());
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
            .with_min_inner_size(CLASSIC_MIN_WINDOW)
            .with_icon(Arc::new(app_icon())),
        // VSync with one queued frame instead of eframe's default two: the brush follows the
        // pointer a frame sooner (ADR-40). Our GPU work is one textured quad.
        wgpu_options: eframe::WgpuConfiguration {
            surface: eframe::SurfaceConfig::LOW_LATENCY,
            ..Default::default()
        },
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
            // ADR-45: API keys in the system keychain, opened on first use (tests and
            // snapshots keep them in memory).
            app.imagegen.system_keys = true;
            // ADR-43: the MCP server keeps its token and endpoint next to the settings and
            // wakes the window when a call arrives.
            app.mcp.config_dir = config::default_dir();
            app.mcp.wake = Some(cc.egui_ctx.clone());
            app.mcp.session = mcp_flag;
            app.mcp.instance = instance;
            if let Some((path, doc, extras, notes)) = opened {
                app.set_picture(doc, extras, path);
                app.ui.note_requester = note_requester(LOADED, &notes);
            }
            if let Some(store) = recovery::Store::platform() {
                app.attach_recovery(store);
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
/// The note requester for `notes`, without the notes about DeluxePaint's own chunks (they
/// stay on stderr); `None` when nothing is left to show.
fn note_requester(title: &str, notes: &[String]) -> Option<NoteRequester> {
    let shown: Vec<String> = notes
        .iter()
        .filter(|n| !files::is_quiet_note(n))
        .cloned()
        .collect();
    (!shown.is_empty()).then(|| NoteRequester::new(title, &shown))
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
    let shot = snapshot_rgba(scale, doc, view, scroll, ui, custom_brush);
    // ADR-44: the docked layer panel right of the screen.
    let (w, h, rgba) = layers::with_docked_strip(shot, doc.palette(), ui, view, scale);
    write_png(path, w, h, &rgba)
}

/// The RGBA image [`snapshot`] writes: its width, height and pixels.
fn snapshot_rgba(
    scale: usize,
    doc: &Document,
    view: View,
    scroll: (i32, i32),
    ui: &UiState,
    custom_brush: bool,
) -> (u32, u32, Vec<u8>) {
    // ADR-44: the picture as it shows, all visible layers.
    let mut screen = view.compose(&doc.composite(), scroll, doc.palette(), ui);
    if custom_brush && ui.toolbox_visible {
        let pens = Pens::from_palette(doc.palette());
        let x0 = view.ui_screen().toolbox_x();
        // ADR-42: under the Import requester, which covers the toolbox.
        import::under_import_requester(&mut screen, ui, (view.ux, view.uy), |screen| {
            dpx_ui::toolbox::draw_brush_unselected(screen, x0, ui.brush, &pens, view.ux, view.uy);
        });
    }
    let (sx, sy) = (scale * view.fx as usize, scale * view.fy as usize);
    let mut rgba = to_rgba(&screen, doc.palette(), sx, sy);
    // ADR-42: the Import thumbnail in true colour over the indexed screen.
    if let Some(req) = &ui.import_requester {
        let frame_w = screen.width() as usize * sx;
        req.overlay(
            &mut rgba,
            frame_w,
            view.ux as usize * sx,
            view.uy as usize * sy,
        );
    }
    // ADR-45: the Generate thumbnail likewise.
    generate::overlay(
        ui,
        &mut rgba,
        screen.width() as usize * sx,
        view.ux as usize * sx,
        view.uy as usize * sy,
    );
    (
        screen.width() * sx as u32,
        screen.height() * sy as u32,
        rgba,
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
    /// The Modern skin's live window: region images, hover, the chrome flags during a pass
    /// (ADR-47, modern.rs).
    modern: modern::State,
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
    /// Play range, Control Panel, Move settings and preview (ADR-38, anim_control.rs).
    anim_control: anim_control::State,
    /// The settings file (ADR-33); `None` in tests, snapshots and with `--no-settings`.
    config: Option<config::ConfigStore>,
    /// Autosave, recovery and the quit question (ADR-34).
    autosave: autosave::State,
    /// AnimBrush and Metamorph (ADR-36, animbrush.rs).
    animbrush: animbrush::AnimBrushState,
    /// Effect > Perspective (ADR-37, perspective.rs).
    perspective: perspective::PerspectiveState,
    /// Tooltips over the toolbox and the Control Panel (ADR-41, tooltip.rs).
    tooltip: tooltip::State,
    /// Picture ▸ Load's Import requester for PNG and JPEG (ADR-42, import.rs).
    import: import::ImportState,
    /// The layer panel (ADR-44, layers.rs).
    layers: layers::State,
    /// Picture ▸ Generate.., its jobs, keys and settings (ADR-45, generate.rs).
    imagegen: generate::ImageGenState,
    /// The MCP server and its jobs on the UI thread (ADR-43, mcp.rs).
    mcp: mcp::Host,
    /// Prefs ▸ AI & MCP's connect flows (ADR-43, mcp_prefs.rs).
    mcp_prefs: mcp_prefs::PrefsState,
}

impl PainterApp {
    fn new() -> Self {
        let (w, h) = DisplayMode::PAL_LORES.size();
        let doc = Document::new(w, h, Palette::dpaint_default_32());
        let extras = files::PictureExtras::for_new_picture(doc.palette());
        Self {
            engine: Engine::new(doc),
            ui: UiState::default(),
            modern: modern::State::default(),
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
            anim_control: anim_control::State::default(),
            config: None,
            autosave: autosave::State::default(),
            animbrush: animbrush::AnimBrushState::default(),
            perspective: perspective::PerspectiveState::default(),
            tooltip: tooltip::State::default(),
            import: import::ImportState::default(),
            layers: layers::State::default(),
            imagegen: generate::ImageGenState::default(),
            mcp: mcp::Host::default(),
            mcp_prefs: mcp_prefs::PrefsState::default(),
        }
    }

    /// Replaces the document with a loaded picture (not undoable, like DeluxePaint's Load).
    /// A requester, note, Show Page or playback is open: it takes the pointer and the keys.
    fn modal_open(&self) -> bool {
        let ui = &self.ui;
        ui.choice_requester.is_some()
            || self.requester.is_some()
            || ui.tool_requester.is_some()
            || self.anim.playing.is_some()
            || ui.anim_requester.is_some()
            || ui.palette_requester.is_some()
            || ui.range_requester.is_some()
            || ui.note_requester.is_some()
            || ui.file_requester.is_some()
            || ui.page_size_requester.is_some()
            || ui.import_requester.is_some()
            || ui.generate_requester.is_some()
            || ui.imagegen_setup.is_some()
            || ui.mcp_requester.is_some()
            || self.show_page.is_some()
            || ui.anim_control.is_some()
            || self.anim_control.preview.is_some()
    }

    fn set_picture(&mut self, doc: Document, extras: files::PictureExtras, path: PathBuf) {
        self.menu_data.loaded_palette = Some(doc.palette().clone());
        // Cycling stays on or off; the next clock tick cycles the new palette.
        self.menu_data.cycled = None;
        self.stroke = None;
        self.anim.playing = None;
        self.engine = Engine::new(doc);
        self.extras = extras;
        // ADR-45: a new engine, so the page's own history of details starts over.
        self.menu_data.page_history.shown = Default::default();
        self.mcp.name = None;
        self.last_drawer = path.parent().map(|p| p.to_string_lossy().into_owned());
        self.picture_path = Some(path);
        self.clamp_colors();
        self.scroll = (0, 0);
        self.dirty = true;
        self.mark_clean();
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
                    // ADR-42: only Picture ▸ Load.. asks how to import PNG and JPEG.
                    FileKind::LoadPicture | FileKind::LoadAnim => {
                        self.load_picture(path, kind == FileKind::LoadPicture)
                    }
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

    /// Loads a picture file; with `ask`, PNG and JPEG pictures open the Import requester
    /// instead (ADR-42).
    fn load_picture(&mut self, path: PathBuf, ask: bool) {
        match files::load(&path) {
            Ok(loaded) if ask && import::asks(&loaded.picture) => {
                self.open_import(path, loaded.picture, loaded.notes);
            }
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
        // ADR-46: an IFF export its display mode cannot show is refused with a note.
        if self.refuse_unshowable(&path) {
            return;
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
                    if matches!(files::save_format(&path), Ok(files::SaveFormat::Project)) {
                        self.mark_clean();
                    }
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
        if self.requester.is_some() {
            View::of(requester::SCREEN)
        } else {
            View::of_target(self.engine.document().target())
        }
    }

    /// The screen currently shown and its scale (physical pixels per screen pixel, may be
    /// fractional). The screen is the picture's screen (or the Screen Format requester's own
    /// screen) at the chosen pixel size, or smaller while it does not fit the window (during a
    /// resize, full screen); it is centred in the window (ADR-13).
    ///
    /// In the Modern skin (ADR-47) the same screen is the picture quad in the middle of the
    /// viewport that bar and dock leave free, at a whole number of physical pixels per
    /// displayed pixel, so everything that maps the pointer to the screen works unchanged.
    fn layout(&self) -> (View, f32) {
        let view = self.view();
        let Some(win) = self.window else {
            return (view, 1.0);
        };
        if let Some(modern) = self.modern_layout() {
            return (view, modern.zoom as f32);
        }
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

    /// Prefs ▸ Pixel Size and Cmd/Ctrl + `+`, `−`, `0`: the window snaps to the new size. In
    /// the Modern skin the window stays and the picture changes its size in it (ADR-47), so
    /// the monitor sets no limit there; the layout shows at most what fits the viewport.
    fn set_pixel_size(&mut self, size: PixelSize) {
        let largest = if self.is_modern() {
            PixelSize::MAX
        } else {
            self.largest_step()
        };
        let size = PixelSize(size.0.clamp(PixelSize::MIN, largest));
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
            ReqResponse::Use(command) => {
                match self.engine.execute(command) {
                    Ok(outcome) => {
                        self.requester = None;
                        // Colours beyond the new palette would be invalid.
                        self.clamp_colors();
                        self.clamp_scroll();
                        // ADR-46: palette colours a change to OCS rounds to 12 bits are named.
                        self.ui.note_requester =
                            screen_format::rounded_note(outcome.rounded_colors);
                    }
                    Err(e) => req.set_message(e.message),
                }
            }
        }
        self.dirty = true;
    }

    /// The page as shown: while the screen is composed the Light Table picture or the playing
    /// frame (ADR-28), else the current frame with the stroke preview while painting, else
    /// the current frame. A frame shows all its visible layers (ADR-44); with one layer
    /// that is the page itself, borrowed. Display only: commands read
    /// `self.engine.document().page()`, the active layer.
    fn page(&self) -> Cow<'_, IndexedImage> {
        if let Some(shown) = &self.anim.shown {
            return Cow::Borrowed(shown);
        }
        // ADR-37: the perspective brush, its wire frame and the cross hairs.
        if let Some(shown) = &self.perspective.shown {
            return Cow::Borrowed(shown);
        }
        let doc = self.engine.document();
        match &self.stroke {
            Some(s) => s.shown(doc),
            None => doc.composite(),
        }
    }

    /// Executes a command; a pending stroke is committed first so the order stays intact.
    fn run(&mut self, command: Command) {
        self.finish_stroke();
        // ADR-15 CLR on a fixed background, under the area stencil of ADR-39.
        let command = self.with_background_and_area(command);
        self.execute(command);
    }

    fn execute(&mut self, command: Command) {
        // ADR-39: Stencil ▸ Paint takes painting commands; the area stencil joins the rest.
        let Some(command) = self.area_route(command) else {
            return;
        };
        if let Err(e) = self.engine.execute(command) {
            eprintln!("command rejected: {e}");
        }
        self.dirty = true;
    }

    fn undo(&mut self) {
        self.finish_stroke();
        if self.stencil_paint_undo(false) {
            return;
        }
        // ADR-39: the engine's undo; the area stencil follows its picture. ADR-45: a
        // ReplacePicture step brings back the page's details outside the document.
        let stepped = self.page_history_step(false);
        Self::report(stepped, ErrorCode::NothingToUndo);
        self.clamp_colors();
        self.clamp_scroll();
        self.dirty = true;
    }

    fn redo(&mut self) {
        self.finish_stroke();
        if self.stencil_paint_undo(true) {
            return;
        }
        // ADR-45: and Redo sets them again.
        let stepped = self.page_history_step(true);
        Self::report(stepped, ErrorCode::NothingToRedo);
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
        // ADR-37: perspective mode stamps on the page, ends on a tool, places the centre.
        if self.perspective_press(hit, right) {
            return;
        }
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
            Hit::Page { x, y } => {
                // ADR-38: the next Move starts where the brush was clicked down.
                self.note_click(x, y);
                self.press_page(x, y, right)
            }
            Hit::AnimPanel(hit) => self.panel_press(hit, right),
            Hit::LayerPanel(hit) => self.layer_panel_press(hit, right),
            Hit::TitleBar | Hit::Indicator | Hit::ToolboxBackground => {}
        }
    }

    /// Runs a menu action (also a key that stands for a menu item and ends Stencil ▸ Paint as
    /// the item does: `p`, `M`). Commands not implemented yet do nothing.
    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        // ADR-39: any other menu item ends Stencil ▸ Paint.
        self.menu_item_ends_stencil_paint(action);
        self.dispatch_action(action, ctx);
    }

    /// Runs a menu action without ending Stencil ▸ Paint: for the shortcut keys that keep it
    /// (Alt-m, Alt-n, Alt-b; ADR-39).
    fn dispatch_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Mode(m) => self.ui.mode = m,
            Action::Pref(p) => self.ui.prefs.toggle(p),
            Action::PixelSize(p) => self.set_pixel_size(p),
            // ADR-47 (modern.rs): the other skin from the next pass on; the Modern skin's
            // colour scheme.
            Action::UiStyle => self.toggle_skin(),
            Action::Theme(theme) => self.ui.theme = theme,
            Action::Command(id) => match id {
                // ADR-39 (palette_stencil.rs): its own items, and the Stencil items while there
                // is an area stencil.
                id if self.takes_adr39(id) => self.adr39_action(id),
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
                // ADR-45 (generate.rs).
                "picture.generate" => self.open_generate(),
                "app.quit" => self.request_quit(ctx),
                // ADR-15 (menu_actions.rs).
                "spare.swap" => self.swap_spare(),
                "spare.copy" => self.copy_to_spare(),
                "spare.merge_front" => self.merge_spare(true),
                "spare.merge_back" => self.merge_spare(false),
                "spare.delete" => self.delete_page(),
                "picture.delete" => self.open_file_requester(FileKind::DeleteFile),
                "app.about" => self.show_about(),
                // ADR-43 (mcp_prefs.rs).
                "app.ai_mcp" => self.open_mcp_prefs(),
                "color.cycle" => {
                    self.toggle_cycling(ctx.input(|i| i.time));
                    // This pass's clock tick has run already; the next one keeps it going.
                    ctx.request_repaint();
                }
                "palette.restore" => self.restore_palette(),
                "palette.load" => self.open_file_requester(FileKind::LoadPalette),
                "palette.save" => self.open_file_requester(FileKind::SavePalette),
                "background.fix" => self.fix_background(),
                "background.free" => self.free_background(),
                // ADR-36 (animbrush.rs).
                id if self.animbrush_takes(id) => self.animbrush_action(id),
                // ADR-38 (anim_control.rs).
                "anim.set_range"
                | "anim.panel"
                | "anim.method_compressed"
                | "anim.method_expanded"
                | "anim.move" => self.anim_control_action(id),
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
                // ADR-37 (perspective.rs).
                id if id.starts_with("perspective.") => self.perspective_action(id),
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
        // ADR-43: the server stops with the app and removes its endpoint file;
        // `mcp-app.json` keeps the saved MCP control setting.
        self.mcp_exit();
    }

    /// Runs before every `ui` pass and while the window is hidden: MCP calls are served
    /// even when the window is minimised (ADR-43).
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.mcp_logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ADR-47: during a Modern pass the classic chrome is off for everything that lays
        // out, draws or hit-tests the screen; the Modern chrome lies around it.
        self.modern_begin_pass();
        self.pass(ui);
        self.modern_end_pass();
        // A switch of the skin asked for in this pass (Prefs ▸ Modern UI, F12) happens
        // between two passes, so no pass is half Classic and half Modern.
        self.modern_apply_switch(ui.ctx());
    }
}

impl PainterApp {
    /// One pass of the window: input, keys, then the screen is uploaded and painted.
    fn pass(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // ADR-44: the time for double-clicks; a held panel button goes up.
        self.layers_pass(&ctx);
        self.ui.has_brush = self.custom_brush.is_some();
        self.ui.can_restore = self.picked_brush.is_some();
        self.animbrush_sync(&ctx);
        self.refresh_effects();
        // ADR-37: perspective mode ends with its brush or tool; its title-bar angles.
        self.refresh_perspective(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.close_requested(&ctx);
        }
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
        let modern = self.modern_window(&ctx);
        if self.modal_open() {
            // The chrome takes no input under a requester: nothing in it is raised.
            self.modern_hover(None);
        }
        if modern {
            // ADR-47: the Modern window is sized freely and never snaps. Forgetting the
            // snap state lets the window return to screen × pixel size back in Classic.
            self.snap = Snap::default();
        } else if let Some(size) = self.snap_window(window.points(), now) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        if self.snap.resized_at.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.clamp_scroll();
        self.persist_settings(&ctx, now);
        // Colour cycling runs in every state: under requesters and menus, during playback.
        self.cycle_frame(&ctx, now);

        // The screen at its pixel size (whole and half steps, ADR-13), centred; each screen
        // pixel is shown fx×fy times `scale` physical pixels (its shape in this mode). When
        // the window has snapped, the screen fills it exactly; otherwise the rest is dark.
        let (view, scale) = self.layout();
        let img_w = ((view.screen_w * view.fx) as f32 * scale).round() as i32;
        let img_h = ((view.screen_h * view.fy) as f32 * scale).round() as i32;
        // ADR-44: a docked layer panel right of the screen is centred with it.
        let frame_w = img_w + self.lay_out_layer_strip(view, scale);
        // ADR-47: in the Modern skin the screen is the picture quad of its layout.
        let modern_layout = self.modern_layout();
        let origin_px = match &modern_layout {
            Some(l) => egui::vec2((l.picture.x * ppp).round(), (l.picture.y * ppp).round()),
            None => egui::vec2(
                ((window.w - frame_w) / 2) as f32,
                ((window.h - img_h) / 2) as f32,
            ),
        };
        let origin = area.min + origin_px / ppp;
        let rect = egui::Rect::from_min_size(origin, egui::vec2(img_w as f32, img_h as f32) / ppp);

        if let Some(wait) = self.autosave_tick(now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.5)));
        }
        // ADR-38: Set Range, Move and the Move preview are modal.
        if self.anim_control_pass(&ctx, origin, ppp, scale, view) {
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        if self.ui.choice_requester.is_some() {
            self.choice_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        // ADR-42: the Import requester after Picture ▸ Load.. of a PNG or JPEG.
        if self.ui.import_requester.is_some() {
            self.import_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        // ADR-45: Picture ▸ Generate.. and the Image Generator setup.
        if self.ui.generate_requester.is_some() || self.ui.imagegen_setup.is_some() {
            self.generate_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        // ADR-43: Prefs ▸ AI & MCP.. (its questions use the choice requester above).
        if self.ui.mcp_requester.is_some() {
            self.mcp_prefs_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
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
                let changed = match &modern_layout {
                    // ADR-47: the Modern cards have their own geometry and write the same
                    // menu state.
                    Some(l) => {
                        let p = pos - area.min;
                        dpx_ui_modern::menu::pointer(l, &mut self.ui.menu, p.x, p.y)
                    }
                    None => {
                        let (sx, sy) = view.ui_pos(to_screen(pos));
                        self.ui.menu.pointer(sw, sh, sx, sy)
                    }
                };
                if changed {
                    self.dirty = true;
                }
            }
            let menu_ctx = self.ui.menu_context();
            if press_l && let Some(a) = self.ui.menu.hovered_action(&menu_ctx) {
                self.run_action(a, &ctx);
                if self.modal_open() {
                    // A requester, note or playback took over: the strip goes, so releasing
                    // the right button later does not run the item a second time.
                    self.ui.menu.close();
                    self.dirty = true;
                }
            }
            if self.ui.menu.active && !down_r {
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
            // ADR-47: what the Modern window has under the pointer: its chrome, the picture
            // quad (the screen) or the canvas around it. `None` in the Classic skin.
            let spot = modern_layout.as_ref().map(|l| {
                let p = pos - area.min;
                dpx_ui_modern::hit(l, p.x, p.y)
            });
            let on_screen = matches!(spot, None | Some(dpx_ui_modern::Spot::Picture { .. }));
            // Screen pixels are page pixels, shifted by the scroll position (or magnified,
            // ADR-14; `None` on the Magnify divider). The screen may be larger than the page
            // (ADR-13): there the pointer is off the page.
            let on = self.screen_to_page(view, (sx, sy)).filter(|_| on_screen);
            if on.is_some() {
                self.pointer_page = on;
            }
            // Every picture shown has the page's size.
            let page = self.engine.document().page().bounds();
            let inside = on.filter(|&(px, py)| page.contains(px, py));
            let hit = match spot {
                // The Modern chrome answers with the Classic skin's hit values.
                Some(dpx_ui_modern::Spot::Ui(hit)) => Some(hit),
                Some(dpx_ui_modern::Spot::Canvas) => None,
                // The screen: in Modern without the classic chrome (see `ui()`), so only
                // the page and what is still shown in the Classic look inside the quad.
                _ => match ui_screen.hit_test(self.palette().len(), &self.ui, ux, uy) {
                    Hit::Page { .. } => inside.map(|(x, y)| Hit::Page { x, y }),
                    h => Some(h),
                },
            };
            self.modern_hover(spot);
            // ADR-41 (tooltip.rs).
            self.tooltip_hover(&ctx, hit);
            // ADR-44: the wheel over the layer panel scrolls its list.
            self.layer_hover(hit);
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
            if (press_l || press_r) && self.layer_rename_press(hit) {
                // ADR-44: the press ended a layer rename (it takes the name, as Return).
            } else if press_r && hit == Some(Hit::TitleBar) {
                match &modern_layout {
                    // ADR-47: the Modern bar's pull-downs, held with the right button too.
                    Some(l) => {
                        let p = pos - area.min;
                        dpx_ui_modern::menu::open(l, &mut self.ui.menu, p.x, p.y);
                    }
                    None => self.ui.menu.open(sw, sh, ux, uy),
                }
                self.dirty = true;
            } else if press_l || press_r {
                if let Some(hit) = hit {
                    self.handle_press(hit, press_r);
                }
            } else if (down_l || down_r) && self.anim_control.knob.is_some() {
                // ADR-38: the Control Panel's slider knob follows the pointer.
                self.drag_knob(ux);
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
            self.tooltip_hover(&ctx, None);
            self.layer_hover(None);
            self.modern_hover(None);
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
        // ADR-44: while a layer name is edited, its gadget takes every key.
        if self.layer_rename_keys(&ctx) {
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        // Tool keys first: while typing text or magnifying they own the keyboard. Keys that
        // arrive in the pass whose Esc ends the text are text keys too.
        let was_typing = self.typing();
        // ADR-37: the keypad belongs to perspective mode (the tools do not see its keys).
        self.perspective_keys(&ctx);
        let keys_taken = self.tool_keys(&ctx);
        if !keys_taken {
            self.page_scroll_input(&ctx, ppp, scale, view);
        }
        if self.show_page.is_some() {
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }
        ctx.input(|i| {
            // ADR-47: in a Modern pass the two flags are kept aside (bar and dock).
            if i.key_pressed(egui::Key::F10) {
                let (title, toolbox) = self.chrome_flags();
                let show = !(*toolbox || *title);
                *toolbox = show;
                *title = show;
                self.dirty = true;
            }
            if i.key_pressed(egui::Key::F9) {
                let (title, _) = self.chrome_flags();
                *title = !*title;
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
        // ADR-47: F12 or Cmd/Ctrl+Shift+U switch between the Classic and the Modern skin
        // (not while text is typed or a key of this pass opened a requester).
        let skin_key = ctx.input(|i| {
            let chord = i.modifiers.command && i.modifiers.shift && !i.modifiers.alt;
            i.key_pressed(egui::Key::F12) || (chord && i.key_pressed(egui::Key::U))
        });
        if skin_key && !was_typing && !self.typing() && !self.modal_open() {
            self.toggle_skin();
        }
        if !was_typing {
            self.menu_keys(&ctx);
            self.anim_keys(&ctx);
            self.animbrush_keys(&ctx);
            self.anim_control_keys(&ctx);
            self.palette_stencil_keys(&ctx);
            // ADR-44: `L` shows or hides the layer panel.
            self.layer_keys(&ctx);
        }
        // U = undo (as in DeluxePaint), Shift+U = redo (DeluxePainter addition).
        // Cmd/Ctrl+Shift+U is the skin switch above, not a redo.
        let (undo_key, shift) = ctx.input(|i| {
            let skin_chord = i.modifiers.command && i.modifiers.shift && !i.modifiers.alt;
            (
                i.key_pressed(egui::Key::U) && !skin_chord,
                i.modifiers.shift,
            )
        });
        // A modal opened by a key of this pass (e.g. the "Nothing to play" note) takes the rest.
        if undo_key && !was_typing && !self.typing() && !self.modal_open() {
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
    /// F11 switch full screen. In the Modern skin `+` / `−` make the picture one step larger
    /// or smaller in the window and `0` as large as fits it (ADR-47).
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
        if self.is_modern() {
            // ADR-47: the same keys zoom the picture; the window keeps its size.
            if bigger {
                self.modern_zoom(1);
            }
            if smaller {
                self.modern_zoom(-1);
            }
            if auto {
                self.modern_zoom_to_fit();
            }
        } else {
            if bigger {
                self.step_pixel_size(1);
            }
            if smaller {
                self.step_pixel_size(-1);
            }
            if auto {
                self.set_pixel_size(PixelSize(self.largest_step()));
            }
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
                    // ADR-45: pasted digits for the custom size (never the U/C/R shortcuts).
                    egui::Event::Paste(text) => keys.extend(dpx_ui::paste_keys(text)),
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
                egui::Event::MouseWheel { unit, delta, .. } if self.layer_wheel(unit, delta) => {}
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
        self.modal_events_with(ctx, origin, ppp, scale, view, at, true)
    }

    /// [`Self::modal_events`]; without `enter_repeats`, a held Return's auto-repeat is left
    /// out, so only a new press answers the requester (ADR-42: the Return that picked the
    /// file in the file requester must not also answer the Import requester it opened).
    #[allow(clippy::too_many_arguments)]
    fn modal_events_with(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
        at: (i32, i32),
        enter_repeats: bool,
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
                // Cmd/Ctrl+V (ADR-45): into the active text field only, never a shortcut.
                egui::Event::Paste(text) => {
                    out.extend(dpx_ui::paste_keys(&text).into_iter().map(ReqEvent::Key));
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    ..
                } => {
                    if repeat && !enter_repeats && key == egui::Key::Enter {
                        continue;
                    }
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
    /// opens it when no other requester, view or menu is open and the Text tool is not
    /// typing; while it is open, it gets the
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
            // `p` typed by the Text tool is a letter (judged before the tool keys run, so a
            // `p` in the pass of the Esc that ends the text is still text, ADR-35).
            let busy = self.ui.note_requester.is_some()
                || self.ui.file_requester.is_some()
                || self.ui.page_size_requester.is_some()
                || self.show_page.is_some()
                || self.ui.menu.active
                || self.typing();
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
                // Against the document palette: the shown one may be cycling.
                if req.colors() != self.engine.document().palette().colors() {
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
            .compose(&self.page(), self.scroll, &palette, &self.ui);
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
        // ADR-41: no tooltip over (or under) a menu, requester or playback opened this pass.
        self.tooltip_guard(ctx);
        // The shown cycling palette follows palette and range changes made earlier in this
        // pass (the clock at the top of `ui()` ran before them).
        self.cycle_tick(ctx.input(|i| i.time));
        // ADR-44: a document changed elsewhere (MCP, generation) redraws the layer panel.
        self.follow_document();
        // Upload the screen at native resolution; the GPU scales it with nearest filtering.
        if self.dirty || self.texture.is_none() {
            self.prepare_perspective_view();
            self.prepare_anim_view();
            self.prepare_stencil_view();
            // ADR-44: after the Control Panel's state (the overlay keeps above it).
            self.sync_layer_panel();
            self.upload_layer_strip(ctx);
            let (screen, rgba) = match (&self.requester, &self.show_page) {
                (Some(req), _) => {
                    let screen = req.render();
                    let rgba = to_rgba(&screen, &requester::palette(), 1, 1);
                    (screen, rgba)
                }
                (None, Some(sp)) => {
                    let screen = sp.render(&self.page(), self.palette().len());
                    let rgba = to_rgba(&screen, self.palette(), 1, 1);
                    (screen, rgba)
                }
                (None, None) => {
                    let screen = self.compose_screen();
                    let mut rgba = to_rgba(&screen, self.palette(), 1, 1);
                    // ADR-42: the Import thumbnail in true colour over the indexed screen.
                    self.import_overlay(&mut rgba, screen.width() as usize);
                    self.generate_overlay(&mut rgba, screen.width() as usize);
                    (screen, rgba)
                }
            };
            self.anim.shown = None;
            self.perspective.shown = None;
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

        // ADR-47: the Modern chrome around the screen instead of the dark surround.
        if self.is_modern() {
            self.modern_paint(ui, ctx, rect, area);
            return;
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
        // ADR-44: the docked layer panel right of the screen.
        self.paint_layer_strip(painter, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One full `ui()` pass at time `t` with `events`; returns when egui wants the next
    /// pass (the real window repaints only then, unless the pointer moves).
    pub(crate) fn ui_pass(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        events: Vec<egui::Event>,
    ) -> std::time::Duration {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 1024.0),
            )),
            time: Some(t),
            events,
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        // As eframe: `logic` (MCP jobs, ADR-43) before `ui` in every pass.
        let mut out = ctx.run_ui(raw, |ui| {
            eframe::App::logic(app, ui.ctx(), &mut frame);
            eframe::App::ui(app, ui, &mut frame)
        });
        // The font atlas and screen uploads are not needed in a test.
        out.textures_delta.clear();
        out.viewport_output[&egui::ViewportId::ROOT].repaint_delay
    }

    /// The window of a [`ui_pass_with`] pass (ADR-47: the Modern skin's window is sized
    /// freely, is drawn at the device scale and follows the system's theme).
    #[derive(Clone, Copy, Debug)]
    pub(crate) struct TestWindow {
        /// The drawing area in points.
        pub size: egui::Vec2,
        /// Physical pixels per point.
        pub ppp: f32,
        /// What the system reports as its theme.
        pub system_theme: Option<egui::Theme>,
        /// The largest texture side the GPU takes.
        pub max_texture_side: usize,
    }

    /// As [`ui_pass`], in `window`. Returns everything the pass asked the window for: its
    /// commands (a new size, a minimum size) and the repaint delay.
    pub(crate) fn ui_pass_with(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        events: Vec<egui::Event>,
        window: TestWindow,
    ) -> egui::ViewportOutput {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, window.size)),
            time: Some(t),
            events,
            system_theme: window.system_theme,
            max_texture_side: Some(window.max_texture_side),
            ..Default::default()
        };
        if let Some(root) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
            root.native_pixels_per_point = Some(window.ppp);
        }
        let mut frame = eframe::Frame::_new_kittest();
        let mut out = ctx.run_ui(raw, |ui| {
            eframe::App::logic(app, ui.ctx(), &mut frame);
            eframe::App::ui(app, ui, &mut frame)
        });
        out.textures_delta.clear();
        out.viewport_output
            .remove(&egui::ViewportId::ROOT)
            .expect("the root viewport")
    }

    pub(crate) fn key_event(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// Presses and releases `key` at `t`, then runs passes only when egui asks for them
    /// (as the idle real window does) until `until`; `seen` is called after every pass.
    pub(crate) fn press_and_idle(
        app: &mut PainterApp,
        ctx: &egui::Context,
        key: egui::Key,
        t: f64,
        until: f64,
        mut seen: impl FnMut(&PainterApp),
    ) {
        let text = match key {
            egui::Key::Num4 => vec![egui::Event::Text("4".into())],
            _ => vec![],
        };
        let mut events = vec![key_event(key, true)];
        events.extend(text);
        let mut delay = ui_pass(app, ctx, t, events);
        let mut t = t + 1.0 / 60.0;
        delay = delay.min(ui_pass(app, ctx, t, vec![key_event(key, false)]));
        while t < until {
            assert!(
                delay < std::time::Duration::from_secs(1),
                "no repaint requested at t = {t}"
            );
            // At most one pass per display frame.
            t += delay.as_secs_f64().max(1.0 / 60.0);
            delay = ui_pass(app, ctx, t, vec![]);
            seen(app);
        }
    }

    #[test]
    fn key_4_plays_every_frame_through_the_real_ui_loop() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        for frame in 0..3 {
            app.engine.execute(Command::GotoFrame { frame }).unwrap();
            let color = frame as u8 + 1;
            app.engine.execute(Command::Clear { color }).unwrap();
        }
        app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let mut shown = std::collections::BTreeSet::new();
        let mut counters = std::collections::BTreeSet::new();
        press_and_idle(&mut app, &ctx, egui::Key::Num4, 0.1, 1.0, |app| {
            shown.insert(app.anim.playing.expect("still playing").shown);
            counters.extend(app.ui.frame_counter);
        });
        assert_eq!(shown.len(), 3, "all frames shown: {shown:?}");
        assert_eq!(counters.len(), 3, "the title counter follows: {counters:?}");
        // Esc stops; the frame on screen becomes the current frame.
        ui_pass(
            &mut app,
            &ctx,
            1.1,
            vec![key_event(egui::Key::Escape, true)],
        );
        assert!(app.anim.playing.is_none());
    }

    #[test]
    fn play_on_a_single_frame_says_how_to_add_frames() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.run(Command::Clear { color: 3 });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // `4`, `3` and `U` in one pass: the note takes the rest of the keys (no Go to Frame
        // requester on top of it, no undo behind it).
        let mut keys = vec![
            key_event(egui::Key::Num4, true),
            egui::Event::Text("4".into()),
        ];
        keys.push(key_event(egui::Key::Num3, true));
        keys.push(key_event(egui::Key::U, true));
        ui_pass(&mut app, &ctx, 0.1, keys);
        assert!(app.anim.playing.is_none());
        assert!(app.ui.anim_requester.is_none(), "no requester on the note");
        assert_eq!(app.page().get(0, 0), Some(3), "no undo behind the note");
        let note = app.ui.note_requester.as_ref().expect("a note");
        assert_eq!(note.title(), anim::NOTHING_TO_PLAY);
        ui_pass(&mut app, &ctx, 0.2, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.ui.note_requester.is_none(), "Return closes it");
        // The menu items do the same.
        for id in ["anim.play_once", "anim.ping_pong"] {
            app.run_action(Action::Command(id), &ctx);
            assert!(app.ui.note_requester.take().is_some(), "{id}");
        }
        assert_eq!(app.engine.document().frame_count(), 1);
    }

    /// The window point (1280×1024 test window, 1 pixel per point) over UI pixel (`x`, `y`),
    /// after a pass has laid the screen out.
    pub(crate) fn ui_point(app: &PainterApp, (x, y): (i32, i32)) -> egui::Pos2 {
        let (view, scale) = app.layout();
        let img_w = ((view.screen_w * view.fx) as f32 * scale).round() as i32;
        let img_h = ((view.screen_h * view.fy) as f32 * scale).round() as i32;
        // ADR-44: centred together with a docked layer panel.
        let frame_w = img_w + app.layer_strip_px(view, scale);
        let (ww, wh) = app.window.map_or((1280, 1024), |w| (w.w, w.h));
        let (ox, oy) = (((ww - frame_w) / 2) as f32, ((wh - img_h) / 2) as f32);
        let sx = (x * view.ux) as f32 + 0.5;
        let sy = (y * view.uy) as f32 + 0.5;
        egui::pos2(
            ox + sx * scale * view.fx as f32,
            oy + sy * scale * view.fy as f32,
        )
    }

    /// UI pixels over the title of the menu holding `id`, over the item whose submenu holds
    /// it, and over `id` itself (from the menu geometry, checked with the pointer tracking).
    pub(crate) fn menu_path(app: &PainterApp, id: &'static str) -> [(i32, i32); 3] {
        let (w, h) = app.view().ui_size();
        let wanted = Action::Command(id);
        let (m, item, sub) = MENUS
            .iter()
            .enumerate()
            .flat_map(|(m, menu)| menu.items.iter().enumerate().map(move |(i, it)| (m, i, it)))
            .find_map(|(m, i, it)| match it.entry {
                menu::Entry::Submenu(items) => items
                    .iter()
                    .position(|s| matches!(s.entry, menu::Entry::Action(a) if a == wanted))
                    .map(|s| (m, i, s)),
                menu::Entry::Action(_) => None,
            })
            .expect("in a submenu");
        let title_x = (0..w)
            .find(|&x| {
                let mut state = dpx_ui::MenuState::default();
                state.open(w, h, x, 1);
                state.menu == Some(m)
            })
            .expect("menu title");
        let mbox = menu::menu_box(m, w, h);
        let menu::Entry::Submenu(items) = MENUS[m].items[item].entry else {
            unreachable!()
        };
        let sbox = menu::submenu_box(&mbox, item, items, w, h);
        let path = [
            (title_x, 1),
            (mbox.x + mbox.w / 2, mbox.item_top(item) + 1),
            (sbox.x + sbox.w / 2, sbox.item_top(sub) + 1),
        ];
        // The pointer tracking agrees.
        let mut state = dpx_ui::MenuState::default();
        state.open(w, h, path[0].0, path[0].1);
        for &(x, y) in &path[1..] {
            state.pointer(w, h, x, y);
        }
        assert_eq!(state.hovered_action(&app.ui.menu_context()), Some(wanted));
        path
    }

    #[test]
    fn play_picked_with_the_left_button_in_the_menu_runs_once() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let [title, control, play] = menu_path(&app, "anim.play").map(|p| ui_point(&app, p));
        let button = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        use egui::PointerButton::{Primary, Secondary};
        // Right button held on the title bar, over Anim ▸ Control ▸ Play, left click on it.
        ui_pass(
            &mut app,
            &ctx,
            0.1,
            vec![
                egui::Event::PointerMoved(title),
                button(title, Secondary, true),
            ],
        );
        assert!(app.ui.menu.active);
        ui_pass(
            &mut app,
            &ctx,
            0.2,
            vec![egui::Event::PointerMoved(control)],
        );
        ui_pass(&mut app, &ctx, 0.3, vec![egui::Event::PointerMoved(play)]);
        ui_pass(&mut app, &ctx, 0.4, vec![button(play, Primary, true)]);
        assert!(app.ui.note_requester.is_some(), "one frame: the note");
        assert!(!app.ui.menu.active, "the note closed the strip");
        // Both buttons go up under the note; Return closes it; nothing runs again.
        ui_pass(
            &mut app,
            &ctx,
            0.5,
            vec![button(play, Primary, false), button(play, Secondary, false)],
        );
        ui_pass(&mut app, &ctx, 0.6, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.ui.note_requester.is_none());
        ui_pass(&mut app, &ctx, 0.7, vec![]);
        assert!(
            app.ui.note_requester.is_none(),
            "Play did not run a second time"
        );
    }

    #[test]
    fn tab_cycles_colours_through_the_real_ui_loop() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        // Colours 8-15 at 11 steps per second, like a waterfall picture.
        app.extras.ranges = vec![dpx_iff::ilbm::ColorRange {
            rate: 3072,
            flags: 1,
            low: 8,
            high: 15,
        }];
        let doc = app.engine.document().palette().clone();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let mut palettes = std::collections::BTreeSet::new();
        press_and_idle(&mut app, &ctx, egui::Key::Tab, 0.1, 1.0, |app| {
            palettes.insert(format!("{:?}", &app.palette().colors()[8..16]));
        });
        assert!(palettes.len() >= 8, "{} palettes in 0.9 s", palettes.len());
        assert_eq!(app.engine.document().palette(), &doc, "display only");
    }

    /// A waterfall-like range: colours 8-15 at 11 steps per second.
    fn waterfall_range(app: &mut PainterApp) {
        app.extras.ranges = vec![dpx_iff::ilbm::ColorRange {
            rate: 3072,
            flags: 1,
            low: 8,
            high: 15,
        }];
    }

    /// Runs a pass with `first` at `t`, then passes only when egui asks for one until
    /// `until`; returns the distinct shown palettes of colours 8-15 seen on the way.
    fn idle_palettes(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        until: f64,
        first: Vec<egui::Event>,
    ) -> usize {
        let mut delay = ui_pass(app, ctx, t, first);
        let mut t = t;
        let mut seen = std::collections::BTreeSet::new();
        while t < until {
            assert!(
                delay < std::time::Duration::from_secs(1),
                "no repaint requested at t = {t}"
            );
            t += delay.as_secs_f64().max(1.0 / 60.0);
            delay = ui_pass(app, ctx, t, vec![]);
            seen.insert(format!("{:?}", &app.palette().colors()[8..16]));
        }
        seen.len()
    }

    #[test]
    fn cycling_runs_under_requesters_and_menus() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        waterfall_range(&mut app);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        press_and_idle(&mut app, &ctx, egui::Key::Tab, 0.1, 0.2, |_| {});
        // A note after a load: Tab there does not reach the paint screen, but the colours
        // keep moving behind it.
        let note = ["ANNO chunk (110 bytes) not kept".to_string()];
        app.ui.note_requester = note_requester(LOADED, &note);
        assert!(app.ui.note_requester.is_some(), "the note is open");
        let n = idle_palettes(&mut app, &ctx, 0.3, 1.3, vec![]);
        assert!(n >= 8, "under the note: {n}");
        ui_pass(&mut app, &ctx, 1.4, vec![key_event(egui::Key::Enter, true)]);
        assert!(app.ui.note_requester.is_none());
        // The Range requester cycles the ranges being edited: switching the range off there
        // stops it at once; Cancel brings the kept one back.
        app.open_ranges();
        let n = idle_palettes(&mut app, &ctx, 1.5, 2.5, vec![]);
        assert!(n >= 8, "under the Range requester: {n}");
        let active = dpx_ui::RangeRequester::active_rect();
        app.range_event(ReqEvent::Press(active.x + 2, active.y + 2));
        app.range_event(ReqEvent::Release);
        let n = idle_palettes(&mut app, &ctx, 2.6, 3.0, vec![]);
        assert_eq!(n, 1, "range switched off in the requester");
        assert_eq!(app.palette(), app.engine.document().palette());
        app.range_event(ReqEvent::Key(ReqKey::Escape));
        assert!(app.ui.range_requester.is_none());
        let n = idle_palettes(&mut app, &ctx, 3.1, 4.1, vec![]);
        assert!(n >= 8, "the kept range after Cancel: {n}");
        // The menu strip, held open with the right button over the title bar.
        let title = egui::pos2(640.0, 4.0);
        let hold = vec![
            egui::Event::PointerMoved(title),
            egui::Event::PointerButton {
                pos: title,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        ui_pass(&mut app, &ctx, 4.2, hold);
        assert!(app.ui.menu.active, "the strip opened");
        let n = idle_palettes(&mut app, &ctx, 4.3, 5.3, vec![]);
        assert!(n >= 8, "with the menu strip: {n}");
        assert!(app.ui.menu.active, "the strip stayed open");
    }

    #[test]
    fn cycling_plays_along_with_the_animation_and_tab_switches_it_there() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        waterfall_range(&mut app);
        app.engine
            .execute(Command::SetFrameCount { count: 3 })
            .unwrap();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        press_and_idle(&mut app, &ctx, egui::Key::Tab, 0.1, 0.2, |_| {});
        press_and_idle(&mut app, &ctx, egui::Key::Num4, 0.3, 0.4, |_| {});
        assert!(app.anim.playing.is_some());
        let n = idle_palettes(&mut app, &ctx, 0.5, 1.5, vec![]);
        assert!(n >= 8, "while playing: {n}");
        // Tab while playing: cycling off, the playback goes on.
        ui_pass(&mut app, &ctx, 1.6, vec![key_event(egui::Key::Tab, true)]);
        ui_pass(&mut app, &ctx, 1.62, vec![key_event(egui::Key::Tab, false)]);
        assert!(app.menu_data.cycle_start.is_none());
        assert!(app.anim.playing.is_some());
        assert_eq!(app.palette(), app.engine.document().palette());
    }

    #[test]
    fn cycle_from_the_menu_asks_for_the_next_pass() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        waterfall_range(&mut app);
        // Settle: the first passes ask for repaints of their own (window size).
        let mut t = 0.0;
        while ui_pass(&mut app, &ctx, t, vec![]) != std::time::Duration::MAX {
            t += 0.01;
            assert!(t < 1.0, "the window does not settle");
        }
        // The menu runs its action after this pass's clock tick.
        let raw = egui::RawInput {
            time: Some(t + 0.1),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            app.run_action(Action::Command("color.cycle"), &ctx);
        });
        out.textures_delta.clear();
        let delay = out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        assert!(delay < std::time::Duration::from_secs(1), "{delay:?}");
        let from = t + 0.1 + delay.as_secs_f64();
        let n = idle_palettes(&mut app, &ctx, from, from + 1.0, vec![]);
        assert!(n >= 8, "{n}");
    }

    #[test]
    fn a_palette_change_shows_cycled_in_the_same_pass() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        waterfall_range(&mut app);
        let before = app.engine.document().palette().clone();
        // Colours 8-15 become shades of red.
        let mut colors = before.colors().to_vec();
        for (i, c) in colors[8..16].iter_mut().enumerate() {
            *c = dpx_core::Rgb8::new(100 + i as u8 * 20, 0, 0);
        }
        app.run(Command::SetPalette { colors });
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        press_and_idle(&mut app, &ctx, egui::Key::Tab, 0.1, 0.5, |_| {});
        // `U` undoes the red after this pass's clock tick: the screen of the same pass
        // already shows the old colours cycling, not the red ones.
        ui_pass(&mut app, &ctx, 0.6, vec![key_event(egui::Key::U, true)]);
        assert_eq!(app.engine.document().palette(), &before);
        let old = &before.colors()[8..16];
        for c in &app.palette().colors()[8..16] {
            assert!(old.contains(c), "{c:?} is not one of the old colours");
        }
    }

    #[test]
    fn a_load_while_cycling_cycles_the_new_palette() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        waterfall_range(&mut app);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        press_and_idle(&mut app, &ctx, egui::Key::Tab, 0.1, 0.5, |_| {});
        assert!(app.menu_data.cycled.is_some());
        // A picture with another palette and no ranges: what is shown is its own palette,
        // never the old picture's cycled one.
        let colors: Vec<dpx_core::Rgb8> = (0..32u8)
            .map(|i| dpx_core::Rgb8::new(i * 8, 0, 255 - i * 8))
            .collect();
        let palette = Palette::new(colors).unwrap();
        let doc = Document::from_picture(
            IndexedImage::new(320, 256, 0),
            palette.clone(),
            Target::default(),
        );
        app.set_picture(
            doc,
            files::PictureExtras::default(),
            PathBuf::from("other.iff"),
        );
        assert_eq!(app.palette(), &palette);
        ui_pass(&mut app, &ctx, 0.6, vec![]);
        assert_eq!(app.palette(), &palette, "no ranges: nothing moves");
        assert!(app.menu_data.cycle_start.is_some(), "cycling stays on");
    }

    #[test]
    fn mixer_ok_without_edits_adds_no_undo_step_while_cycling() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        waterfall_range(&mut app);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        press_and_idle(&mut app, &ctx, egui::Key::Tab, 0.1, 0.5, |_| {});
        assert_ne!(
            app.palette(),
            app.engine.document().palette(),
            "cycling shows"
        );
        assert!(!app.engine.can_undo());
        app.run_action(Action::Command("palette.mixer"), &ctx);
        app.palette_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.palette_requester.is_none());
        assert!(!app.engine.can_undo(), "Ok without edits changes nothing");
    }

    #[test]
    fn paint_screen_keys_wait_while_typing_text() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        app.select_tool(Tool::Text, false);
        app.handle_press(Hit::Page { x: 20, y: 20 }, false);
        app.release();
        assert!(app.typing());
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let typed = |c: &str| vec![egui::Event::Text(c.into())];
        let mut j = vec![key_event(egui::Key::J, true)];
        j.extend(typed("j"));
        ui_pass(&mut app, &ctx, 0.1, j);
        ui_pass(&mut app, &ctx, 0.12, vec![key_event(egui::Key::J, false)]);
        ui_pass(&mut app, &ctx, 0.2, typed("?"));
        ui_pass(&mut app, &ctx, 0.3, vec![key_event(egui::Key::Tab, true)]);
        assert!(app.menu_data.spare.is_none(), "j is typed, not Swap Spare");
        assert!(app.ui.note_requester.is_none(), "? is typed, not About");
        assert!(app.menu_data.cycle_start.is_none(), "Tab does not cycle");
        assert!(app.typing());
        // `j` in the same pass as the Esc that ends the text is still a text key.
        let mut end = typed("j");
        end.push(key_event(egui::Key::J, true));
        end.push(key_event(egui::Key::Escape, true));
        ui_pass(&mut app, &ctx, 0.4, end);
        assert!(!app.typing());
        assert!(
            app.menu_data.spare.is_none(),
            "j with Esc is not Swap Spare"
        );
        // `p` is a letter while typing, not the Color Mixer (which would end the text).
        app.handle_press(Hit::Page { x: 20, y: 40 }, false);
        app.release();
        assert!(app.typing());
        let mut p = vec![key_event(egui::Key::P, true)];
        p.extend(typed("p"));
        ui_pass(&mut app, &ctx, 0.5, p);
        ui_pass(&mut app, &ctx, 0.52, vec![key_event(egui::Key::P, false)]);
        assert!(app.ui.palette_requester.is_none(), "p is typed");
        assert!(app.typing());
        // Also in the pass of the Esc that ends the text.
        let mut end = vec![key_event(egui::Key::P, true)];
        end.extend(typed("p"));
        end.push(key_event(egui::Key::Escape, true));
        ui_pass(&mut app, &ctx, 0.6, end);
        assert!(!app.typing());
        assert!(app.ui.palette_requester.is_none(), "p with Esc is typed");
    }

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
        // Picture ▸ Flip ▸ Vert (Flip is item 6 since Generate.., ADR-45).
        assert_eq!(parse_menu_spec("0:6:1"), Some((0, Some(6), Some(1))));
        assert_eq!(parse_menu_spec("0:5:1"), None, "Print.. has no submenu");
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
        assert_eq!(&*app.page(), &img);
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
            chipset: None,
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
        // An ILBM with a chunk DeluxePainter does not keep, one with only DeluxePaint's own
        // chunks (settings, thumbnail) and a plain one.
        let img = IndexedImage::new(320, 256, 0);
        let mut src = dpx_iff::Ilbm::from_indexed(img.clone(), Palette::dpaint_default_32());
        let chunk = |id: &[u8; 4]| dpx_iff::ilbm::RawChunk {
            id: *id,
            data: vec![0; 10],
        };
        src.other_chunks.push(chunk(b"ANNO"));
        let write = |name: &str, i: &dpx_iff::Ilbm| {
            let bytes = dpx_iff::ilbm::write(i, dpx_iff::Compression::ByteRun1).unwrap();
            std::fs::write(dir.join(name), bytes).unwrap();
        };
        write("Notes.iff", &src);
        src.other_chunks = vec![chunk(b"DPPS"), chunk(b"DPPV"), chunk(b"TINY")];
        write("DPaint.iff", &src);
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
            .compose(&app.page(), app.scroll, app.palette(), &app.ui);
        let ok = (ox + 128, oy + req.height() - 10);
        app.note_event(ReqEvent::Press(ok.0 - ox, ok.1 - oy));
        app.note_event(ReqEvent::Release);
        assert!(app.ui.note_requester.is_none(), "Ok closes");
        let without = app
            .view()
            .compose(&app.page(), app.scroll, app.palette(), &app.ui);
        assert_ne!(with, without);

        // A picture without notes opens none, and neither does one with only DeluxePaint's
        // own chunks (their notes go to stderr).
        for name in ["DPaint.iff", "Plain.iff"] {
            app.run_action(Action::Command("picture.load"), &ctx);
            type_into(&mut app, name);
            app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
            assert!(app.ui.file_requester.is_none(), "{name} loaded");
            assert!(app.ui.note_requester.is_none(), "{name}: no note");
        }

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
        assert!(app.ui.file_requester.is_none(), "closed");
        // ADR-42: the Import requester asks first; Top left (Tab, Down), then Return.
        assert!(app.ui.import_requester.is_some());
        app.import_event(ReqEvent::Key(ReqKey::Tab));
        app.import_event(ReqEvent::Key(ReqKey::Down));
        app.import_event(ReqEvent::Key(ReqKey::Enter));
        assert!(app.ui.import_requester.is_none(), "loaded");
        assert_eq!(app.engine.document().target(), before, "kept the screen");
        // 320x256 LoRes: 7:6 shown on square pixels → 299x256, on a screen-sized page.
        assert_eq!((app.page().width(), app.page().height()), (320, 256));
        let at = |x: i32| app.palette().get(app.page().get(x, 100).unwrap()).unwrap();
        assert_eq!(at(10), dpx_core::Rgb8::new(200, 40, 40));
        assert_eq!(at(290), dpx_core::Rgb8::new(40, 40, 200));
        assert_eq!(
            at(310),
            Palette::dpaint_default_32().get(0).unwrap(),
            "rest of the page: the BG colour"
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
        let preview = app.page().into_owned();
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
        assert_eq!(&*app.page(), &preview);

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
        assert_eq!(&*app.page(), &preview);
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
            .compose(&app.page(), (0, 0), app.palette(), &app.ui);
        assert_eq!(screen.get(5, 100), Some(0));
        let mut marked = app.page().into_owned();
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

    // ---- ADR-38: Anim control (Set Range, Panel, Method, Move, Shift-1 … Shift-5, M,
    // Alt-a) through the real ui() loop.

    /// A key pressed and let go in one pass with `modifiers` held, as egui-winit reports
    /// it: the logical key (what the layout types, when egui has a name for it, else the
    /// physical key), the physical key and the typed text.
    pub(crate) fn chord(
        key: egui::Key,
        physical: egui::Key,
        modifiers: egui::Modifiers,
        text: &str,
    ) -> Vec<egui::Event> {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: Some(physical),
            pressed,
            repeat: false,
            modifiers,
        };
        let mut events = vec![event(true)];
        if !text.is_empty() {
            events.push(egui::Event::Text(text.into()));
        }
        events.push(event(false));
        events
    }

    /// A plain key pressed and let go in one pass.
    pub(crate) fn tap(key: egui::Key) -> Vec<egui::Event> {
        vec![key_event(key, true), key_event(key, false)]
    }

    pub(crate) fn pointer_button(
        pos: egui::Pos2,
        button: egui::PointerButton,
        pressed: bool,
    ) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A click with `button` on UI pixel `at`: down in one pass, up in the next; returns
    /// the repaint delay egui asks for after it.
    pub(crate) fn click_ui(
        app: &mut PainterApp,
        ctx: &egui::Context,
        t: f64,
        at: (i32, i32),
        button: egui::PointerButton,
    ) -> std::time::Duration {
        let pos = ui_point(app, at);
        ui_pass(
            app,
            ctx,
            t,
            vec![
                egui::Event::PointerMoved(pos),
                pointer_button(pos, button, true),
            ],
        );
        ui_pass(app, ctx, t + 0.02, vec![pointer_button(pos, button, false)])
    }

    /// Picks the menu item at the end of `path` (UI pixels, the menu title first) as on the
    /// Amiga: right button down on the title bar, along the path, up over the item.
    pub(crate) fn pick(app: &mut PainterApp, ctx: &egui::Context, t: f64, path: &[(i32, i32)]) {
        use egui::PointerButton::Secondary;
        let points: Vec<egui::Pos2> = path.iter().map(|&p| ui_point(app, p)).collect();
        ui_pass(
            app,
            ctx,
            t,
            vec![
                egui::Event::PointerMoved(points[0]),
                pointer_button(points[0], Secondary, true),
            ],
        );
        assert!(app.ui.menu.active, "the menu strip");
        for (i, &p) in points.iter().enumerate().skip(1) {
            ui_pass(
                app,
                ctx,
                t + 0.01 * i as f64,
                vec![egui::Event::PointerMoved(p)],
            );
        }
        let last = points[points.len() - 1];
        ui_pass(
            app,
            ctx,
            t + 0.1,
            vec![pointer_button(last, Secondary, false)],
        );
        assert!(!app.ui.menu.active);
    }

    /// UI pixels over the title of the menu holding the top-level item `id` and over it.
    pub(crate) fn menu_item_path(app: &PainterApp, id: &'static str) -> [(i32, i32); 2] {
        let (w, h) = app.view().ui_size();
        let wanted = Action::Command(id);
        let (m, item) = MENUS
            .iter()
            .enumerate()
            .find_map(|(m, menu)| {
                menu.items
                    .iter()
                    .position(|it| matches!(it.entry, menu::Entry::Action(a) if a == wanted))
                    .map(|i| (m, i))
            })
            .expect("a top-level item");
        let title_x = (0..w)
            .find(|&x| {
                let mut state = dpx_ui::MenuState::default();
                state.open(w, h, x, 1);
                state.menu == Some(m)
            })
            .expect("menu title");
        let mbox = menu::menu_box(m, w, h);
        let path = [(title_x, 1), (mbox.x + mbox.w / 2, mbox.item_top(item) + 1)];
        let mut state = dpx_ui::MenuState::default();
        state.open(w, h, path[0].0, path[0].1);
        state.pointer(w, h, path[1].0, path[1].1);
        assert_eq!(state.hovered_action(&app.ui.menu_context()), Some(wanted));
        path
    }

    /// Passes as the idle window runs them (only when egui asks) while a playback runs;
    /// returns the frames it showed (each once, in order) and the time it ended.
    fn play_out(app: &mut PainterApp, ctx: &egui::Context, mut t: f64) -> (Vec<usize>, f64) {
        let mut shown: Vec<usize> = app.anim.playing.iter().map(|p| p.shown).collect();
        let mut delay = ui_pass(app, ctx, t, vec![]);
        while let Some(p) = app.anim.playing {
            if shown.last() != Some(&p.shown) {
                shown.push(p.shown);
            }
            assert!(t < 10.0, "the playback ends");
            assert!(
                delay < std::time::Duration::from_secs(1),
                "no repaint at {t}"
            );
            t += delay.as_secs_f64().max(1.0 / 60.0);
            delay = ui_pass(app, ctx, t, vec![]);
        }
        (shown, t)
    }

    fn adr38_app(frames: u32) -> PainterApp {
        let mut app = PainterApp::new();
        app.engine
            .execute(Command::SetFrameCount { count: frames })
            .unwrap();
        app.engine
            .execute(Command::SetFrameRate { fps: 10 })
            .unwrap();
        app
    }

    #[test]
    fn shifted_digits_reach_the_ends_repeat_go_to_and_play_backward() {
        use dpx_core::anim::PlayMode;
        use egui::Key::{Exclamationmark, Num1, Num2, Num3, Num4, Num5, Quote};
        let ctx = egui::Context::default();
        let mut app = adr38_app(5);
        app.engine.execute(Command::GotoFrame { frame: 2 }).unwrap();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let shift = egui::Modifiers::SHIFT;
        let current = |app: &PainterApp| app.engine.document().current_frame();
        // Shift-2 on a German layout types '"' (egui: Quote): the last frame.
        ui_pass(&mut app, &ctx, 0.1, chord(Quote, Num2, shift, "\""));
        assert_eq!(current(&app), 4);
        // Shift-1 on a US layout types '!': the first frame.
        ui_pass(
            &mut app,
            &ctx,
            0.2,
            chord(Exclamationmark, Num1, shift, "!"),
        );
        assert_eq!(current(&app), 0);
        // Shift-3 ('#' has no egui key: the physical one) before any Go To: the requester.
        ui_pass(&mut app, &ctx, 0.3, chord(Num3, Num3, shift, "#"));
        assert!(app.ui.anim_requester.is_some(), "the Go to Frame requester");
        let mut keys = tap(egui::Key::Backspace);
        keys.push(egui::Event::Text("4".into()));
        keys.extend(tap(egui::Key::Enter));
        ui_pass(&mut app, &ctx, 0.4, keys);
        assert!(app.ui.anim_requester.is_none());
        assert_eq!(current(&app), 3);
        ui_pass(
            &mut app,
            &ctx,
            0.5,
            chord(Exclamationmark, Num1, shift, "!"),
        );
        assert_eq!(current(&app), 0);
        ui_pass(&mut app, &ctx, 0.6, chord(Num3, Num3, shift, "#"));
        assert!(app.ui.anim_requester.is_none(), "no requester now");
        assert_eq!(current(&app), 3, "Shift-3 repeats the last Go To");
        // Shift-4 loops backward until Esc.
        ui_pass(&mut app, &ctx, 0.7, chord(Num4, Num4, shift, "$"));
        assert_eq!(
            app.anim.playing.map(|p| p.mode),
            Some(PlayMode::LoopBackward)
        );
        ui_pass(&mut app, &ctx, 0.8, tap(egui::Key::Escape));
        assert!(app.anim.playing.is_none());
        // Shift-5 plays every frame once backward and stops on the first.
        ui_pass(&mut app, &ctx, 0.9, chord(Num5, Num5, shift, "%"));
        assert_eq!(
            app.anim.playing.map(|p| p.mode),
            Some(PlayMode::OnceBackward)
        );
        let (shown, _) = play_out(&mut app, &ctx, 0.95);
        assert_eq!(shown, [4, 3, 2, 1, 0]);
        assert_eq!(current(&app), 0);
    }

    #[test]
    fn m_draws_a_move_and_alt_a_toggles_the_panel_through_the_ui_loop() {
        use egui::Key::{A, Backspace, Enter, M, U};
        let ctx = egui::Context::default();
        let mut app = adr38_app(4);
        app.ui.fg = 5;
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let view = app.view();
        assert_eq!((view.ux, view.uy), (1, 1), "UI pixels are screen pixels");
        // The 1-pixel brush clicked down at (40, 30).
        click_ui(&mut app, &ctx, 0.1, (40, 30), egui::PointerButton::Primary);
        assert_eq!(app.page().get(40, 30), Some(5));
        // Shift-M: the Move requester. Dist X 12 (it has the cursor), Return = Draw.
        ui_pass(
            &mut app,
            &ctx,
            0.2,
            chord(M, M, egui::Modifiers::SHIFT, "M"),
        );
        assert!(matches!(
            app.ui.anim_control,
            Some(dpx_ui::AnimControlRequester::Move(_))
        ));
        let mut keys = tap(Backspace);
        keys.push(egui::Event::Text("12".into()));
        keys.extend(tap(Enter));
        ui_pass(&mut app, &ctx, 0.3, keys);
        assert!(app.ui.anim_control.is_none(), "Draw closes it");
        let doc = app.engine.document();
        for (f, x) in [40, 43, 46, 49].into_iter().enumerate() {
            assert_eq!(doc.frame(f).unwrap().get(x, 30), Some(5), "frame {}", f + 1);
        }
        // `U` takes the whole move back in one step.
        ui_pass(&mut app, &ctx, 0.4, chord(U, U, egui::Modifiers::NONE, "u"));
        let doc = app.engine.document();
        assert_eq!(doc.frame(3).unwrap().get(49, 30), Some(0));
        assert_eq!(
            doc.frame(0).unwrap().get(40, 30),
            Some(5),
            "the click stays"
        );
        // Alt-a (macOS: Option-a types 'å', egui reports the physical A) shows the panel
        // and hides it again.
        let alt_a = || chord(A, A, egui::Modifiers::ALT, "å");
        ui_pass(&mut app, &ctx, 0.5, alt_a());
        assert!(app.anim_control.panel);
        ui_pass(&mut app, &ctx, 0.6, vec![]);
        assert!(app.ui.anim_panel.is_some(), "the panel is drawn");
        ui_pass(&mut app, &ctx, 0.7, alt_a());
        ui_pass(&mut app, &ctx, 0.8, vec![]);
        assert!(!app.anim_control.panel);
        assert!(app.ui.anim_panel.is_none());
        assert_eq!(
            app.engine.document().frame(0).unwrap().get(40, 30),
            Some(5),
            "Alt-a is not an undo"
        );
    }

    #[test]
    fn cancel_after_a_refused_move_brings_back_the_settings_from_before() {
        use dpx_core::anim_move::MovePath;
        use egui::Key::{Backspace, Enter, Escape, M, Tab};
        let ctx = egui::Context::default();
        let mut app = adr38_app(4);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let shift_m = || chord(M, M, egui::Modifiers::SHIFT, "M");
        let shown = |app: &PainterApp| match &app.ui.anim_control {
            Some(dpx_ui::AnimControlRequester::Move(r)) => r.as_ref().clone(),
            _ => panic!("the Move requester"),
        };
        // A move that is drawn (Dist X 12 over the 4 frames) keeps its settings.
        ui_pass(&mut app, &ctx, 0.1, shift_m());
        let mut keys = tap(Backspace);
        keys.push(egui::Event::Text("12".into()));
        keys.extend(tap(Enter));
        ui_pass(&mut app, &ctx, 0.2, keys);
        assert!(app.ui.anim_control.is_none(), "drawn");
        let kept = MovePath {
            dist: [12, 0, 0],
            count: 4,
            ..MovePath::default()
        };
        ui_pass(&mut app, &ctx, 0.3, shift_m());
        assert_eq!(shown(&app), dpx_ui::MoveRequester::new(&kept, 4));
        // With a 100×100 brush, Dist Z -310 and Count 999 with Trails is refused.
        app.custom_brush = Some(dpx_core::BrushImage::from_image(
            &IndexedImage::new(100, 100, 3),
            None,
        ));
        let mut keys = [tap(Tab), tap(Tab), tap(Backspace)].concat();
        keys.push(egui::Event::Text("-310".into()));
        for _ in 0..6 {
            keys.extend(tap(Tab));
        }
        keys.extend(tap(Backspace));
        keys.push(egui::Event::Text("999".into()));
        ui_pass(&mut app, &ctx, 0.4, keys);
        let typed = MovePath {
            dist: [12, 0, -310],
            count: 999,
            ..MovePath::default()
        };
        let mut expected = dpx_ui::MoveRequester::new(&typed, 4);
        for _ in 0..8 {
            expected.handle(ReqEvent::Key(ReqKey::Tab));
        }
        assert_eq!(shown(&app), expected, "typed in");
        let revision = app.engine.document().revision();
        let screen = app.view().ui_screen();
        let Some(req) = &app.ui.anim_control else {
            panic!("the Move requester");
        };
        let (ox, oy) = req.origin(screen.width, screen.height);
        click_ui(
            &mut app,
            &ctx,
            0.5,
            (ox + 68, oy + 114),
            egui::PointerButton::Primary,
        );
        assert!(app.ui.anim_control.is_some(), "Trails refused: still open");
        assert_eq!(app.engine.document().revision(), revision, "nothing drawn");
        let pens = dpx_ui::Pens::from_palette(app.engine.document().palette());
        let mut quiet = shown(&app);
        quiet.set_message("");
        assert_ne!(shown(&app).render(&pens), quiet.render(&pens), "the reason");
        // Esc = Cancel. Move again shows the settings from before (AGA p.255).
        ui_pass(&mut app, &ctx, 0.7, tap(Escape));
        assert!(app.ui.anim_control.is_none());
        ui_pass(&mut app, &ctx, 0.8, shift_m());
        assert_eq!(shown(&app), dpx_ui::MoveRequester::new(&kept, 4));
    }

    #[test]
    fn the_control_panel_works_with_the_pointer_through_the_ui_loop() {
        use dpx_core::anim::PlayMode;
        use dpx_ui::PanelButton as B;
        use dpx_ui::anim_panel::Layout;
        use egui::PointerButton::{Primary, Secondary};
        let ctx = egui::Context::default();
        let mut app = adr38_app(5);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        // Anim ▸ Control ▸ Panel On/Off.
        let path = menu_path(&app, "anim.panel");
        pick(&mut app, &ctx, 0.1, &path);
        assert!(app.anim_control.panel);
        ui_pass(&mut app, &ctx, 0.3, vec![]);
        let screen = app.view().ui_screen();
        let layout = Layout::new(screen.width, screen.height, app.ui.toolbox_visible);
        let centre = |r: dpx_core::Rect| (r.x + r.w / 2, r.y + r.h / 2);
        let current = |app: &PainterApp| app.engine.document().current_frame();
        let count = |app: &PainterApp| app.engine.document().frame_count();
        // Passes 0.1 s apart, forward only.
        let clock = std::cell::Cell::new(0.4);
        let next = || {
            let t = clock.get();
            clock.set(t + 0.1);
            t
        };
        let click = |app: &mut PainterApp, at: (i32, i32), button| {
            click_ui(app, &ctx, next(), at, button);
        };
        let press =
            |app: &mut PainterApp, b: B, button| click(app, centre(layout.button(b)), button);
        press(&mut app, B::Last, Primary);
        assert_eq!(current(&app), 4);
        assert_eq!(app.anim_control.held, None, "up again");
        press(&mut app, B::First, Primary);
        assert_eq!(current(&app), 0);
        press(&mut app, B::Next, Primary);
        assert_eq!(current(&app), 1);
        press(&mut app, B::Previous, Primary);
        assert_eq!(current(&app), 0);
        press(&mut app, B::Add, Primary);
        assert_eq!(count(&app), 6);
        press(&mut app, B::Delete, Primary);
        assert_eq!(count(&app), 5);
        press(&mut app, B::LightTable, Primary);
        assert!(app.anim.light_on);
        press(&mut app, B::LightTable, Primary);
        assert!(!app.anim.light_on);
        for (b, mode) in [
            (B::OnceForward, PlayMode::Once),
            (B::LoopForward, PlayMode::Loop),
            (B::PingPong, PlayMode::PingPong),
            (B::LoopBackward, PlayMode::LoopBackward),
            (B::OnceBackward, PlayMode::OnceBackward),
        ] {
            press(&mut app, b, Primary);
            assert_eq!(app.anim.playing.map(|p| p.mode), Some(mode), "{b:?}");
            ui_pass(&mut app, &ctx, next(), tap(egui::Key::Escape));
            assert!(app.anim.playing.is_none());
        }
        // Go To with the right button: the Go to Frame requester.
        press(&mut app, B::GoTo, Secondary);
        assert!(app.ui.anim_requester.is_some());
        ui_pass(&mut app, &ctx, next(), tap(egui::Key::Escape));
        assert!(app.ui.anim_requester.is_none());
        // The slider: a click right of the knob steps one frame, the knob drags.
        press(&mut app, B::First, Primary);
        let track = layout.track;
        click(
            &mut app,
            (track.x + track.w - 2, track.y + track.h / 2),
            Primary,
        );
        assert_eq!(current(&app), 1);
        click(&mut app, (track.x + 1, track.y + track.h / 2), Primary);
        assert_eq!(current(&app), 0);
        let knob = app.ui.anim_panel.expect("the panel").knob(&layout);
        let grab = ui_point(&app, centre(knob));
        ui_pass(
            &mut app,
            &ctx,
            next(),
            vec![
                egui::Event::PointerMoved(grab),
                pointer_button(grab, Primary, true),
            ],
        );
        assert!(app.anim_control.knob.is_some(), "the knob is held");
        let end = ui_point(&app, (track.x + track.w - 1, knob.y));
        ui_pass(&mut app, &ctx, next(), vec![egui::Event::PointerMoved(end)]);
        assert_eq!(current(&app), 4, "dragged to the last frame");
        let home = ui_point(&app, (track.x, knob.y));
        ui_pass(
            &mut app,
            &ctx,
            next(),
            vec![egui::Event::PointerMoved(home)],
        );
        assert_eq!(current(&app), 0, "and back to the first");
        ui_pass(
            &mut app,
            &ctx,
            next(),
            vec![pointer_button(home, Primary, false)],
        );
        assert!(app.anim_control.knob.is_none());
        // The panel took every press: nothing was painted under it.
        let doc = app.engine.document();
        for f in 0..doc.frame_count() {
            let frame = doc.frame(f).unwrap();
            assert!(frame.pixels().iter().all(|&p| p == 0), "frame {}", f + 1);
        }
    }

    #[test]
    fn set_range_method_and_move_work_from_the_menus() {
        use egui::Key::{Backspace, Enter, Num2, Num4, Num5, Tab};
        use egui::PointerButton::Primary;
        let ctx = egui::Context::default();
        let mut app = adr38_app(10);
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        let screen = app.view().ui_screen();
        // Anim ▸ Control ▸ Set Range..: Frames 3 to 6.
        let path = menu_path(&app, "anim.set_range");
        pick(&mut app, &ctx, 0.1, &path);
        let Some(req @ dpx_ui::AnimControlRequester::PlayRange(_)) = &app.ui.anim_control else {
            panic!("the Set Range requester");
        };
        let (ox, oy) = req.origin(screen.width, screen.height);
        click_ui(&mut app, &ctx, 0.3, (ox + 10, oy + 31), Primary);
        let mut keys = tap(Backspace);
        keys.push(egui::Event::Text("3".into()));
        keys.extend(tap(Tab));
        keys.extend(tap(Backspace));
        keys.extend(tap(Backspace));
        keys.push(egui::Event::Text("6".into()));
        keys.extend(tap(Enter));
        ui_pass(&mut app, &ctx, 0.4, keys);
        assert!(app.ui.anim_control.is_none());
        assert_eq!(app.anim_control.range, Some((2, 5)));
        // From the last frame, `4` (Play) starts there: the range does not apply to it.
        let shift = egui::Modifiers::SHIFT;
        ui_pass(&mut app, &ctx, 0.5, chord(Num2, Num2, shift, "@"));
        assert_eq!(app.engine.document().current_frame(), 9);
        let none = egui::Modifiers::NONE;
        ui_pass(&mut app, &ctx, 0.6, chord(Num4, Num4, none, "4"));
        assert_eq!(app.anim.playing.map(|p| p.shown), Some(9));
        ui_pass(&mut app, &ctx, 0.7, tap(egui::Key::Escape));
        // `5` (Play once) plays frames 3 to 6 and stops on 6.
        ui_pass(&mut app, &ctx, 0.8, chord(Num5, Num5, none, "5"));
        let (shown, t) = play_out(&mut app, &ctx, 0.85);
        assert_eq!(shown, [2, 3, 4, 5]);
        assert_eq!(app.engine.document().current_frame(), 5);
        // Anim ▸ Method: a radio pair with a check mark; no command.
        let revision = app.engine.document().revision();
        let checked = |app: &PainterApp, id: &'static str| {
            Action::Command(id).checked(&app.ui.menu_context())
        };
        let path = menu_path(&app, "anim.method_expanded");
        pick(&mut app, &ctx, t + 0.1, &path);
        assert!(app.ui.anim_expanded);
        assert_eq!(checked(&app, "anim.method_expanded"), Some(true));
        assert_eq!(checked(&app, "anim.method_compressed"), Some(false));
        let path = menu_path(&app, "anim.method_compressed");
        pick(&mut app, &ctx, t + 0.3, &path);
        assert!(!app.ui.anim_expanded);
        assert_eq!(checked(&app, "anim.method_compressed"), Some(true));
        assert_eq!(app.engine.document().revision(), revision);
        // Anim ▸ Move..: the requester; Preview plays the wire frame over every frame (Count
        // shows the frame count) and brings the requester back.
        let path = menu_item_path(&app, "anim.move");
        pick(&mut app, &ctx, t + 0.5, &path);
        let Some(req @ dpx_ui::AnimControlRequester::Move(_)) = &app.ui.anim_control else {
            panic!("the Move requester");
        };
        let (ox, oy) = req.origin(screen.width, screen.height);
        let mut delay = click_ui(&mut app, &ctx, t + 0.7, (ox + 8, oy + 114), Primary);
        assert!(app.anim_control.preview.is_some(), "the preview plays");
        let mut t = t + 0.72;
        let mut counters = std::collections::BTreeSet::new();
        while app.anim_control.preview.is_some() {
            counters.extend(app.ui.frame_counter);
            assert!(t < 10.0, "the preview ends");
            assert!(
                delay < std::time::Duration::from_secs(1),
                "no repaint at {t}"
            );
            t += delay.as_secs_f64().max(1.0 / 60.0);
            delay = ui_pass(&mut app, &ctx, t, vec![]);
        }
        assert_eq!(counters.len(), 10, "every frame: {counters:?}");
        assert!(
            matches!(
                app.ui.anim_control,
                Some(dpx_ui::AnimControlRequester::Move(_))
            ),
            "the requester is back"
        );
        assert_eq!(app.engine.document().revision(), revision, "nothing drawn");
        ui_pass(&mut app, &ctx, t + 0.1, tap(egui::Key::Escape));
        assert!(app.ui.anim_control.is_none());
    }
}
