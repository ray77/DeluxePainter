//! DeluxePainter desktop application.
//!
//! `deluxepainter`                       start the GUI
//! `deluxepainter --snapshot out.png`    render the start screen headless to a PNG
//!     [--scale N]                       integer scale (default 2)
//!     [--palette gold16]                use the 16-colour demo palette
//!     [--fg N] [--tool NAME[:filled]]   foreground colour and selected tool
//!     [--menu M[:I[:S]]]                open menu M (0 = Picture), highlight item I / sub item S
//! `deluxepainter --font-preview out.png` render sample lines of the UI font (3×)
//!     [--requester]                     show the Screen Format requester instead
//! `deluxepainter --open pic.iff`        start the GUI (or a snapshot) with a picture
//! `deluxepainter info <file>`           describe an IFF ILBM or PNG picture
//! `deluxepainter convert <in> <out>`    convert between IFF ILBM and PNG

mod files;

use std::path::Path;
use std::sync::Arc;

use dpx_core::{
    Axis, Command, DisplayMode, Document, Engine, ErrorCode, IndexedImage, Palette, Target, raster,
};
use dpx_ui::menu::{self, MENUS};
use dpx_ui::requester::{self, ReqEvent, ReqKey, ReqResponse, ScreenFormatRequester};
use dpx_ui::{Action, Hit, PaintMode, Screen, Tool, UiState, to_rgba};
use eframe::egui;

/// Initial window size in points: PAL LoRes at 3×.
const START_W: f32 = 960.0;
const START_H: f32 = 768.0;

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
    fn of(mode: DisplayMode) -> Self {
        let (w, h) = mode.size();
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

    /// The UI layout screen (always LoRes geometry: 320 wide).
    fn ui_screen(&self) -> Screen {
        Screen::new(self.screen_w / self.ux, self.screen_h / self.uy)
    }

    fn compose(&self, page: &IndexedImage, palette: &Palette, ui: &UiState) -> IndexedImage {
        self.ui_screen()
            .render_scaled(page, palette, ui, self.ux, self.uy)
    }

    /// Screen pixel → UI pixel.
    fn ui_pos(self, (x, y): (i32, i32)) -> (i32, i32) {
        (x.div_euclid(self.ux), y.div_euclid(self.uy))
    }
}

