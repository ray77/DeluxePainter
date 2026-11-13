//! The app side of the Modern skin (ADR-47): the live window and the headless snapshot.
//!
//! The live window is the Classic window with other surroundings. The picture quad of the
//! Modern layout is the classic screen rectangle, composed without the classic chrome and
//! shown as the same nearest-neighbour texture, so every mapping from the pointer to the
//! screen and the page, every tool, key and requester works unchanged. Around it the bar and
//! the dock are drawn by `dpx-ui-modern` on the CPU, each into its own image that is redone
//! only when its display list changes; the dock answers a click with the Classic skin's
//! [`Hit`] values, so `handle_press` does not know which skin it came from. The pull-down
//! menus (held with the right button on the bar, as in Classic) and the tooltips of the dock
//! buttons are one more image over everything; they write and read the same `MenuState`
//! and `Tooltip` as the Classic skin.
//!
//! During a Modern pass the two classic chrome flags (`title_visible`, `toolbox_visible`)
//! are switched off in the shared state and kept aside here: the classic code then lays
//! out, draws and hit-tests a screen without title bar and toolbox. They still mean "bar
//! shown" and "dock shown" (F9, F10) and are back in the shared state after the pass.

use dpx_core::Document;
use dpx_ui::{Hit, KeyMode, PixelSize, ThemeChoice, Tooltip, TooltipTarget, UiState, UiStyle};
use dpx_ui_modern::paint::{List, Rect};
use dpx_ui_modern::{
    Chrome, Frame, Image, MainLayout, Model, Picture, Renderer, Spot, Theme, ThemeKind, Toggles,
    build, layout,
};
use eframe::egui;

use crate::{CLASSIC_MIN_WINDOW, PainterApp, View, arg_value, snapshot_rgba, write_png};

/// The parts of the window that are rasterised on their own, in painting order.
const REGIONS: usize = 3;
/// Index of each region in [`State::regions`] (and of its count in `State::renders`).
const VIEWPORT: usize = 0;
#[cfg(test)]
const DOCK: usize = 1;
#[cfg(test)]
const BAR: usize = 2;
const REGION_NAMES: [&str; REGIONS] = ["modern-viewport", "modern-dock", "modern-bar"];
const OVERLAY_NAME: &str = "modern-overlay";

/// One region's image as it is on the GPU: one texture, or several tiles when the region is
/// larger than the GPU's largest texture (a 4K or 5K window at device scale 2).
#[derive(Default)]
struct Region {
    /// What the image was made from: the list's fingerprint with place, size, device scale,
    /// background and tile size.
    key: Option<u64>,
    /// The tiles row by row, each with its rectangle in physical pixels of the window.
    tiles: Vec<([i32; 4], egui::TextureHandle)>,
    /// How often the region was rasterised.
    renders: u64,
}

/// `px` (left, top, width, height) cut into tiles of at most `side` × `side`, row by row.
fn tiles(px: [i32; 4], side: i32) -> Vec<[i32; 4]> {
    let side = side.max(1);
    let mut out = Vec::new();
    let mut y = 0;
    while y < px[3] {
        let h = side.min(px[3] - y);
        let mut x = 0;
        while x < px[2] {
            let w = side.min(px[2] - x);
            out.push([px[0] + x, px[1] + y, w, h]);
            x += w;
        }
        y += h;
    }
    out
}

/// What the live Modern window keeps between passes.
#[derive(Default)]
pub(crate) struct State {
    /// During a pass: the bar and dock flags, taken out of the shared state.
    held: Option<Chrome>,
    /// The part of the chrome under the pointer, drawn raised.
    hover: Option<Hit>,
    renderer: Renderer,
    regions: [Region; REGIONS],
    /// What lies over the regions: the open menu's cards or a tooltip. Its image is
    /// transparent where nothing is drawn.
    overlay: Region,
    /// The other skin was asked for in this pass (Prefs ▸ Modern UI, F12); it is shown from
    /// the next pass on.
    switch: bool,
    /// The window's minimum size is the Modern one.
    min_size: bool,
    /// The theme of the last painted pass.
    #[cfg(test)]
    theme: Option<ThemeKind>,
}

impl State {
    /// How often each region was rasterised so far, by [`VIEWPORT`], [`DOCK`], [`BAR`].
    #[cfg(test)]
    pub(crate) fn renders(&self) -> [u64; REGIONS] {
        [VIEWPORT, DOCK, BAR].map(|i| self.regions[i].renders)
    }

    /// The theme the last pass was painted in.
    #[cfg(test)]
    pub(crate) fn theme(&self) -> Option<ThemeKind> {
        self.theme
    }

    /// How often the overlay (menu cards, tooltip) was rasterised so far, and whether one
    /// is shown now.
    #[cfg(test)]
    pub(crate) fn overlay(&self) -> (u64, bool) {
        (self.overlay.renders, self.overlay.key.is_some())
    }
}

impl Region {
    /// Nothing is shown here: the image goes.
    fn clear(&mut self) {
        self.key = None;
        self.tiles.clear();
    }

    /// Makes the image show `list` in the pixel rectangle `px` of the window, rasterising
    /// it only when what it is made from changed. `background`: the opaque colour under
    /// the list, or `None` for an image that is transparent where the list draws nothing.
    #[allow(clippy::too_many_arguments)]
    fn show(
        &mut self,
        ctx: &egui::Context,
        renderer: &mut Renderer,
        name: &str,
        list: &List,
        px: [i32; 4],
        scale: f32,
        background: Option<dpx_ui_modern::paint::Color>,
        side: i32,
    ) {
        let clear = dpx_ui_modern::paint::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
        let key = region_key(list, px, scale, background.unwrap_or(clear), side);
        if self.key == Some(key) {
            return;
        }
        let cut = tiles(px, side);
        if cut.len() != self.tiles.len() {
            self.tiles.clear();
        }
        for (n, tile) in cut.into_iter().enumerate() {
            let (origin, w, h) = ((tile[0], tile[1]), tile[2] as u32, tile[3] as u32);
            // The picture is not among a list's images: the quad is a texture of its own.
            let rgba = match background {
                Some(colour) => renderer.render_region(list, &[], origin, w, h, scale, colour),
                None => renderer.render_overlay(list, origin, w, h, scale),
            };
            // Premultiplied alpha; for an opaque image the same bytes as straight alpha.
            let image = egui::ColorImage::from_rgba_premultiplied(
                [tile[2] as usize, tile[3] as usize],
                &rgba,
            );
            match self.tiles.get_mut(n) {
                Some((at, texture)) => {
                    *at = tile;
                    texture.set(image, egui::TextureOptions::NEAREST);
                }
                None => self.tiles.push((
                    tile,
                    ctx.load_texture(name, image, egui::TextureOptions::NEAREST),
                )),
            }
        }
        self.key = Some(key);
        self.renders += 1;
    }
}

