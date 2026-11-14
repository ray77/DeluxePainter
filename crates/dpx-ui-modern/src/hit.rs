//! What lies under a point of the Modern main window. The chrome answers with the same
//! [`Hit`] values as the Classic skin, so everything behind a click (`handle_press` and the
//! tool, brush, palette and indicator actions) is shared; a point on the picture answers with
//! the picture pixel, which the app maps to the page exactly as in Classic.

use dpx_ui::layer_panel::BUTTONS;
use dpx_ui::toolbox::TOOL_ROWS;
use dpx_ui::{Hit, LayerHit, LayerPanel, UiState};

use crate::layout::{LayerColumn, MainLayout};
use crate::paint::Rect;

/// What a point of the window is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    /// The chrome: the bar, a part of the dock or of the layer column.
    Ui(Hit),
    /// The picture quad, at pixel (`x`, `y`) of its image.
    Picture { x: i32, y: i32 },
    /// The canvas around the picture, or a point outside the window.
    Canvas,
}

/// Whether a point at (`x`, `y`) of button `rect` is in the lower-right half, which selects
/// the filled variant of a split tool. The button is divided from its upper-right to its
/// lower-left corner.
pub fn in_filled_half(rect: &Rect, x: f32, y: f32) -> bool {
    (x - rect.x) / rect.w + (y - rect.y) / rect.h >= 1.0
}

/// What the layer column has at (`x`, `y`), a point inside it, for the layers `panel` shows.
/// The answers are the Classic panel's: an eye box, a name, the rename field, a button, a
/// scroll arrow (while there is something to scroll) or nothing.
pub fn layer_hit(column: &LayerColumn, panel: &LayerPanel, x: f32, y: f32) -> LayerHit {
    let cap = column.capacity;
    let scrolls = panel.more_above() || panel.more_below(cap);
    if scrolls && column.scroll_up.contains(x, y) {
        return LayerHit::ScrollUp;
    }
    if scrolls && column.scroll_down.contains(x, y) {
        return LayerHit::ScrollDown;
    }
    if let Some(i) = column.buttons.iter().position(|r| r.contains(x, y)) {
        return LayerHit::Button(BUTTONS[i]);
    }
    if column.list.contains(x, y)
        && let Some(slot) = column.slot_at(y)
        && let Some(layer) = panel.layer_at(slot)
    {
        let name = column.name(slot);
        if panel.renaming() == Some(layer) && name.contains(x, y) {
            return LayerHit::Field;
        }
        return if x < name.x {
            LayerHit::Eye(layer)
        } else {
            LayerHit::Name(layer)
        };
    }
    LayerHit::Background
}

/// What is under the window point (`x`, `y`) while the interface is in state `ui`: as
/// [`hit`], and inside the layer column the part of the layer panel that is there.
pub fn hit_in(layout: &MainLayout, ui: &UiState, x: f32, y: f32) -> Spot {
    match (hit(layout, x, y), &layout.layers, &ui.layer_panel) {
        (Spot::Ui(Hit::LayerPanel(_)), Some(column), Some(panel)) => {
            Spot::Ui(Hit::LayerPanel(layer_hit(column, panel, x, y)))
        }
        (spot, ..) => spot,
    }
}