/// The document for a loaded picture: its display mode is guessed from CAMG and size.
fn document_for(p: &files::Picture) -> Document {
    let img = p.image();
    let mode = DisplayMode::guess(p.camg(), img.width(), img.height());
    Document::from_picture(
        img.clone(),
        p.palette().clone(),
        Target {
            mode,
            ..Target::default()
        },
    )
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
        (Some("info" | "convert"), _) => {
            eprintln!("usage: deluxepainter info <file> | convert <in> <out>");
            std::process::exit(2);
        }
        _ => {}
    }
    let opened = arg_value(&args, "--open").map(|p| match files::load(Path::new(&p)) {
        Ok(loaded) => {
            loaded.notes.iter().for_each(|n| eprintln!("note: {n}"));
            loaded.picture
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
            Some(p) => document_for(p),
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
        let mut ui = UiState::default();
        if let Some(fg) = arg_value(&args, "--fg").and_then(|s| s.parse().ok()) {
            ui.fg = fg;
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
        if let Err(e) = snapshot(path, scale, &doc, &ui) {
            eprintln!("snapshot failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DeluxePainter")
            .with_inner_size([START_W, START_H])
            .with_min_inner_size([320.0, 256.0])
            .with_icon(Arc::new(app_icon())),
        ..Default::default()
    };
    eframe::run_native(
        "DeluxePainter",
        options,
        Box::new(move |_cc| {
            let mut app = PainterApp::new();
            if let Some(p) = opened {
                app.engine = Engine::new(document_for(&p));
            }
            Ok(Box::new(app))
        }),
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

/// Renders the screen of `doc` headless; each screen pixel becomes `scale` × its displayed
/// shape (e.g. HiRes pixels are twice as tall as wide).
fn snapshot(
    path: &str,
    scale: usize,
    doc: &Document,
    ui: &UiState,
) -> Result<(), Box<dyn std::error::Error>> {
    let view = View::of(doc.target().mode);
    let screen = view.compose(doc.page(), doc.palette(), ui);
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
    stroke: Option<Stroke>,
    /// Picture ▸ Screen Format while it is open (shown on its own screen).
    requester: Option<ScreenFormatRequester>,
    /// Requester-screen pointer position while the left button is held.
    req_pointer: Option<(i32, i32)>,
}

/// A freehand stroke in progress: drawn into a preview copy of the page with the same raster
/// code the engine uses, and committed as one [`Command::Stroke`] when the button is released.
struct Stroke {
    preview: IndexedImage,
    brush: Vec<(i32, i32)>,
    color: u8,
    continuous: bool,
    points: Vec<(i32, i32)>,
}

impl PainterApp {
    fn new() -> Self {
        let (w, h) = DisplayMode::PAL_LORES.size();
        let doc = Document::new(w, h, Palette::dpaint_default_32());
        Self {
            engine: Engine::new(doc),
            ui: UiState::default(),
            texture: None,
            dirty: true,
            stroke: None,
            requester: None,
            req_pointer: None,
        }
    }

    fn palette(&self) -> &Palette {
        self.engine.document().palette()
    }

    /// The screen currently shown: the requester screen or the document's display mode.
    fn view(&self) -> View {
        if self.requester.is_some() {
            View::of(requester::SCREEN)
        } else {
            View::of(self.engine.document().target().mode)
        }
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
                    let max = (self.palette().len() - 1) as u8;
                    self.ui.fg = self.ui.fg.min(max);
                    self.ui.bg = self.ui.bg.min(max);
                }
                Err(e) => req.set_message(e.message),
            },
        }
        self.dirty = true;
    }

    /// The page as shown: the stroke preview while painting, else the document page.
    fn page(&self) -> &IndexedImage {
        self.stroke
            .as_ref()
            .map_or(self.engine.document().page(), |s| &s.preview)
    }

    /// Executes a command; a pending stroke is committed first so the order stays intact.
    fn run(&mut self, command: Command) {
        self.finish_stroke();
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
        self.dirty = true;
    }

    fn redo(&mut self) {
        self.finish_stroke();
        Self::report(self.engine.redo(), ErrorCode::NothingToRedo);
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

    fn start_stroke(&mut self, x: i32, y: i32, color: u8) {
        self.finish_stroke();
        let brush = dpx_ui::toolbox::BUILTIN_BRUSHES[self.ui.brush].footprint();
        let continuous = self.ui.tool == Tool::ContinuousFreehand;
        let mut preview = self.engine.document().page().clone();
        raster::stroke(&mut preview, &brush, color, &[(x, y)], continuous);
        self.stroke = Some(Stroke {
            preview,
            brush,
            color,
            continuous,
            points: vec![(x, y)],
        });
        self.dirty = true;
    }

    fn finish_stroke(&mut self) {
        if let Some(s) = self.stroke.take() {
            self.execute(Command::Stroke {
                brush: s.brush,
                color: s.color,
                points: s.points,
                continuous: s.continuous,
            });
        }
    }

    fn select_tool(&mut self, tool: Tool, filled_half: bool) {
        match tool {
            Tool::Undo => self.undo(),
            Tool::Clear => self.run(Command::Clear { color: self.ui.bg }),
            _ => {
                self.ui.tool = tool;
                self.ui.filled = filled_half;
            }
        }
        self.dirty = true;
    }

    fn handle_press(&mut self, hit: Hit, right: bool) {
        match hit {
            // DeluxePaint has one undo level; the right button on UNDO is our redo.
            Hit::Tool {
                tool: Tool::Undo, ..
            } if right => self.redo(),
            Hit::Tool { tool, filled_half } => self.select_tool(tool, filled_half),
            Hit::Brush(i) => {
                self.ui.brush = i;
                self.dirty = true;
            }
            Hit::PaletteCell(i) => {
                if right {
                    self.ui.bg = i;
                } else {
                    self.ui.fg = i;
                }
                self.dirty = true;
            }
            Hit::Page { x, y } => {
                if matches!(
                    self.ui.tool,
                    Tool::DottedFreehand | Tool::ContinuousFreehand
                ) {
                    let color = if right { self.ui.bg } else { self.ui.fg };
                    self.start_stroke(x, y, color);
                }
            }
            Hit::TitleBar | Hit::Indicator | Hit::ToolboxBackground => {}
        }
    }

    /// Runs a menu action. Commands not implemented yet do nothing.
    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Mode(m) => self.ui.mode = m,
            Action::Pref(p) => self.ui.prefs.toggle(p),
            Action::Command(id) => match id {
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
                "app.quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                _ => {}
            },
        }
        self.dirty = true;
    }

    fn handle_drag(&mut self, x: i32, y: i32) {
        let Some(s) = &mut self.stroke else {
            return;
        };
        let last = *s.points.last().expect("a stroke starts with one point");
        if last == (x, y) {
            return;
        }
        // Stamping the previous point again changes nothing, so the preview equals the
        // committed stroke (see `raster::stroke`).
        raster::stroke(
            &mut s.preview,
            &s.brush,
            s.color,
            &[last, (x, y)],
            s.continuous,
        );
        s.points.push((x, y));
        self.dirty = true;
    }
}

impl eframe::App for PainterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let area = ctx.content_rect();

        // Integer scale in physical pixels, letterboxed and snapped to the device pixel grid.
        // Each screen pixel is shown fx×fy times larger (its shape in this display mode).
        let view = self.view();
        let phys_w = (area.width() * ppp).floor() as i32;
        let phys_h = (area.height() * ppp).floor() as i32;
        let (unit_w, unit_h) = (view.screen_w * view.fx, view.screen_h * view.fy);
        let scale = (phys_w / unit_w).min(phys_h / unit_h).max(1);
        let img_w = unit_w * scale;
        let img_h = unit_h * scale;
        let origin_px = egui::vec2(((phys_w - img_w) / 2) as f32, ((phys_h - img_h) / 2) as f32);
        let origin = area.min + origin_px / ppp;
        let rect = egui::Rect::from_min_size(origin, egui::vec2(img_w as f32, img_h as f32) / ppp);

        if self.requester.is_some() {
            self.requester_input(&ctx, origin, ppp, scale, view);
            self.upload_and_paint(ui, &ctx, rect, area);
            return;
        }

        // Input.
        let (hover, press_l, press_r, down_l, down_r) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.secondary_pressed(),
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
            )
        });
        let to_screen = |p: egui::Pos2| -> (i32, i32) {
            let v = (p - origin) * ppp;
            (
                (v.x / (scale * view.fx) as f32).floor() as i32,
                (v.y / (scale * view.fy) as f32).floor() as i32,
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
            // Page hits use screen pixels, which are page pixels.
            let hit = match ui_screen.hit_test(self.palette().len(), &self.ui, ux, uy) {
                Hit::Page { .. } => Hit::Page { x: sx, y: sy },
                h => h,
            };
            let on_page = match hit {
                Hit::Page { x, y } => Some((x, y)),
                _ => None,
            };
            if on_page.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if self.ui.coords != on_page {
                self.ui.coords = on_page;
                self.dirty |= self.ui.prefs.get(dpx_ui::Pref::Coords);
            }
            if press_r && hit == Hit::TitleBar {
                self.ui.menu.open(sw, sh, ux, uy);
                self.dirty = true;
            } else if press_l || press_r {
                self.handle_press(hit, press_r);
            } else if (down_l || down_r) && rect.contains(pos) {
                self.handle_drag(sx, sy);
            }
        }
        if !(down_l || down_r) {
            self.finish_stroke();
        }
        if self.ui.menu.active {
            // Keyboard shortcuts are ignored while the menu strip is shown.
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
        // U = undo (as in DeluxePaint), Shift+U = redo (DeluxePainter addition).
        let (undo_key, shift) = ctx.input(|i| (i.key_pressed(egui::Key::U), i.modifiers.shift));
        if undo_key {
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
    /// Pointer and keys while the Screen Format requester is open (left button only).
    fn requester_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: i32,
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
                (v.x / (scale * view.fx) as f32).floor() as i32,
                (v.y / (scale * view.fy) as f32).floor() as i32,
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
        let keys: Vec<ReqKey> = ctx.input(|i| {
            [
                (egui::Key::ArrowUp, ReqKey::Up),
                (egui::Key::ArrowDown, ReqKey::Down),
                (egui::Key::Enter, ReqKey::Enter),
                (egui::Key::Escape, ReqKey::Escape),
                (egui::Key::U, ReqKey::Char('u')),
                (egui::Key::C, ReqKey::Char('c')),
                (egui::Key::R, ReqKey::Char('r')),
            ]
            .into_iter()
            .filter(|(k, _)| i.key_pressed(*k))
            .map(|(_, r)| r)
            .collect()
        });
        for k in keys {
            self.requester_event(ReqEvent::Key(k));
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
            let (screen, rgba) = match &self.requester {
                Some(req) => {
                    let screen = req.render();
                    let rgba = to_rgba(&screen, &requester::palette(), 1, 1);
                    (screen, rgba)
                }
                None => {
                    let screen = self.view().compose(self.page(), self.palette(), &self.ui);
                    let rgba = to_rgba(&screen, self.palette(), 1, 1);
                    (screen, rgba)
                }
            };
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

        let painter = ui.painter();
        painter.rect_filled(area, 0.0, egui::Color32::BLACK);
        if let Some(t) = &self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(t.id(), rect, uv, egui::Color32::WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_spec_parsing() {
        assert_eq!(parse_menu_spec("0"), Some((0, None, None)));
        assert_eq!(parse_menu_spec("0:4:1"), Some((0, Some(4), Some(1))));
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
}
