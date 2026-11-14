//! The app side of the Modern skin (ADR-47): the live window and the headless snapshot.
//!
//! The live window is the Classic window with other surroundings. The picture quad of the
//! Modern layout is the classic screen rectangle, composed without the classic chrome and
//! shown as the same nearest-neighbour texture, so every mapping from the pointer to the
//! screen and the page, every tool, key and requester works unchanged. Around it the bar,
//! the dock and the layer column (Prefs ▸ Layers) are drawn by `dpx-ui-modern` on the CPU,
//! each into its own image that is redone only when its display list changes; dock and layer
//! column answer a click with the Classic skin's [`Hit`] values, so `handle_press` does not
//! know which skin it came from. A requester, a note, Show Page and playback are shown in
//! the Classic look inside the quad; the chrome around lies under a scrim then and takes no
//! input. The Screen Format requester, which has a screen of its own, gets the whole window. The pull-down
//! menus (held with the right button on the bar, as in Classic) and the tooltips of the dock
//! buttons are one more image over everything; they write and read the same `MenuState`
//! and `Tooltip` as the Classic skin.
//!
//! During a Modern pass the two classic chrome flags (`title_visible`, `toolbox_visible`)
//! are switched off in the shared state and kept aside here: the classic code then lays
//! out, draws and hit-tests a screen without title bar and toolbox. They still mean "bar
//! shown" and "dock shown" (F9, F10) and are back in the shared state after the pass.

use dpx_core::Document;
use dpx_core::IndexedImage;
use dpx_ui::{Hit, KeyMode, PixelSize, ThemeChoice, Tooltip, TooltipTarget, UiState, UiStyle};
use dpx_ui_modern::paint::{List, Rect};
use dpx_ui_modern::{
    Chrome, Frame, Image, MainLayout, Model, Picture, Renderer, Spot, Theme, ThemeKind, Toggles,
    build, layout,
};
use eframe::egui;

use crate::{CLASSIC_MIN_WINDOW, PainterApp, View, arg_value, snapshot_rgba, write_png};

/// The parts of the window that are rasterised on their own, in painting order.
const REGIONS: usize = 4;
/// Index of each region in [`State::regions`] (and of its count in `State::renders`).
const VIEWPORT: usize = 0;
#[cfg(test)]
const DOCK: usize = 1;
#[cfg(test)]
const BAR: usize = 2;
#[cfg(test)]
const LAYERS: usize = 3;
const REGION_NAMES: [&str; REGIONS] = [
    "modern-viewport",
    "modern-dock",
    "modern-bar",
    "modern-layers",
];
/// The rows the layer list is given while the column is not shown (F10): all of them.
const ALL_ROWS: usize = 4096;
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
    /// While a mouse button is held outside the menus: the chrome and the Pixel Size the
    /// layout had when it went down. F9, F10, `L` and the zoom keys change the shared state
    /// at once, as in Classic, but the picture stays where it is until the button is up, so
    /// a stroke or a rubber band does not get a stray segment from a quad that moved under
    /// the resting pointer.
    frozen: Option<(Chrome, PixelSize)>,
    /// The size of the Modern window in whole points, as last seen while it was neither
    /// maximised nor full screen: what the next start opens (settings key `modern_window`).
    window_size: Option<(u16, u16)>,
    /// The window was checked against the monitor since the Modern skin was shown.
    fitted: bool,
    /// The theme of the last painted pass.
    #[cfg(test)]
    theme: Option<ThemeKind>,
}

impl State {
    /// The remembered size of the Modern window, in points.
    pub(crate) fn window_size(&self) -> Option<(u16, u16)> {
        self.window_size
    }

    /// Sets the remembered size of the Modern window (from the settings file).
    pub(crate) fn set_window_size(&mut self, size: Option<(u16, u16)>) {
        self.window_size = size;
    }

    /// How often each region was rasterised so far, by [`VIEWPORT`], [`DOCK`], [`BAR`].
    #[cfg(test)]
    pub(crate) fn renders(&self) -> [u64; 3] {
        [VIEWPORT, DOCK, BAR].map(|i| self.regions[i].renders)
    }

    /// How often the layer column was rasterised so far.
    #[cfg(test)]
    pub(crate) fn layer_renders(&self) -> u64 {
        self.regions[LAYERS].renders
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
                // Not a flag of its own: see `modern_chrome`.
                layers: false,
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

    /// Which parts of the Modern chrome are shown. The layer column is shown with the dock
    /// while Prefs ▸ Layers is on (F10 hides both). The Screen Format requester has a screen
    /// of its own, as in Classic: it gets the whole window, without any chrome.
    fn modern_chrome(&self) -> Chrome {
        let none = Chrome {
            bar: false,
            dock: false,
            layers: false,
        };
        if self.requester.is_some() {
            return none;
        }
        let flags = self.modern.held.unwrap_or(Chrome {
            bar: self.ui.title_visible,
            dock: self.ui.toolbox_visible,
            layers: false,
        });
        Chrome {
            layers: flags.dock && self.layer_panel_on(),
            ..flags
        }
    }

    /// How many rows the Modern layer column has room for; `None` in the Classic skin. All
    /// rows while the column is not shown, so nothing is scrolled away when it comes back.
    pub(crate) fn modern_layer_rows(&self) -> Option<usize> {
        let layout = self.modern_layout()?;
        Some(layout.layers.map_or(ALL_ROWS, |column| column.capacity))
    }

    /// The screen for the picture quad or the Classic window. The Modern skin draws the
    /// tooltips of its own chrome (dock and layer column) itself, so they are not part of
    /// the quad; the Control Panel's, which is still shown in the Classic look, is.
    pub(crate) fn compose_for_skin(&mut self) -> IndexedImage {
        let own = self.is_modern()
            && self
                .ui
                .tooltip
                .as_ref()
                .is_some_and(|tip| !matches!(tip.target, TooltipTarget::Panel(_)));
        if !own {
            return self.compose_screen();
        }
        let tip = self.ui.tooltip.take();
        let screen = self.compose_screen();
        self.ui.tooltip = tip;
        screen
    }

    /// The layout of the Modern window as of the last pass's window size; `None` in the
    /// Classic skin and before the first pass.
    pub(crate) fn modern_layout(&self) -> Option<MainLayout> {
        if !self.is_modern() {
            return None;
        }
        let win = self.window?;
        let view = self.view();
        let hosts = self.modern_dimmed();
        // A held mouse button keeps the layout it went down in (see `State::frozen`); a
        // requester that opened meanwhile has the pointer and is laid out as it is.
        let (chrome, pixel_size) = match self.modern.frozen.filter(|_| !self.modal_open()) {
            Some(kept) => kept,
            None => (self.modern_chrome(), self.ui.pixel_size),
        };
        Some(MainLayout::for_picture(
            (win.w as f32 / win.ppp, win.h as f32 / win.ppp),
            win.ppp,
            chrome,
            self.palette().len(),
            picture_of(view, pixel_size, hosts),
        ))
    }

    /// Before a pass, after [`PainterApp::modern_begin_pass`]: a mouse button that is held
    /// (not on the menus, not under a requester) keeps the layout it went down in.
    pub(crate) fn modern_hold_layout(&mut self, ctx: &egui::Context) {
        let held = ctx.input(|i| i.pointer.any_down());
        if !self.is_modern() || !held || self.ui.menu.active || self.modal_open() {
            if !held || !self.is_modern() {
                // Let go: `modern_release_layout` moved the picture after the last pass.
                self.modern.frozen = None;
            } else if self.modern.frozen.take().is_some() {
                // The menus or a requester took over while the button is held.
                self.dirty = true;
            }
            return;
        }
        if self.modern.frozen.is_none() {
            self.modern.frozen = Some((self.modern_chrome(), self.ui.pixel_size));
        }
    }

    /// After a pass: with no button held any more, what the keys changed during the
    /// gesture is laid out. The pass that saw the release still ran in the old layout, so
    /// the stroke ended where the pointer was.
    pub(crate) fn modern_release_layout(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        if let Some(kept) = self.modern.frozen.take()
            && kept != (self.modern_chrome(), self.ui.pixel_size)
        {
            self.dirty = true;
            ctx.request_repaint();
        }
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
                self.modern.frozen = None;
                self.modern.fitted = false;
            }
            self.dirty = true;
        }
        if modern && !self.free {
            let size = ctx.content_rect().size();
            // Once after the start or a switch: a window larger than the monitor (a size
            // remembered from a larger one) comes back onto it. Later the user sizes the
            // window freely.
            if !self.modern.fitted
                && let Some(monitor) = self.monitor
            {
                self.modern.fitted = true;
                if let Some(fit) = fit_to_monitor(size, monitor - crate::MONITOR_MARGIN) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(fit));
                }
            }
            // What the next start opens. A maximised or full-screen window is not a size
            // to come back to.
            let whole = |v: f32| v.round().clamp(1.0, f32::from(u16::MAX)) as u16;
            self.modern.window_size = Some((whole(size.x), whole(size.y)));
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
        let per_ui = picture_of(self.view(), self.ui.pixel_size, false).per_ui_pixel;
        if let Some(size) = layout::pixel_size_stepped(l.zoom, step, per_ui, l.scale) {
            self.set_pixel_size(size);
        }
    }

    /// Cmd/Ctrl + `0` in the Modern skin: the picture as large as fits the viewport.
    pub(crate) fn modern_zoom_to_fit(&mut self) {
        let Some(l) = self.modern_layout() else {
            return;
        };
        let per_ui = picture_of(self.view(), self.ui.pixel_size, false).per_ui_pixel;
        self.set_pixel_size(layout::pixel_size_for(l.fits, per_ui, l.scale));
    }

    /// The theme to paint in: the fixed choice, or the system's (dark when it says nothing).
    fn modern_theme(&self, ctx: &egui::Context) -> ThemeKind {
        let system_dark = ctx.system_theme().map(|t| t == egui::Theme::Dark);
        ThemeKind::resolve(self.ui.theme, system_dark)
    }

    /// The display lists of the regions with their rectangles, by [`VIEWPORT`], [`DOCK`],
    /// [`BAR`] and [`LAYERS`] (they do not overlap): the viewport is the canvas colour and
    /// the picture's shadow. A hidden region has no entry.
    fn modern_lists(&self, l: &MainLayout, theme: &Theme) -> [Option<(Rect, List)>; REGIONS] {
        let model = self.modern_model();
        [
            (l.viewport.w > 0.0 && l.viewport.h > 0.0)
                .then(|| (l.viewport, build::canvas(l, theme, model.dimmed))),
            l.dock
                .as_ref()
                .map(|dock| (dock.rect, build::dock(l, theme, &model))),
            l.bar.map(|bar| (bar, build::title_bar(l, theme, &model))),
            l.layers
                .as_ref()
                .map(|column| (column.rect, build::layers(l, theme, &model))),
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
        // The pass laid the picture out before its keys and clicks ran. When they changed
        // the layout (F9, F10, Prefs ▸ Layers, a zoom step, a requester with a screen of
        // its own), the next pass draws the picture at its new place, at once.
        let quad = egui::Rect::from_min_size(
            area.min + egui::vec2(l.picture.x, l.picture.y),
            egui::vec2(l.picture.w, l.picture.h),
        );
        if (quad.min - rect.min).abs().max_elem() > 0.01
            || (quad.size() - rect.size()).abs().max_elem() > 0.01
        {
            ctx.request_repaint();
        }
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
                && !l.over_chrome
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
        if l.over_chrome
            && let Some(screen) = &self.texture
        {
            // A hosted requester larger than the viewport: over the veiled chrome.
            painter
                .with_clip_rect(area)
                .image(screen.id(), rect, uv, egui::Color32::WHITE);
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
            dimmed: self.modern_dimmed(),
        }
    }

    /// The chrome lies under the scrim: a requester, a note or Show Page has the pointer
    /// and the keys. Playback and the Move preview take them too, but they are the picture
    /// itself running, not a question: the chrome stays as it is while they play.
    fn modern_dimmed(&self) -> bool {
        self.modal_open() && self.anim.playing.is_none() && self.anim_control.preview.is_none()
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
    ///
    /// While a mouse button is held the switch waits: the same window point lies over
    /// another page pixel in the other skin, and a stroke or rubber band that survived the
    /// switch would be joined to it. It happens after the pass that sees the button go up.
    pub(crate) fn modern_apply_switch(&mut self, ctx: &egui::Context) {
        if self.modern.switch && ctx.input(|i| i.pointer.any_down()) {
            return;
        }
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
        if l.over_chrome {
            // As `modern_paint`: a hosted requester larger than the viewport lies over
            // the veiled chrome.
            list.image(l.picture, build::PICTURE_IMAGE);
        }
        if let Some((_, over)) = self.overlay_list(&l, &theme) {
            list.append(over);
        }
        // As `upload_and_paint`: the requester's own screen, Show Page, or the screen.
        let (screen, rgba) = match (&self.requester, &self.show_page) {
            (Some(req), _) => {
                let screen = req.render();
                let rgba = dpx_ui::to_rgba(&screen, &crate::requester::palette(), 1, 1);
                (screen, rgba)
            }
            (None, Some(sp)) => {
                let screen = sp.render(&self.page(), self.palette().len());
                let rgba = dpx_ui::to_rgba(&screen, self.palette(), 1, 1);
                (screen, rgba)
            }
            (None, None) => {
                let screen = self.compose_for_skin();
                let mut rgba = dpx_ui::to_rgba(&screen, self.palette(), 1, 1);
                self.import_overlay(&mut rgba, screen.width() as usize);
                self.generate_overlay(&mut rgba, screen.width() as usize);
                (screen, rgba)
            }
        };
        // The pixel shape of the display mode, as the window's texture is stretched.
        let view = self.view();
        let (screen_w, screen_h) = (screen.width(), screen.height());
        let rgba = stretch(&rgba, screen_w, screen_h, view.fx as u32, view.fy as u32);
        let (screen_w, screen_h) = (screen_w * view.fx as u32, screen_h * view.fy as u32);
        self.modern_end_pass();
        let picture = Image {
            width: screen_w,
            height: screen_h,
            rgba: &rgba,
        };
        let px = pixels(&l.window, l.scale);
        let (w, h) = (px[2] as u32, px[3] as u32);
        let out = Renderer::new().render(&list, &[picture], w, h, l.scale, theme.canvas);
        (w, h, out)
    }
}

/// `rgba` (`w` × `h`) with every pixel `fx` wide and `fy` high.
#[cfg(test)]
fn stretch(rgba: &[u8], w: u32, h: u32, fx: u32, fy: u32) -> Vec<u8> {
    if (fx, fy) == (1, 1) {
        return rgba.to_vec();
    }
    let mut out = Vec::with_capacity(rgba.len() * (fx * fy) as usize);
    for y in 0..h as usize {
        let row = &rgba[y * w as usize * 4..(y + 1) * w as usize * 4];
        let wide: Vec<u8> = row
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| std::iter::repeat_n(p, fx as usize).flatten().copied())
            .collect();
        for _ in 0..fy {
            out.extend_from_slice(&wide);
        }
    }
    out
}