/// A fingerprint of everything a region's image depends on.
fn region_key(
    list: &List,
    px: [i32; 4],
    scale: f32,
    background: dpx_ui_modern::paint::Color,
    tile: i32,
) -> u64 {
    let mut h = list.fingerprint();
    let mut word = |v: u32| {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    px.iter().for_each(|&v| word(v as u32));
    word(tile as u32);
    word(scale.to_bits());
    word(u32::from_le_bytes([
        background.r,
        background.g,
        background.b,
        background.a,
    ]));
    h
}

/// The rectangle in whole physical pixels: left, top, width, height.
fn pixels(rect: &Rect, scale: f32) -> [i32; 4] {
    let px = |v: f32| (v * scale).round() as i32;
    let (x, y) = (px(rect.x), px(rect.y));
    [x, y, px(rect.right()) - x, px(rect.bottom()) - y]
}

impl PainterApp {
    /// The Modern skin is the one shown.
    pub(crate) fn is_modern(&self) -> bool {
        self.ui.ui_style == UiStyle::Modern
    }

    /// Before a pass: in the Modern skin the classic chrome flags move aside (see the module
    /// text), so nothing of the classic title bar and toolbox is laid out, drawn or hit.
    pub(crate) fn modern_begin_pass(&mut self) {
        if self.is_modern() && self.modern.held.is_none() {
            self.modern.held = Some(Chrome {
                bar: std::mem::replace(&mut self.ui.title_visible, false),
                dock: std::mem::replace(&mut self.ui.toolbox_visible, false),
            });
        }
    }

    /// After a pass: the flags are back in the shared state, with what F9 and F10 did.
    pub(crate) fn modern_end_pass(&mut self) {
        if let Some(chrome) = self.modern.held.take() {
            self.ui.title_visible = chrome.bar;
            self.ui.toolbox_visible = chrome.dock;
        }
    }

    /// The flags "title bar (bar) shown" and "toolbox (dock) shown", wherever they are now.
    pub(crate) fn chrome_flags(&mut self) -> (&mut bool, &mut bool) {
        match &mut self.modern.held {
            Some(chrome) => (&mut chrome.bar, &mut chrome.dock),
            None => (&mut self.ui.title_visible, &mut self.ui.toolbox_visible),
        }
    }

    /// Which parts of the Modern chrome are shown.
    fn modern_chrome(&self) -> Chrome {
        self.modern.held.unwrap_or(Chrome {
            bar: self.ui.title_visible,
            dock: self.ui.toolbox_visible,
        })
    }

    /// The layout of the Modern window as of the last pass's window size; `None` in the
    /// Classic skin and before the first pass.
    pub(crate) fn modern_layout(&self) -> Option<MainLayout> {
        if !self.is_modern() {
            return None;
        }
        let win = self.window?;
        let view = self.view();
        Some(MainLayout::for_picture(
            (win.w as f32 / win.ppp, win.h as f32 / win.ppp),
            win.ppp,
            self.modern_chrome(),
            self.palette().len(),
            picture_of(view, self.ui.pixel_size),
        ))
    }

    /// Once per pass: whether the Modern skin is shown. A change of the skin changes the
    /// window's minimum size (the Modern layout needs more room than a 1x LoRes screen).
    pub(crate) fn modern_window(&mut self, ctx: &egui::Context) -> bool {
        let modern = self.is_modern();
        if self.modern.min_size != modern {
            self.modern.min_size = modern;
            let (w, h) = layout::MIN_WINDOW;
            let min = if modern { [w, h] } else { CLASSIC_MIN_WINDOW };
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min.into()));
            if modern {
                // A window smaller than the Modern layout is made for grows to its least
                // size (a minimum size alone does not resize a window on every system).
                let size = ctx.content_rect().size();
                if size.x < w || size.y < h {
                    let grown = egui::vec2(size.x.max(w), size.y.max(h));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(grown));
                }
            } else {
                // Back in Classic: the region images are not needed any more.
                self.modern.regions = Default::default();
                self.modern.overlay = Region::default();
                self.modern.hover = None;
            }
            self.dirty = true;
        }
        modern
    }

    /// The pointer is over `spot` (`None`: outside the window, or the Classic skin): the
    /// part of the chrome under it is drawn raised.
    pub(crate) fn modern_hover(&mut self, spot: Option<Spot>) {
        self.modern.hover = match spot {
            Some(Spot::Ui(hit)) => Some(hit),
            _ => None,
        };
    }

    /// Cmd/Ctrl + `+` / `−` in the Modern skin: the picture one step larger or smaller.
    /// Steps are counted from what is shown (a small window may show less than Prefs ▸ Pixel
    /// Size asks for), and every step changes the picture by at least one physical pixel
    /// per pixel; the window is not resized.
    pub(crate) fn modern_zoom(&mut self, step: i32) {
        let Some(l) = self.modern_layout() else {
            return;
        };
        if step > 0 && l.zoom >= l.fits {
            return;
        }
        let per_ui = picture_of(self.view(), self.ui.pixel_size).per_ui_pixel;
        if let Some(size) = layout::pixel_size_stepped(l.zoom, step, per_ui, l.scale) {
            self.set_pixel_size(size);
        }
    }

    /// Cmd/Ctrl + `0` in the Modern skin: the picture as large as fits the viewport.
    pub(crate) fn modern_zoom_to_fit(&mut self) {
        let Some(l) = self.modern_layout() else {
            return;
        };
        let per_ui = picture_of(self.view(), self.ui.pixel_size).per_ui_pixel;
        self.set_pixel_size(layout::pixel_size_for(l.fits, per_ui, l.scale));
    }

    /// The theme to paint in: the fixed choice, or the system's (dark when it says nothing).
    fn modern_theme(&self, ctx: &egui::Context) -> ThemeKind {
        let system_dark = ctx.system_theme().map(|t| t == egui::Theme::Dark);
        ThemeKind::resolve(self.ui.theme, system_dark)
    }

    /// The display lists of the regions with their rectangles, in painting order: viewport
    /// (canvas colour and the picture's shadow), dock, bar. A hidden region has no entry.
    fn modern_lists(&self, l: &MainLayout, theme: &Theme) -> [Option<(Rect, List)>; REGIONS] {
        let model = self.modern_model();
        [
            (l.viewport.w > 0.0 && l.viewport.h > 0.0)
                .then(|| (l.viewport, build::canvas(l, theme))),
            l.dock
                .as_ref()
                .map(|dock| (dock.rect, build::dock(l, theme, &model))),
            l.bar.map(|bar| (bar, build::title_bar(l, theme, &model))),
        ]
    }

    /// Paints the Modern window: the regions' images one to one, and the screen texture
    /// (uploaded by `upload_and_paint` as in Classic) into the picture quad `rect`. A region
    /// is rasterised again only when what it shows, its place or the device scale changed.
    pub(crate) fn modern_paint(
        &mut self,
        ui: &egui::Ui,
        ctx: &egui::Context,
        rect: egui::Rect,
        area: egui::Rect,
    ) {
        let Some(l) = self.modern_layout() else {
            return;
        };
        let kind = self.modern_theme(ctx);
        #[cfg(test)]
        {
            self.modern.theme = Some(kind);
        }
        let theme = Theme::of(kind);
        let lists = self.modern_lists(&l, &theme);
        let painter = ui.painter();
        let color = |c: dpx_ui_modern::paint::Color| egui::Color32::from_rgb(c.r, c.g, c.b);
        painter.rect_filled(area, 0.0, color(theme.canvas));
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let to_points = |px: [i32; 4]| {
            egui::Rect::from_min_size(
                area.min + egui::vec2(px[0] as f32, px[1] as f32) / l.scale,
                egui::vec2(px[2] as f32, px[3] as f32) / l.scale,
            )
        };
        // The largest texture the GPU takes; a larger region is cut into tiles.
        let side = ctx.input(|i| i.max_texture_side).min(i32::MAX as usize) as i32;
        for (i, entry) in lists.iter().enumerate() {
            let state = &mut self.modern;
            let region = &mut state.regions[i];
            let px = entry.as_ref().map(|(bounds, _)| pixels(bounds, l.scale));
            let (Some((_, list)), Some(px)) = (entry, px.filter(|p| p[2] > 0 && p[3] > 0)) else {
                // Hidden (F9, F10): its image goes.
                region.clear();
                continue;
            };
            let (renderer, name) = (&mut state.renderer, REGION_NAMES[i]);
            region.show(
                ctx,
                renderer,
                name,
                list,
                px,
                l.scale,
                Some(theme.canvas),
                side,
            );
            for (at, texture) in &region.tiles {
                painter.image(texture.id(), to_points(*at), uv, egui::Color32::WHITE);
            }
            if i == VIEWPORT
                && let Some(screen) = &self.texture
            {
                // The picture: over the canvas and its shadow, never over bar or dock.
                painter.with_clip_rect(to_points(px)).image(
                    screen.id(),
                    rect,
                    uv,
                    egui::Color32::WHITE,
                );
            }
        }
        // Over everything, the picture included: the open menu or a tooltip.
        let over = self.overlay_list(&l, &theme);
        let state = &mut self.modern;
        let px = over.as_ref().map(|(bounds, _)| pixels(bounds, l.scale));
        match (&over, px.filter(|p| p[2] > 0 && p[3] > 0)) {
            (Some((_, list)), Some(px)) => {
                let renderer = &mut state.renderer;
                state
                    .overlay
                    .show(ctx, renderer, OVERLAY_NAME, list, px, l.scale, None, side);
                for (at, texture) in &state.overlay.tiles {
                    painter.image(texture.id(), to_points(*at), uv, egui::Color32::WHITE);
                }
            }
            _ => state.overlay.clear(),
        }
    }

    /// What lies over the regions, with its rectangle: the cards of the menu that is held
    /// open, or the tooltip of the dock button the pointer rests on.
    fn overlay_list(&self, l: &MainLayout, theme: &Theme) -> Option<(Rect, List)> {
        build::overlay(l, theme, &self.modern_model())
    }

    /// What the Modern chrome shows.
    fn modern_model(&self) -> Model<'_> {
        Model {
            ui: &self.ui,
            palette: self.palette(),
            custom_brush: self.custom_brush.is_some(),
            toggles: Toggles {
                grid: self.tools.grid,
                symmetry: self.tools.symmetry,
                magnify: self.tools.magnify.is_some(),
            },
            hover: self.modern.hover,
        }
    }

    /// Prefs ▸ Modern UI, F12 and Cmd/Ctrl+Shift+U: the other skin. It is shown from the
    /// next pass on, because the classic chrome flags are held aside until this one ends.
    pub(crate) fn toggle_skin(&mut self) {
        self.modern.switch ^= true;
    }

    /// After a pass: the switch asked for in it. Document, tool, scroll position, Magnify
    /// and everything else are shared; an open menu and a tooltip belong to the skin that
    /// drew them and go. Back in Classic the window snaps to the screen again (the snap
    /// state was forgotten in every Modern pass); in Modern it keeps its size.
    pub(crate) fn modern_apply_switch(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.modern.switch) {
            return;
        }
        self.ui.ui_style = self.ui.ui_style.toggled();
        self.ui.menu.close();
        self.ui.tooltip = None;
        self.tooltip = crate::tooltip::State::default();
        self.modern.hover = None;
        self.dirty = true;
        ctx.request_repaint();
    }

    /// The Modern window as one image, made from the same layout, lists and screen as the
    /// live window paints: width, height (physical pixels) and RGBA8. For tests, which
    /// cannot read the window's textures back.
    #[cfg(test)]
    pub(crate) fn modern_window_image(&mut self, kind: ThemeKind) -> (u32, u32, Vec<u8>) {
        self.modern_begin_pass();
        let l = self.modern_layout().expect("a Modern window after a pass");
        let theme = Theme::of(kind);
        let mut list = List::new();
        for (_, part) in self.modern_lists(&l, &theme).into_iter().flatten() {
            list.append(part);
        }
        if let Some((_, over)) = self.overlay_list(&l, &theme) {
            list.append(over);
        }
        let screen = self.compose_screen();
        let rgba = dpx_ui::to_rgba(&screen, self.palette(), 1, 1);
        self.modern_end_pass();
        let picture = Image {
            width: screen.width(),
            height: screen.height(),
            rgba: &rgba,
        };
        let px = pixels(&l.window, l.scale);
        let (w, h) = (px[2] as u32, px[3] as u32);
        let out = Renderer::new().render(&list, &[picture], w, h, l.scale, theme.canvas);
        (w, h, out)
    }
}

/// The picture quad's image for the screen `view` at Prefs ▸ Pixel Size `pixel_size`: the
/// screen with its pixel shape applied.
fn picture_of(view: View, pixel_size: PixelSize) -> Picture {
    Picture {
        size: (
            (view.screen_w * view.fx) as u32,
            (view.screen_h * view.fy) as u32,
        ),
        per_ui_pixel: (view.fx * view.ux) as u32,
        pixel_size,
    }
}

/// The window a Modern snapshot shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SnapshotOptions {
    pub theme: ThemeKind,
    /// What Prefs ▸ Theme marks: the theme when `--theme` names one, else System.
    pub choice: ThemeChoice,
    /// Window size in points.
    pub size: (f32, f32),
    /// Physical pixels per point.
    pub device_scale: f32,
    /// A resting pointer, in window points.
    pub pointer: Option<(f32, f32)>,
    /// The pointer has rested long enough: the tooltip of the button under it shows.
    pub tooltip: bool,
}

/// The snapshot flags that only the Modern skin knows.
const MODERN_FLAGS: [&str; 5] = [
    "--theme",
    "--size",
    "--device-scale",
    "--pointer",
    "--tooltip",
];

/// Default window of a Modern snapshot, in points.
const DEFAULT_SIZE: (f32, f32) = (1280.0, 800.0);
/// Largest window side a snapshot accepts, in points.
const MAX_SIDE: f32 = 8192.0;

fn pair(spec: &str, separator: char) -> Option<(f32, f32)> {
    let (a, b) = spec.split_once(separator)?;
    let (a, b): (f32, f32) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a.is_finite() && b.is_finite()).then_some((a, b))
}

