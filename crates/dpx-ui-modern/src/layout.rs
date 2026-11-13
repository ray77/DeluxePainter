//! Where everything is in the Modern main window (ADR-47): the bar at the top, the dock at
//! the right in the order of the DeluxePaint toolbox (brushes, tools, colour indicator,
//! palette) and the picture viewport in what is left. All rectangles are in points, and every
//! edge lies on a whole physical pixel at the window's device scale, so flat surfaces have
//! sharp edges and the picture is an exact integer enlargement.

use dpx_ui::PixelSize;
use dpx_ui::screen::palette_grid;
use dpx_ui::toolbox::TOOL_ROWS;

use crate::font;
use crate::paint::{Rect, Weight};

/// Height of the bar.
pub const BAR_H: f32 = 30.0;
/// Width of the dock.
pub const DOCK_W: f32 = 92.0;
/// Free space between the dock's edge and its content.
pub const DOCK_PAD: f32 = 7.0;
/// Space between two tool buttons.
pub const GAP: f32 = 6.0;
/// Space between two sections of the dock.
pub const SECTION_GAP: f32 = 9.0;
/// Width of a tool button; its height is the same unless the window is too low.
pub const BUTTON: f32 = 36.0;
/// Least height of a tool button in a low window.
pub const BUTTON_MIN_H: f32 = 22.0;
/// Height of the brush well.
pub const BRUSH_WELL_H: f32 = 44.0;
/// Height of the colour indicator.
pub const INDICATOR_H: f32 = 30.0;
/// Height of a palette cell when there is room, and the least it shrinks to.
pub const CELL_H: f32 = 15.0;
pub const CELL_MIN_H: f32 = 7.0;
/// The smallest window the layout is made for.
pub const MIN_WINDOW: (f32, f32) = (720.0, 520.0);

/// Free space left and right of a menu title's text.
pub const TITLE_PAD: f32 = 8.0;
/// Left edge of the first menu title.
pub const TITLES_X: f32 = 6.0;

/// The built-in brushes as the well shows them, by index into `BUILTIN_BRUSHES`: the round
/// ones growing to the right above the squares growing to the right, the two scatter brushes
/// in the last column.
pub const BRUSH_GRID: [[usize; 5]; 2] = [[0, 1, 2, 3, 8], [7, 6, 5, 4, 9]];

/// Which parts of the chrome are shown (F9 hides the bar; F10 hides the bar and the dock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub bar: bool,
    pub dock: bool,
}

impl Chrome {
    pub const ALL: Chrome = Chrome {
        bar: true,
        dock: true,
    };
}

/// The dock's rectangles.
#[derive(Clone, Debug, PartialEq)]
pub struct Dock {
    pub rect: Rect,
    /// The card behind the built-in brushes.
    pub brush_well: Rect,
    /// The cell of each built-in brush, by brush index.
    pub brushes: [Rect; 10],
    /// The button of each tool, in the order of `TOOL_ROWS` (row by row).
    pub tools: [Rect; 18],
    /// The foreground/background colour indicator.
    pub indicator: Rect,
    /// The area of the palette cells.
    pub palette: Rect,
    /// The cell of each shown palette colour, by colour index (column-major, as in
    /// DeluxePaint: colour 1 is below colour 0).
    pub cells: Vec<Rect>,
    pub palette_cols: usize,
    pub palette_rows: usize,
}

/// The layout of the main window.
#[derive(Clone, Debug, PartialEq)]
pub struct MainLayout {
    /// The window, from (0, 0).
    pub window: Rect,
    /// Physical pixels per point.
    pub scale: f32,
    pub bar: Option<Rect>,
    /// The hit rectangle of each menu title in the bar (empty without the bar).
    pub menu_titles: Vec<Rect>,
    pub dock: Option<Dock>,
    /// Everything that is not chrome: canvas colour with the picture in its middle.
    pub viewport: Rect,
    /// The picture quad. It may be larger than the viewport when even one physical pixel per
    /// picture pixel does not fit; it is clipped to the viewport then.
    pub picture: Rect,
    /// Physical pixels per picture pixel (a whole number, at least 1).
    pub zoom: u32,
    /// The picture's size in its own pixels (the screen as displayed: pixel shape applied).
    pub picture_size: (u32, u32),
}