/// A requester or note of the shared interface state is open (what a snapshot can show).
fn has_requester(ui: &UiState) -> bool {
    ui.choice_requester.is_some()
        || ui.tool_requester.is_some()
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
        || ui.anim_control.is_some()
}

/// The size to ask for when a Modern window of `size` points does not fit into `avail`
/// points of the monitor: cut to it, but never below the Modern minimum. `None` when it
/// fits.
fn fit_to_monitor(size: egui::Vec2, avail: egui::Vec2) -> Option<egui::Vec2> {
    let (min_w, min_h) = layout::MIN_WINDOW;
    let fit = egui::vec2(
        size.x.min(avail.x).max(min_w),
        size.y.min(avail.y).max(min_h),
    );
    ((fit - size).abs().max_elem() >= 1.0).then_some(fit)
}

/// The window a start in the Modern skin opens, in points: the remembered size, at least
/// the Modern minimum, or the size the design was made for. Prefs ▸ Pixel Size is the
/// picture's zoom in this skin and says nothing about the window.
pub(crate) fn start_window(remembered: Option<(u16, u16)>) -> egui::Vec2 {
    let (min_w, min_h) = layout::MIN_WINDOW;
    match remembered {
        Some((w, h)) => egui::vec2(f32::from(w).max(min_w), f32::from(h).max(min_h)),
        None => egui::vec2(DEFAULT_SIZE.0, DEFAULT_SIZE.1),
    }
}

