//! DeluxePainter desktop application.
//!
//! `deluxepainter`                       start the GUI
//! `deluxepainter --snapshot out.png`    render the start screen headless to a PNG
//!     [--scale N]                       integer scale (default 2)
//!     [--palette gold16]                use the 16-colour demo palette
//!     [--fg N] [--tool NAME[:filled]]   foreground colour and selected tool
//!     [--menu M[:I[:S]]]                open menu M (0 = Picture), highlight item I / sub item S
//! `deluxepainter --font-preview out.png` render sample lines of the UI font (3×)

use std::sync::Arc;

use dpx_core::{IndexedImage, Palette, raster};
use dpx_ui::menu::{self, MENUS};
use dpx_ui::{Action, Hit, PaintMode, Screen, Tool, UiState, to_rgba};
use eframe::egui;

const SCREEN_W: i32 = 320;
const SCREEN_H: i32 = 256;
const MAX_UNDO: usize = 64;

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
        let palette = match arg_value(&args, "--palette").as_deref() {
            Some("gold16") => gold16(),
            _ => Palette::dpaint_default_32(),
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
        if let Err(e) = snapshot(path, scale, palette, &ui) {
            eprintln!("snapshot failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DeluxePainter")
            .with_inner_size([(SCREEN_W * 3) as f32, (SCREEN_H * 3) as f32])
            .with_min_inner_size([SCREEN_W as f32, SCREEN_H as f32])
            .with_icon(Arc::new(app_icon())),
        ..Default::default()
    };
    eframe::run_native(
        "DeluxePainter",
        options,
        Box::new(|_cc| Ok(Box::new(PainterApp::new()))),
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

fn snapshot(
    path: &str,
    scale: usize,
    palette: Palette,
    ui: &UiState,
) -> Result<(), Box<dyn std::error::Error>> {
    let page = IndexedImage::new(SCREEN_W as u32, SCREEN_H as u32, 0);
    let screen = Screen::new(SCREEN_W, SCREEN_H).render(&page, &palette, ui);
    let rgba = to_rgba(&screen, &palette, scale, scale);
    write_png(
        path,
        SCREEN_W as u32 * scale as u32,
        SCREEN_H as u32 * scale as u32,
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
    palette: Palette,
    page: IndexedImage,
    ui: UiState,
    screen: Screen,
    undo: Vec<IndexedImage>,
    texture: Option<egui::TextureHandle>,
    dirty: bool,
    /// Last painted page position and the colour while a stroke is in progress.
    stroke: Option<((i32, i32), u8)>,
}

impl PainterApp {
    fn new() -> Self {
        Self {
            palette: Palette::dpaint_default_32(),
            page: IndexedImage::new(SCREEN_W as u32, SCREEN_H as u32, 0),
            ui: UiState::default(),
            screen: Screen::new(SCREEN_W, SCREEN_H),
            undo: Vec::new(),
            texture: None,
            dirty: true,
            stroke: None,
        }
    }

    fn push_undo(&mut self) {
        if self.undo.len() == MAX_UNDO {
            self.undo.remove(0);
        }
        self.undo.push(self.page.clone());
    }

    fn stamp(&mut self, x: i32, y: i32, color: u8) {
        for (dx, dy) in dpx_ui::toolbox::BUILTIN_BRUSHES[self.ui.brush].footprint() {
            self.page.set(x + dx, y + dy, color);
        }
        self.dirty = true;
    }

    fn select_tool(&mut self, tool: Tool, filled_half: bool) {
        match tool {
            Tool::Undo => {
                if let Some(prev) = self.undo.pop() {
                    self.page = prev;
                }
            }
            Tool::Clear => {
                self.push_undo();
                let (w, h) = (self.page.width() as i32, self.page.height() as i32);
                self.page.fill_rect(0, 0, w, h, self.ui.bg);
            }
            _ => {
                self.ui.tool = tool;
                self.ui.filled = filled_half;
            }
        }
        self.dirty = true;
    }

    fn handle_press(&mut self, hit: Hit, right: bool) {
        match hit {
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
                    self.push_undo();
                    let color = if right { self.ui.bg } else { self.ui.fg };
                    self.stamp(x, y, color);
                    self.stroke = Some(((x, y), color));
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
                "picture.flip_horizontal" => {
                    self.push_undo();
                    self.page.flip_horizontal();
                }
                "picture.flip_vertical" => {
                    self.push_undo();
                    self.page.flip_vertical();
                }
                // DeluxePaint cannot undo these two; DeluxePainter keeps them undoable.
                "color.bg_to_fg" => {
                    self.push_undo();
                    self.page.replace_index(self.ui.bg, self.ui.fg);
                }
                "color.swap_bg_fg" => {
                    self.push_undo();
                    self.page.swap_indices(self.ui.bg, self.ui.fg);
                }
                "palette.default" if self.palette.len() == 32 => {
                    self.palette = Palette::dpaint_default_32();
                }
                "app.quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                _ => {}
            },
        }
        self.dirty = true;
    }

    fn handle_drag(&mut self, x: i32, y: i32) {
        let Some((last, color)) = self.stroke else {
            return;
        };
        if last == (x, y) {
            return;
        }
        match self.ui.tool {
            Tool::ContinuousFreehand => {
                for (px, py) in raster::line_points(last, (x, y)).into_iter().skip(1) {
                    self.stamp(px, py, color);
                }
            }
            _ => self.stamp(x, y, color),
        }
        self.stroke = Some(((x, y), color));
    }
}

impl eframe::App for PainterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let area = ctx.content_rect();

        // Integer scale in physical pixels, letterboxed and snapped to the device pixel grid.
        let phys_w = (area.width() * ppp).floor() as i32;
        let phys_h = (area.height() * ppp).floor() as i32;
        let scale = (phys_w / SCREEN_W).min(phys_h / SCREEN_H).max(1);
        let img_w = SCREEN_W * scale;
        let img_h = SCREEN_H * scale;
        let origin_px = egui::vec2(((phys_w - img_w) / 2) as f32, ((phys_h - img_h) / 2) as f32);
        let origin = area.min + origin_px / ppp;
        let rect = egui::Rect::from_min_size(origin, egui::vec2(img_w as f32, img_h as f32) / ppp);

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
                (v.x / scale as f32).floor() as i32,
                (v.y / scale as f32).floor() as i32,
            )
        };
        let (sw, sh) = (SCREEN_W, SCREEN_H);
        if self.ui.menu.active {
            // Menu strip held with the right button: track, multi-select with the left button,
            // select on release.
            if let Some(pos) = hover {
                let (sx, sy) = to_screen(pos);
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
            let hit = self.screen.hit_test(self.palette.len(), &self.ui, sx, sy);
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
                self.ui.menu.open(sw, sh, sx, sy);
                self.dirty = true;
            } else if press_l || press_r {
                self.handle_press(hit, press_r);
            } else if (down_l || down_r) && rect.contains(pos) {
                self.handle_drag(sx, sy);
            }
        }
        if !(down_l || down_r) {
            self.stroke = None;
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
        if ctx.input(|i| i.key_pressed(egui::Key::U)) {
            self.select_tool(Tool::Undo, false);
        }
        self.upload_and_paint(ui, &ctx, rect, area);
    }
}