/// What is under the window point (`x`, `y`). This is the layout alone: the layer column
/// answers as its background everywhere ([`hit_in`] knows its rows and buttons).
pub fn hit(layout: &MainLayout, x: f32, y: f32) -> Spot {
    if !layout.window.contains(x, y) {
        return Spot::Canvas;
    }
    if layout.bar.is_some_and(|bar| bar.contains(x, y)) {
        return Spot::Ui(Hit::TitleBar);
    }
    if layout
        .layers
        .as_ref()
        .is_some_and(|c| c.rect.contains(x, y))
    {
        return Spot::Ui(Hit::LayerPanel(LayerHit::Background));
    }
    if let Some(dock) = layout.dock.as_ref().filter(|d| d.rect.contains(x, y)) {
        if let Some(i) = dock.tools.iter().position(|r| r.contains(x, y)) {
            let tool = TOOL_ROWS[i / 2][i % 2];
            return Spot::Ui(Hit::Tool {
                tool,
                filled_half: tool.is_split() && in_filled_half(&dock.tools[i], x, y),
            });
        }
        if let Some(i) = dock.brushes.iter().position(|r| r.contains(x, y)) {
            return Spot::Ui(Hit::Brush(i));
        }
        if dock.indicator.contains(x, y) {
            return Spot::Ui(Hit::Indicator);
        }
        if let Some(i) = dock.cells.iter().position(|r| r.contains(x, y)) {
            return Spot::Ui(Hit::PaletteCell(i as u8));
        }
        return Spot::Ui(Hit::ToolboxBackground);
    }
    if layout.viewport.contains(x, y) && layout.picture.contains(x, y) {
        let (w, h) = layout.picture_size;
        let pixel = |at: f32, size: f32, n: u32| {
            if layout.shown >= 1.0 {
                // In physical pixels, where the picture is an exact enlargement.
                let zoom = i64::from(layout.zoom.max(1));
                (((at * layout.scale).floor() as i64) / zoom).clamp(0, i64::from(n) - 1) as i32
            } else {
                // A screen shown smaller than one physical pixel per pixel: by proportion.
                ((f64::from(at) / f64::from(size.max(1e-3)) * f64::from(n)).floor() as i64)
                    .clamp(0, i64::from(n) - 1) as i32
            }
        };
        return Spot::Picture {
            x: pixel(x - layout.picture.x, layout.picture.w, w),
            y: pixel(y - layout.picture.y, layout.picture.h, h),
        };
    }
    Spot::Canvas
}