/// Reads `--ui modern` and its companions `--theme dark|light` (default dark, never the
/// machine's setting), `--size WxH` in points (default 1280x800), `--device-scale 1|2`,
/// `--pointer X,Y` (window points) and `--tooltip` (the pointer has rested: the tooltip of
/// the button under it shows). `None` without `--ui modern`: a Classic snapshot, which
/// these flags do not touch.
pub(crate) fn snapshot_options(args: &[String]) -> Result<Option<SnapshotOptions>, String> {
    let modern = match arg_value(args, "--ui").as_deref() {
        None | Some("classic") => false,
        Some("modern") => true,
        Some(other) => return Err(format!("bad --ui '{other}' (expected classic or modern)")),
    };
    if !modern {
        for flag in MODERN_FLAGS {
            if args.iter().any(|a| a == flag) {
                return Err(format!("{flag} needs --ui modern"));
            }
        }
        return Ok(None);
    }
    if args.iter().any(|a| a == "--hover") {
        // Classic's flag is in UI pixels of the classic screen, which Modern does not show.
        return Err("--hover is the Classic skin's; use --pointer X,Y --tooltip".into());
    }
    let (theme, choice) = match arg_value(args, "--theme").as_deref() {
        None => (ThemeKind::Dark, ThemeChoice::System),
        Some("dark") => (ThemeKind::Dark, ThemeChoice::Dark),
        Some("light") => (ThemeKind::Light, ThemeChoice::Light),
        Some(other) => return Err(format!("bad --theme '{other}' (expected dark or light)")),
    };
    let size = match arg_value(args, "--size") {
        None => DEFAULT_SIZE,
        Some(spec) => pair(&spec, 'x')
            .filter(|&(w, h)| {
                let (min_w, min_h) = dpx_ui_modern::layout::MIN_WINDOW;
                w >= min_w && h >= min_h && w <= MAX_SIDE && h <= MAX_SIDE
            })
            .ok_or_else(|| {
                let (w, h) = dpx_ui_modern::layout::MIN_WINDOW;
                format!("bad --size '{spec}' (expected WxH in points, at least {w}x{h})")
            })?,
    };
    let device_scale = match arg_value(args, "--device-scale").as_deref() {
        None | Some("1") => 1.0,
        Some("2") => 2.0,
        Some(other) => return Err(format!("bad --device-scale '{other}' (expected 1 or 2)")),
    };
    let pointer = match arg_value(args, "--pointer") {
        None => None,
        Some(spec) => Some(
            pair(&spec, ',')
                .ok_or_else(|| format!("bad --pointer '{spec}' (expected X,Y in window points)"))?,
        ),
    };
    let tooltip = args.iter().any(|a| a == "--tooltip");
    if tooltip && pointer.is_none() {
        return Err("--tooltip needs --pointer X,Y".into());
    }
    Ok(Some(SnapshotOptions {
        theme,
        choice,
        size,
        device_scale,
        pointer,
        tooltip,
    }))
}

/// The interface state the picture quad is composed with: the shared state without the
/// classic chrome, which the Modern skin draws itself. Requesters and the Animation Control
/// Panel stay: they are shown in the Classic look inside the quad until they are converted.
fn quad_ui(ui: &UiState) -> UiState {
    let mut bare = ui.clone();
    bare.title_visible = false;
    bare.toolbox_visible = false;
    bare.menu = dpx_ui::MenuState::default();
    bare.layer_panel = None;
    bare.tooltip = None;
    bare
}

/// The Modern main window as an image: width, height (physical pixels) and RGBA8.
pub(crate) fn snapshot_image(
    options: &SnapshotOptions,
    doc: &Document,
    view: View,
    scroll: (i32, i32),
    ui: &UiState,
    custom_brush: bool,
) -> (u32, u32, Vec<u8>) {
    // The classic screen without its chrome, one image pixel per displayed pixel (the pixel
    // shape of the display mode applied), exactly as a Classic snapshot at scale 1 makes it.
    let (pw, ph, picture) = snapshot_rgba(1, doc, view, scroll, &quad_ui(ui), false);
    // What the chrome shows: the Modern skin with its theme (the marks of Prefs ▸ Modern UI
    // and Prefs ▸ Theme) and, for a rested pointer, the tooltip of the button under it.
    let mut shown = ui.clone();
    shown.ui_style = UiStyle::Modern;
    shown.theme = options.choice;
    // A Classic tooltip (the Control Panel's, inside the quad) is not the chrome's.
    shown.tooltip = None;
    let layout = Frame {
        size: options.size,
        scale: options.device_scale,
        theme: Theme::of(options.theme),
        model: Model::new(&shown, doc.palette()),
        picture: Image {
            width: pw,
            height: ph,
            rgba: &picture,
        },
        per_ui_pixel: (view.fx * view.ux) as u32,
    }
    .layout();
    let hover = match options
        .pointer
        .map(|(x, y)| dpx_ui_modern::hit(&layout, x, y))
    {
        Some(Spot::Ui(hit)) => Some(hit),
        _ => None,
    };
    if options.tooltip && !shown.menu.active {
        shown.tooltip = hover
            .and_then(TooltipTarget::of_hit)
            .map(|target| Tooltip::new(target, KeyMode::Paint));
        if shown.tooltip.is_none() {
            eprintln!("note: --tooltip: no button with a tooltip under --pointer");
        }
    }
    let frame = Frame {
        size: options.size,
        scale: options.device_scale,
        theme: Theme::of(options.theme),
        model: Model {
            custom_brush,
            hover,
            ..Model::new(&shown, doc.palette())
        },
        picture: Image {
            width: pw,
            height: ph,
            rgba: &picture,
        },
        per_ui_pixel: (view.fx * view.ux) as u32,
    };
    dpx_ui_modern::render_window(&frame, &mut Renderer::new())
}

