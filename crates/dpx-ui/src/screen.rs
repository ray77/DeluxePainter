//! Renders the DeluxePaint screen (page + title bar + toolbox + palette) into an indexed
//! buffer in Amiga screen pixels. Geometry is measured from LoRes reference screenshots;
//! see `docs/design/AMIGA_GUI_FIDELITY.md`.

use dpx_core::{IndexedImage, Palette};

use crate::file_requester::FileRequester;
use crate::font::Font;
use crate::menu::{self, MenuContext, MenuState, PixelSize, Pref, Prefs};
use crate::note_requester::NoteRequester;
use crate::page_size_requester::PageSizeRequester;
use crate::pens::Pens;
use crate::toolbox::{self, BUILTIN_BRUSHES, ICON, IconState, TOOL_ROWS, Tool};

/// Program name shown in the screen title bar.
pub const APP_TITLE: &str = "DeluxePainter";

/// LoRes geometry (screen pixels).
pub mod geom {
    /// Title bar: row 0 shine, rows 1..=9 face, row 10 dark.
    pub const TITLE_H: i32 = 11;
    pub const TITLE_TEXT_X: i32 = 3;
    pub const TITLE_TEXT_Y: i32 = 1;
    /// Depth gadget: 15×9 at `screen_w - 16`, y 1.
    pub const DEPTH_W: i32 = 15;
    pub const TOOLBOX_W: i32 = 24;
    pub const BRUSH_BLOCK_Y: i32 = 11;
    pub const BRUSH_BLOCK_W: i32 = 23;
    pub const BRUSH_BLOCK_H: i32 = 20;
    pub const TOOLS_Y: i32 = 32;
    pub const BUTTON: i32 = 11;
    pub const PITCH: i32 = 12;
    pub const INDICATOR_DX: i32 = 5;
    pub const INDICATOR_Y: i32 = 142;
    pub const INDICATOR_W: i32 = 11;
    pub const INDICATOR_H: i32 = 5;
    pub const PALETTE_Y: i32 = 152;
}

/// Paint modes shown in the title bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintMode {
    Matte,
    Color,
    Replace,
    Smear,
    Shade,
    Blend,
    Cycle,
    Smooth,
    Mix,
    HBrite,
}

impl PaintMode {
    pub fn title(self) -> &'static str {
        match self {
            PaintMode::Matte => "Matte",
            PaintMode::Color => "Color",
            PaintMode::Replace => "Replc",
            PaintMode::Smear => "Smear",
            PaintMode::Shade => "Shade",
            PaintMode::Blend => "Blend",
            PaintMode::Cycle => "Cycle",
            PaintMode::Smooth => "Smooth",
            PaintMode::Mix => "Mix",
            PaintMode::HBrite => "HBrite",
        }
    }
}

/// Visible interface state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiState {
    pub tool: Tool,
    /// For split shape tools: the filled (lower-right) half is selected.
    pub filled: bool,
    pub brush: usize,
    pub fg: u8,
    pub bg: u8,
    pub mode: PaintMode,
    pub toolbox_visible: bool,
    pub title_visible: bool,
    /// Pointer position on the page, shown at the right of the title bar with Prefs ▸ Coords.
    pub coords: Option<(i32, i32)>,
    pub prefs: Prefs,
    /// Prefs ▸ Pixel Size.
    pub pixel_size: PixelSize,
    pub menu: MenuState,
    /// Picture ▸ Load/Save while it is open (drawn over everything else).
    pub file_requester: Option<FileRequester>,
    /// Picture ▸ Page Size while it is open.
    pub page_size_requester: Option<PageSizeRequester>,
    /// Notes after Load/Save while shown (drawn over everything else).
    pub note_requester: Option<NoteRequester>,
    /// Color ▸ Palette ▸ Mixer (the Color Mixer) while it is open.
    pub palette_requester: Option<crate::palette_requester::PaletteRequester>,
    /// Color ▸ Ranges.. (the Range requester) while it is open.
    pub range_requester: Option<crate::range_requester::RangeRequester>,
    /// A custom brush is current; the app keeps it in sync (ghosts Brush menu items).
    pub has_brush: bool,
    /// A picked-up brush exists (Brush ▸ Restore).
    pub can_restore: bool,
    /// Gridding, Symmetry, Spacing, Fill Type or Make Stencil while open (ADR-20).
    pub tool_requester: Option<crate::tool_requesters::ToolRequester>,
    /// Effect flags shown after the paint mode in the title bar (A/a, P, S/s, T; ADR-31); the
    /// app keeps them in sync.
    pub flags: String,
}