/// The index of the menu whose title in the bar is under (`x`, `y`).
pub fn menu_title(layout: &MainLayout, x: f32, y: f32) -> Option<usize> {
    let bar = layout.bar?;
    // The whole height of the bar belongs to a title: the pointer comes from below.
    layout
        .menu_titles
        .iter()
        .position(|r| bar.contains(x, y) && x >= r.x && x < r.right())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Chrome;
    use dpx_ui::{PixelSize, Tool};

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

    const CASES: [((f32, f32), f32, usize); 5] = [
        ((1280.0, 800.0), 1.0, 32),
        ((1280.0, 800.0), 2.0, 64),
        ((720.0, 520.0), 1.0, 64),
        ((1001.0, 640.0), 1.5, 16),
        ((900.0, 600.0), 1.25, 2),
    ];

    #[test]
    fn the_centre_of_every_tool_half_hits_that_tool() {
        for (window, scale, colours) in CASES {
            let l = layout(window, scale, colours);
            let dock = l.dock.as_ref().expect("dock");
            for (i, r) in dock.tools.iter().enumerate() {
                let tool = TOOL_ROWS[i / 2][i % 2];
                // A quarter in from the upper-left and from the lower-right corner.
                let upper = (r.x + r.w * 0.25, r.y + r.h * 0.25);
                let lower = (r.x + r.w * 0.75, r.y + r.h * 0.75);
                assert_eq!(
                    hit(&l, upper.0, upper.1),
                    Spot::Ui(Hit::Tool {
                        tool,
                        filled_half: false
                    }),
                    "{tool:?} upper half, {window:?} at {scale}x"
                );
                assert_eq!(
                    hit(&l, lower.0, lower.1),
                    Spot::Ui(Hit::Tool {
                        tool,
                        filled_half: tool.is_split()
                    }),
                    "{tool:?} lower half, {window:?} at {scale}x"
                );
                assert_eq!(l.tool_button(tool), Some(*r));
            }
        }
    }

    #[test]
    fn the_centre_of_every_brush_cell_and_palette_cell_hits_it() {
        for (window, scale, colours) in CASES {
            let l = layout(window, scale, colours);
            let dock = l.dock.as_ref().expect("dock");
            for (i, r) in dock.brushes.iter().enumerate() {
                let (x, y) = r.center();
                assert_eq!(
                    hit(&l, x, y),
                    Spot::Ui(Hit::Brush(i)),
                    "{window:?} at {scale}x"
                );
            }
            assert_eq!(dock.cells.len(), colours.min(64));
            for (i, r) in dock.cells.iter().enumerate() {
                let (x, y) = r.center();
                assert_eq!(
                    hit(&l, x, y),
                    Spot::Ui(Hit::PaletteCell(i as u8)),
                    "{window:?} at {scale}x with {colours} colours"
                );
            }
            let (x, y) = dock.indicator.center();
            assert_eq!(hit(&l, x, y), Spot::Ui(Hit::Indicator));
        }
    }

    #[test]
    fn bar_dock_gaps_canvas_and_outside() {
        let l = layout((1280.0, 800.0), 1.0, 32);
        let dock = l.dock.as_ref().expect("dock");
        assert_eq!(hit(&l, 600.0, 15.0), Spot::Ui(Hit::TitleBar));
        // The bar runs over the dock's column too.
        assert_eq!(hit(&l, 1270.0, 15.0), Spot::Ui(Hit::TitleBar));
        // Between the two tool columns, and in the dock's padding.
        let gap = (dock.tools[0].right() + 3.0, dock.tools[0].y + 10.0);
        assert_eq!(hit(&l, gap.0, gap.1), Spot::Ui(Hit::ToolboxBackground));
        assert_eq!(hit(&l, 1190.0, 400.0), Spot::Ui(Hit::ToolboxBackground));
        // Below the palette.
        assert_eq!(hit(&l, 1230.0, 795.0), Spot::Ui(Hit::ToolboxBackground));
        // Left of the picture: canvas.
        assert_eq!(hit(&l, 50.0, 400.0), Spot::Canvas);
        for (x, y) in [(-1.0, 10.0), (10.0, -1.0), (1280.0, 10.0), (10.0, 800.0)] {
            assert_eq!(
                hit(&l, x, y),
                Spot::Canvas,
                "({x}, {y}) is outside the window"
            );
        }
    }

    #[test]
    fn a_point_on_the_picture_gives_its_pixel() {
        for scale in [1.0, 2.0] {
            let l = layout((1280.0, 800.0), scale, 32);
            // 3 points per pixel at both scales.
            let p = l.picture;
            assert_eq!(hit(&l, p.x, p.y), Spot::Picture { x: 0, y: 0 });
            assert_eq!(hit(&l, p.x + 2.9, p.y + 2.9), Spot::Picture { x: 0, y: 0 });
            assert_eq!(hit(&l, p.x + 3.0, p.y + 6.0), Spot::Picture { x: 1, y: 2 });
            assert_eq!(
                hit(&l, p.right() - 0.1, p.bottom() - 0.1),
                Spot::Picture { x: 319, y: 255 }
            );
            assert_eq!(hit(&l, p.right(), p.y + 5.0), Spot::Canvas);
            let (cx, cy) = p.center();
            assert_eq!(hit(&l, cx, cy), Spot::Picture { x: 160, y: 128 });
        }
    }

    #[test]
    fn a_screen_shown_smaller_answers_from_its_first_to_its_last_pixel() {
        use crate::layout::{MIN_WINDOW, Picture};
        // An interlaced HiRes screen in the smallest window, beside the layer column: less
        // than one physical pixel per pixel, whole in the viewport.
        for scale in [1.0, 1.25] {
            let l = MainLayout::for_picture(
                MIN_WINDOW,
                scale,
                Chrome::WITH_LAYERS,
                32,
                Picture {
                    size: (640, 512),
                    per_ui_pixel: 2,
                    pixel_size: PixelSize(6),
                    hosts_requester: false,
                },
            );
            assert!(l.shown < 1.0, "{scale}: {}", l.shown);
            let p = l.picture;
            assert!(l.viewport.encloses(&p));
            assert_eq!(hit(&l, p.x, p.y), Spot::Picture { x: 0, y: 0 });
            assert_eq!(
                hit(&l, p.right() - 0.01, p.bottom() - 0.01),
                Spot::Picture { x: 639, y: 511 }
            );
            let (cx, cy) = p.center();
            assert_eq!(hit(&l, cx, cy), Spot::Picture { x: 320, y: 256 });
            // Left to right the pixels only grow, and none is skipped by more than one.
            let mut last = 0;
            let step = 1.0 / scale;
            let mut x = p.x;
            while x < p.right() {
                let Spot::Picture { x: sx, .. } = hit(&l, x, cy) else {
                    panic!("({x}, {cy}) is on the picture");
                };
                assert!(sx >= last && sx <= last + 2, "{scale}: {last} -> {sx}");
                last = sx;
                x += step;
            }
            assert!(last >= 638, "{scale}: {last}");
        }
    }

    #[test]
    fn hidden_chrome_takes_no_clicks() {
        let l = MainLayout::new(
            (1280.0, 800.0),
            1.0,
            Chrome {
                bar: false,
                dock: false,
                layers: false,
            },
            32,
            (320, 256),
            PixelSize::default(),
        );
        assert_eq!(hit(&l, 600.0, 2.0), Spot::Canvas);
        assert!(matches!(hit(&l, 640.0, 400.0), Spot::Picture { .. }));
        assert_eq!(menu_title(&l, 20.0, 10.0), None);
    }

    #[test]
    fn the_split_follows_the_diagonal_of_a_button_that_is_not_square() {
        let l = layout((720.0, 520.0), 1.0, 64);
        let r = l.tool_button(Tool::Rectangle).expect("dock");
        assert!(r.h < r.w, "the smallest window has low buttons");
        // Just inside the upper-right and the lower-left corner: on the diagonal's two sides.
        assert!(!in_filled_half(&r, r.x + 1.0, r.y + 1.0));
        assert!(in_filled_half(&r, r.right() - 1.0, r.bottom() - 1.0));
        assert!(!in_filled_half(
            &r,
            r.x + r.w * 0.5 - 1.0,
            r.y + r.h * 0.5 - 1.0
        ));
        assert!(in_filled_half(
            &r,
            r.x + r.w * 0.5 + 1.0,
            r.y + r.h * 0.5 + 1.0
        ));
    }

    #[test]
    fn each_menu_title_answers_over_the_whole_bar_height() {
        let l = layout((1280.0, 800.0), 1.0, 32);
        for (i, r) in l.menu_titles.iter().enumerate() {
            let (cx, _) = r.center();
            for y in [0.0, 15.0, 29.0] {
                assert_eq!(
                    menu_title(&l, cx, y),
                    Some(i),
                    "{}",
                    dpx_ui::menu::MENUS[i].title
                );
            }
            assert_eq!(menu_title(&l, cx, 31.0), None, "below the bar");
        }
        assert_eq!(
            menu_title(&l, l.status_x() + 5.0, 15.0),
            None,
            "on the status text"
        );
    }

    fn panel(count: usize, active: usize, capacity: usize) -> LayerPanel {
        let mut panel = LayerPanel::default();
        let rows = (0..count)
            .map(|i| dpx_ui::LayerRow {
                name: format!("Layer {i}"),
                visible: true,
            })
            .collect();
        panel.update(rows, active, [true; 6], capacity);
        panel
    }

    fn with_layers(window: (f32, f32), scale: f32) -> MainLayout {
        MainLayout::new(
            window,
            scale,
            Chrome::WITH_LAYERS,
            32,
            (320, 256),
            PixelSize::default(),
        )
    }

    #[test]
    fn the_layer_column_answers_with_the_classic_panels_hits() {
        for (window, scale) in [
            ((1280.0, 800.0), 1.0),
            ((1280.0, 800.0), 2.0),
            ((720.0, 520.0), 1.0),
            ((1001.0, 640.0), 1.5),
            ((900.0, 600.0), 1.25),
        ] {
            let l = with_layers(window, scale);
            let column = l.layers.as_ref().expect("the layer column");
            let ui = UiState {
                layer_panel: Some(panel(3, 1, column.capacity)),
                ..UiState::default()
            };
            let at = |x: f32, y: f32| hit_in(&l, &ui, x, y);
            let layer = |hit: LayerHit| Spot::Ui(Hit::LayerPanel(hit));
            // Rows from the top layer down.
            for slot in 0..3 {
                let n = 2 - slot;
                let (ex, ey) = column.eye(slot).center();
                assert_eq!(at(ex, ey), layer(LayerHit::Eye(n)), "{window:?} {scale}");
                let (nx, ny) = column.name(slot).center();
                assert_eq!(at(nx, ny), layer(LayerHit::Name(n)), "{window:?} {scale}");
                // Every corner of the row belongs to that row.
                let r = column.row(slot);
                let e = 0.2 / scale;
                assert_eq!(at(r.x + e, r.y + e), layer(LayerHit::Eye(n)));
                assert_eq!(
                    at(r.right() - e, r.bottom() - e),
                    layer(LayerHit::Name(n)),
                    "{window:?} {scale}"
                );
            }
            // An empty row, the title and the gaps: nothing.
            let (x, y) = column.row(3).center();
            assert_eq!(at(x, y), layer(LayerHit::Background));
            let (x, y) = column.header.center();
            assert_eq!(at(x, y), layer(LayerHit::Background));
            for (i, r) in column.buttons.iter().enumerate() {
                let (x, y) = r.center();
                assert_eq!(at(x, y), layer(LayerHit::Button(BUTTONS[i])));
            }
            // The arrows scroll only while rows are scrolled away.
            let (ux, uy) = column.scroll_up.center();
            let (dx, dy) = column.scroll_down.center();
            assert_eq!(at(ux, uy), layer(LayerHit::Background));
            // The layout alone knows the column, not its rows.
            let (nx, ny) = column.name(0).center();
            assert_eq!(hit(&l, nx, ny), layer(LayerHit::Background));
            // Without a panel the column is background.
            assert_eq!(
                hit_in(&l, &UiState::default(), nx, ny),
                layer(LayerHit::Background)
            );
            // Left of the column the picture or the canvas, right of it the dock.
            assert!(!matches!(
                at(column.rect.x - 1.0, 300.0),
                Spot::Ui(Hit::LayerPanel(_))
            ));
            assert!(matches!(
                at(column.rect.right() + 1.0, 300.0),
                Spot::Ui(
                    Hit::ToolboxBackground | Hit::Tool { .. } | Hit::Brush(_) | Hit::PaletteCell(_)
                )
            ));

            // More layers than rows: the arrows work, and the rows follow the scroll.
            let many = column.capacity + 5;
            let mut scrolled = panel(many, many - 1, column.capacity);
            scrolled.scroll_by(2, column.capacity);
            let ui = UiState {
                layer_panel: Some(scrolled),
                ..UiState::default()
            };
            let at = |x: f32, y: f32| hit_in(&l, &ui, x, y);
            assert_eq!(at(ux, uy), layer(LayerHit::ScrollUp));
            assert_eq!(at(dx, dy), layer(LayerHit::ScrollDown));
            let (nx, ny) = column.name(0).center();
            assert_eq!(at(nx, ny), layer(LayerHit::Name(many - 3)));
            let last = column.capacity - 1;
            let (nx, ny) = column.name(last).center();
            assert_eq!(at(nx, ny), layer(LayerHit::Name(many - 3 - last)));
        }
    }

    #[test]
    fn the_name_being_edited_is_the_field() {
        let l = with_layers((1280.0, 800.0), 1.0);
        let column = l.layers.as_ref().expect("the layer column");
        let mut panel = panel(3, 2, column.capacity);
        panel.begin_rename(1);
        let ui = UiState {
            layer_panel: Some(panel),
            ..UiState::default()
        };
        let at = |r: Rect| {
            let (x, y) = r.center();
            hit_in(&l, &ui, x, y)
        };
        // Layer 1 is in row 1: its name is the gadget, its eye still the eye.
        assert_eq!(
            at(column.name(1)),
            Spot::Ui(Hit::LayerPanel(LayerHit::Field))
        );
        assert_eq!(
            at(column.eye(1)),
            Spot::Ui(Hit::LayerPanel(LayerHit::Eye(1)))
        );
        assert_eq!(
            at(column.name(0)),
            Spot::Ui(Hit::LayerPanel(LayerHit::Name(2)))
        );
    }
}