/// Writes the Modern main window to the PNG `path`.
pub(crate) fn snapshot(
    path: &str,
    options: &SnapshotOptions,
    doc: &Document,
    view: View,
    scroll: (i32, i32),
    ui: &UiState,
    custom_brush: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (w, h, rgba) = snapshot_image(options, doc, view, scroll, ui, custom_brush);
    write_png(path, w, h, &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{DisplayMode, Palette};
    use dpx_ui::{Action, Tool};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn without_ui_modern_the_snapshot_stays_classic() {
        assert_eq!(snapshot_options(&args(&["--snapshot", "a.png"])), Ok(None));
        assert_eq!(snapshot_options(&args(&["--ui", "classic"])), Ok(None));
        // The Modern flags alone are a mistake, not silently ignored.
        for flag in MODERN_FLAGS {
            let e = snapshot_options(&args(&[flag, "x"])).expect_err(flag);
            assert!(e.contains("--ui modern"), "{e}");
        }
        assert!(snapshot_options(&args(&["--ui", "retro"])).is_err());
    }

    #[test]
    fn the_modern_flags_have_defaults_and_are_checked() {
        assert_eq!(
            snapshot_options(&args(&["--ui", "modern"])),
            Ok(Some(SnapshotOptions {
                theme: ThemeKind::Dark,
                choice: ThemeChoice::System,
                size: (1280.0, 800.0),
                device_scale: 1.0,
                pointer: None,
                tooltip: false,
            }))
        );
        let all = [
            "--ui",
            "modern",
            "--theme",
            "light",
            "--size",
            "1440x900",
            "--device-scale",
            "2",
            "--pointer",
            "1210,150",
            "--tooltip",
        ];
        assert_eq!(
            snapshot_options(&args(&all)),
            Ok(Some(SnapshotOptions {
                theme: ThemeKind::Light,
                choice: ThemeChoice::Light,
                size: (1440.0, 900.0),
                device_scale: 2.0,
                pointer: Some((1210.0, 150.0)),
                tooltip: true,
            }))
        );
        for bad in [
            ["--theme", "sepia"],
            ["--size", "1280"],
            ["--size", "100x100"],
            ["--size", "99999x800"],
            ["--size", "NaNx800"],
            ["--device-scale", "3"],
            ["--device-scale", "1.5"],
            ["--pointer", "12"],
            ["--hover", "300,40"],
        ] {
            let mut list = vec!["--ui", "modern"];
            list.extend(bad);
            assert!(snapshot_options(&args(&list)).is_err(), "{bad:?}");
        }
    }

    fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }

    /// The quad holds the page and nothing of the classic chrome: where the Classic skin has
    /// its title bar and toolbox, the Modern picture shows page pixels.
    #[test]
    fn the_picture_quad_is_the_page_without_classic_chrome() {
        let (w, h) = DisplayMode::PAL_LORES.size();
        let palette = Palette::dpaint_default_32();
        let mut doc = Document::new(w, h, palette.clone());
        // Colour 5 everywhere: the classic title bar and toolbox would not be colour 5.
        let mut engine = dpx_core::Engine::new(doc.clone());
        engine
            .execute(dpx_core::Command::Clear { color: 5 })
            .expect("clear");
        doc = engine.document().clone();
        let view = View::of_target(doc.target());
        let ui = UiState {
            tool: Tool::Line,
            ..UiState::default()
        };
        let options = SnapshotOptions {
            theme: ThemeKind::Dark,
            choice: ThemeChoice::Dark,
            size: (1280.0, 800.0),
            device_scale: 1.0,
            pointer: Some((1210.0, 105.0)),
            tooltip: false,
        };
        let (iw, ih, rgba) = snapshot_image(&options, &doc, view, (0, 0), &ui, false);
        assert_eq!((iw, ih), (1280, 800));
        let c = palette.get(5).expect("colour 5");
        let page = [c.r, c.g, c.b];
        // The quad is 960 x 768 at (114, 31): its corners, where Classic draws chrome.
        for (x, y) in [
            (114, 31),
            (1073, 31),
            (114, 798),
            (1073, 798),
            (600, 400),
            (1060, 300),
        ] {
            assert_eq!(pixel(&rgba, iw, x, y), page, "({x}, {y})");
        }
        // The chrome is the Modern one: the selected Line button is on the accent, and the
        // pointer raises the button it rests on (Dotted Freehand, top left of the tools).
        let theme = Theme::dark();
        let accent = [theme.accent.r, theme.accent.g, theme.accent.b];
        let hover = [theme.hover.r, theme.hover.g, theme.hover.b];
        assert_eq!(pixel(&rgba, iw, 1199, 141), accent);
        assert_eq!(pixel(&rgba, iw, 1199, 99), hover);
    }

    // ---- The live window (slice 1b of ADR-47) ----

    use crate::tests::{TestWindow, chord, pointer_button, tap, ui_pass_with, ui_point};
    use dpx_ui::toolbox::TOOL_ROWS;
    use dpx_ui::{Pref, Screen};
    use egui::PointerButton::{Primary, Secondary};

    /// Something to point at, in either skin.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) enum Place {
        /// A tool button; `true`: its lower-right (filled) half.
        Tool(Tool, bool),
        /// A built-in brush, by index.
        Brush(usize),
        /// A palette cell, by colour.
        Cell(u8),
        /// A page pixel shown on the screen (no Magnify).
        Page(i32, i32),
    }

    /// The window point over `place` in the Modern window, `(fx, fy)` of the way across the
    /// button, cell or page pixel (0.5, 0.5: its centre), after a pass has laid it out.
    pub(crate) fn modern_point_at(
        app: &PainterApp,
        place: Place,
        (fx, fy): (f32, f32),
    ) -> egui::Pos2 {
        let l = app.modern_layout().expect("a Modern window after a pass");
        let dock = l.dock.as_ref();
        let within = |r: Rect| egui::pos2(r.x + r.w * fx, r.y + r.h * fy);
        match place {
            Place::Tool(tool, filled) => {
                let r = l.tool_button(tool).expect("the dock is shown");
                let f = if filled { 0.72 } else { 0.28 };
                egui::pos2(r.x + r.w * f, r.y + r.h * f)
            }
            Place::Brush(i) => within(dock.expect("the dock is shown").brushes[i]),
            Place::Cell(i) => within(dock.expect("the dock is shown").cells[usize::from(i)]),
            Place::Page(x, y) => {
                let view = app.view();
                // One screen pixel is `fx` × `fy` picture pixels of `zoom` physical pixels.
                let step = l.zoom as f32 / l.scale;
                let (sx, sy) = ((x - app.scroll.0) as f32, (y - app.scroll.1) as f32);
                egui::pos2(
                    l.picture.x + (sx + fx) * view.fx as f32 * step,
                    l.picture.y + (sy + fy) * view.fy as f32 * step,
                )
            }
        }
    }

    /// The window point over the middle of `place` in the Modern window.
    pub(crate) fn modern_point(app: &PainterApp, place: Place) -> egui::Pos2 {
        modern_point_at(app, place, (0.5, 0.5))
    }

    /// The window point over `place` in the Classic window (a LoRes screen): a UI pixel the
    /// classic hit test answers for it, from the middle of all that do.
    fn classic_point(app: &PainterApp, place: Place) -> egui::Pos2 {
        let view = app.view();
        assert_eq!((view.ux, view.uy), (1, 1), "a LoRes screen");
        let wanted = match place {
            Place::Page(x, y) => return ui_point(app, (x - app.scroll.0, y - app.scroll.1)),
            Place::Tool(tool, filled_half) => Hit::Tool { tool, filled_half },
            Place::Brush(i) => Hit::Brush(i),
            Place::Cell(i) => Hit::PaletteCell(i),
        };
        let (w, h) = view.ui_size();
        let screen = Screen::new(w, h);
        let all: Vec<(i32, i32)> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| screen.hit_test(app.palette().len(), &app.ui, x, y) == wanted)
            .collect();
        assert!(!all.is_empty(), "{place:?} is on the classic screen");
        ui_point(app, all[all.len() / 2])
    }

    /// A window of one skin that tests drive with real `ui()` passes.
    struct Rig {
        app: PainterApp,
        ctx: egui::Context,
        t: f64,
        size: egui::Vec2,
        ppp: f32,
        system: Option<egui::Theme>,
        /// The largest texture side the window reports.
        max_texture_side: usize,
        /// The window commands of all passes so far.
        commands: Vec<egui::ViewportCommand>,
    }

    impl Rig {
        fn new(style: UiStyle, size: (f32, f32), ppp: f32) -> Rig {
            let mut app = PainterApp::new();
            app.ui.ui_style = style;
            let ctx = egui::Context::default();
            // As `main()`: Cmd/Ctrl +/− are ours, not egui's zoom.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            let mut rig = Rig {
                app,
                ctx,
                t: 0.0,
                size: size.into(),
                ppp,
                system: None,
                max_texture_side: 8192,
                commands: Vec::new(),
            };
            rig.pass(vec![]);
            rig
        }

        fn classic() -> Rig {
            Rig::new(UiStyle::Classic, (1280.0, 1024.0), 1.0)
        }

        fn modern() -> Rig {
            Rig::new(UiStyle::Modern, (1280.0, 800.0), 1.0)
        }

        fn pass(&mut self, events: Vec<egui::Event>) {
            self.t += 0.02;
            let window = TestWindow {
                size: self.size,
                ppp: self.ppp,
                system_theme: self.system,
                max_texture_side: self.max_texture_side,
            };
            let out = ui_pass_with(&mut self.app, &self.ctx, self.t, events, window);
            self.commands.extend(out.commands);
        }

        fn point(&self, place: Place) -> egui::Pos2 {
            if self.app.is_modern() {
                modern_point(&self.app, place)
            } else {
                classic_point(&self.app, place)
            }
        }

        fn move_to(&mut self, place: Place) {
            let pos = self.point(place);
            self.pass(vec![egui::Event::PointerMoved(pos)]);
        }

        fn click(&mut self, place: Place, button: egui::PointerButton) {
            let pos = self.point(place);
            self.pass(vec![
                egui::Event::PointerMoved(pos),
                pointer_button(pos, button, true),
            ]);
            self.pass(vec![pointer_button(pos, button, false)]);
        }

        /// Press on the first place, move over the others, let go on the last.
        fn drag(&mut self, path: &[Place], button: egui::PointerButton) {
            let first = self.point(path[0]);
            self.pass(vec![
                egui::Event::PointerMoved(first),
                pointer_button(first, button, true),
            ]);
            let mut last = first;
            for &place in &path[1..] {
                last = self.point(place);
                self.pass(vec![egui::Event::PointerMoved(last)]);
            }
            self.pass(vec![pointer_button(last, button, false)]);
        }

        /// A character key as typed on a US keyboard (capitals with Shift).
        fn key(&mut self, c: char) {
            let name = match c {
                '/' => "Slash".to_string(),
                '<' => "Comma".to_string(),
                '>' | '.' => "Period".to_string(),
                '[' => "OpenBracket".to_string(),
                ']' => "CloseBracket".to_string(),
                '=' => "Equals".to_string(),
                '-' => "Minus".to_string(),
                c => c.to_ascii_uppercase().to_string(),
            };
            let key = egui::Key::from_name(&name).unwrap_or_else(|| panic!("a key for '{c}'"));
            let shift = c.is_ascii_uppercase() || matches!(c, '<' | '>');
            let modifiers = if shift {
                egui::Modifiers::SHIFT
            } else {
                egui::Modifiers::NONE
            };
            self.pass(chord(key, key, modifiers, &c.to_string()));
        }

        fn tap(&mut self, key: egui::Key) {
            self.pass(tap(key));
        }

        /// Cmd/Ctrl + `key`.
        fn command(&mut self, key: egui::Key) {
            let held = egui::Modifiers::COMMAND;
            let mut events = vec![egui::Event::ModifiersChanged(held)];
            events.extend(chord(key, key, held, ""));
            self.pass(events);
            self.pass(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        }

        fn sizes_asked(&self) -> usize {
            self.commands
                .iter()
                .filter(|c| matches!(c, egui::ViewportCommand::InnerSize(_)))
                .count()
        }

        /// Everything a click or key may change that both skins must agree on: the whole
        /// interface state, the tool settings, the picture, the brush, undo, the scroll
        /// position and what is open.
        fn state(&self) -> String {
            let app = &self.app;
            // The skin itself is the one thing the two windows may differ in.
            let ui = UiState {
                ui_style: UiStyle::Classic,
                ..app.ui.clone()
            };
            format!(
                "{:?}",
                (
                    &ui,
                    &app.tools,
                    app.page().into_owned(),
                    app.engine.document().palette(),
                    app.custom_brush.is_some(),
                    (app.engine.can_undo(), app.engine.can_redo()),
                    app.scroll,
                    app.modal_open(),
                )
            )
        }
    }

    /// Runs `script` in a Classic and in a Modern window; after every step both must be in
    /// the same state. `script` reports each step through its second argument.
    fn in_both_skins(modern: Rig, script: impl Fn(&mut Rig, &mut dyn FnMut(&mut Rig, String))) {
        let run = |mut rig: Rig| {
            let mut states = vec![("start".to_string(), rig.state())];
            script(&mut rig, &mut |rig: &mut Rig, step: String| {
                states.push((step, rig.state()));
            });
            states
        };
        let (classic, modern) = (run(Rig::classic()), run(modern));
        assert_eq!(classic.len(), modern.len());
        assert!(classic.len() > 1, "the script reported its steps");
        for ((step, c), (_, m)) in classic.iter().zip(&modern) {
            if c != m {
                // The first difference, not two whole states.
                let at = c
                    .bytes()
                    .zip(m.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                let from = at.saturating_sub(120);
                panic!(
                    "the skins differ after '{step}':\n classic ..{}\n modern  ..{}",
                    &c[from..(at + 120).min(c.len())],
                    &m[from..(at + 120).min(m.len())]
                );
            }
        }
    }

    /// Every tool button with both buttons (split tools: both halves), every built-in brush
    /// and palette cells with both buttons.
    fn click_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        // Something on the page for Undo and Clear to act on.
        rig.click(Place::Page(40, 40), Primary);
        seen(rig, "a dot on the page".into());
        for tool in TOOL_ROWS.into_iter().flatten() {
            for filled in [false, true] {
                if filled && !tool.is_split() {
                    continue;
                }
                for button in [Primary, Secondary] {
                    rig.click(Place::Tool(tool, filled), button);
                    seen(
                        rig,
                        format!("{button:?} on {tool:?} (filled half: {filled})"),
                    );
                    // A right click may have opened the tool's requester: Esc closes it.
                    if rig.app.modal_open() {
                        rig.tap(egui::Key::Escape);
                        assert!(
                            !rig.app.modal_open(),
                            "Esc closes the requester of {tool:?}"
                        );
                        seen(rig, format!("Esc after {tool:?}"));
                    }
                }
            }
        }
        for brush in 0..10 {
            rig.click(Place::Brush(brush), Primary);
            assert_eq!(rig.app.ui.brush, brush);
            seen(rig, format!("brush {brush}"));
        }
        rig.click(Place::Brush(3), Secondary);
        seen(rig, "right button on brush 3".into());
        for colour in [0u8, 1, 7, 15, 16, 30, 31] {
            rig.click(Place::Cell(colour), Primary);
            assert_eq!(rig.app.ui.fg, colour);
            seen(rig, format!("foreground {colour}"));
            let other = 31 - colour;
            rig.click(Place::Cell(other), Secondary);
            assert_eq!(rig.app.ui.bg, other);
            seen(rig, format!("background {other}"));
        }
    }

    #[test]
    fn every_tool_brush_and_palette_click_does_the_same_in_both_skins() {
        in_both_skins(Rig::modern(), click_script);
    }

    /// The same clicks at twice the device scale and in an odd window at 1.5.
    #[test]
    fn clicks_do_the_same_at_other_device_scales() {
        in_both_skins(
            Rig::new(UiStyle::Modern, (1280.0, 800.0), 2.0),
            click_script,
        );
        in_both_skins(
            Rig::new(UiStyle::Modern, (1001.0, 700.0), 1.5),
            click_script,
        );
    }

    /// Keys of every kind (tools, toggles, paint modes, chrome, colours, brush size, undo,
    /// scrolling, a requester) and strokes on the page between them.
    fn key_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use egui::Key;
        let line = [
            Place::Page(20, 30),
            Place::Page(60, 50),
            Place::Page(110, 90),
        ];
        for c in ['v', 'R', 'c', 'E', 'D', 's', 'q', 'f', 'b', 'd'] {
            rig.key(c);
            seen(rig, format!("key {c}"));
        }
        rig.drag(&line, Primary);
        seen(rig, "a freehand stroke".into());
        rig.key('R');
        rig.drag(&[Place::Page(150, 100), Place::Page(200, 160)], Secondary);
        seen(rig, "a filled rectangle with the right button".into());
        for c in ['g', '/'] {
            rig.key(c);
            seen(rig, format!("key {c}"));
        }
        // Magnify around the page pixel under the pointer. It splits the screen differently
        // in the two skins (Classic keeps the part under its toolbox free), so the pointer
        // then waits on a button, where both skins see no page pixel.
        rig.move_to(Place::Page(100, 100));
        rig.key('m');
        assert_eq!(rig.app.tools.magnify, Some((100, 100)));
        seen(rig, "key m".into());
        rig.move_to(Place::Tool(Tool::Undo, false));
        for c in [
            '>', '>', '<', 'm', '/', 'g', ']', ']', '[', '=', '=', '-', '.',
        ] {
            rig.key(c);
            seen(rig, format!("key {c}"));
        }
        for key in [
            Key::F2,
            Key::F3,
            Key::F6,
            Key::F1,
            Key::F9,
            Key::F9,
            Key::F10,
        ] {
            rig.tap(key);
            seen(rig, format!("{key:?}"));
        }
        // Without bar and dock: the stroke still lands on the same page pixels.
        rig.key('v');
        rig.drag(&[Place::Page(300, 10), Place::Page(250, 200)], Primary);
        seen(rig, "a line under where the toolbox was".into());
        rig.tap(Key::F10);
        seen(rig, "F10 again".into());
        // The Modern picture moved when bar and dock came back; the pointer follows it.
        rig.move_to(Place::Page(250, 200));
        for c in ['u', 'u', 'U', 'j', 'j', 'K', 'u'] {
            rig.key(c);
            seen(rig, format!("key {c}"));
        }
        // A requester by key and away again; then the page pixel under the pointer.
        rig.key('p');
        assert!(rig.app.ui.palette_requester.is_some(), "the Color Mixer");
        seen(rig, "p".into());
        rig.tap(Key::Escape);
        seen(rig, "Esc".into());
        rig.app.ui.prefs.set(Pref::Coords, true);
        rig.move_to(Place::Page(77, 99));
        assert_eq!(rig.app.ui.coords, Some((77, 99)));
        seen(rig, "the pointer on the page with Coords".into());
        rig.key(',');
        seen(rig, "comma picks the colour under the pointer".into());
        for key in [Key::ArrowRight, Key::ArrowDown, Key::Tab, Key::Tab] {
            rig.tap(key);
            seen(rig, format!("{key:?}"));
        }
    }

    #[test]
    fn a_key_script_with_strokes_does_the_same_in_both_skins() {
        in_both_skins(Rig::modern(), key_script);
        in_both_skins(Rig::new(UiStyle::Modern, (1280.0, 800.0), 2.0), key_script);
    }

    fn zoom(rig: &Rig) -> (u32, u8) {
        let l = rig.app.modern_layout().expect("layout");
        (l.zoom, rig.app.ui.pixel_size.0)
    }

    #[test]
    fn command_plus_minus_and_zero_zoom_the_picture_and_never_resize_the_window() {
        use egui::Key;
        let mut rig = Rig::modern();
        // 320 × 256 at Pixel Size 3x in a 1188 × 770 viewport: three pixels per pixel.
        assert_eq!(zoom(&rig), (3, 6));
        let quad = |rig: &Rig| rig.app.modern_layout().expect("layout").picture;
        assert_eq!((quad(&rig).w, quad(&rig).h), (960.0, 768.0));
        // Each press changes what is shown: at 1x by a whole pixel per pixel.
        rig.command(Key::Minus);
        assert_eq!(zoom(&rig), (2, 5));
        assert_eq!((quad(&rig).w, quad(&rig).h), (640.0, 512.0));
        rig.command(Key::Minus);
        assert_eq!(zoom(&rig), (1, 3));
        rig.command(Key::Minus);
        assert_eq!(zoom(&rig), (1, 3), "1x is the smallest");
        rig.command(Key::Plus);
        assert_eq!(zoom(&rig), (2, 4));
        rig.command(Key::Equals);
        assert_eq!(zoom(&rig), (3, 6));
        rig.command(Key::Plus);
        assert_eq!(zoom(&rig), (3, 6), "no larger than fits the viewport");
        rig.command(Key::Minus);
        rig.command(Key::Num0);
        assert_eq!(zoom(&rig), (3, 6), "Cmd/Ctrl+0: as large as fits");
        // Prefs ▸ Pixel Size is the same value: more than fits is cut down, and the next
        // step down starts from what is shown.
        rig.app
            .run_action(Action::PixelSize(PixelSize(16)), &rig.ctx.clone());
        rig.pass(vec![]);
        assert_eq!(zoom(&rig), (3, 16));
        rig.command(Key::Minus);
        assert_eq!(zoom(&rig), (2, 5));
        // The picture stays centred and the page follows the pointer at every size.
        rig.key('s');
        rig.click(Place::Page(310, 250), Primary);
        assert_eq!(rig.app.page().get(310, 250), Some(rig.app.ui.fg));
        // Idle passes past the Classic skin's snap delay: the window was never asked to
        // change its size.
        for _ in 0..30 {
            rig.pass(vec![]);
        }
        assert_eq!(rig.sizes_asked(), 0, "{:?}", rig.commands);

        // The Classic skin, for contrast: the same key resizes the window.
        let mut classic = Rig::classic();
        classic.commands.clear();
        classic.command(Key::Minus);
        classic.pass(vec![]);
        assert_eq!(classic.app.ui.pixel_size, PixelSize(5));
        assert!(classic.sizes_asked() > 0, "{:?}", classic.commands);
    }

    #[test]
    fn zoom_steps_are_single_physical_pixels_at_2x() {
        use egui::Key;
        let mut rig = Rig::new(UiStyle::Modern, (1280.0, 800.0), 2.0);
        assert_eq!(zoom(&rig), (6, 6));
        rig.command(Key::Minus);
        assert_eq!(zoom(&rig), (5, 5));
        rig.command(Key::Minus);
        assert_eq!(zoom(&rig), (4, 4));
        rig.command(Key::Num0);
        assert_eq!(zoom(&rig), (6, 6));
        assert_eq!(rig.sizes_asked(), 0);
    }

    #[test]
    fn the_modern_window_does_not_snap_and_the_classic_one_snaps_again_afterwards() {
        // A window no pixel size fits exactly; in Classic it would snap after the delay.
        let mut rig = Rig::new(UiStyle::Modern, (1100.0, 730.0), 1.0);
        for _ in 0..40 {
            rig.pass(vec![]);
        }
        assert_eq!(rig.sizes_asked(), 0, "{:?}", rig.commands);
        let (w, h) = layout::MIN_WINDOW;
        let minimum = |rig: &Rig| {
            rig.commands
                .iter()
                .filter_map(|c| match c {
                    egui::ViewportCommand::MinInnerSize(s) => Some((s.x, s.y)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(minimum(&rig), [(w, h)], "the Modern minimum, sent once");
        // Back in Classic the window returns to the screen at the pixel size.
        rig.app.ui.ui_style = UiStyle::Classic;
        rig.pass(vec![]);
        assert!(
            rig.commands
                .contains(&egui::ViewportCommand::InnerSize(egui::vec2(960.0, 768.0)))
        );
        assert_eq!(
            minimum(&rig),
            [(w, h), (CLASSIC_MIN_WINDOW[0], CLASSIC_MIN_WINDOW[1])]
        );
    }

    #[test]
    fn the_chrome_flags_are_the_shared_ones_between_passes() {
        let mut rig = Rig::modern();
        // After a pass the shared state says what is shown, as in Classic.
        assert!(rig.app.ui.title_visible && rig.app.ui.toolbox_visible);
        let l = rig.app.modern_layout().expect("layout");
        assert!(l.bar.is_some() && l.dock.is_some());
        rig.tap(egui::Key::F10);
        assert!(!rig.app.ui.title_visible && !rig.app.ui.toolbox_visible);
        let l = rig.app.modern_layout().expect("layout");
        assert!(l.bar.is_none() && l.dock.is_none());
        assert_eq!(l.viewport, l.window);
        rig.tap(egui::Key::F9);
        assert!(rig.app.ui.title_visible && !rig.app.ui.toolbox_visible);
        rig.tap(egui::Key::F10);
        assert!(!rig.app.ui.title_visible && !rig.app.ui.toolbox_visible);
        rig.tap(egui::Key::F10);
        assert!(rig.app.ui.title_visible && rig.app.ui.toolbox_visible);
        // The picture has the whole screen: Magnify splits it over its full width.
        rig.key('m');
        let view = rig.app.view();
        let m = rig
            .app
            .magnify_layout(view, rig.app.toolbox_cover(view))
            .expect("Magnify is on");
        assert_eq!(m.end, view.screen_w);
    }

    #[test]
    fn every_corner_of_a_page_pixel_is_that_pixel_at_every_device_scale() {
        for (size, ppp) in [
            ((1280.0, 800.0), 1.0),
            ((1280.0, 800.0), 2.0),
            ((1001.0, 700.0), 1.5),
            ((900.0, 620.0), 1.25),
        ] {
            let mut rig = Rig::new(UiStyle::Modern, size, ppp);
            rig.app.ui.prefs.set(Pref::Coords, true);
            for (x, y) in [(0, 0), (319, 255), (17, 23), (160, 128)] {
                for corner in [(0.02, 0.02), (0.98, 0.02), (0.02, 0.98), (0.98, 0.98)] {
                    let pos = modern_point_at(&rig.app, Place::Page(x, y), corner);
                    rig.pass(vec![egui::Event::PointerMoved(pos)]);
                    assert_eq!(
                        rig.app.ui.coords,
                        Some((x, y)),
                        "corner {corner:?} at {ppp}x in {size:?}"
                    );
                }
            }
            // Just outside the quad there is canvas: no page pixel, no stroke.
            let l = rig.app.modern_layout().expect("layout");
            let outside = egui::pos2(l.picture.x - 0.6 / ppp, l.picture.y + 5.0);
            rig.pass(vec![
                egui::Event::PointerMoved(outside),
                pointer_button(outside, Primary, true),
            ]);
            rig.pass(vec![pointer_button(outside, Primary, false)]);
            assert_eq!(rig.app.ui.coords, None);
            assert!(
                !rig.app.engine.can_undo(),
                "a click on the canvas paints nothing"
            );
        }
    }

    #[test]
    fn a_region_is_rasterised_again_only_when_it_changes() {
        let mut rig = Rig::modern();
        // The first pass made every region once.
        assert_eq!(rig.app.modern.renders(), [1, 1, 1]);
        for _ in 0..5 {
            rig.pass(vec![]);
        }
        assert_eq!(rig.app.modern.renders(), [1, 1, 1], "idle passes");
        // The pointer on a tool button raises it: the dock only.
        rig.move_to(Place::Tool(Tool::Curve, false));
        assert_eq!(rig.app.modern.renders(), [1, 2, 1]);
        rig.move_to(Place::Tool(Tool::Curve, false));
        assert_eq!(rig.app.modern.renders(), [1, 2, 1], "the same button again");
        // Selecting it: the dock again.
        rig.click(Place::Tool(Tool::Curve, false), Primary);
        assert_eq!(rig.app.modern.renders(), [1, 3, 1]);
        // Painting changes the picture, which is no region: nothing is rasterised.
        rig.key('s');
        let before = rig.app.modern.renders();
        rig.drag(
            &[
                Place::Page(10, 10),
                Place::Page(90, 70),
                Place::Page(200, 30),
            ],
            Primary,
        );
        assert!(rig.app.engine.can_undo());
        assert_eq!(rig.app.modern.renders()[VIEWPORT], 1);
        assert_eq!(rig.app.modern.renders()[BAR], before[BAR]);
        // The coordinates in the bar follow the pointer: the bar only.
        rig.app.ui.prefs.set(Pref::Coords, true);
        rig.move_to(Place::Page(5, 5));
        let before = rig.app.modern.renders();
        rig.move_to(Place::Page(6, 5));
        let after = rig.app.modern.renders();
        assert_eq!(after[BAR], before[BAR] + 1);
        assert_eq!(
            (after[VIEWPORT], after[DOCK]),
            (before[VIEWPORT], before[DOCK])
        );
        // A paint mode shows in the bar only; a zoom step moves the picture's shadow.
        rig.tap(egui::Key::F4);
        let moded = rig.app.modern.renders();
        assert_eq!(moded, [after[0], after[1], after[2] + 1]);
        rig.command(egui::Key::Minus);
        assert_eq!(rig.app.modern.renders()[VIEWPORT], moded[VIEWPORT] + 1);
        // A new window size or device scale redoes everything.
        let before = rig.app.modern.renders();
        rig.size = egui::vec2(1000.0, 700.0);
        rig.pass(vec![]);
        rig.ppp = 2.0;
        rig.pass(vec![]);
        let after = rig.app.modern.renders();
        for i in [VIEWPORT, DOCK, BAR] {
            assert_eq!(after[i], before[i] + 2, "region {i}");
        }
    }

    #[test]
    fn the_theme_follows_the_system_unless_it_is_fixed() {
        use dpx_ui::ThemeChoice;
        let mut rig = Rig::modern();
        assert_eq!(
            rig.app.modern.theme(),
            Some(ThemeKind::Dark),
            "nothing known: dark"
        );
        rig.system = Some(egui::Theme::Light);
        rig.pass(vec![]);
        // egui learns the system's theme in the pass that brings it.
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Light));
        rig.system = Some(egui::Theme::Dark);
        rig.pass(vec![]);
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Dark));
        rig.app.ui.theme = ThemeChoice::Light;
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Light), "fixed");
        rig.app.ui.theme = ThemeChoice::Dark;
        rig.system = Some(egui::Theme::Light);
        rig.pass(vec![]);
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Dark), "fixed");
    }

    /// What the live window shows, as an image: probes, and PNGs for a look at them when
    /// `DPX_MODERN_SHOTS` names a directory.
    #[test]
    fn the_live_window_shows_the_state_it_shares_with_classic() {
        let shot = |rig: &mut Rig, kind: ThemeKind, name: &str| {
            let (w, h, rgba) = rig.app.modern_window_image(kind);
            if let Ok(dir) = std::env::var("DPX_MODERN_SHOTS") {
                let path = format!("{dir}/{name}.png");
                write_png(&path, w, h, &rgba).expect("a writable directory");
            }
            (w, rgba)
        };
        let at = |(w, rgba): &(u32, Vec<u8>), l: &MainLayout, x: f32, y: f32| {
            pixel(rgba, *w, (x * l.scale) as u32, (y * l.scale) as u32)
        };
        let rgb = |c: dpx_ui_modern::paint::Color| [c.r, c.g, c.b];
        for (kind, ppp) in [(ThemeKind::Dark, 1.0), (ThemeKind::Light, 2.0)] {
            let theme = Theme::of(kind);
            let mut rig = Rig::new(UiStyle::Modern, (1280.0, 800.0), ppp);
            // A filled circle in colour 5, selected by key and drawn with the pointer.
            rig.click(Place::Cell(5), Primary);
            rig.key('C');
            rig.drag(&[Place::Page(160, 128), Place::Page(200, 128)], Primary);
            rig.key('g');
            rig.move_to(Place::Tool(Tool::Text, false));
            let name = format!("live-{kind:?}-{ppp}x").to_lowercase();
            let image = shot(&mut rig, kind, &name);
            let l = rig.app.modern_layout().expect("layout");
            let colour = |i: u8| {
                let c = rig.app.palette().get(i).expect("colour");
                [c.r, c.g, c.b]
            };
            // The picture: the circle's middle and the page beside it, at the quad's place.
            let page = |x: i32, y: i32| {
                let p = modern_point(&rig.app, Place::Page(x, y));
                at(&image, &l, p.x, p.y)
            };
            assert_eq!(page(160, 128), colour(5), "{name}");
            assert_eq!(page(190, 128), colour(5), "{name}");
            assert_eq!(page(10, 10), colour(0), "{name}");
            // The chrome: the chosen half of Circle on the accent, Grid ringed, Text raised
            // under the pointer, bar and canvas in the theme's colours.
            let circle = l.tool_button(Tool::Circle).expect("button");
            assert_eq!(
                at(&image, &l, circle.right() - 4.5, circle.bottom() - 9.5),
                rgb(theme.accent),
                "{name}"
            );
            let text = l.tool_button(Tool::Text).expect("button");
            assert_eq!(
                at(&image, &l, text.x + 4.5, text.y + 9.5),
                rgb(theme.hover),
                "{name}"
            );
            let grid = l.tool_button(Tool::Grid).expect("button");
            assert_eq!(
                at(&image, &l, grid.x + grid.w / 2.0, grid.y + 0.7),
                rgb(theme.accent),
                "{name}: the ring of a toggle that is on"
            );
            let bar = l.bar.expect("bar");
            assert_eq!(
                at(&image, &l, bar.right() - 3.0, bar.y + 3.0),
                rgb(theme.panel)
            );
            assert_eq!(
                at(&image, &l, l.viewport.x + 3.0, l.viewport.y + 3.0),
                rgb(theme.canvas)
            );
            // Nothing of the classic chrome is in the picture: its top-left and top-right
            // corners are page.
            assert_eq!(page(0, 0), colour(0), "{name}");
            assert_eq!(page(319, 0), colour(0), "{name}");
        }
        // Magnify, a requester and the Control Panel are shown in the Classic look inside
        // the quad for now.
        let mut rig = Rig::modern();
        rig.key('s');
        rig.drag(&[Place::Page(150, 120), Place::Page(170, 135)], Primary);
        rig.key('m');
        shot(&mut rig, ThemeKind::Dark, "live-magnify");
        rig.key('m');
        rig.click(Place::Tool(Tool::Grid, false), Secondary);
        assert!(rig.app.ui.tool_requester.is_some());
        shot(&mut rig, ThemeKind::Light, "live-requester");
        rig.tap(egui::Key::Escape);
        rig.pass(chord(egui::Key::A, egui::Key::A, egui::Modifiers::ALT, ""));
        assert!(
            rig.app.ui.anim_panel.is_some(),
            "Alt-a shows the Control Panel"
        );
        shot(&mut rig, ThemeKind::Dark, "live-anim-panel");
        rig.tap(egui::Key::F10);
        shot(&mut rig, ThemeKind::Dark, "live-f10");
    }

    // ---- Menus, tooltips, the switch and the theme (slice 1c of ADR-47) ----

    use dpx_ui::menu::{self as classic_menu, Entry, MENUS};
    use dpx_ui::{PaintMode, ThemeChoice as Choice};
    use dpx_ui_modern::menu as modern_menu;

    /// Where the item that runs `wanted` is: its menu, its item and, when it is in a
    /// submenu, its place there.
    fn menu_place(wanted: Action) -> (usize, usize, Option<usize>) {
        for (m, menu) in MENUS.iter().enumerate() {
            for (i, item) in menu.items.iter().enumerate() {
                match item.entry {
                    Entry::Action(a) if a == wanted => return (m, i, None),
                    Entry::Submenu(items) => {
                        if let Some(s) = items.iter().position(|it| it.action() == Some(wanted)) {
                            return (m, i, Some(s));
                        }
                    }
                    Entry::Action(_) => {}
                }
            }
        }
        panic!("{wanted:?} is in no menu");
    }

    impl Rig {
        /// The window points on the way to the item that runs `wanted`, in this rig's skin:
        /// over the menu's title, over its item and, for a submenu, over the item there.
        fn menu_points(&self, wanted: Action) -> Vec<egui::Pos2> {
            let (m, item, sub) = menu_place(wanted);
            let sub_items = match MENUS[m].items[item].entry {
                Entry::Submenu(items) => Some(items),
                Entry::Action(_) => None,
            };
            if self.app.is_modern() {
                let l = self.app.modern_layout().expect("layout");
                let middle = |r: Rect| egui::pos2(r.x + r.w / 2.0, r.y + r.h / 2.0);
                let card = modern_menu::menu_card(&l, m).expect("the bar is shown");
                let mut points = vec![middle(l.menu_titles[m]), middle(card.row(item))];
                if let (Some(s), Some(items)) = (sub, sub_items) {
                    let sub_card = modern_menu::submenu_card(&l, &card, item, items);
                    points.push(middle(sub_card.row(s)));
                }
                points
            } else {
                let (w, h) = self.app.view().ui_size();
                let slot = classic_menu::title_slots()[m];
                let mbox = classic_menu::menu_box(m, w, h);
                let mut pixels = vec![
                    (slot.x + 2, 1),
                    (mbox.x + mbox.w / 2, mbox.item_top(item) + 1),
                ];
                if let (Some(s), Some(items)) = (sub, sub_items) {
                    let sbox = classic_menu::submenu_box(&mbox, item, items, w, h);
                    pixels.push((sbox.x + sbox.w / 2, sbox.item_top(s) + 1));
                }
                pixels.into_iter().map(|p| ui_point(&self.app, p)).collect()
            }
        }

        /// The right button goes down on the title of the menu that holds `wanted` and the
        /// pointer moves to its item; the button stays down. Returns where the pointer is.
        fn hold_menu(&mut self, wanted: Action) -> egui::Pos2 {
            let points = self.menu_points(wanted);
            self.pass(vec![
                egui::Event::PointerMoved(points[0]),
                pointer_button(points[0], Secondary, true),
            ]);
            assert!(self.app.ui.menu.active, "the menus are held");
            for &p in &points[1..] {
                self.pass(vec![egui::Event::PointerMoved(p)]);
            }
            let (m, item, sub) = menu_place(wanted);
            let menu = self.app.ui.menu;
            assert_eq!(
                (menu.menu, menu.item, menu.sub_item),
                (Some(m), Some(item), sub),
                "the pointer is over {wanted:?}"
            );
            points[points.len() - 1]
        }

        /// Picks the item that runs `wanted` as on the Amiga: right button down on the
        /// title, along to the item, up over it. This is `pick` for either skin.
        fn pick(&mut self, wanted: Action) {
            let last = self.hold_menu(wanted);
            self.pass(vec![pointer_button(last, Secondary, false)]);
            assert!(!self.app.ui.menu.active, "letting go closes the menus");
        }
    }

    /// An item of every menu, picked with the right button: a plain command, one in a
    /// submenu, radio and check items, a ghosted item, an item that opens a requester, and
    /// the left button while the menu is held (multi-select).
    fn menu_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        // Something on the page that Flip moves and BG -> FG recolours.
        rig.click(Place::Page(40, 40), Primary);
        rig.click(Place::Cell(5), Primary);
        seen(rig, "a dot on the page".into());
        let rest = Place::Page(200, 200);
        for action in [
            Action::Command("picture.flip_horizontal"),
            Action::Mode(PaintMode::Smear),
            Action::Command("anim.method_expanded"),
            Action::Command("color.bg_to_fg"),
            Action::Command("antialias.high"),
            Action::Pref(Pref::Coords),
            Action::Pref(Pref::Coords),
            Action::PixelSize(PixelSize(4)),
            Action::PixelSize(PixelSize(6)),
            // Ghosted: nothing happens.
            Action::Command("picture.print"),
            Action::Command("brush.halve"),
        ] {
            rig.pick(action);
            assert!(!rig.app.modal_open(), "{action:?} opens nothing");
            // The menus are not in the same place in the two skins: the pointer goes to
            // the same page pixel before the states are compared.
            rig.move_to(rest);
            seen(rig, format!("picked {action:?}"));
        }
        assert_eq!(rig.app.ui.mode, PaintMode::Smear);
        assert!(rig.app.ui.anim_expanded);
        // Items that open a requester: it is the same one, and Esc closes it.
        for id in ["palette.mixer", "picture.page_size", "anim.set_rate"] {
            rig.pick(Action::Command(id));
            assert!(rig.app.modal_open(), "{id} opens its requester");
            seen(rig, format!("picked {id}"));
            rig.tap(egui::Key::Escape);
            assert!(!rig.app.modal_open(), "Esc closes {id}");
            rig.move_to(rest);
            seen(rig, format!("Esc after {id}"));
        }
        // Multi-select: the left button picks while the right one holds the menu open.
        let at = rig.hold_menu(Action::Mode(PaintMode::Shade));
        rig.pass(vec![pointer_button(at, Primary, true)]);
        rig.pass(vec![pointer_button(at, Primary, false)]);
        assert_eq!(rig.app.ui.mode, PaintMode::Shade);
        assert!(rig.app.ui.menu.active, "the menu stays open");
        seen(rig, "left click on Shade in the held menu".into());
        let at = rig.menu_points(Action::Mode(PaintMode::Blend))[1];
        rig.pass(vec![egui::Event::PointerMoved(at)]);
        rig.pass(vec![pointer_button(at, Primary, true)]);
        rig.pass(vec![pointer_button(at, Primary, false)]);
        assert_eq!(rig.app.ui.mode, PaintMode::Blend);
        seen(rig, "left click on Blend".into());
        // Let go over Cycle: one more pick; the menu closes.
        let at = rig.menu_points(Action::Mode(PaintMode::Cycle))[1];
        rig.pass(vec![egui::Event::PointerMoved(at)]);
        rig.pass(vec![pointer_button(at, Secondary, false)]);
        assert_eq!(rig.app.ui.mode, PaintMode::Cycle);
        assert!(!rig.app.ui.menu.active);
        rig.move_to(rest);
        seen(rig, "let go over Cycle".into());
        // Keys do nothing while the menu is held; letting go away from it picks nothing.
        rig.hold_menu(Action::Mode(PaintMode::Matte));
        rig.tap(egui::Key::F3);
        assert_eq!(rig.app.ui.mode, PaintMode::Cycle, "no hotkey under a menu");
        let away = rig.point(rest);
        rig.pass(vec![egui::Event::PointerMoved(away)]);
        rig.pass(vec![pointer_button(away, Secondary, false)]);
        assert_eq!(rig.app.ui.mode, PaintMode::Cycle);
        assert!(!rig.app.ui.menu.active);
        seen(rig, "let go away from the menu".into());
    }

    #[test]
    fn menu_picks_do_the_same_in_both_skins() {
        in_both_skins(Rig::modern(), menu_script);
        in_both_skins(Rig::new(UiStyle::Modern, (1001.0, 700.0), 1.5), menu_script);
    }

    /// Every item of every menu and submenu is reached with the pointer in the live Modern
    /// window, and the action under it is the menu model's.
    #[test]
    fn every_menu_item_is_under_the_pointer_in_the_live_window() {
        let mut rig = Rig::new(UiStyle::Modern, (900.0, 620.0), 2.0);
        let mut seen = 0;
        for menu in MENUS.iter() {
            for item in menu.items {
                let actions: Vec<Action> = match item.entry {
                    Entry::Action(a) => vec![a],
                    Entry::Submenu(items) => items.iter().filter_map(|i| i.action()).collect(),
                };
                for action in actions {
                    let last = rig.hold_menu(action);
                    let ctx = rig.app.ui.menu_context();
                    assert_eq!(
                        rig.app.ui.menu.hovered_action(&ctx),
                        Some(action).filter(|a| a.enabled(&ctx)),
                        "{action:?}"
                    );
                    // Back to the bar beside the titles before letting go: no item is
                    // under the pointer, so nothing is picked.
                    let _ = last;
                    let bar = egui::pos2(880.0, 10.0);
                    rig.pass(vec![egui::Event::PointerMoved(bar)]);
                    assert_eq!(rig.app.ui.menu.item, None);
                    rig.pass(vec![pointer_button(bar, Secondary, false)]);
                    assert!(!rig.app.ui.menu.active && !rig.app.modal_open());
                    seen += 1;
                }
            }
        }
        assert!(seen > 100, "{seen} items");
        assert!(!rig.app.engine.can_undo(), "nothing was picked");
    }

    #[test]
    fn f12_and_the_chord_switch_the_skin_from_the_next_pass_on() {
        use egui::Key;
        let mut rig = Rig::classic();
        // Something to undo and redo, to see that the chord is not Shift+U.
        rig.click(Place::Page(30, 30), Primary);
        rig.key('u');
        assert!(rig.app.engine.can_redo());
        let before = (rig.app.ui.tool, rig.app.scroll, rig.app.page().into_owned());
        rig.tap(Key::F12);
        assert!(rig.app.is_modern(), "F12: Modern");
        // The same pass was still a Classic one; the next one lays the Modern window out.
        assert!(rig.app.modern_layout().is_some());
        rig.pass(vec![]);
        assert!(rig.app.ui.title_visible && rig.app.ui.toolbox_visible);
        assert_eq!(
            (rig.app.ui.tool, rig.app.scroll, rig.app.page().into_owned()),
            before,
            "the document and the tool are shared"
        );
        // Painting works at once in the new skin.
        rig.click(Place::Page(100, 90), Primary);
        assert_eq!(rig.app.page().get(100, 90), Some(rig.app.ui.fg));
        rig.key('u');
        // Cmd/Ctrl+Shift+U: back, and no redo.
        let held = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let chord_u = |rig: &mut Rig| {
            let mut events = vec![egui::Event::ModifiersChanged(held)];
            events.extend(chord(Key::U, Key::U, held, ""));
            rig.pass(events);
            rig.pass(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        };
        chord_u(&mut rig);
        assert!(!rig.app.is_modern(), "the chord: Classic");
        assert!(rig.app.engine.can_redo(), "the chord is not a redo");
        assert_eq!(rig.app.page().get(100, 90), Some(0));
        chord_u(&mut rig);
        assert!(rig.app.is_modern());
        // Not while a requester is open, and not while text is typed.
        rig.key('p');
        assert!(rig.app.modal_open());
        rig.tap(Key::F12);
        assert!(rig.app.is_modern(), "a requester takes the key");
        rig.tap(Key::Escape);
        rig.click(Place::Tool(Tool::Text, false), Primary);
        rig.click(Place::Page(50, 50), Primary);
        assert!(rig.app.typing());
        rig.tap(Key::F12);
        chord_u(&mut rig);
        assert!(rig.app.is_modern(), "typing takes the keys");
        rig.tap(Key::Escape);
        assert!(!rig.app.typing());
        rig.tap(Key::F12);
        assert!(!rig.app.is_modern());
    }

    #[test]
    fn a_small_classic_window_grows_to_the_modern_minimum() {
        let mut rig = Rig::new(UiStyle::Classic, (640.0, 512.0), 1.0);
        rig.commands.clear();
        rig.tap(egui::Key::F12);
        rig.pass(vec![]);
        let (w, h) = layout::MIN_WINDOW;
        assert!(
            rig.commands
                .contains(&egui::ViewportCommand::InnerSize(egui::vec2(w, h))),
            "{:?}",
            rig.commands
        );
        // A window that is large enough keeps its size.
        let mut rig = Rig::classic();
        rig.commands.clear();
        rig.tap(egui::Key::F12);
        for _ in 0..30 {
            rig.pass(vec![]);
        }
        assert_eq!(rig.sizes_asked(), 0, "{:?}", rig.commands);
    }

    #[test]
    fn prefs_switches_the_skin_and_the_theme_and_both_are_remembered() {
        let dir = std::env::temp_dir().join(format!("dpx-modern-switch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory");
        let (store, first) = crate::config::ConfigStore::open(&dir);
        assert_eq!(first.ui_style, UiStyle::Classic, "a first start is Classic");
        let mut rig = Rig::classic();
        rig.app.config = Some(store);
        // Classic: Prefs ▸ Theme is ghosted, Prefs ▸ Modern UI switches.
        let ctx = rig.app.ui.menu_context();
        assert_eq!(Action::UiStyle.checked(&ctx), Some(false));
        rig.pick(Action::Theme(Choice::Light));
        assert_eq!(rig.app.ui.theme, Choice::System, "no theme in Classic");
        rig.pick(Action::UiStyle);
        assert!(rig.app.is_modern());
        rig.pass(vec![]);
        // Modern: the item is checked; the theme items work and are radio marks.
        let ctx = rig.app.ui.menu_context();
        assert_eq!(Action::UiStyle.checked(&ctx), Some(true));
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Dark));
        rig.pick(Action::Theme(Choice::Light));
        assert_eq!(rig.app.ui.theme, Choice::Light);
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Light));
        let ctx = rig.app.ui.menu_context();
        assert_eq!(Action::Theme(Choice::Light).checked(&ctx), Some(true));
        assert_eq!(Action::Theme(Choice::System).checked(&ctx), Some(false));
        // A fixed theme does not follow the system; System does.
        rig.system = Some(egui::Theme::Dark);
        rig.pass(vec![]);
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Light));
        rig.pick(Action::Theme(Choice::System));
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Dark));
        rig.system = Some(egui::Theme::Light);
        rig.pass(vec![]);
        rig.pass(vec![]);
        assert_eq!(rig.app.modern.theme(), Some(ThemeKind::Light));
        rig.pick(Action::Theme(Choice::Dark));
        // The settings file gets both once they have stopped changing.
        for _ in 0..6 {
            rig.t += 2.0;
            rig.pass(vec![]);
        }
        let (_, saved) = crate::config::ConfigStore::open(&dir);
        assert_eq!(
            (saved.ui_style, saved.theme),
            (UiStyle::Modern, Choice::Dark)
        );
        // The next start shows what was chosen.
        let mut next = PainterApp::new();
        saved.apply(&mut next);
        assert!(next.is_modern());
        assert_eq!(next.ui.theme, Choice::Dark);
        // And back by menu, in the Modern skin's own menu.
        rig.pick(Action::UiStyle);
        assert!(!rig.app.is_modern());
        for _ in 0..6 {
            rig.t += 2.0;
            rig.pass(vec![]);
        }
        let (_, saved) = crate::config::ConfigStore::open(&dir);
        assert_eq!(
            (saved.ui_style, saved.theme),
            (UiStyle::Classic, Choice::Dark)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn shot(rig: &mut Rig, kind: ThemeKind, name: &str) -> (u32, Vec<u8>) {
        let (w, h, rgba) = rig.app.modern_window_image(kind);
        if let Ok(dir) = std::env::var("DPX_MODERN_SHOTS") {
            let path = format!("{dir}/{name}.png");
            write_png(&path, w, h, &rgba).expect("a writable directory");
        }
        (w, rgba)
    }

    fn probe((w, rgba): &(u32, Vec<u8>), l: &MainLayout, x: f32, y: f32) -> [u8; 3] {
        pixel(rgba, *w, (x * l.scale) as u32, (y * l.scale) as u32)
    }

    fn rgb(c: dpx_ui_modern::paint::Color) -> [u8; 3] {
        [c.r, c.g, c.b]
    }

    /// The held menu is drawn by the Modern skin over the window (nothing of the classic
    /// strip is in the picture), costs one overlay image and a new bar, and goes with the
    /// button.
    #[test]
    fn the_held_menu_is_an_overlay_over_the_modern_window() {
        for (kind, ppp) in [(ThemeKind::Dark, 1.0), (ThemeKind::Light, 2.0)] {
            let theme = Theme::of(kind);
            let mut rig = Rig::new(UiStyle::Modern, (1280.0, 800.0), ppp);
            rig.app.ui.theme = match kind {
                ThemeKind::Dark => Choice::Dark,
                ThemeKind::Light => Choice::Light,
            };
            rig.click(Place::Cell(5), Primary);
            rig.key('R');
            rig.drag(&[Place::Page(20, 10), Place::Page(300, 240)], Primary);
            rig.pass(vec![]);
            let before = rig.app.modern.renders();
            assert_eq!(rig.app.modern.overlay(), (0, false));
            rig.hold_menu(Action::Mode(PaintMode::Smear));
            let after = rig.app.modern.renders();
            assert_eq!(
                (after[VIEWPORT], after[DOCK]),
                (before[VIEWPORT], before[DOCK]),
                "viewport and dock are not redone for a menu"
            );
            assert!(after[BAR] > before[BAR], "the open title is marked");
            let (renders, shown) = rig.app.modern.overlay();
            assert!(shown && renders >= 1);
            // The pointer moving inside the same row redoes nothing.
            let at = rig.menu_points(Action::Mode(PaintMode::Smear))[1];
            rig.pass(vec![egui::Event::PointerMoved(at + egui::vec2(3.0, 2.0))]);
            assert_eq!(rig.app.modern.overlay(), (renders, true));
            assert_eq!(rig.app.modern.renders(), after);

            let name = format!("live-menu-{kind:?}-{ppp}x").to_lowercase();
            let image = shot(&mut rig, kind, &name);
            let l = rig.app.modern_layout().expect("layout");
            let (m, item, _) = menu_place(Action::Mode(PaintMode::Smear));
            let card = modern_menu::menu_card(&l, m).expect("card");
            let row = card.row(item);
            // The row under the pointer and the open title are on the accent; another row
            // is the card; beside the card the picture shows (the rectangle in colour 5).
            assert_eq!(
                probe(&image, &l, row.x + 3.0, row.y + 12.0),
                rgb(theme.accent)
            );
            let other = card.row(item + 2);
            assert_eq!(
                probe(&image, &l, other.x + 3.0, other.y + 12.0),
                rgb(theme.raised)
            );
            let title = l.menu_titles[m];
            assert_eq!(
                probe(&image, &l, title.x + 2.0, title.y + title.h / 2.0),
                rgb(theme.accent)
            );
            let c = rig.app.palette().get(5).expect("colour 5");
            let beside = modern_point(&rig.app, Place::Page(150, 100));
            assert!(!card.rect.inset(-20.0).contains(beside.x, beside.y));
            assert_eq!(probe(&image, &l, beside.x, beside.y), [c.r, c.g, c.b]);
            // The top of the picture is page, not a classic menu strip.
            let top = modern_point(&rig.app, Place::Page(310, 0));
            let page0 = rig.app.palette().get(0).expect("colour 0");
            assert_eq!(probe(&image, &l, top.x, top.y), [page0.r, page0.g, page0.b]);

            rig.pass(vec![pointer_button(at, Secondary, false)]);
            assert_eq!(rig.app.ui.mode, PaintMode::Smear);
            assert!(!rig.app.modern.overlay().1, "the overlay is gone");
        }
    }

    /// A pointer resting on a dock button shows the shared tooltip in the Modern look,
    /// beside the dock; the Classic drawing of it is not in the picture.
    #[test]
    fn a_resting_pointer_shows_the_tooltip_beside_the_dock() {
        let mut rig = Rig::modern();
        rig.app.ui.theme = Choice::Light;
        let theme = Theme::light();
        rig.move_to(Place::Tool(Tool::Line, false));
        assert_eq!(rig.app.ui.tooltip, None, "not at once");
        rig.t += crate::tooltip::DELAY + 0.1;
        rig.pass(vec![]);
        let tip = rig
            .app
            .ui
            .tooltip
            .clone()
            .expect("the tooltip after the delay");
        assert_eq!(tip.text, "Straight Line  v");
        let (renders, shown) = rig.app.modern.overlay();
        assert!(shown);
        for _ in 0..3 {
            rig.pass(vec![]);
        }
        assert_eq!(rig.app.modern.overlay(), (renders, true), "idle passes");
        let l = rig.app.modern_layout().expect("layout");
        let rect = build::tooltip_rect(&l, &tip).expect("a dock button's tooltip");
        let (dock, button) = (
            l.dock.as_ref().expect("dock").rect,
            l.tool_button(Tool::Line).expect("button"),
        );
        assert!(rect.right() < dock.x && rect.x > l.picture.x, "{rect:?}");
        assert!(rect.y < button.center().1 && rect.bottom() > button.center().1);
        let image = shot(&mut rig, ThemeKind::Light, "live-tooltip-light");
        assert_eq!(
            probe(&image, &l, rect.x + 4.0, rect.y + rect.h / 2.0),
            rgb(theme.raised)
        );
        // The other half of a split button at once, with its own text.
        rig.move_to(Place::Tool(Tool::Rectangle, true));
        assert_eq!(
            rig.app.ui.tooltip.as_ref().map(|t| t.text.as_str()),
            Some("Filled Rectangle  R")
        );
        assert!(rig.app.modern.overlay().0 > renders);
        // A press hides it; so does the held menu.
        rig.click(Place::Tool(Tool::Rectangle, true), Primary);
        assert_eq!(rig.app.ui.tooltip, None);
        assert!(!rig.app.modern.overlay().1);
        // A tooltip shown in one skin does not survive the switch to the other.
        rig.move_to(Place::Tool(Tool::Curve, false));
        rig.t += crate::tooltip::DELAY + 0.1;
        rig.pass(vec![]);
        assert!(rig.app.ui.tooltip.is_some());
        rig.tap(egui::Key::F12);
        assert_eq!(rig.app.ui.tooltip, None);
        assert_eq!(rig.app.ui.menu, dpx_ui::MenuState::default());
    }
}