impl PainterApp {
    fn upload_and_paint(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        rect: egui::Rect,
        area: egui::Rect,
    ) {
        // Upload the screen at native resolution; the GPU scales it with nearest filtering.
        if self.dirty || self.texture.is_none() {
            let screen = self.screen.render(&self.page, &self.palette, &self.ui);
            let rgba = to_rgba(&screen, &self.palette, 1, 1);
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [SCREEN_W as usize, SCREEN_H as usize],
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
        app.page.set(0, 0, 5);
        app.run_action(Action::Command("picture.flip_horizontal"), &ctx);
        assert_eq!(app.page.get(SCREEN_W - 1, 0), Some(5));
        app.run_action(Action::Command("picture.flip_vertical"), &ctx);
        assert_eq!(app.page.get(SCREEN_W - 1, SCREEN_H - 1), Some(5));
        app.select_tool(Tool::Undo, false);
        app.select_tool(Tool::Undo, false);
        assert_eq!(app.page.get(0, 0), Some(5), "flips are undoable");

        app.run_action(Action::Mode(PaintMode::Smear), &ctx);
        assert_eq!(app.ui.mode, PaintMode::Smear);
        app.run_action(Action::Pref(dpx_ui::Pref::Coords), &ctx);
        assert!(app.ui.prefs.get(dpx_ui::Pref::Coords));

        // BG -> FG recolours background pixels with the foreground colour.
        app.run_action(Action::Command("color.bg_to_fg"), &ctx);
        assert_eq!(app.page.get(1, 1), Some(app.ui.fg));
    }
}