/// The picture quad's image for the screen `view` at Prefs ▸ Pixel Size `pixel_size`: the
/// screen with its pixel shape applied. `hosts_requester`: it shows a requester, a note or
/// Show Page.
fn picture_of(view: View, pixel_size: PixelSize, hosts_requester: bool) -> Picture {
    Picture {
        hosts_requester,
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
    let dimmed = has_requester(ui);
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
    // `--layers-panel`: the column has its own number of rows.
    if let (Some(panel), Some(column)) = (&mut shown.layer_panel, &layout.layers) {
        panel.docked = true;
        panel.hidden = dimmed;
        panel.scroll_by(0, column.capacity);
    }
    let hover = match options
        .pointer
        .filter(|_| !dimmed)
        .map(|(x, y)| dpx_ui_modern::hit_in(&layout, &shown, x, y))
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
            dimmed,
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
    use dpx_ui::{LayerButton, LayerHit, Pref, Screen};
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
        /// A pixel of the screen that is shown (the picture's, or the Screen Format
        /// requester's own): where requesters and the Control Panel are.
        Screen(i32, i32),
        /// A part of the layer panel (Classic) or the layer column (Modern).
        Layer(LayerHit),
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
            Place::Page(x, y) => modern_point_at(
                app,
                Place::Screen(x - app.scroll.0, y - app.scroll.1),
                (fx, fy),
            ),
            Place::Screen(sx, sy) => {
                let view = app.view();
                // One screen pixel is `fx` × `fy` picture pixels of `shown` physical pixels.
                let step = l.shown / l.scale;
                egui::pos2(
                    l.picture.x + (sx as f32 + fx) * view.fx as f32 * step,
                    l.picture.y + (sy as f32 + fy) * view.fy as f32 * step,
                )
            }
            Place::Layer(hit) => {
                let column = l.layers.as_ref().expect("the layer column is shown");
                let panel = app.ui.layer_panel.as_ref().expect("the layer panel");
                let slot = |layer: usize| {
                    panel
                        .slot_of(layer, column.capacity)
                        .expect("the layer's row is shown")
                };
                within(match hit {
                    LayerHit::Eye(layer) => column.eye(slot(layer)),
                    LayerHit::Name(layer) => column.name(slot(layer)),
                    LayerHit::Field => column.name(slot(panel.renaming().expect("a rename"))),
                    LayerHit::Button(b) => column.button(b),
                    LayerHit::ScrollUp => column.scroll_up,
                    LayerHit::ScrollDown => column.scroll_down,
                    LayerHit::Background => column.header,
                })
            }
        }
    }

    /// The window point `(fx, fy)` of the way across screen pixel (`sx`, `sy`) in the
    /// Classic window (one physical pixel per point).
    fn classic_screen_point(
        app: &PainterApp,
        (sx, sy): (i32, i32),
        (fx, fy): (f32, f32),
    ) -> egui::Pos2 {
        let (view, scale) = app.layout();
        let img_w = ((view.screen_w * view.fx) as f32 * scale).round() as i32;
        let img_h = ((view.screen_h * view.fy) as f32 * scale).round() as i32;
        let frame_w = img_w + app.layer_strip_px(view, scale);
        let win = app.window.expect("a window after a pass");
        let (ox, oy) = (((win.w - frame_w) / 2) as f32, ((win.h - img_h) / 2) as f32);
        egui::pos2(
            ox + (sx as f32 + fx) * scale * view.fx as f32,
            oy + (sy as f32 + fy) * scale * view.fy as f32,
        )
    }

    /// The window point over the middle of `place` in the Modern window.
    pub(crate) fn modern_point(app: &PainterApp, place: Place) -> egui::Pos2 {
        modern_point_at(app, place, (0.5, 0.5))
    }

    /// The window point over `place` in the Classic window (a LoRes screen): a UI pixel the
    /// classic hit test answers for it, from the middle of all that do.
    fn classic_point(app: &PainterApp, place: Place) -> egui::Pos2 {
        let view = app.view();
        let wanted = match place {
            Place::Page(x, y) => return ui_point(app, (x - app.scroll.0, y - app.scroll.1)),
            Place::Screen(x, y) => return classic_screen_point(app, (x, y), (0.5, 0.5)),
            Place::Tool(tool, filled_half) => Hit::Tool { tool, filled_half },
            Place::Brush(i) => Hit::Brush(i),
            Place::Cell(i) => Hit::PaletteCell(i),
            Place::Layer(hit) => Hit::LayerPanel(hit),
        };
        assert_eq!((view.ux, view.uy), (1, 1), "a LoRes screen");
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
        /// When the last pass wanted the next one.
        repaint: std::time::Duration,
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
                repaint: std::time::Duration::MAX,
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
            self.repaint = out.repaint_delay;
            self.commands.extend(out.commands);
        }

        fn point(&self, place: Place) -> egui::Pos2 {
            self.point_at(place, (0.5, 0.5))
        }

        /// The point `frac` of the way across `place` (in the Classic window only for a
        /// screen pixel; its buttons are found by their middle).
        fn point_at(&self, place: Place, frac: (f32, f32)) -> egui::Pos2 {
            match (self.app.is_modern(), place) {
                (true, _) => modern_point_at(&self.app, place, frac),
                (false, Place::Screen(x, y)) => classic_screen_point(&self.app, (x, y), frac),
                (false, _) => classic_point(&self.app, place),
            }
        }

        /// A click `frac` of the way across `place`.
        fn click_at(&mut self, place: Place, frac: (f32, f32), button: egui::PointerButton) {
            let pos = self.point_at(place, frac);
            self.pass(vec![
                egui::Event::PointerMoved(pos),
                pointer_button(pos, button, true),
            ]);
            self.pass(vec![pointer_button(pos, button, false)]);
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
            // The skin itself is the one thing the two windows may differ in, and with it
            // where the layer panel is: over the screen, or a column beside the picture.
            let mut ui = UiState {
                ui_style: UiStyle::Classic,
                ..app.ui.clone()
            };
            if let Some(panel) = &mut ui.layer_panel {
                panel.docked = false;
            }
            let doc = app.engine.document();
            let layers: Vec<(&str, bool)> = doc
                .layers()
                .iter()
                .map(|l| (l.name.as_str(), l.visible))
                .collect();
            format!(
                "{:?}",
                (
                    (layers, doc.active_layer()),
                    (doc.current_frame(), doc.frame_count()),
                    (&app.requester, app.show_page.is_some()),
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

    // ---- The layer column, requesters in the quad, the Control Panel (slice 1d of ADR-47) ----

    use dpx_core::Command;
    use dpx_ui::{ReqEvent, ReqResponse};

    fn layer(hit: LayerHit) -> Place {
        Place::Layer(hit)
    }

    fn layer_names(rig: &Rig) -> Vec<String> {
        let doc = rig.app.engine.document();
        doc.layers().iter().map(|l| l.name.clone()).collect()
    }

    /// Two clicks on `place` a moment after whatever was clicked before.
    fn double_click(rig: &mut Rig, place: Place) {
        rig.t += 1.0;
        rig.click(place, Primary);
        rig.click(place, Primary);
    }

    /// The layer panel through its whole job: shown by key, a new layer, painting on it,
    /// the eye, selecting, every button, the right button, renaming with Return, Esc and a
    /// click elsewhere, undo and redo.
    fn layer_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use LayerButton as B;
        let count = |rig: &Rig| rig.app.engine.document().layer_count();
        let active = |rig: &Rig| rig.app.engine.document().active_layer();
        rig.key('L');
        assert!(rig.app.ui.layer_panel.is_some(), "Shift-L shows the panel");
        seen(rig, "Shift-L".into());
        rig.click(Place::Page(40, 40), Primary);
        seen(rig, "a dot on the Background".into());
        rig.click(layer(LayerHit::Button(B::New)), Primary);
        assert_eq!((count(rig), active(rig)), (2, 1));
        seen(rig, "New".into());
        rig.click(Place::Cell(5), Primary);
        rig.click(Place::Page(60, 60), Primary);
        assert_eq!(rig.app.page().get(60, 60), Some(5));
        assert_eq!(
            rig.app.engine.document().layer(0).unwrap().get(60, 60),
            Some(0)
        );
        seen(rig, "a dot on the new layer".into());
        // The eye hides the layer and shows it again.
        rig.t += 1.0;
        rig.click(layer(LayerHit::Eye(1)), Primary);
        assert_eq!(rig.app.page().get(60, 60), Some(0), "hidden");
        seen(rig, "the eye of layer 1".into());
        rig.t += 1.0;
        rig.click(layer(LayerHit::Eye(1)), Primary);
        assert_eq!(rig.app.page().get(60, 60), Some(5), "shown again");
        seen(rig, "the eye again".into());
        // A click on a row selects its layer; the right button does nothing.
        rig.t += 1.0;
        rig.click(layer(LayerHit::Name(0)), Primary);
        assert_eq!(active(rig), 0);
        seen(rig, "select the Background".into());
        rig.t += 1.0;
        rig.click(layer(LayerHit::Name(1)), Secondary);
        rig.click(layer(LayerHit::Button(B::New)), Secondary);
        assert_eq!((count(rig), active(rig)), (2, 0));
        seen(rig, "the right button on a row and on New".into());
        // Every button, each a command of its own.
        for (button, layers, now) in [
            (B::Up, 2, 1),
            (B::Down, 2, 0),
            (B::Copy, 3, 1),
            (B::New, 4, 2),
            (B::Merge, 3, 1),
            (B::Delete, 2, 0),
            // Ghosted on the bottom layer: nothing happens.
            (B::Down, 2, 0),
            (B::Merge, 2, 0),
        ] {
            rig.t += 1.0;
            rig.click(layer(LayerHit::Button(button)), Primary);
            assert_eq!((count(rig), active(rig)), (layers, now), "{button:?}");
            assert_eq!(
                rig.app.ui.layer_panel.as_ref().and_then(|p| p.held),
                None,
                "{button:?} is up again"
            );
            seen(rig, format!("{button:?}"));
        }
        // A double-click on a name edits it; Return takes what was typed.
        double_click(rig, layer(LayerHit::Name(1)));
        assert_eq!(rig.app.renaming_layer(), Some(1));
        seen(rig, "a double-click on layer 1".into());
        let old = layer_names(rig)[1].clone();
        rig.pass(vec![egui::Event::Text(" hills".into())]);
        rig.tap(egui::Key::ArrowLeft);
        rig.tap(egui::Key::Backspace);
        seen(rig, "typed".into());
        // Keys go to the gadget, not to the tools: `v` would select Line.
        let tool = rig.app.ui.tool;
        rig.key('v');
        assert_eq!(rig.app.ui.tool, tool);
        rig.tap(egui::Key::Enter);
        assert_eq!(rig.app.renaming_layer(), None);
        assert_eq!(layer_names(rig)[1], format!("{old} hilvs"));
        seen(rig, "Return".into());
        // Esc keeps the old name.
        double_click(rig, layer(LayerHit::Name(0)));
        assert_eq!(rig.app.renaming_layer(), Some(0));
        rig.pass(vec![egui::Event::Text("x".into())]);
        rig.tap(egui::Key::Escape);
        assert_eq!(rig.app.renaming_layer(), None);
        assert_eq!(layer_names(rig)[0], "Background");
        seen(rig, "Esc".into());
        // A click in the gadget keeps it open; a click elsewhere takes the name and does
        // nothing else (no dot on the page).
        double_click(rig, layer(LayerHit::Name(0)));
        rig.pass(vec![egui::Event::Text("!".into())]);
        rig.click(layer(LayerHit::Field), Primary);
        assert_eq!(rig.app.renaming_layer(), Some(0));
        seen(rig, "a click in the gadget".into());
        let page = rig.app.page().into_owned();
        rig.click(Place::Page(100, 200), Primary);
        assert_eq!(rig.app.renaming_layer(), None);
        assert_eq!(layer_names(rig)[0], "Background!");
        assert_eq!(
            rig.app.page().into_owned(),
            page,
            "the click only took the name"
        );
        seen(rig, "a click on the page takes the name".into());
        // Undo and redo are the document's.
        for c in ['u', 'u', 'U'] {
            rig.key(c);
            seen(rig, format!("key {c}"));
        }
        assert_eq!(layer_names(rig)[0], "Background");
        rig.key('L');
        assert!(rig.app.ui.layer_panel.is_none(), "Shift-L hides it");
        seen(rig, "Shift-L again".into());
    }

    #[test]
    fn the_layer_panel_does_the_same_in_both_skins() {
        in_both_skins(Rig::modern(), layer_script);
        in_both_skins(
            Rig::new(UiStyle::Modern, (1001.0, 700.0), 1.5),
            layer_script,
        );
        in_both_skins(Rig::new(UiStyle::Modern, (720.0, 520.0), 2.0), layer_script);
    }

    /// The column is part of the Modern layout: beside the picture, never in it; it goes
    /// with the dock (F10) and with Prefs ▸ Layers, and the picture is laid out again at
    /// once.
    #[test]
    fn the_layer_column_stands_beside_the_picture() {
        let mut rig = Rig::modern();
        let wide = rig.app.modern_layout().expect("layout");
        assert!(wide.layers.is_none());
        rig.app.run(Command::Clear { color: 5 });
        rig.pass(vec![]);
        rig.key('L');
        assert_eq!(
            rig.repaint,
            std::time::Duration::ZERO,
            "the picture moved: another pass at once"
        );
        rig.pass(vec![]);
        let l = rig.app.modern_layout().expect("layout");
        let column = l.layers.clone().expect("the layer column");
        let dock = l.dock.as_ref().expect("dock").rect;
        assert_eq!(column.rect.right(), dock.x);
        assert_eq!(l.viewport.w, wide.viewport.w - column.rect.w);
        assert!(l.viewport.encloses(&l.picture));
        let panel = rig.app.ui.layer_panel.clone().expect("the panel");
        assert!(panel.docked, "beside the picture");
        // The picture is the whole page: where Classic lays its panel there is page.
        let image = shot(&mut rig, ThemeKind::Dark, "live-layers-dark");
        let c = rig.app.palette().get(5).expect("colour 5");
        for (x, y) in [(0, 0), (319, 0), (200, 100), (239, 250), (319, 255)] {
            let p = modern_point(&rig.app, Place::Page(x, y));
            assert_eq!(probe(&image, &l, p.x, p.y), [c.r, c.g, c.b], "({x}, {y})");
        }
        // The column itself: the active row is a raised card with the accent at its edge.
        let theme = Theme::dark();
        let row = column.row(0);
        assert_eq!(
            probe(&image, &l, row.right() - 12.0, row.y + row.h / 2.0),
            rgb(theme.raised)
        );
        assert_eq!(
            probe(&image, &l, row.x + 5.5, row.y + row.h / 2.0),
            rgb(theme.accent)
        );
        // A name being edited is a field in the row, with the canvas colour as its ground.
        rig.app.run(Command::AddLayer {
            name: Some("Clouds".into()),
        });
        rig.pass(vec![]);
        double_click(&mut rig, layer(LayerHit::Name(1)));
        rig.pass(vec![egui::Event::Text(" at dusk".into())]);
        assert_eq!(
            rig.app
                .ui
                .layer_panel
                .as_ref()
                .and_then(|p| p.rename_text()),
            Some("Clouds at dusk".into())
        );
        let editing = shot(&mut rig, ThemeKind::Dark, "live-layers-rename");
        let field = column.name(0);
        assert_eq!(
            probe(&editing, &l, field.right() - 6.0, field.y + field.h / 2.0),
            rgb(theme.canvas)
        );
        rig.tap(egui::Key::Enter);
        assert_eq!(layer_names(&rig), ["Background", "Clouds at dusk"]);
        rig.click(layer(LayerHit::Name(0)), Primary);
        // Painting still lands on the page pixel under the pointer.
        rig.click(Place::Cell(9), Primary);
        rig.click(Place::Page(300, 250), Primary);
        assert_eq!(rig.app.page().get(300, 250), Some(9));
        // A click on the column never paints, wherever it lands.
        let before = rig.app.page().into_owned();
        for place in [
            layer(LayerHit::Background),
            layer(LayerHit::Name(0)),
            layer(LayerHit::ScrollUp),
        ] {
            rig.t += 1.0;
            rig.click(place, Primary);
        }
        assert_eq!(rig.app.page().into_owned(), before);
        // F10 hides the column with the dock, F9 only the bar.
        rig.tap(egui::Key::F10);
        let bare = rig.app.modern_layout().expect("layout");
        assert!(bare.layers.is_none() && bare.dock.is_none());
        assert!(rig.app.ui.layer_panel.is_some(), "Prefs ▸ Layers stays on");
        rig.tap(egui::Key::F10);
        rig.tap(egui::Key::F9);
        let no_bar = rig.app.modern_layout().expect("layout");
        assert!(no_bar.bar.is_none());
        assert_eq!(no_bar.layers.expect("the layer column").rect.y, 0.0);
        rig.tap(egui::Key::F9);
        // Back in Classic the panel lies over the screen again.
        rig.tap(egui::Key::F12);
        rig.pass(vec![]);
        let panel = rig.app.ui.layer_panel.clone().expect("the panel");
        assert!(!panel.docked);
        let (w, h) = rig.app.view().ui_size();
        let classic = dpx_ui::layer_panel::Layout::of(&rig.app.ui, w, h).expect("shown");
        let pens = dpx_ui::Pens::from_palette(rig.app.palette());
        let screen = rig.app.compose_screen();
        assert_eq!(screen.get(classic.panel.x, 100), Some(pens.dark));
    }

    /// More layers than rows: the wheel over the column, the arrows and a new active layer
    /// scroll the list; the wheel over the picture does not.
    #[test]
    fn the_wheel_and_the_arrows_scroll_the_layer_column() {
        let mut rig = Rig::new(UiStyle::Modern, (720.0, 520.0), 1.0);
        rig.key('L');
        for _ in 0..29 {
            rig.app.run(Command::AddLayer { name: None });
        }
        rig.pass(vec![]);
        let l = rig.app.modern_layout().expect("layout");
        let rows = l.layers.as_ref().expect("the layer column").capacity;
        assert_eq!(rows, 16);
        let scroll = |rig: &Rig| rig.app.ui.layer_panel.as_ref().expect("panel").scroll;
        // The active layer is the top one: its row shows.
        assert_eq!(rig.app.engine.document().active_layer(), 29);
        assert_eq!(scroll(&rig), 0);
        let wheel = |rig: &mut Rig, place: Place, lines: f32| {
            let pos = rig.point(place);
            rig.pass(vec![egui::Event::PointerMoved(pos)]);
            rig.pass(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, lines),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
            }]);
        };
        wheel(&mut rig, layer(LayerHit::Background), -3.0);
        assert_eq!(scroll(&rig), 3);
        // The rows followed: the top row is layer 26 now, and a click selects it.
        rig.click(layer(LayerHit::Name(26)), Primary);
        assert_eq!(rig.app.engine.document().active_layer(), 26);
        let top = modern_point(&rig.app, layer(LayerHit::Name(26)));
        let column = l.layers.as_ref().expect("the layer column");
        assert!(column.row(0).contains(top.x, top.y));
        wheel(&mut rig, layer(LayerHit::Background), -100.0);
        assert_eq!(scroll(&rig), 30 - rows, "no further than the last row");
        wheel(&mut rig, layer(LayerHit::Background), 2.0);
        assert_eq!(scroll(&rig), 30 - rows - 2);
        // Over the picture the wheel is the page's.
        wheel(&mut rig, Place::Page(100, 100), -3.0);
        assert_eq!(scroll(&rig), 30 - rows - 2);
        // The arrows: one row each, ghosted at the ends.
        rig.t += 1.0;
        rig.click(layer(LayerHit::ScrollDown), Primary);
        assert_eq!(scroll(&rig), 30 - rows - 1);
        rig.t += 1.0;
        rig.click(layer(LayerHit::ScrollUp), Primary);
        rig.t += 1.0;
        rig.click(layer(LayerHit::ScrollUp), Primary);
        assert_eq!(scroll(&rig), 30 - rows - 3);
        // Another active layer (here by command, as an MCP client would) scrolls into view.
        rig.app.run(Command::SelectLayer { layer: 0 });
        rig.pass(vec![]);
        assert_eq!(scroll(&rig), 30 - rows);
        let image = shot(&mut rig, ThemeKind::Light, "live-layers-scrolled");
        let _ = image;
        // A larger window has more rows: nothing stays scrolled away needlessly.
        rig.size = egui::vec2(1280.0, 800.0);
        rig.app.dirty = true;
        rig.pass(vec![]);
        rig.pass(vec![]);
        let l = rig.app.modern_layout().expect("layout");
        let rows = l.layers.as_ref().expect("the layer column").capacity;
        assert!(rows < 30);
        assert_eq!(scroll(&rig), 30 - rows);
    }

    /// The layer column is its own region: the pointer over it, a click and a rename redo
    /// it and nothing else; painting redoes none.
    #[test]
    fn the_layer_column_is_rasterised_on_its_own() {
        let mut rig = Rig::modern();
        assert_eq!(rig.app.modern.layer_renders(), 0);
        rig.key('L');
        rig.pass(vec![]);
        rig.app.run(Command::AddLayer { name: None });
        rig.pass(vec![]);
        let layers = rig.app.modern.layer_renders();
        assert!(layers >= 1);
        let others = rig.app.modern.renders();
        for _ in 0..4 {
            rig.pass(vec![]);
        }
        assert_eq!(rig.app.modern.layer_renders(), layers, "idle passes");
        rig.move_to(layer(LayerHit::Name(0)));
        assert_eq!(
            rig.app.modern.layer_renders(),
            layers + 1,
            "the row is raised"
        );
        rig.move_to(layer(LayerHit::Name(0)));
        assert_eq!(rig.app.modern.layer_renders(), layers + 1);
        rig.move_to(layer(LayerHit::Button(LayerButton::Copy)));
        assert_eq!(rig.app.modern.layer_renders(), layers + 2);
        assert_eq!(rig.app.modern.renders(), others, "nothing else");
        // The tooltip of a column button: the Modern overlay left of the column. The
        // Classic drawing of it is not in the picture.
        let plain = rig.app.modern_window_image(ThemeKind::Dark);
        rig.t += crate::tooltip::DELAY + 0.1;
        rig.pass(vec![]);
        let tip = rig.app.ui.tooltip.clone().expect("the tooltip");
        assert_eq!(tip.target, TooltipTarget::Layer(LayerButton::Copy));
        assert!(rig.app.modern.overlay().1, "drawn by the Modern skin");
        let l = rig.app.modern_layout().expect("layout");
        let rect = build::tooltip_rect(&l, &tip).expect("its place");
        let column = l.layers.as_ref().expect("the layer column").rect;
        assert!(rect.right() < column.x);
        let tipped = shot(&mut rig, ThemeKind::Dark, "live-layers-tooltip");
        let (w, _, plain) = plain;
        let differing: Vec<(u32, u32)> = (0..plain.len() / 4)
            .filter(|i| plain[i * 4..i * 4 + 4] != tipped.1[i * 4..i * 4 + 4])
            .map(|i| (i as u32 % w, i as u32 / w))
            .collect();
        assert!(!differing.is_empty());
        let reach = rect.inset(-dpx_ui_modern::menu::SHADOW_REACH - 1.0);
        for (x, y) in differing {
            assert!(
                reach.contains(x as f32 + 0.5, y as f32 + 0.5),
                "({x}, {y}) changed outside the Modern tooltip {rect:?}"
            );
        }
        // Painting leaves the column alone (the pointer has left it first).
        rig.move_to(Place::Page(10, 10));
        let layers = rig.app.modern.layer_renders();
        rig.drag(&[Place::Page(10, 10), Place::Page(80, 60)], Primary);
        rig.move_to(Place::Page(80, 61));
        assert_eq!(rig.app.modern.layer_renders(), layers);
    }

    /// The first and the last pixel, row by row, of a 640 × 512 area where `wanted` holds:
    /// for a button, its top-left and its bottom-right pixel.
    fn pixels_where(wanted: impl Fn(i32, i32) -> bool) -> ((i32, i32), (i32, i32)) {
        let all: Vec<(i32, i32)> = (0..512)
            .flat_map(|y| (0..640).map(move |x| (x, y)))
            .filter(|&(x, y)| wanted(x, y))
            .collect();
        (
            *all.first().expect("a pixel that answers"),
            *all.last().expect("a pixel that answers"),
        )
    }

    /// The screen pixel of UI pixel (`x`, `y`) of the screen `rig` shows.
    fn ui_pixel(rig: &Rig, (x, y): (i32, i32)) -> Place {
        let view = rig.app.view();
        Place::Screen(x * view.ux, y * view.uy)
    }

    const NEAR: (f32, f32) = (0.1, 0.1);
    const FAR: (f32, f32) = (0.9, 0.9);

    /// A tool's settings (Grid, with the right button on its tool): Cancel by its first
    /// pixel, Ok by its last; a pixel beside a button is not the button.
    fn tool_requester_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::ToolReqResponse as R;
        let (w, h) = rig.app.view().ui_size();
        for (wanted, corner) in [(R::Cancel, NEAR), (R::Ok, FAR)] {
            rig.click(Place::Tool(Tool::Grid, false), Secondary);
            let req = rig.app.ui.tool_requester.clone().expect("Grid's requester");
            let (ox, oy) = req.origin(w, h);
            let (first, last) = pixels_where(|x, y| {
                let mut r = req.clone();
                r.handle(ReqEvent::Press(x, y)) == wanted || r.handle(ReqEvent::Release) == wanted
            });
            seen(rig, format!("the requester for {wanted:?}"));
            // One pixel above the button's top row: not the button.
            let above = ui_pixel(rig, (ox + first.0, oy + first.1 - 1));
            rig.click_at(above, (0.5, 0.9), Primary);
            assert!(rig.app.ui.tool_requester.is_some(), "beside {wanted:?}");
            seen(rig, format!("a click above {wanted:?}"));
            let pixel = if corner == NEAR { first } else { last };
            let at = ui_pixel(rig, (ox + pixel.0, oy + pixel.1));
            rig.click_at(at, corner, Primary);
            assert!(rig.app.ui.tool_requester.is_none(), "{wanted:?} closes it");
            seen(rig, format!("{wanted:?}"));
        }
    }

    /// The file requester (Picture ▸ Load..): typing goes to its File field; a pixel beside
    /// Cancel is not Cancel; Cancel by its last pixel.
    fn file_requester_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::FileResponse as R;
        rig.pick(Action::Command("picture.load"));
        let req = rig.app.ui.file_requester.clone().expect("Load..");
        seen(rig, "Load..".into());
        let (w, h) = rig.app.view().ui_size();
        let (ox, oy) = dpx_ui::FileRequester::origin(w, h);
        let (first, last) = pixels_where(|x, y| {
            let mut r = req.clone();
            r.handle(ReqEvent::Press(x, y), 0) == R::Cancel
                || r.handle(ReqEvent::Release, 0) == R::Cancel
        });
        rig.pass(vec![egui::Event::Text("abc".into())]);
        let file = rig.app.ui.file_requester.as_ref().map(|r| r.file());
        assert_eq!(file.as_deref(), Some("abc"));
        seen(rig, "typed into the File field".into());
        let above = ui_pixel(rig, (ox + first.0, oy + first.1 - 1));
        rig.click_at(above, (0.5, 0.9), Primary);
        assert!(rig.app.ui.file_requester.is_some(), "beside Cancel");
        seen(rig, "a click above Cancel".into());
        rig.click_at(ui_pixel(rig, (ox + last.0, oy + last.1)), FAR, Primary);
        assert!(rig.app.ui.file_requester.is_none(), "Cancel closes it");
        seen(rig, "Cancel".into());
    }

    /// Screen Format, on its own HiRes screen: the Chipset gadget, then Cancel by its
    /// first pixel. In the Modern window it has the whole window.
    fn screen_format_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::ScreenFormatRequester as Req;
        let target = rig.app.engine.document().target();
        rig.pick(Action::Command("picture.screen_format"));
        rig.pass(vec![]);
        let req = rig.app.requester.clone().expect("Screen Format");
        seen(rig, "Screen Format".into());
        if let Some(l) = rig.app.modern_layout() {
            assert!(l.bar.is_none() && l.dock.is_none() && l.layers.is_none());
            assert_eq!(l.viewport, l.window);
            assert!(l.viewport.encloses(&l.picture), "the whole requester shows");
        }
        let chipset = req.chipset();
        let g = Req::chipset_gadget();
        rig.click(Place::Screen(g.x + g.w / 2, g.y + g.h / 2), Primary);
        let now = rig.app.requester.as_ref().map(Req::chipset);
        assert!(
            now.is_some() && now != Some(chipset),
            "the gadget took the click"
        );
        seen(rig, "Chipset".into());
        let (first, _) = pixels_where(|x, y| {
            let mut r = req.clone();
            r.handle(ReqEvent::Press(x, y)) == ReqResponse::Cancel
                || r.handle(ReqEvent::Release) == ReqResponse::Cancel
        });
        rig.click_at(Place::Screen(first.0, first.1), NEAR, Primary);
        assert!(rig.app.requester.is_none(), "Cancel closes it");
        assert_eq!(rig.app.engine.document().target(), target);
        rig.pass(vec![]);
        seen(rig, "Cancel".into());
        if let Some(l) = rig.app.modern_layout() {
            assert!(l.bar.is_some() && l.dock.is_some(), "the chrome is back");
        }
    }

    /// Show Page on a page larger than the screen: a click moves the view there.
    fn show_page_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        rig.app.run(Command::SetPageSize {
            width: 640,
            height: 512,
            fill: 0,
        });
        rig.pass(vec![]);
        seen(rig, "a 640 x 512 page".into());
        rig.pick(Action::Command("picture.show_page"));
        assert!(rig.app.show_page.is_some());
        seen(rig, "Show Page".into());
        rig.click_at(Place::Screen(250, 200), FAR, Primary);
        assert!(rig.app.show_page.is_none(), "the click chose the view");
        assert_ne!(rig.app.scroll, (0, 0));
        seen(rig, "a click at (250, 200)".into());
        rig.app.scroll_to(0, 0);
        rig.app.run(Command::SetPageSize {
            width: 320,
            height: 256,
            fill: 0,
        });
        rig.pass(vec![]);
        seen(rig, "the screen-sized page again".into());
    }

    /// The Color Mixer: a click on a colour selects it, a click in a slider changes it.
    fn color_mixer_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::PaletteRequester as Req;
        rig.key('p');
        let (w, h) = rig.app.view().ui_size();
        let (ox, oy) = Req::origin(w, h);
        let cell = rig
            .app
            .ui
            .palette_requester
            .as_ref()
            .expect("the Color Mixer")
            .cell_rect(7);
        seen(rig, "the Color Mixer".into());
        // The cell's last pixel, at its far corner.
        let last = (ox + cell.x + cell.w - 1, oy + cell.y + cell.h - 1);
        rig.click_at(ui_pixel(rig, last), FAR, Primary);
        let mixer = |rig: &Rig| {
            let req = rig.app.ui.palette_requester.as_ref().expect("the Mixer");
            (req.selected(), req.colors()[7])
        };
        assert_eq!(mixer(rig).0, 7, "the colour under the pointer");
        seen(rig, "colour 7".into());
        let before = mixer(rig).1;
        let slider = Req::slider_rect(1);
        let at = (ox + slider.x + slider.w / 3, oy + slider.y + slider.h / 2);
        rig.click(ui_pixel(rig, at), Primary);
        assert_ne!(mixer(rig).1, before, "the slider took the click");
        seen(rig, "the G slider".into());
        rig.tap(egui::Key::Escape);
        assert!(rig.app.ui.palette_requester.is_none());
        seen(rig, "Esc".into());
    }

    /// The Import requester of a true-colour image: Cancel by its last pixel.
    fn import_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::ImportResponse as R;
        let image = dpx_core::RgbaImage::new(40, 30, dpx_core::Rgba8::new(200, 40, 40, 255));
        rig.app
            .open_import_image("probe.png".into(), image, None, Vec::new());
        rig.pass(vec![]);
        let req = rig.app.ui.import_requester.clone().expect("Import");
        seen(rig, "Import".into());
        let (ox, oy) = req.origin();
        let (_, last) = pixels_where(|x, y| {
            let mut r = req.clone();
            r.handle(ReqEvent::Press(x, y)) == R::Cancel || r.handle(ReqEvent::Release) == R::Cancel
        });
        rig.click_at(ui_pixel(rig, (ox + last.0, oy + last.1)), FAR, Primary);
        assert!(rig.app.ui.import_requester.is_none(), "Cancel closes it");
        seen(rig, "Cancel".into());
    }

    /// Picture ▸ Generate..: Cancel by its first pixel.
    fn generate_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::GenerateResponse as R;
        rig.pick(Action::Command("picture.generate"));
        let req = rig.app.ui.generate_requester.clone().expect("Generate");
        seen(rig, "Generate..".into());
        let (ox, oy) = req.origin();
        let (first, _) = pixels_where(|x, y| {
            let mut r = req.clone();
            r.handle(ReqEvent::Press(x, y)) == R::Cancel || r.handle(ReqEvent::Release) == R::Cancel
        });
        rig.click_at(ui_pixel(rig, (ox + first.0, oy + first.1)), NEAR, Primary);
        assert!(rig.app.ui.generate_requester.is_none(), "Cancel closes it");
        seen(rig, "Cancel".into());
    }

    /// Prefs ▸ AI & MCP..: Close by its last pixel.
    fn mcp_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::{McpControl, McpResponse};
        rig.pick(Action::Command("app.ai_mcp"));
        let req = rig.app.ui.mcp_requester.clone().expect("AI & MCP");
        seen(rig, "AI & MCP..".into());
        let (ox, oy) = req.origin();
        let close = McpResponse::Pick(McpControl::Close);
        let (_, last) = pixels_where(|x, y| {
            let mut r = req.clone();
            r.handle(ReqEvent::Press(x, y)) == close || r.handle(ReqEvent::Release) == close
        });
        rig.click_at(ui_pixel(rig, (ox + last.0, oy + last.1)), FAR, Primary);
        assert!(rig.app.ui.mcp_requester.is_none(), "Close closes it");
        seen(rig, "Close".into());
    }

    type Script = fn(&mut Rig, &mut dyn FnMut(&mut Rig, String));

    /// One requester of each kind, with the layer column shown: each opens in the Modern
    /// window, takes clicks on the first and last pixels of its buttons through the
    /// unchanged mapping, closes, and leaves the same state as in the Classic window.
    const MODAL_SCRIPTS: [(&str, Script); 8] = [
        ("tool settings", tool_requester_script),
        ("file", file_requester_script),
        ("Screen Format", screen_format_script),
        ("Show Page", show_page_script),
        ("Color Mixer", color_mixer_script),
        ("Import", import_script),
        ("Generate", generate_script),
        ("AI & MCP", mcp_script),
    ];

    fn modal_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        rig.click(Place::Page(40, 40), Primary);
        rig.key('L');
        seen(rig, "a dot and the layer panel".into());
        for (name, script) in MODAL_SCRIPTS {
            script(rig, &mut |rig: &mut Rig, step: String| {
                if !rig.app.modal_open() {
                    // The requester's button is not at the same place of the two windows:
                    // the pointer rests on the same screen pixel before they are compared.
                    rig.move_to(Place::Screen(30, 200));
                }
                seen(rig, format!("{name}: {step}"));
            });
            assert!(!rig.app.modal_open(), "{name} is closed");
        }
    }

    #[test]
    fn one_requester_of_each_kind_works_in_the_quad_as_in_classic() {
        in_both_skins(Rig::modern(), modal_script);
    }

    #[test]
    fn requesters_work_in_the_quad_at_other_device_scales() {
        in_both_skins(
            Rig::new(UiStyle::Modern, (1280.0, 800.0), 2.0),
            modal_script,
        );
        in_both_skins(
            Rig::new(UiStyle::Modern, (1001.0, 700.0), 1.5),
            modal_script,
        );
    }

    /// Under a requester the chrome is veiled and takes no input: no tool, no colour, no
    /// menu, no layer, nothing raised. Esc closes the requester and the chrome is itself
    /// again.
    #[test]
    fn the_chrome_is_veiled_and_takes_no_input_under_a_requester() {
        type Open = fn(&mut Rig);
        let open: [(&str, Open); 3] = [
            ("tool settings", |rig| {
                rig.click(Place::Tool(Tool::Grid, false), Secondary)
            }),
            ("Color Mixer", |rig| rig.key('p')),
            ("file", |rig| rig.pick(Action::Command("picture.load"))),
        ];
        for (kind, ppp) in [(ThemeKind::Dark, 1.0), (ThemeKind::Light, 2.0)] {
            let theme = Theme::of(kind);
            let mut rig = Rig::new(UiStyle::Modern, (1280.0, 800.0), ppp);
            rig.key('L');
            rig.app.run(Command::AddLayer { name: None });
            rig.pass(vec![]);
            let l = rig.app.modern_layout().expect("layout");
            let (bar, dock) = (l.bar.expect("bar"), l.dock.as_ref().expect("dock").rect);
            let column = l.layers.as_ref().expect("the layer column").clone();
            let spots = [
                (bar.right() - 3.0, bar.y + 3.0),
                (dock.x + 3.0, dock.bottom() - 3.0),
                (column.header.right() - 40.0, column.header.y + 5.0),
            ];
            let lit = shot(&mut rig, kind, "live-lit");
            for (x, y) in spots {
                assert_eq!(probe(&lit, &l, x, y), rgb(theme.panel));
            }
            for (name, open) in open {
                let before = rig.app.modern.renders();
                open(&mut rig);
                assert!(rig.app.modal_open(), "{name} is open");
                let after = rig.app.modern.renders();
                for i in [VIEWPORT, DOCK, BAR] {
                    assert!(after[i] > before[i], "{name}: region {i} is veiled");
                }
                let file = format!("live-requester-{name}-{kind:?}").replace(' ', "-");
                let image = shot(&mut rig, kind, &file.to_lowercase());
                for (x, y) in spots {
                    let c = probe(&image, &l, x, y);
                    assert_ne!(c, rgb(theme.panel), "{name}: veiled at ({x}, {y})");
                    assert!(c.iter().zip(rgb(theme.panel)).all(|(a, b)| *a <= b));
                }
                // The requester is in the picture, which is not veiled: its middle differs
                // from the page, and the page's corner is the page.
                let corner = modern_point(&rig.app, Place::Page(1, 254));
                assert_eq!(
                    probe(&image, &l, corner.x, corner.y),
                    probe(&lit, &l, corner.x, corner.y),
                    "{name}"
                );
                // Clicks on the chrome do nothing.
                let state = (
                    rig.app.ui.tool,
                    rig.app.ui.fg,
                    rig.app.ui.bg,
                    rig.app.ui.brush,
                    rig.app.engine.document().active_layer(),
                    rig.app.engine.document().layer_count(),
                    rig.app.engine.document().revision(),
                );
                let title = l.menu_titles[2];
                let places = [
                    modern_point(&rig.app, Place::Tool(Tool::Line, false)),
                    modern_point(&rig.app, Place::Brush(4)),
                    modern_point(&rig.app, Place::Cell(12)),
                    modern_point(&rig.app, layer(LayerHit::Name(0))),
                    modern_point(&rig.app, layer(LayerHit::Eye(1))),
                    modern_point(&rig.app, layer(LayerHit::Button(LayerButton::New))),
                    egui::pos2(title.x + title.w / 2.0, title.y + title.h / 2.0),
                ];
                for pos in places {
                    for button in [Primary, Secondary] {
                        rig.pass(vec![
                            egui::Event::PointerMoved(pos),
                            pointer_button(pos, button, true),
                        ]);
                        assert!(!rig.app.ui.menu.active, "{name}: no menu");
                        assert_eq!(rig.app.modern.hover, None, "{name}: nothing is raised");
                        rig.pass(vec![pointer_button(pos, button, false)]);
                    }
                }
                rig.t += crate::tooltip::DELAY + 0.1;
                rig.pass(vec![]);
                assert_eq!(rig.app.ui.tooltip, None, "{name}: no tooltip");
                assert!(rig.app.modal_open(), "{name} is still open");
                assert_eq!(
                    state,
                    (
                        rig.app.ui.tool,
                        rig.app.ui.fg,
                        rig.app.ui.bg,
                        rig.app.ui.brush,
                        rig.app.engine.document().active_layer(),
                        rig.app.engine.document().layer_count(),
                        rig.app.engine.document().revision(),
                    ),
                    "{name}"
                );
                rig.tap(egui::Key::Escape);
                assert!(!rig.app.modal_open(), "Esc closes {name}");
                rig.pass(vec![]);
                let again = shot(&mut rig, kind, "live-lit-again");
                for (x, y) in spots {
                    assert_eq!(probe(&again, &l, x, y), rgb(theme.panel), "{name}");
                }
            }
        }
    }

    /// Screen Format has the whole Modern window for its own screen (no chrome, as in
    /// Classic); Show Page is the picture's screen under veiled chrome. Playback is the
    /// picture running: the chrome stays as it is, and still takes no input.
    #[test]
    fn full_screen_views_in_the_modern_window() {
        let theme = Theme::dark();
        let mut rig = Rig::new(UiStyle::Modern, (720.0, 520.0), 1.0);
        rig.key('L');
        rig.click(Place::Cell(5), Primary);
        rig.key('R');
        rig.drag(&[Place::Page(40, 40), Place::Page(200, 160)], Primary);
        rig.pass(vec![]);
        let lit = shot(&mut rig, ThemeKind::Dark, "live-compact-layers");
        let main = rig.app.modern_layout().expect("layout");
        let bar = main.bar.expect("bar");
        let spot = (bar.right() - 3.0, bar.y + 3.0);
        assert_eq!(probe(&lit, &main, spot.0, spot.1), rgb(theme.panel));

        // Screen Format in the smallest window: all of its 640 × 512 screen shows.
        rig.pick(Action::Command("picture.screen_format"));
        rig.pass(vec![]);
        assert!(rig.app.requester.is_some());
        let l = rig.app.modern_layout().expect("layout");
        assert!(l.bar.is_none() && l.dock.is_none() && l.layers.is_none());
        assert_eq!((l.picture.w, l.picture.h), (640.0, 512.0));
        assert!(l.window.encloses(&l.picture));
        let image = shot(&mut rig, ThemeKind::Dark, "live-screen-format");
        // Its screen's first and last pixel are in the image, where the layout says.
        let (w, h, rgba) = rig.app.modern_window_image(ThemeKind::Dark);
        assert_eq!((w, h), (720, 520));
        let screen = rig.app.requester.as_ref().expect("requester").render();
        let palette = crate::requester::palette();
        for (x, y) in [(0, 0), (639, 255), (320, 128), (17, 200)] {
            let c = palette
                .get(screen.get(x, y).expect("a pixel"))
                .expect("a colour");
            let p = modern_point(&rig.app, Place::Screen(x, y));
            assert_eq!(
                pixel(&rgba, w, p.x as u32, p.y as u32),
                [c.r, c.g, c.b],
                "requester pixel ({x}, {y})"
            );
        }
        // Around it: the canvas, veiled as under every requester.
        let around = probe(&image, &l, 3.0, 3.0);
        assert_ne!(around, rgb(theme.canvas));
        assert!(around.iter().zip(rgb(theme.canvas)).all(|(a, b)| *a <= b));
        rig.tap(egui::Key::Escape);
        assert!(rig.app.requester.is_none());
        rig.pass(vec![]);
        assert_eq!(rig.app.modern_layout().expect("layout"), main);

        // Show Page: the chrome stays where it is, veiled; the screen is as large as the
        // window lets it be (beside the column it would be 1x), over the chrome. A key
        // ends it.
        rig.key('S');
        assert!(rig.app.show_page.is_some(), "S: Show Page");
        let image = shot(&mut rig, ThemeKind::Dark, "live-show-page");
        let shown = rig.app.modern_layout().expect("layout");
        assert_eq!(
            (&shown.bar, &shown.dock, &shown.layers, shown.viewport),
            (&main.bar, &main.dock, &main.layers, main.viewport)
        );
        assert_eq!((main.zoom, main.over_chrome), (1, false));
        assert_eq!((shown.zoom, shown.over_chrome), (2, true));
        assert_eq!(shown.picture, Rect::new(40.0, 4.0, 640.0, 512.0));
        // The screen's corners are in the image, over the veiled dock and bar.
        let (w, _, rgba) = rig.app.modern_window_image(ThemeKind::Dark);
        let screen = rig
            .app
            .show_page
            .as_ref()
            .expect("Show Page")
            .render(&rig.app.page(), rig.app.palette().len());
        for (x, y) in [(0, 0), (319, 0), (0, 255), (319, 255)] {
            let c = rig
                .app
                .palette()
                .get(screen.get(x, y).expect("a pixel"))
                .expect("a colour");
            let (px, py) = ((40 + 2 * x + 1) as u32, (4 + 2 * y + 1) as u32);
            assert_eq!(pixel(&rgba, w, px, py), [c.r, c.g, c.b], "({x}, {y})");
        }
        assert_ne!(probe(&image, &main, spot.0, spot.1), rgb(theme.panel));
        rig.tap(egui::Key::Escape);
        assert!(rig.app.show_page.is_none());

        // Playback: not veiled, and a click on the dock ends it without selecting a tool.
        rig.app.run(Command::SetFrameCount { count: 4 });
        rig.pass(vec![]);
        rig.key('4');
        assert!(rig.app.anim.playing.is_some(), "4 plays");
        let image = shot(&mut rig, ThemeKind::Dark, "live-playback");
        assert_eq!(probe(&image, &main, spot.0, spot.1), rgb(theme.panel));
        let tool = rig.app.ui.tool;
        rig.click(Place::Tool(Tool::Line, false), Primary);
        assert_eq!(rig.app.ui.tool, tool);
        rig.tap(egui::Key::Escape);
        assert!(rig.app.anim.playing.is_none());
    }

    /// How many page pixels are painted, and the corners of the box around them.
    type Painted = (usize, Option<((i32, i32), (i32, i32))>);

    /// The page pixels that are not the background colour: how many, and their bounding
    /// box.
    fn painted(rig: &Rig) -> Painted {
        let page = rig.app.page();
        let mut count = 0;
        let mut bounds: Option<((i32, i32), (i32, i32))> = None;
        for y in 0..page.height() as i32 {
            for x in 0..page.width() as i32 {
                if page.get(x, y) != Some(0) {
                    count += 1;
                    let ((x0, y0), (x1, y1)) = bounds.unwrap_or(((x, y), (x, y)));
                    bounds = Some(((x0.min(x), y0.min(y)), (x1.max(x), y1.max(y))));
                }
            }
        }
        (count, bounds)
    }

    /// Every display mode in small and large Modern windows, with and without the layer
    /// column: the screen is whole inside the viewport (smaller than one physical pixel per
    /// pixel where it must be, as in Classic), and its first and last pixel are under the
    /// pointer and can be painted.
    #[test]
    fn every_display_mode_is_shown_whole_and_can_be_painted_to_its_corners() {
        for mode in dpx_core::DisplayMode::all() {
            for (size, ppp) in [
                ((720.0, 520.0), 1.0),
                ((1024.0, 768.0), 1.0),
                ((1024.0, 768.0), 2.0),
            ] {
                for layers in [false, true] {
                    let what = format!("{mode:?} in {size:?} at {ppp}x, layers {layers}");
                    let mut rig = Rig::new(UiStyle::Modern, size, ppp);
                    let (w, h) = mode.size();
                    rig.app.run(Command::SetFormat {
                        mode,
                        colors: 16,
                        width: w,
                        height: h,
                        retain: false,
                        screen: None,
                        chipset: None,
                    });
                    if layers {
                        rig.key('L');
                    }
                    rig.pass(vec![]);
                    let l = rig.app.modern_layout().expect("layout");
                    assert_eq!(l.layers.is_some(), layers, "{what}");
                    assert!(l.viewport.encloses(&l.picture), "{what}: {:?}", l.picture);
                    assert!(!l.over_chrome, "{what}");
                    let (w, h) = (w as i32, h as i32);
                    assert_eq!(painted(&rig).0, 0, "{what}");
                    for (x, y) in [(0, 0), (w - 1, h - 1), (w - 1, 0), (0, h - 1)] {
                        rig.move_to(Place::Page(x, y));
                        assert_eq!(rig.app.ui.coords, Some((x, y)), "{what}");
                        rig.click(Place::Page(x, y), Primary);
                        assert_ne!(rig.app.page().get(x, y), Some(0), "{what}: ({x}, {y})");
                    }
                    // Just outside the quad there is no page.
                    let p = l.picture;
                    for pos in [
                        egui::pos2(p.x - 0.75, p.y + 5.0),
                        egui::pos2(p.right() + 0.75, p.y + 5.0),
                    ] {
                        if l.viewport.contains(pos.x, pos.y) {
                            rig.pass(vec![egui::Event::PointerMoved(pos)]);
                            assert_eq!(rig.app.ui.coords, None, "{what}: {pos:?}");
                        }
                    }
                }
            }
        }
    }

    /// A requester on a screen that the viewport would cut or show small lies in the middle
    /// of the window, over the veiled chrome, and is clicked where it is drawn.
    #[test]
    fn a_requester_is_whole_and_readable_in_a_small_window() {
        // Picture ▸ Load.. on PAL HiRes interlace, the smallest window, the layer column
        // shown: the requester's screen is whole in the window, and Cancel answers on its
        // last pixel (which lay under the layer column before).
        let mut rig = Rig::new(UiStyle::Modern, (720.0, 520.0), 1.0);
        let mode = dpx_core::DisplayMode::new(dpx_core::Video::Pal, dpx_core::Resolution::Hi, true);
        rig.app.run(Command::SetFormat {
            mode,
            colors: 16,
            width: 640,
            height: 512,
            retain: false,
            screen: None,
            chipset: None,
        });
        rig.key('L');
        rig.pass(vec![]);
        let plain = rig.app.modern_layout().expect("layout");
        assert!(plain.layers.is_some() && plain.shown < 1.0);
        file_requester_script(&mut rig, &mut |rig, step| {
            if step == "Load.." {
                rig.pass(vec![]);
                let l = rig.app.modern_layout().expect("layout");
                assert!(
                    l.over_chrome && l.window.encloses(&l.picture),
                    "{:?}",
                    l.picture
                );
                assert_eq!((l.shown, l.picture.w, l.picture.h), (1.0, 640.0, 512.0));
                // The chrome did not move.
                assert_eq!((&l.dock, &l.layers), (&plain.dock, &plain.layers));
            }
        });
        rig.pass(vec![]);
        assert_eq!(rig.app.modern_layout().expect("layout"), plain);

        // The Color Mixer on a LoRes picture in an 800 x 600 window with the column: two
        // physical pixels per pixel instead of one, and its cells and sliders answer.
        for (size, ppp) in [
            ((800.0, 600.0), 1.0),
            ((720.0, 520.0), 1.0),
            ((720.0, 520.0), 1.5),
        ] {
            let mut rig = Rig::new(UiStyle::Modern, size, ppp);
            rig.key('L');
            rig.pass(vec![]);
            let plain = rig.app.modern_layout().expect("layout");
            color_mixer_script(&mut rig, &mut |rig, step| {
                rig.pass(vec![]);
                let l = rig.app.modern_layout().expect("layout");
                if rig.app.modal_open() {
                    assert!(l.over_chrome, "{size:?} {ppp}: {step}");
                    assert!(l.zoom > plain.zoom, "{size:?} {ppp}: {step}");
                    assert!(l.window.encloses(&l.picture), "{size:?} {ppp}: {step}");
                } else {
                    assert_eq!(l, plain, "{size:?} {ppp}: {step}");
                }
            });
        }
        // In a roomy window the requester is where the picture is: nothing moves.
        let mut rig = Rig::modern();
        rig.key('L');
        rig.pass(vec![]);
        let plain = rig.app.modern_layout().expect("layout");
        rig.key('p');
        rig.pass(vec![]);
        assert!(rig.app.modal_open());
        assert_eq!(rig.app.modern_layout().expect("layout"), plain);
    }

    /// A freehand stroke from (50, 50) to (60, 60) with `during` run while the button is
    /// still held and the pointer rests; returns what is painted after the release.
    fn stroke_with(rig: &mut Rig, during: &dyn Fn(&mut Rig)) -> Painted {
        let from = rig.point(Place::Page(50, 50));
        rig.pass(vec![
            egui::Event::PointerMoved(from),
            pointer_button(from, Primary, true),
        ]);
        let mut to = from;
        for step in 1..=10 {
            to = rig.point(Place::Page(50 + step, 50 + step));
            rig.pass(vec![egui::Event::PointerMoved(to)]);
        }
        during(rig);
        rig.pass(vec![]);
        rig.pass(vec![pointer_button(to, Primary, false)]);
        rig.pass(vec![]);
        painted(rig)
    }

    const STROKE: Painted = (11, Some(((50, 50), (60, 60))));

    /// F12 or the chord while a mouse button is held: the stroke ends as it would have
    /// without the key, and the other skin is shown once the button is up.
    #[test]
    fn the_skin_switch_waits_for_a_held_mouse_button() {
        for style in UiStyle::ALL {
            for chord_key in [false, true] {
                let what = format!("{style:?}, chord {chord_key}");
                let mut plain = Rig::new(style, (1280.0, 1024.0), 1.0);
                plain.key('d');
                assert_eq!(stroke_with(&mut plain, &|_| {}), STROKE, "{what}");
                let mut rig = Rig::new(style, (1280.0, 1024.0), 1.0);
                rig.key('d');
                let got = stroke_with(&mut rig, &|rig| {
                    if chord_key {
                        let held = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
                        let mut events = vec![egui::Event::ModifiersChanged(held)];
                        events.extend(chord(egui::Key::U, egui::Key::U, held, ""));
                        rig.pass(events);
                        rig.pass(vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
                    } else {
                        rig.tap(egui::Key::F12);
                    }
                    // Still the skin the button went down in.
                    assert_eq!(rig.app.ui.ui_style, style, "{what}");
                });
                assert_eq!(got, STROKE, "{what}");
                assert_eq!(rig.app.ui.ui_style, style.toggled(), "{what}");
                assert_eq!(rig.app.page(), plain.app.page(), "{what}");
                // The key twice while held: the same skin afterwards.
                let mut rig = Rig::new(style, (1280.0, 1024.0), 1.0);
                rig.key('d');
                let got = stroke_with(&mut rig, &|rig| {
                    rig.tap(egui::Key::F12);
                    rig.tap(egui::Key::F12);
                });
                assert_eq!((got, rig.app.ui.ui_style), (STROKE, style), "{what}");
            }
        }
    }

    /// F9, F10, `L` and the zoom keys during a held stroke: the flags change at once as in
    /// Classic, but the Modern picture keeps its place until the button is up, so the
    /// stroke has no stray segment and both skins leave the same picture and state.
    fn held_keys_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        rig.key('d');
        seen(rig, "d".into());
        type During = (&'static str, fn(&mut Rig));
        let keys: [During; 5] = [
            ("F10", |rig| rig.tap(egui::Key::F10)),
            ("F10 back", |rig| rig.tap(egui::Key::F10)),
            ("F9", |rig| rig.tap(egui::Key::F9)),
            ("F9 back", |rig| rig.tap(egui::Key::F9)),
            ("L", |rig| rig.key('L')),
        ];
        for (name, key) in keys {
            let flags = (rig.app.ui.title_visible, rig.app.ui.toolbox_visible);
            let panel = rig.app.ui.layer_panel.is_some();
            let quad = rig.app.modern_layout().map(|l| l.picture);
            let got = stroke_with(rig, &|rig| {
                key(rig);
                // The shared state changed at once ..
                assert_ne!(
                    (
                        (rig.app.ui.title_visible, rig.app.ui.toolbox_visible),
                        rig.app.ui.layer_panel.is_some()
                    ),
                    (flags, panel),
                    "{name}"
                );
                // .. the Modern picture did not move.
                assert_eq!(rig.app.modern_layout().map(|l| l.picture), quad, "{name}");
            });
            assert_eq!(got, STROKE, "{name}");
            if rig.app.is_modern() {
                // After the release it lies where the new chrome puts it.
                assert_ne!(rig.app.modern_layout().map(|l| l.picture), quad, "{name}");
            }
            // The Modern picture moved under the resting pointer; both skins compare with
            // the pointer over the same page pixel.
            rig.move_to(Place::Page(100, 100));
            seen(rig, format!("a stroke with {name} while held"));
            rig.key('u');
            assert_eq!(painted(rig).0, 0, "{name}: undone");
            seen(rig, format!("undone after {name}"));
        }
    }

    #[test]
    fn chrome_keys_during_a_held_stroke_do_the_same_in_both_skins() {
        in_both_skins(
            Rig::new(UiStyle::Modern, (1280.0, 1024.0), 1.0),
            held_keys_script,
        );
        in_both_skins(
            Rig::new(UiStyle::Modern, (1001.0, 700.0), 1.5),
            held_keys_script,
        );
    }

    /// A zoom step during a held stroke waits for the release in the Modern skin too.
    #[test]
    fn a_zoom_step_during_a_held_stroke_keeps_the_picture_in_place() {
        let mut rig = Rig::new(UiStyle::Modern, (1280.0, 1024.0), 1.0);
        rig.key('d');
        let before = rig.app.modern_layout().expect("layout");
        let got = stroke_with(&mut rig, &|rig| {
            rig.command(egui::Key::Minus);
            assert_eq!(
                rig.app.modern_layout().expect("layout").picture,
                before.picture
            );
        });
        assert_eq!(got, STROKE);
        let after = rig.app.modern_layout().expect("layout");
        assert_eq!(after.zoom, before.zoom - 1);
        // A press on the menu bar is not a gesture on the picture: Prefs ▸ Layers picked
        // with the left button while the menus are held shows the column at once.
        let mut rig = Rig::modern();
        rig.hold_menu(Action::Pref(dpx_ui::Pref::LayerPanel));
        let pos = *rig
            .menu_points(Action::Pref(dpx_ui::Pref::LayerPanel))
            .last()
            .expect("the item");
        rig.pass(vec![pointer_button(pos, Primary, true)]);
        rig.pass(vec![]);
        assert!(rig.app.modern_layout().expect("layout").layers.is_some());
    }

    /// The Modern window is sized freely: a start in the Modern skin opens the size it had,
    /// not screen x Pixel Size (the picture's zoom there, which may be 8x).
    #[test]
    fn a_start_in_the_modern_skin_opens_the_remembered_window() {
        use crate::config::Config;
        let modern = |pixel_size, modern_window| Config {
            ui_style: UiStyle::Modern,
            pixel_size,
            modern_window,
            ..Config::default()
        };
        // Whatever the zoom was: the design size at the first Modern start ..
        for pixel_size in [PixelSize::MIN, 6, 10, PixelSize::MAX] {
            assert_eq!(
                crate::start_window(&modern(pixel_size, None)),
                egui::vec2(1280.0, 800.0)
            );
            // .. and later the window as it was, never below the Modern minimum.
            assert_eq!(
                crate::start_window(&modern(pixel_size, Some((1000, 700)))),
                egui::vec2(1000.0, 700.0)
            );
            assert_eq!(
                crate::start_window(&modern(pixel_size, Some((300, 2000)))),
                egui::vec2(720.0, 2000.0)
            );
        }
        // Classic is as before: the screen at the Pixel Size, whatever Modern remembered.
        let classic = |pixel_size| Config {
            pixel_size,
            modern_window: Some((1000, 700)),
            ..Config::default()
        };
        assert_eq!(crate::start_window(&classic(6)), egui::vec2(960.0, 768.0));
        assert_eq!(crate::start_window(&classic(4)), egui::vec2(640.0, 512.0));
        assert_eq!(
            crate::start_window(&classic(16)),
            egui::vec2(2560.0, 2048.0)
        );

        // The live window records its size while the Modern skin is shown, and keeps it
        // through a visit to Classic.
        let mut rig = Rig::new(UiStyle::Modern, (1001.0, 640.0), 1.0);
        assert_eq!(Config::capture(&rig.app).modern_window, Some((1001, 640)));
        rig.size = egui::vec2(900.0, 600.0);
        rig.pass(vec![]);
        assert_eq!(Config::capture(&rig.app).modern_window, Some((900, 600)));
        rig.tap(egui::Key::F12);
        rig.size = egui::vec2(640.0, 512.0);
        rig.pass(vec![]);
        rig.pass(vec![]);
        assert!(!rig.app.is_modern());
        assert_eq!(Config::capture(&rig.app).modern_window, Some((900, 600)));
        assert_eq!(Config::capture(&Rig::classic().app).modern_window, None);
        // A settings file brings it back into a fresh app.
        let mut fresh = PainterApp::new();
        Config::capture(&rig.app).apply(&mut fresh);
        assert_eq!(Config::capture(&fresh).modern_window, Some((900, 600)));

        // A remembered window larger than the monitor is cut to it, once.
        let avail = egui::vec2(1470.0, 856.0);
        assert_eq!(
            fit_to_monitor(egui::vec2(2560.0, 2048.0), avail),
            Some(egui::vec2(1470.0, 856.0))
        );
        assert_eq!(
            fit_to_monitor(egui::vec2(1280.0, 2048.0), avail),
            Some(egui::vec2(1280.0, 856.0))
        );
        assert_eq!(fit_to_monitor(egui::vec2(1280.0, 800.0), avail), None);
        assert_eq!(fit_to_monitor(egui::vec2(1470.0, 856.0), avail), None);
        // Never below the Modern minimum, even on a tiny monitor.
        assert_eq!(
            fit_to_monitor(egui::vec2(1280.0, 800.0), egui::vec2(640.0, 400.0)),
            Some(egui::vec2(720.0, 520.0))
        );
        assert_eq!(
            fit_to_monitor(egui::vec2(720.0, 520.0), egui::vec2(640.0, 400.0)),
            None
        );
    }

    /// The Animation Control Panel is still drawn in the Classic look inside the picture;
    /// its buttons are where they are drawn and do the same as in Classic.
    fn anim_panel_script(rig: &mut Rig, seen: &mut dyn FnMut(&mut Rig, String)) {
        use dpx_ui::PanelButton as B;
        rig.app.run(Command::SetFrameCount { count: 6 });
        rig.pass(vec![]);
        rig.pass(chord(egui::Key::A, egui::Key::A, egui::Modifiers::ALT, ""));
        assert!(rig.app.ui.anim_panel.is_some(), "Alt-a shows the panel");
        rig.pass(vec![]);
        seen(rig, "Alt-a".into());
        let frame = |rig: &Rig| rig.app.engine.document().current_frame();
        for (button, now) in [
            (B::Last, 5),
            (B::First, 0),
            (B::Next, 1),
            (B::Next, 2),
            (B::Previous, 1),
        ] {
            // The panel spans the screen where the classic toolbox does not cover it; the
            // Modern picture has no toolbox in it.
            let (w, h) = rig.app.view().ui_size();
            let toolbox = rig.app.ui.toolbox_visible && !rig.app.is_modern();
            let r = dpx_ui::anim_panel::Layout::new(w, h, toolbox).button(button);
            rig.t += 1.0;
            rig.click(ui_pixel(rig, (r.x + r.w / 2, r.y + r.h / 2)), Primary);
            assert_eq!(frame(rig), now, "{button:?}");
            // The pointer away from the panel, on the same page pixel in both skins.
            rig.move_to(Place::Page(30, 30));
            seen(rig, format!("{button:?}"));
        }
        // Painting above the panel lands on the frame shown.
        rig.click(Place::Page(50, 50), Primary);
        assert_eq!(rig.app.page().get(50, 50), Some(rig.app.ui.fg));
        seen(rig, "a dot on frame 2".into());
        rig.pass(chord(egui::Key::A, egui::Key::A, egui::Modifiers::ALT, ""));
        assert!(rig.app.ui.anim_panel.is_none());
        seen(rig, "Alt-a again".into());
    }

    #[test]
    fn the_control_panel_works_inside_the_quad_as_in_classic() {
        in_both_skins(Rig::modern(), anim_panel_script);
        in_both_skins(
            Rig::new(UiStyle::Modern, (1001.0, 700.0), 1.5),
            anim_panel_script,
        );
    }
}