impl UiState {
    /// State the menus need for check marks and ghosted items.
    pub fn menu_context(&self) -> MenuContext {
        MenuContext {
            mode: self.mode,
            prefs: self.prefs,
            pixel_size: self.pixel_size,
            // EHB/HAM display modes arrive with the hardware profiles (roadmap phase 5).
            ehb: false,
            ham: false,
            has_brush: self.has_brush,
            can_restore: self.can_restore,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            tool: Tool::ContinuousFreehand,
            filled: false,
            brush: 0,
            fg: 1,
            bg: 0,
            mode: PaintMode::Color,
            toolbox_visible: true,
            title_visible: true,
            coords: None,
            prefs: Prefs::default(),
            pixel_size: PixelSize::default(),
            menu: MenuState::default(),
            file_requester: None,
            page_size_requester: None,
            note_requester: None,
            palette_requester: None,
            range_requester: None,
            has_brush: false,
            can_restore: false,
            tool_requester: None,
            flags: String::new(),
        }
    }
}

/// Pen of the screen area outside the page, for interface pixel (`u`, `v`): a dark area with
/// sparse face-coloured dots in a staggered 4×4 lattice (1 in 8 pixels), so the page edge is
/// clearly visible while the area stays calm. A DeluxePainter addition (DeluxePaint pages were
/// never smaller than the screen); the owner chose "dark pattern, page edge clear" (ADR-13).
fn off_page_pen(u: i32, v: i32, pens: &Pens) -> u8 {
    let (u, v) = (u.rem_euclid(4), v.rem_euclid(4));
    if (u, v) == (0, 0) || (u, v) == (2, 2) {
        pens.face
    } else {
        pens.dark
    }
}

/// Paints the off-page pattern on every pixel of `img` not covered by `page` (at 0,0): the
/// right and bottom strips only. `img` is expected to be pre-filled with `pens.dark`, so only
/// the dots are drawn.
fn fill_off_page(img: &mut IndexedImage, page: &IndexedImage, ux: i32, uy: i32, pens: &Pens) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (pw, ph) = (page.width() as i32, page.height() as i32);
    for y in 0..h {
        let v = y / uy;
        // Rows below the page are uncovered over the full width, others right of the page.
        let from = if y >= ph { 0 } else { pw.min(w) };
        for x in from..w {
            let pen = off_page_pen(x / ux, v, pens);
            if pen != pens.dark {
                img.set(x, y, pen);
            }
        }
    }
}

/// Palette grid layout under the toolbox: (columns, rows, cell width, cell height).
pub fn palette_grid(colors: usize, screen_h: i32) -> (i32, i32, i32, i32) {
    let shown = colors.clamp(1, 64) as i32;
    let cols = (shown / 8).clamp(1, 4);
    let rows = (shown + cols - 1) / cols;
    let cell_w = geom::TOOLBOX_W / cols;
    let cell_h = ((screen_h - geom::PALETTE_Y) / rows).max(1);
    (cols, rows, cell_w, cell_h)
}

/// What lies under a screen position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    TitleBar,
    Tool { tool: Tool, filled_half: bool },
    Brush(usize),
    Indicator,
    PaletteCell(u8),
    ToolboxBackground,
    Page { x: i32, y: i32 },
}