impl MainLayout {
    /// Lays out a window of `window` points at `scale` physical pixels per point.
    /// `palette_len` is the number of colours of the picture's palette; `picture_size` the
    /// size of the picture quad's image in pixels; `pixel_size` the wanted enlargement (Prefs
    /// ▸ Pixel Size, in half points per pixel), which is cut down to what fits the viewport.
    pub fn new(
        window: (f32, f32),
        scale: f32,
        chrome: Chrome,
        palette_len: usize,
        picture_size: (u32, u32),
        pixel_size: PixelSize,
    ) -> MainLayout {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        // Edges on whole physical pixels.
        let snap = |v: f32| (v * scale).round() / scale;
        let window = Rect::new(0.0, 0.0, snap(window.0.max(1.0)), snap(window.1.max(1.0)));
        let bar_h = if chrome.bar {
            snap(BAR_H).min(window.h)
        } else {
            0.0
        };
        let dock_w = if chrome.dock {
            snap(DOCK_W).min(window.w)
        } else {
            0.0
        };
        let bar = chrome.bar.then(|| Rect::new(0.0, 0.0, window.w, bar_h));
        let viewport = Rect::new(0.0, bar_h, window.w - dock_w, window.h - bar_h);

        let menu_titles = if chrome.bar {
            let mut x = TITLES_X;
            dpx_ui::menu::MENUS
                .iter()
                .map(|m| {
                    let w =
                        font::measure(m.title, font::SIZE_BAR, Weight::Regular) + 2.0 * TITLE_PAD;
                    let r = Rect::new(
                        snap(x),
                        snap(4.0),
                        snap(x + w) - snap(x),
                        snap(bar_h - 4.0) - snap(4.0),
                    );
                    x += w;
                    r
                })
                .collect()
        } else {
            Vec::new()
        };

        let dock = chrome.dock.then(|| {
            dock(
                Rect::new(window.w - dock_w, bar_h, dock_w, window.h - bar_h),
                palette_len,
                &snap,
            )
        });

        // The picture: a whole number of physical pixels per picture pixel, centred.
        let (pw, ph) = (picture_size.0.max(1), picture_size.1.max(1));
        let (vw_px, vh_px) = (
            (viewport.w * scale).round().max(0.0) as u32,
            (viewport.h * scale).round().max(0.0) as u32,
        );
        let wanted = (f32::from(pixel_size.0) / 2.0 * scale).floor().max(1.0) as u32;
        let fits = (vw_px / pw).min(vh_px / ph).max(1);
        let zoom = wanted.min(fits);
        let (w_px, h_px) = (pw * zoom, ph * zoom);
        let origin = |view_px: u32, size_px: u32| {
            ((i64::from(view_px) - i64::from(size_px)).div_euclid(2)) as f32 / scale
        };
        let picture = Rect::new(
            viewport.x + origin(vw_px, w_px),
            viewport.y + origin(vh_px, h_px),
            w_px as f32 / scale,
            h_px as f32 / scale,
        );

        MainLayout {
            window,
            scale,
            bar,
            menu_titles,
            dock,
            viewport,
            picture,
            zoom,
            picture_size: (pw, ph),
        }
    }

    /// The x where the bar's status text starts: after the menu titles and their separator.
    pub fn status_x(&self) -> f32 {
        self.menu_titles
            .last()
            .map_or(TITLES_X + TITLE_PAD, |r| r.right() + 14.0)
    }

    /// The button of `tool`, when the dock is shown.
    pub fn tool_button(&self, tool: dpx_ui::Tool) -> Option<Rect> {
        let i = TOOL_ROWS.iter().flatten().position(|&t| t == tool)?;
        Some(self.dock.as_ref()?.tools[i])
    }
}