pub struct Screen {
    pub width: i32,
    pub height: i32,
}

impl Screen {
    pub fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }

    pub fn toolbox_x(&self) -> i32 {
        self.width - geom::TOOLBOX_W
    }

    /// Composes the full screen. `page` is the visible part of the page, at the screen's
    /// top-left; it may be smaller than the screen, and the rest then shows the off-page
    /// pattern ([`off_page_pen`]). The UI is drawn on top.
    pub fn render(&self, page: &IndexedImage, palette: &Palette, ui: &UiState) -> IndexedImage {
        let pens = Pens::from_palette(palette);
        let mut img = IndexedImage::new(self.width as u32, self.height as u32, pens.dark);
        fill_off_page(&mut img, page, 1, 1, &pens);
        img.blit(page, 0, 0);
        self.draw_ui(&mut img, palette, ui, &pens);
        img
    }

    /// Composes a screen whose pixels are finer than LoRes (HiRes, SuperHiRes, laced). `self`
    /// has LoRes geometry; the result is `ux`×`uy` times larger. The page keeps its own
    /// pixels, while every UI pixel covers `ux`×`uy` screen pixels, so the interface looks
    /// the same in every mode until HiRes artwork exists (AMIGA_GUI_FIDELITY.md). Screen
    /// pixels not covered by `page` show the off-page pattern, `ux`×`uy` pixels per cell.
    pub fn render_scaled(
        &self,
        page: &IndexedImage,
        palette: &Palette,
        ui: &UiState,
        ux: i32,
        uy: i32,
    ) -> IndexedImage {
        if (ux, uy) == (1, 1) {
            return self.render(page, palette, ui);
        }
        let pens = Pens::from_palette(palette);
        let (sw, sh) = (self.width * ux, self.height * uy);
        let mut out = IndexedImage::new(sw as u32, sh as u32, pens.dark);
        fill_off_page(&mut out, page, ux, uy, &pens);
        out.blit(page, 0, 0);
        // Draw the UI over two different backgrounds: where both agree, the UI drew a pixel.
        let layer = |fill: u8| {
            let mut img = IndexedImage::new(self.width as u32, self.height as u32, fill);
            self.draw_ui(&mut img, palette, ui, &pens);
            img
        };
        let (a, b) = (layer(0), layer(1));
        for y in 0..self.height {
            for x in 0..self.width {
                let p = a.get(x, y);
                if p == b.get(x, y)
                    && let Some(p) = p
                {
                    out.fill_rect(x * ux, y * uy, ux, uy, p);
                }
            }
        }
        out
    }

    fn draw_ui(&self, img: &mut IndexedImage, palette: &Palette, ui: &UiState, pens: &Pens) {
        if ui.toolbox_visible {
            self.draw_toolbox(img, palette, ui, pens);
        }
        if ui.title_visible {
            if ui.menu.active {
                let ctx = ui.menu_context();
                menu::draw(img, pens, &ctx, &ui.menu, self.width, self.height);
            } else {
                self.draw_title_bar(img, ui, pens);
            }
        }
        if let Some(req) = &ui.file_requester {
            req.draw(img, pens);
        }
        if let Some(req) = &ui.page_size_requester {
            req.draw(img, pens);
        }
        if let Some(req) = &ui.note_requester {
            req.draw(img, pens);
        }
        if let Some(req) = &ui.palette_requester {
            req.draw(img, pens);
        }
        if let Some(req) = &ui.range_requester {
            req.draw(img, pens);
        }
        if let Some(req) = &ui.tool_requester {
            req.draw(img, pens, palette);
        }
    }

    fn bevel_box(img: &mut IndexedImage, x: i32, y: i32, w: i32, h: i32, pens: &Pens) {
        img.fill_rect(x, y, w, h, pens.face);
        img.hline(x, x + w - 1, y, pens.shine);
        img.vline(x, y, y + h - 1, pens.shine);
    }

    fn draw_title_bar(&self, img: &mut IndexedImage, ui: &UiState, pens: &Pens) {
        let w = self.width;
        img.fill_rect(0, 0, w, geom::TITLE_H, pens.face);
        img.hline(0, w - 1, 0, pens.shine);
        img.hline(0, w - 1, geom::TITLE_H - 1, pens.dark);
        let font = Font::builtin();
        let title = if ui.flags.is_empty() {
            format!("{APP_TITLE}  {}", ui.mode.title())
        } else {
            format!("{APP_TITLE}  {}  {}", ui.mode.title(), ui.flags)
        };
        font.draw(
            img,
            geom::TITLE_TEXT_X,
            geom::TITLE_TEXT_Y,
            &title,
            pens.dark,
        );
        if let Some((x, y)) = ui.coords.filter(|_| ui.prefs.get(Pref::Coords)) {
            let text = format!("{x:4} {y:4}");
            let tx = w - geom::DEPTH_W - 3 - font.text_width(&text);
            font.draw(img, tx, geom::TITLE_TEXT_Y, &text, pens.dark);
        }
        self.draw_depth_gadget(img, pens);
    }

    /// Two overlapping rectangles (screen depth gadget), 15×9.
    fn draw_depth_gadget(&self, img: &mut IndexedImage, pens: &Pens) {
        const ROWS: [&str; 9] = [
            "###############",
            "##........#####",
            "##.######.#####",
            "##.##........##",
            "##.##........##",
            "##...........##",
            "#####........##",
            "#####........##",
            "###############",
        ];
        let x0 = self.width - geom::DEPTH_W - 1;
        for (dy, row) in ROWS.iter().enumerate() {
            for (dx, c) in row.chars().enumerate() {
                let pen = if c == '#' { pens.dark } else { pens.face };
                img.set(x0 + dx as i32, 1 + dy as i32, pen);
            }
        }
    }

    fn draw_toolbox(&self, img: &mut IndexedImage, palette: &Palette, ui: &UiState, pens: &Pens) {
        let x0 = self.toolbox_x();
        // Panel background (gaps between buttons).
        img.fill_rect(
            x0,
            geom::TITLE_H,
            geom::TOOLBOX_W,
            geom::PALETTE_Y - geom::TITLE_H,
            pens.dark,
        );

        // Built-in brushes.
        Self::bevel_box(
            img,
            x0,
            geom::BRUSH_BLOCK_Y,
            geom::BRUSH_BLOCK_W,
            geom::BRUSH_BLOCK_H,
            pens,
        );
        for (i, b) in BUILTIN_BRUSHES.iter().enumerate() {
            let selected = i == ui.brush;
            let (fill, ink) = if selected {
                (pens.dark, pens.face)
            } else {
                (pens.face, pens.dark)
            };
            if selected {
                let (bx, by, bw, bh) = b.select_box;
                img.fill_rect(x0 + bx, by, bw, bh, fill);
            }
            for &(px, py) in b.pixels {
                img.set(x0 + px, py, ink);
            }
        }

        // Tool buttons.
        for (row, pair) in TOOL_ROWS.iter().enumerate() {
            for (col, &tool) in pair.iter().enumerate() {
                let bx = x0 + col as i32 * geom::PITCH;
                let by = geom::TOOLS_Y + row as i32 * geom::PITCH;
                Self::bevel_box(img, bx, by, geom::BUTTON, geom::BUTTON, pens);
                // Selected: only the chosen variant, inverted (face on dark).
                let selected = tool == ui.tool;
                let state = if selected {
                    IconState::Selected {
                        filled: tool.is_split() && ui.filled,
                    }
                } else {
                    IconState::Normal
                };
                let (ink, paper) = if selected {
                    (pens.face, pens.dark)
                } else {
                    (pens.dark, pens.face)
                };
                let icon = toolbox::icon(tool, state);
                for iy in 0..ICON {
                    for ix in 0..ICON {
                        let pen = if icon.get(ix, iy) { ink } else { paper };
                        img.set(bx + 1 + ix as i32, by + 1 + iy as i32, pen);
                    }
                }
            }
        }

        // Colour indicator: FG inside a bevelled frame.
        let ix = x0 + geom::INDICATOR_DX;
        img.hline(
            ix,
            ix + geom::INDICATOR_W - 1,
            geom::INDICATOR_Y,
            pens.shine,
        );
        img.vline(
            ix,
            geom::INDICATOR_Y,
            geom::INDICATOR_Y + geom::INDICATOR_H - 1,
            pens.shine,
        );
        img.fill_rect(
            ix + 1,
            geom::INDICATOR_Y + 1,
            geom::INDICATOR_W - 1,
            geom::INDICATOR_H - 1,
            ui.fg,
        );

        // Palette.
        let (cols, rows, cw, ch) = palette_grid(palette.len(), self.height);
        for c in 0..cols {
            for r in 0..rows {
                let idx = c * rows + r;
                if idx as usize >= palette.len() {
                    continue;
                }
                img.fill_rect(x0 + c * cw, geom::PALETTE_Y + r * ch, cw, ch, idx as u8);
            }
        }
        // FG cell: 1-px face frame, offset one pixel up/left (as in the reference).
        let fg = i32::from(ui.fg);
        if (fg as usize) < palette.len() {
            let (fx, fy) = (x0 + (fg / rows) * cw, geom::PALETTE_Y + (fg % rows) * ch);
            img.hline(fx - 1, fx + cw - 1, fy - 1, pens.face);
            img.hline(fx - 1, fx + cw - 1, fy + ch - 1, pens.face);
            img.vline(fx - 1, fy - 1, fy + ch - 1, pens.face);
            img.vline(fx + cw - 1, fy - 1, fy + ch - 1, pens.face);
        }
    }

    pub fn hit_test(&self, palette_len: usize, ui: &UiState, x: i32, y: i32) -> Hit {
        if ui.title_visible && y < geom::TITLE_H {
            return Hit::TitleBar;
        }
        let x0 = self.toolbox_x();
        if ui.toolbox_visible && x >= x0 && y >= geom::TITLE_H {
            let lx = x - x0;
            if y >= geom::PALETTE_Y {
                let (cols, rows, cw, ch) = palette_grid(palette_len, self.height);
                let (c, r) = (
                    (lx / cw).min(cols - 1),
                    ((y - geom::PALETTE_Y) / ch).min(rows - 1),
                );
                let idx = c * rows + r;
                return if (idx as usize) < palette_len {
                    Hit::PaletteCell(idx as u8)
                } else {
                    Hit::ToolboxBackground
                };
            }
            if (geom::INDICATOR_Y..geom::INDICATOR_Y + geom::INDICATOR_H).contains(&y) {
                return Hit::Indicator;
            }
            if (geom::BRUSH_BLOCK_Y..geom::BRUSH_BLOCK_Y + geom::BRUSH_BLOCK_H).contains(&y) {
                return BUILTIN_BRUSHES
                    .iter()
                    .position(|b| {
                        let (bx, by, bw, bh) = b.select_box;
                        (bx..bx + bw).contains(&lx) && (by..by + bh).contains(&y)
                    })
                    .map_or(Hit::ToolboxBackground, Hit::Brush);
            }
            let ty = y - geom::TOOLS_Y;
            if ty >= 0 {
                let (row, col) = ((ty / geom::PITCH) as usize, (lx / geom::PITCH) as usize);
                let (ix, iy) = (lx % geom::PITCH - 1, ty % geom::PITCH - 1);
                if row < TOOL_ROWS.len()
                    && col < 2
                    && ix < geom::BUTTON - 1
                    && iy < geom::BUTTON - 1
                {
                    let tool = TOOL_ROWS[row][col];
                    let filled_half = tool.is_split()
                        && toolbox::in_filled_half(
                            ix.clamp(0, 8) as usize,
                            iy.clamp(0, 8) as usize,
                        );
                    return Hit::Tool { tool, filled_half };
                }
            }
            return Hit::ToolboxBackground;
        }
        Hit::Page { x, y }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_render_keeps_page_pixels_and_enlarges_the_ui() {
        let pal = Palette::dpaint_default_32();
        let ui = UiState::default();
        let lores = Screen::new(320, 256);
        // HiRes laced: 640×512, UI pixels 2×2.
        let mut page = IndexedImage::new(640, 512, 0);
        page.set(1, 101, 5);
        let hi = lores.render_scaled(&page, &pal, &ui, 2, 2);
        assert_eq!((hi.width(), hi.height()), (640, 512));
        assert_eq!(hi.get(1, 101), Some(5), "page pixel at its own resolution");
        assert_eq!(hi.get(0, 101), Some(0));
        let small = lores.render(&IndexedImage::new(320, 256, 0), &pal, &ui);
        for (x, y) in [(0, 0), (5, 3), (319, 255), (300, 40), (310, 200)] {
            let p = small.get(x, y);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                assert_eq!(hi.get(2 * x + dx, 2 * y + dy), p, "UI pixel ({x},{y})");
            }
        }
        assert_eq!(lores.render_scaled(&page, &pal, &ui, 1, 1).width(), 320);
    }

    #[test]
    fn pixel_size_defaults_to_3x() {
        assert_eq!(UiState::default().pixel_size, PixelSize(6), "3x");
    }

    #[test]
    fn off_page_pattern_right_and_below_page() {
        let pal = Palette::dpaint_default_32();
        let pens = Pens::from_palette(&pal);
        let ui = UiState {
            toolbox_visible: false,
            title_visible: false,
            ..UiState::default()
        };
        let screen = Screen::new(320, 256);
        let page = IndexedImage::new(100, 80, 5);
        let lo = screen.render(&page, &pal, &ui);
        // Inside the page: untouched. Outside: sparse staggered dots, dark elsewhere.
        assert_eq!(lo.get(99, 79), Some(5));
        assert_eq!(lo.get(100, 80), Some(pens.face));
        assert_eq!(lo.get(101, 80), Some(pens.dark));
        assert_eq!(lo.get(100, 81), Some(pens.dark));
        assert_eq!(lo.get(200, 8), Some(pens.face));
        assert_eq!(lo.get(202, 10), Some(pens.face), "staggered");
        assert_eq!(lo.get(201, 10), Some(pens.dark));
        assert_eq!(lo.get(12, 200), Some(pens.face));
        assert_eq!(lo.get(11, 201), Some(pens.dark));
        let hi = screen.render_scaled(&page, &pal, &ui, 2, 1);
        assert_eq!(hi.width(), 640);
        assert_eq!(hi.get(99, 79), Some(5));
        // UI pixel u = x / 2: screen x 200 and 201 form the dot u=100, 202 and 203 are u=101.
        assert_eq!(hi.get(200, 8), Some(pens.face));
        assert_eq!(hi.get(201, 8), Some(pens.face));
        assert_eq!(hi.get(202, 8), Some(pens.dark));
        assert_eq!(hi.get(203, 8), Some(pens.dark));
        assert_eq!(hi.get(200, 9), Some(pens.dark));
        assert_eq!(hi.get(200, 100), Some(pens.face));
    }

    #[test]
    fn full_page_shows_no_pattern() {
        let pal = Palette::dpaint_default_32();
        let pens = Pens::from_palette(&pal);
        let ui = UiState {
            toolbox_visible: false,
            title_visible: false,
            ..UiState::default()
        };
        let page = IndexedImage::new(320, 256, 7);
        let img = Screen::new(320, 256).render(&page, &pal, &ui);
        assert!((0..256).all(|y| (0..320).all(|x| img.get(x, y) == Some(7))));
        assert_ne!(pens.face, 7);
        let hi =
            Screen::new(320, 256).render_scaled(&IndexedImage::new(640, 256, 7), &pal, &ui, 2, 1);
        assert!((0..256).all(|y| (0..640).all(|x| hi.get(x, y) == Some(7))));
    }

    fn render_default() -> (IndexedImage, Pens) {
        let pal = Palette::dpaint_default_32();
        let page = IndexedImage::new(320, 256, 0);
        let img = Screen::new(320, 256).render(&page, &pal, &UiState::default());
        (img, Pens::from_palette(&pal))
    }

    #[test]
    fn title_bar_rows_match_reference() {
        let (img, p) = render_default();
        assert_eq!(img.get(100, 0), Some(p.shine));
        assert_eq!(img.get(100, 9), Some(p.face));
        assert_eq!(img.get(100, 10), Some(p.dark));
    }

    #[test]
    fn title_shows_program_name_and_mode() {
        let (img, p) = render_default();
        let mut expected = IndexedImage::new(320, 256, 0);
        let text = format!("{APP_TITLE}  Color");
        let end = Font::builtin().draw(
            &mut expected,
            geom::TITLE_TEXT_X,
            geom::TITLE_TEXT_Y,
            &text,
            1,
        );
        assert_eq!(APP_TITLE, "DeluxePainter");
        for y in geom::TITLE_TEXT_Y..geom::TITLE_TEXT_Y + 8 {
            for x in 0..end {
                let ink = expected.get(x, y) == Some(1);
                let got = img.get(x, y);
                assert_eq!(got == Some(p.dark), ink, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn coords_need_the_preference() {
        let pal = Palette::dpaint_default_32();
        let page = IndexedImage::new(320, 256, 0);
        let screen = Screen::new(320, 256);
        let mut ui = UiState {
            coords: Some((123, 45)),
            ..UiState::default()
        };
        let off = screen.render(&page, &pal, &ui);
        ui.prefs.set(Pref::Coords, true);
        let on = screen.render(&page, &pal, &ui);
        let differs = (200..300).any(|x| (1..9).any(|y| off.get(x, y) != on.get(x, y)));
        assert!(differs, "coordinates appear only with Prefs > Coords");
    }

    #[test]
    fn toolbox_button_grid_matches_reference() {
        let (img, p) = render_default();
        // Left button of the line row: shine top/left edge at (296, 44), gap column at 307.
        assert_eq!(img.get(296, 44), Some(p.shine));
        assert_eq!(img.get(306, 44), Some(p.shine));
        assert_eq!(img.get(307, 50), Some(p.dark));
        assert_eq!(img.get(308, 44), Some(p.shine));
        assert_eq!(img.get(300, 55), Some(p.dark)); // row gap
    }

    #[test]
    fn palette_is_four_columns_of_eight_for_32_colours() {
        assert_eq!(palette_grid(32, 256), (4, 8, 6, 13));
        assert_eq!(palette_grid(16, 256), (2, 8, 12, 13));
    }

    #[test]
    fn hit_testing() {
        let s = Screen::new(320, 256);
        let ui = UiState::default();
        assert_eq!(s.hit_test(32, &ui, 5, 5), Hit::TitleBar);
        assert_eq!(s.hit_test(32, &ui, 100, 100), Hit::Page { x: 100, y: 100 });
        assert_eq!(
            s.hit_test(32, &ui, 300, 50),
            Hit::Tool {
                tool: Tool::Line,
                filled_half: false
            }
        );
        // Lower-right of the rectangle button selects the filled variant.
        assert_eq!(
            s.hit_test(32, &ui, 296 + 8, 68 + 8),
            Hit::Tool {
                tool: Tool::Rectangle,
                filled_half: true
            }
        );
        assert_eq!(s.hit_test(32, &ui, 297, 153), Hit::PaletteCell(0));
        assert_eq!(s.hit_test(32, &ui, 303, 166), Hit::PaletteCell(9));
    }
}