fn dock(rect: Rect, palette_len: usize, snap: &dyn Fn(f32) -> f32) -> Dock {
    let x0 = snap(rect.x + DOCK_PAD);
    let inner_w = snap(rect.right() - DOCK_PAD) - x0;
    let (cols, rows, ..) = palette_grid(palette_len, 256);
    let (cols, rows) = (cols.max(1) as usize, rows.max(1) as usize);
    let shown = palette_len.clamp(1, 64).min(cols * rows);

    // Heights: everything but the tool buttons and the palette is fixed; a low window first
    // shrinks the palette cells, then the buttons.
    let fixed =
        DOCK_PAD + BRUSH_WELL_H + SECTION_GAP + SECTION_GAP + INDICATOR_H + SECTION_GAP + DOCK_PAD;
    let gaps = 8.0 * GAP;
    let room = (rect.h - fixed - gaps).max(0.0);
    let palette_least = rows as f32 * CELL_MIN_H;
    let button_h = ((room - palette_least) / 9.0)
        .floor()
        .clamp(BUTTON_MIN_H, BUTTON);
    let tools_h = 9.0 * button_h + gaps;
    let palette_h = (rect.h - fixed - tools_h).clamp(0.0, rows as f32 * CELL_H);

    let mut y = rect.y + DOCK_PAD;
    let brush_well = Rect::new(x0, snap(y), inner_w, snap(y + BRUSH_WELL_H) - snap(y));
    let mut brushes = [Rect::default(); 10];
    let (pad, cell_w, cell_h) = (3.0, (inner_w - 6.0) / 5.0, (BRUSH_WELL_H - 6.0) / 2.0);
    for (row, indices) in BRUSH_GRID.iter().enumerate() {
        for (col, &index) in indices.iter().enumerate() {
            let (cx, cy) = (
                x0 + pad + col as f32 * cell_w,
                y + pad + row as f32 * cell_h,
            );
            brushes[index] = Rect::new(
                snap(cx),
                snap(cy),
                snap(cx + cell_w) - snap(cx),
                snap(cy + cell_h) - snap(cy),
            );
        }
    }
    y += BRUSH_WELL_H + SECTION_GAP;

    let mut tools = [Rect::default(); 18];
    let button_w = (inner_w - GAP) / 2.0;
    for (i, slot) in tools.iter_mut().enumerate() {
        let (row, col) = (i / 2, i % 2);
        let (bx, by) = (
            x0 + col as f32 * (button_w + GAP),
            y + row as f32 * (button_h + GAP),
        );
        *slot = Rect::new(
            snap(bx),
            snap(by),
            snap(bx + button_w) - snap(bx),
            snap(by + button_h) - snap(by),
        );
    }
    y += tools_h + SECTION_GAP;

    let indicator = Rect::new(x0, snap(y), inner_w, snap(y + INDICATOR_H) - snap(y));
    y += INDICATOR_H + SECTION_GAP;

    let palette = Rect::new(x0, snap(y), inner_w, snap(y + palette_h) - snap(y));
    let (cw, ch) = (inner_w / cols as f32, palette_h / rows as f32);
    let cells = (0..shown)
        .map(|i| {
            let (col, row) = (i / rows, i % rows);
            let (cx, cy) = (x0 + col as f32 * cw, y + row as f32 * ch);
            Rect::new(
                snap(cx),
                snap(cy),
                snap(cx + cw) - snap(cx),
                snap(cy + ch) - snap(cy),
            )
        })
        .collect();

    Dock {
        rect,
        brush_well,
        brushes,
        tools,
        indicator,
        palette,
        cells,
        palette_cols: cols,
        palette_rows: rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];
    const PALETTES: [usize; 7] = [2, 4, 8, 16, 32, 64, 256];

    fn layout(window: (f32, f32), scale: f32, colours: usize) -> MainLayout {
        MainLayout::new(
            window,
            scale,
            Chrome::ALL,
            colours,
            (320, 256),
            PixelSize::default(),
        )
    }

    /// Every rectangle a click can land on, with a name for messages.
    fn parts(l: &MainLayout) -> Vec<(String, Rect)> {
        let dock = l.dock.as_ref().expect("dock shown");
        let mut all = vec![("brush well".to_string(), dock.brush_well)];
        all.extend(
            dock.tools
                .iter()
                .enumerate()
                .map(|(i, r)| (format!("tool {i}"), *r)),
        );
        all.push(("indicator".into(), dock.indicator));
        all.extend(
            dock.cells
                .iter()
                .enumerate()
                .map(|(i, r)| (format!("cell {i}"), *r)),
        );
        all
    }

    fn on_pixels(v: f32, scale: f32) -> bool {
        ((v * scale) - (v * scale).round()).abs() < 1e-3
    }

    #[test]
    fn the_start_window_has_the_measures_of_the_design() {
        let l = layout((1280.0, 800.0), 1.0, 32);
        assert_eq!(l.bar, Some(Rect::new(0.0, 0.0, 1280.0, 30.0)));
        assert_eq!(l.viewport, Rect::new(0.0, 30.0, 1188.0, 770.0));
        let dock = l.dock.as_ref().expect("dock");
        assert_eq!(dock.rect, Rect::new(1188.0, 30.0, 92.0, 770.0));
        assert_eq!(dock.tools[0], Rect::new(1195.0, 90.0, 36.0, 36.0));
        assert_eq!(dock.tools[1], Rect::new(1237.0, 90.0, 36.0, 36.0));
        assert_eq!(dock.tools[2].y, 132.0);
        assert_eq!(
            (dock.palette_cols, dock.palette_rows, dock.cells.len()),
            (4, 8, 32)
        );
        // 3x fits: 960 x 768 centred in 1188 x 770.
        assert_eq!(l.zoom, 3);
        assert_eq!(l.picture, Rect::new(114.0, 31.0, 960.0, 768.0));
        assert_eq!(l.menu_titles.len(), dpx_ui::menu::MENUS.len());
        assert!(l.status_x() > l.menu_titles[6].right());
    }

    #[test]
    fn nothing_overlaps_or_leaves_the_window_down_to_the_minimum_size() {
        for window in [
            MIN_WINDOW,
            (1280.0, 800.0),
            (1920.0, 1200.0),
            (901.3, 613.7),
        ] {
            for scale in SCALES {
                for colours in PALETTES {
                    let l = layout(window, scale, colours);
                    let what = format!("{window:?} at {scale}x with {colours} colours");
                    let (bar, dock) = (l.bar.expect("bar"), l.dock.as_ref().expect("dock"));
                    assert!(!bar.intersects(&dock.rect), "{what}");
                    assert!(!bar.intersects(&l.viewport), "{what}");
                    assert!(!dock.rect.intersects(&l.viewport), "{what}");
                    for r in [bar, dock.rect, l.viewport] {
                        assert!(l.window.encloses(&r), "{what}: {r:?}");
                    }
                    for t in &l.menu_titles {
                        assert!(bar.encloses(t), "{what}: title {t:?}");
                    }
                    let parts = parts(&l);
                    for (i, (name, r)) in parts.iter().enumerate() {
                        assert!(r.w > 0.0 && r.h > 0.0, "{what}: {name} is empty");
                        assert!(
                            dock.rect.encloses(r),
                            "{what}: {name} {r:?} leaves the dock"
                        );
                        for (other, o) in &parts[i + 1..] {
                            assert!(!r.intersects(o), "{what}: {name} overlaps {other}");
                        }
                    }
                    for (i, b) in dock.brushes.iter().enumerate() {
                        assert!(dock.brush_well.encloses(b), "{what}: brush {i}");
                        for o in &dock.brushes[i + 1..] {
                            assert!(!b.intersects(o), "{what}: brush {i} overlaps another");
                        }
                    }
                    for c in &dock.cells {
                        assert!(dock.palette.inset(-1e-3).encloses(c), "{what}: {c:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_edge_lies_on_a_physical_pixel() {
        for scale in SCALES {
            for window in [MIN_WINDOW, (1280.0, 800.0), (1001.0, 777.0)] {
                let l = layout(window, scale, 32);
                let mut rects = parts(&l);
                rects.push(("bar".into(), l.bar.expect("bar")));
                rects.push(("viewport".into(), l.viewport));
                rects.push(("picture".into(), l.picture));
                rects.extend(l.menu_titles.iter().map(|r| ("title".to_string(), *r)));
                let dock = l.dock.as_ref().expect("dock");
                rects.extend(dock.brushes.iter().map(|r| ("brush".to_string(), *r)));
                for (name, r) in rects {
                    for v in [r.x, r.y, r.right(), r.bottom()] {
                        assert!(on_pixels(v, scale), "{window:?} at {scale}x: {name} {r:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_low_window_shrinks_palette_cells_first_and_buttons_second() {
        let roomy = layout((1280.0, 800.0), 1.0, 32);
        let dock = roomy.dock.as_ref().expect("dock");
        assert_eq!((dock.tools[0].h, dock.cells[0].h), (BUTTON, CELL_H));
        // 64 colours in 16 rows still fit beside full buttons at 800 points.
        let many = layout((1280.0, 800.0), 1.0, 64);
        let dock = many.dock.as_ref().expect("dock");
        assert_eq!((dock.palette_rows, dock.tools[0].h), (16, BUTTON));
        assert!(dock.cells[0].h >= 12.0, "{}", dock.cells[0].h);
        // The smallest window: lower buttons, cells still there.
        let low = layout(MIN_WINDOW, 1.0, 64);
        let dock = low.dock.as_ref().expect("dock");
        assert!(dock.tools[0].h < BUTTON && dock.tools[0].h >= BUTTON_MIN_H);
        assert!(dock.cells[0].h >= CELL_MIN_H, "{}", dock.cells[0].h);
        assert_eq!(dock.tools[0].w, BUTTON, "buttons keep their width");
    }

    #[test]
    fn the_picture_is_a_whole_enlargement_cut_to_what_fits() {
        let pixel = |half_points| PixelSize(half_points);
        let at =
            |window, scale, size| MainLayout::new(window, scale, Chrome::ALL, 32, (320, 256), size);
        // Wanted 3x, fits.
        assert_eq!(at((1280.0, 800.0), 1.0, pixel(6)).zoom, 3);
        // The same window at twice the device scale: six physical pixels per pixel, the same
        // picture in points.
        let hi = at((1280.0, 800.0), 2.0, pixel(6));
        assert_eq!((hi.zoom, hi.picture.w, hi.picture.h), (6, 960.0, 768.0));
        // Half steps are whole physical pixels at 2x and round down at 1x.
        assert_eq!(at((1280.0, 800.0), 2.0, pixel(5)).zoom, 5);
        assert_eq!(at((1280.0, 800.0), 1.0, pixel(5)).zoom, 2);
        // Wanted 8x, only 3x fits; the smallest window still shows 1x.
        assert_eq!(at((1280.0, 800.0), 1.0, pixel(16)).zoom, 3);
        assert_eq!(at(MIN_WINDOW, 1.0, pixel(6)).zoom, 1);
        // Centred in the viewport.
        let l = at((1000.0, 700.0), 1.0, pixel(4));
        let (cx, cy) = l.picture.center();
        let (vx, vy) = l.viewport.center();
        assert!(
            (cx - vx).abs() <= 0.5 && (cy - vy).abs() <= 0.5,
            "{:?}",
            l.picture
        );
        // A picture larger than the viewport stays at 1x and is centred over it.
        let big = MainLayout::new((800.0, 600.0), 1.0, Chrome::ALL, 32, (1280, 1024), pixel(6));
        assert_eq!(big.zoom, 1);
        assert!(big.picture.x < big.viewport.x && big.picture.y < big.viewport.y);
    }

    #[test]
    fn hidden_chrome_gives_its_room_to_the_viewport() {
        let size = (1280.0, 800.0);
        let at = |chrome| MainLayout::new(size, 1.0, chrome, 32, (320, 256), PixelSize::default());
        let no_bar = at(Chrome {
            bar: false,
            dock: true,
        });
        assert!(no_bar.bar.is_none() && no_bar.menu_titles.is_empty());
        assert_eq!(no_bar.viewport, Rect::new(0.0, 0.0, 1188.0, 800.0));
        assert_eq!(no_bar.dock.as_ref().expect("dock").rect.y, 0.0);
        let bare = at(Chrome {
            bar: false,
            dock: false,
        });
        assert!(bare.dock.is_none());
        assert_eq!(bare.viewport, bare.window);
    }

    #[test]
    fn palette_cells_are_column_major_and_brushes_follow_the_grid() {
        let l = layout((1280.0, 800.0), 1.0, 32);
        let dock = l.dock.as_ref().expect("dock");
        assert_eq!(
            dock.cells[1].x, dock.cells[0].x,
            "colour 1 is below colour 0"
        );
        assert!(dock.cells[1].y > dock.cells[0].y);
        assert!(
            dock.cells[8].x > dock.cells[0].x,
            "colour 8 starts the next column"
        );
        assert_eq!(dock.cells[8].y, dock.cells[0].y);
        // Round brushes left to right in the upper row, the square 2 under the round 1.
        assert!(dock.brushes[0].x < dock.brushes[1].x && dock.brushes[2].x < dock.brushes[3].x);
        assert_eq!(dock.brushes[7].x, dock.brushes[0].x);
        assert!(dock.brushes[7].y > dock.brushes[0].y);
        assert_eq!(l.tool_button(dpx_ui::Tool::Line), Some(dock.tools[2]));
        // Small palettes use fewer, wider columns.
        let two = layout((1280.0, 800.0), 1.0, 2);
        let dock = two.dock.as_ref().expect("dock");
        assert_eq!((dock.palette_cols, dock.cells.len()), (1, 2));
    }
}
