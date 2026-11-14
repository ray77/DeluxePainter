//! A whole Modern main window as one image, for headless snapshots and tests. The live
//! window uses the same layout, lists and rasteriser.

use crate::build::{self, Model};
use crate::layout::{Chrome, MainLayout, Picture};
use crate::raster::{Image, Renderer};
use crate::theme::Theme;

/// What a window shows and how large it is.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// The window's size in points.
    pub size: (f32, f32),
    /// Physical pixels per point.
    pub scale: f32,
    pub theme: Theme,
    pub model: Model<'a>,
    /// The picture quad's image: the screen as the Classic skin composes it without its
    /// chrome, pixel shape applied.
    pub picture: Image<'a>,
    /// Picture pixels per interface (LoRes) pixel: 1 for a LoRes screen, 2 for HiRes or
    /// interlace, 4 for SuperHiRes.
    pub per_ui_pixel: u32,
}

impl Frame<'_> {
    /// The layout of this frame: the bar and the dock follow the shared visibility flags
    /// (F9, F10), the layer column the shared layer panel (Prefs ▸ Layers), the picture's
    /// enlargement Prefs ▸ Pixel Size.
    pub fn layout(&self) -> MainLayout {
        let ui = self.model.ui;
        MainLayout::for_picture(
            self.size,
            self.scale,
            Chrome {
                bar: ui.title_visible,
                dock: ui.toolbox_visible,
                // The layer column goes with the dock (F10) and needs a panel to show.
                layers: ui.toolbox_visible && ui.layer_panel.is_some(),
            },
            self.model.palette.len(),
            Picture {
                size: (self.picture.width, self.picture.height),
                per_ui_pixel: self.per_ui_pixel,
                pixel_size: ui.pixel_size,
                // A requester in the picture's image may use the whole window.
                hosts_requester: self.model.dimmed,
            },
        )
    }
}

/// Renders `frame`: the width and height in physical pixels and opaque RGBA8.
pub fn render_window(frame: &Frame<'_>, renderer: &mut Renderer) -> (u32, u32, Vec<u8>) {
    let layout = frame.layout();
    let list = build::window(&layout, &frame.theme, &frame.model);
    let (w, h) = (
        (layout.window.w * layout.scale).round().max(1.0) as u32,
        (layout.window.h * layout.scale).round().max(1.0) as u32,
    );
    let rgba = renderer.render(
        &list,
        &[frame.picture],
        w,
        h,
        layout.scale,
        frame.theme.canvas,
    );
    (w, h, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::Color;
    use crate::theme::ThemeKind;
    use dpx_core::Palette;
    use dpx_ui::toolbox::TOOL_ROWS;
    use dpx_ui::{Hit, Tool, UiState};

    /// A 320×256 picture: every pixel its own colour, so a misplaced or blended pixel shows.
    fn picture() -> Vec<u8> {
        (0..256u32)
            .flat_map(|y| {
                (0..320u32).flat_map(move |x| [(x % 256) as u8, y as u8, (x / 2) as u8, 255])
            })
            .collect()
    }

    struct Shot {
        w: u32,
        scale: f32,
        rgba: Vec<u8>,
    }

    impl Shot {
        /// The colour of the pixel under the point (`x`, `y`).
        fn at(&self, x: f32, y: f32) -> Color {
            let (px, py) = ((x * self.scale) as u32, (y * self.scale) as u32);
            let i = ((py * self.w + px) * 4) as usize;
            Color {
                r: self.rgba[i],
                g: self.rgba[i + 1],
                b: self.rgba[i + 2],
                a: self.rgba[i + 3],
            }
        }
    }

    fn shoot(ui: &UiState, palette: &Palette, kind: ThemeKind, scale: f32) -> (Shot, MainLayout) {
        let rgba = picture();
        let frame = Frame {
            size: (1280.0, 800.0),
            scale,
            theme: Theme::of(kind),
            model: Model::new(ui, palette),
            picture: Image {
                width: 320,
                height: 256,
                rgba: &rgba,
            },
            per_ui_pixel: 1,
        };
        let (w, h, out) = render_window(&frame, &mut Renderer::new());
        assert_eq!((w, h), ((1280.0 * scale) as u32, (800.0 * scale) as u32));
        assert_eq!(out.len(), (w * h * 4) as usize);
        (
            Shot {
                w,
                scale,
                rgba: out,
            },
            frame.layout(),
        )
    }

    const CASES: [(ThemeKind, f32); 4] = [
        (ThemeKind::Dark, 1.0),
        (ThemeKind::Dark, 2.0),
        (ThemeKind::Light, 1.0),
        (ThemeKind::Light, 2.0),
    ];

    /// The live window keeps one image per region. Each must be that part of the whole
    /// window's image, at whole and fractional device scales.
    #[test]
    fn a_region_rendered_alone_equals_its_part_of_the_window() {
        let (palette, rgba) = (Palette::dpaint_default_32(), picture());
        let ui = UiState {
            tool: Tool::Circle,
            filled: true,
            coords: Some((123, 45)),
            ..UiState::default()
        };
        for (kind, scale) in [
            (ThemeKind::Dark, 1.0),
            (ThemeKind::Light, 2.0),
            (ThemeKind::Dark, 1.5),
        ] {
            let frame = Frame {
                size: (1001.0, 640.0),
                scale,
                theme: Theme::of(kind),
                model: Model::new(&ui, &palette),
                picture: Image {
                    width: 320,
                    height: 256,
                    rgba: &rgba,
                },
                per_ui_pixel: 1,
            };
            let mut renderer = Renderer::new();
            let (w, _, whole) = render_window(&frame, &mut renderer);
            let l = frame.layout();
            let regions = [
                (
                    "bar",
                    l.bar.expect("bar"),
                    build::title_bar(&l, &frame.theme, &frame.model),
                ),
                (
                    "dock",
                    l.dock.as_ref().expect("dock").rect,
                    build::dock(&l, &frame.theme, &frame.model),
                ),
                (
                    "viewport",
                    l.viewport,
                    build::canvas(&l, &frame.theme, false),
                ),
            ];
            for (name, rect, list) in regions {
                let px = |v: f32| (v * scale).round() as i32;
                let (x0, y0) = (px(rect.x), px(rect.y));
                let (rw, rh) = (
                    (px(rect.right()) - x0) as u32,
                    (px(rect.bottom()) - y0) as u32,
                );
                let part = renderer.render_region(
                    &list,
                    &[frame.picture],
                    (x0, y0),
                    rw,
                    rh,
                    scale,
                    frame.theme.canvas,
                );
                assert_eq!(part.len(), (rw * rh * 4) as usize);
                let (mut differing, mut worst) = (0u32, 0u8);
                for y in 0..rh {
                    let from = (((y0 as u32 + y) * w + x0 as u32) * 4) as usize;
                    let row = (y * rw * 4) as usize;
                    let (a, b) = (
                        &part[row..row + (rw * 4) as usize],
                        &whole[from..from + (rw * 4) as usize],
                    );
                    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
                        if p != q {
                            differing += 1;
                            let d = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max();
                            worst = worst.max(d.unwrap_or(0));
                        }
                    }
                }
                // tiny-skia flattens a curve a little differently when it is moved, so a
                // few anti-aliased edge pixels of icons differ slightly; nothing else does.
                assert!(
                    differing * 2000 <= rw * rh && worst <= 64,
                    "{name}, {kind:?} at {scale}x: {differing} pixels differ, by up to {worst}"
                );
            }
        }
    }

    #[test]
    fn every_pixel_is_opaque() {
        let (ui, palette) = (UiState::default(), Palette::dpaint_default_32());
        let (shot, _) = shoot(&ui, &palette, ThemeKind::Dark, 1.0);
        assert!(shot.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }

    #[test]
    fn palette_swatch_centres_show_the_palette_colours() {
        let palette = Palette::dpaint_default_32();
        // Foreground and background on two cells: rings must not reach the centre.
        let ui = UiState {
            fg: 5,
            bg: 9,
            ..UiState::default()
        };
        for (kind, scale) in CASES {
            let (shot, l) = shoot(&ui, &palette, kind, scale);
            let dock = l.dock.as_ref().expect("dock");
            assert_eq!(dock.cells.len(), 32);
            for (i, cell) in dock.cells.iter().enumerate() {
                let c = palette.get(i as u8).expect("colour");
                let (x, y) = cell.center();
                assert_eq!(
                    shot.at(x, y),
                    Color::rgb(c.r, c.g, c.b),
                    "colour {i}, {kind:?} {scale}x"
                );
            }
            // The foreground cell has an accent ring at its edge, the others do not.
            let theme = Theme::of(kind);
            let edge = |i: usize| {
                let cell = dock.cells[i];
                shot.at(cell.x + cell.w / 2.0, cell.y + 1.6)
            };
            assert_eq!(edge(5), theme.accent, "{kind:?} {scale}x");
            assert_eq!(edge(9), theme.text_dim, "{kind:?} {scale}x");
            assert_eq!(
                edge(12),
                theme.hairline,
                "a plain cell has a fine rim, {kind:?} {scale}x"
            );
        }
    }

    #[test]
    fn the_indicator_shows_foreground_on_background() {
        let palette = Palette::dpaint_default_32();
        let ui = UiState {
            fg: 14,
            bg: 3,
            ..UiState::default()
        };
        let colour = |i: u8| {
            let c = palette.get(i).expect("colour");
            Color::rgb(c.r, c.g, c.b)
        };
        for (kind, scale) in CASES {
            let (shot, l) = shoot(&ui, &palette, kind, scale);
            let r = l.dock.as_ref().expect("dock").indicator;
            let (cx, cy) = r.center();
            assert_eq!(shot.at(cx, cy), colour(14), "{kind:?} {scale}x");
            assert_eq!(shot.at(r.x + 8.0, cy), colour(3), "{kind:?} {scale}x");
            assert_eq!(shot.at(r.right() - 8.0, cy), colour(3), "{kind:?} {scale}x");
        }
    }

    #[test]
    fn the_selected_tool_is_on_the_accent_and_the_others_on_the_button_colour() {
        let palette = Palette::dpaint_default_32();
        for (kind, scale) in CASES {
            let theme = Theme::of(kind);
            for &tool in TOOL_ROWS.iter().flatten().filter(|t| !t.is_action()) {
                for filled in [false, true]
                    .into_iter()
                    .take(1 + usize::from(tool.is_split()))
                {
                    let ui = UiState {
                        tool,
                        filled,
                        ..UiState::default()
                    };
                    let (shot, l) = shoot(&ui, &palette, kind, scale);
                    let what = format!("{tool:?} filled {filled}, {kind:?} {scale}x");
                    let r = l.tool_button(tool).expect("button");
                    // Away from the icon: near the upper-left and the lower-right corner,
                    // inside the rounded corners and off the split ticks.
                    let upper = shot.at(r.x + 4.5, r.y + 9.5);
                    let lower = shot.at(r.right() - 4.5, r.bottom() - 9.5);
                    if tool.is_split() {
                        let (on, off) = if filled {
                            (lower, upper)
                        } else {
                            (upper, lower)
                        };
                        assert_eq!(on, theme.accent, "{what}: the chosen half");
                        assert_eq!(off, theme.raised, "{what}: the other half");
                    } else {
                        assert_eq!(upper, theme.accent, "{what}");
                        assert_eq!(lower, theme.accent, "{what}");
                    }
                    // Another button keeps the button colour.
                    let other = if tool == Tool::Line {
                        Tool::Grid
                    } else {
                        Tool::Line
                    };
                    let o = l.tool_button(other).expect("button");
                    assert_eq!(
                        shot.at(o.x + 4.5, o.y + 9.5),
                        theme.raised,
                        "{what}: {other:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_tool_button_carries_ink() {
        let palette = Palette::dpaint_default_32();
        // Clear is an action and never selected: every icon is drawn in the text colour on
        // the button colour, except the selected start tool's.
        let ui = UiState {
            tool: Tool::Zoom,
            ..UiState::default()
        };
        for (kind, scale) in CASES {
            let theme = Theme::of(kind);
            let (shot, l) = shoot(&ui, &palette, kind, scale);
            for &tool in TOOL_ROWS.iter().flatten().filter(|&&t| t != Tool::Zoom) {
                let r = l.tool_button(tool).expect("button");
                // Ink of any strength (thin strokes and small dots are mostly anti-aliased
                // at 1x), counted inside the icon's box, away from corners and split ticks.
                let (mut ink, mut full, mut ground) = (0, 0, 0);
                let inner = r.inset(6.0);
                let step = 1.0 / scale;
                let mut y = inner.y + 0.5 * step;
                while y < inner.bottom() {
                    let mut x = inner.x + 0.5 * step;
                    while x < inner.right() {
                        let c = shot.at(x, y);
                        ink += usize::from(c != theme.raised);
                        full += usize::from(c == theme.text);
                        ground += usize::from(c == theme.raised);
                        x += step;
                    }
                    y += step;
                }
                let pixels = (inner.w * inner.h * scale * scale) as usize;
                let what = format!("{tool:?} {kind:?} {scale}x");
                assert!(
                    ink * 100 >= pixels * 4,
                    "{what}: {ink} ink pixels of {pixels}"
                );
                assert!(full > 0, "{what}: no pixel reaches the full text colour");
                assert!(ground * 2 > pixels, "{what}: the icon covers its button");
            }
        }
    }

    #[test]
    fn the_bar_has_the_panel_colour_and_text_in_it() {
        let (ui, palette) = (UiState::default(), Palette::dpaint_default_32());
        for (kind, scale) in CASES {
            let theme = Theme::of(kind);
            let (shot, l) = shoot(&ui, &palette, kind, scale);
            let bar = l.bar.expect("bar");
            // Free of text: the far right, above and below the text line, between titles.
            for (x, y) in [(1100.0, 15.0), (300.0, 2.0), (300.0, 26.0), (2.0, 15.0)] {
                assert_eq!(shot.at(x, y), theme.panel, "({x}, {y}) {kind:?} {scale}x");
            }
            assert_eq!(
                shot.at(600.0, bar.bottom() - 0.5),
                theme.hairline,
                "{kind:?} {scale}x"
            );
            // Ink inside every title's rectangle, and inside the status text.
            let has_ink = |r: crate::paint::Rect| {
                let step = 1.0 / scale;
                let mut y = r.y;
                while y < r.bottom() {
                    let mut x = r.x;
                    while x < r.right() {
                        if shot.at(x, y) != theme.panel {
                            return true;
                        }
                        x += step;
                    }
                    y += step;
                }
                false
            };
            for (i, r) in l.menu_titles.iter().enumerate() {
                assert!(has_ink(*r), "menu title {i} {kind:?} {scale}x");
            }
            let status = crate::paint::Rect::new(l.status_x(), 6.0, 90.0, 18.0);
            assert!(has_ink(status), "status {kind:?} {scale}x");
        }
    }

    /// A black page in the dark theme: one physical pixel of hairline says where it ends.
    #[test]
    fn a_fine_rim_marks_the_edge_of_the_picture() {
        let (ui, palette) = (UiState::default(), Palette::dpaint_default_32());
        for (kind, scale) in CASES {
            let theme = Theme::of(kind);
            let (shot, l) = shoot(&ui, &palette, kind, scale);
            let (p, px) = (l.picture, 1.0 / scale);
            let (cx, cy) = p.center();
            for (x, y) in [
                (p.x - px, cy),
                (p.right(), cy),
                (cx, p.y - px),
                (cx, p.bottom()),
                (p.x - px, p.y - px),
                (p.right(), p.bottom()),
            ] {
                assert_eq!(
                    shot.at(x, y),
                    theme.hairline,
                    "{kind:?} {scale}x at {x}, {y}"
                );
            }
            // The rim stands out from a black page and from the canvas.
            assert!(
                theme.hairline.contrast(Color::rgb(0, 0, 0)) >= 1.5,
                "{kind:?}"
            );
            assert_ne!(theme.hairline, theme.canvas);
        }
    }

    /// A screen that does not fit at one physical pixel per pixel (HiRes interlace in the
    /// smallest window) is drawn whole, from its first pixel to its last.
    #[test]
    fn a_screen_larger_than_the_viewport_is_drawn_whole() {
        let (ui, palette) = (UiState::default(), Palette::dpaint_default_32());
        // 640 x 512: red at the left edge, green at the right, blue rows at top and bottom.
        let mut rgba = vec![0u8; 640 * 512 * 4];
        for y in 0..512usize {
            for x in 0..640usize {
                let at = (y * 640 + x) * 4;
                let colour = match (x, y) {
                    (_, 0..=1) | (_, 510..) => [0, 0, 255],
                    (0..=1, _) => [255, 0, 0],
                    (638.., _) => [0, 255, 0],
                    _ => [90, 90, 90],
                };
                rgba[at..at + 3].copy_from_slice(&colour);
                rgba[at + 3] = 255;
            }
        }
        let frame = Frame {
            size: crate::layout::MIN_WINDOW,
            scale: 1.0,
            theme: Theme::dark(),
            model: Model::new(&ui, &palette),
            picture: Image {
                width: 640,
                height: 512,
                rgba: &rgba,
            },
            per_ui_pixel: 2,
        };
        let l = frame.layout();
        assert!(l.shown < 1.0 && l.viewport.encloses(&l.picture), "{l:?}");
        let (w, _, out) = render_window(&frame, &mut Renderer::new());
        let shot = Shot {
            w,
            scale: 1.0,
            rgba: out,
        };
        let p = l.picture;
        let (cx, cy) = p.center();
        assert_eq!(shot.at(p.x, cy), Color::rgb(255, 0, 0), "left edge");
        assert_eq!(
            shot.at(p.right() - 1.0, cy),
            Color::rgb(0, 255, 0),
            "right edge"
        );
        assert_eq!(shot.at(cx, p.y), Color::rgb(0, 0, 255), "top edge");
        assert_eq!(
            shot.at(cx, p.bottom() - 1.0),
            Color::rgb(0, 0, 255),
            "bottom"
        );
        assert_eq!(shot.at(cx, cy), Color::rgb(90, 90, 90));
        assert_eq!(shot.at(p.x - 1.0, cy), Theme::dark().hairline);
    }

    #[test]
    fn the_picture_is_enlarged_in_exact_blocks_on_the_canvas() {
        let (ui, palette) = (UiState::default(), Palette::dpaint_default_32());
        let source = picture();
        for (kind, scale) in CASES {
            let theme = Theme::of(kind);
            let (shot, l) = shoot(&ui, &palette, kind, scale);
            let zoom = l.zoom;
            assert_eq!(zoom, (3.0 * scale) as u32);
            let (x0, y0) = (
                (l.picture.x * scale).round() as u32,
                (l.picture.y * scale).round() as u32,
            );
            // Every physical pixel of a few rows and columns of blocks.
            for sy in [0u32, 1, 100, 255] {
                for sx in [0u32, 1, 7, 160, 319] {
                    let i = ((sy * 320 + sx) * 4) as usize;
                    let want = Color::rgb(source[i], source[i + 1], source[i + 2]);
                    for dy in 0..zoom {
                        for dx in 0..zoom {
                            let (px, py) = (x0 + sx * zoom + dx, y0 + sy * zoom + dy);
                            let at = ((py * shot.w + px) * 4) as usize;
                            assert_eq!(
                                &shot.rgba[at..at + 3],
                                &[want.r, want.g, want.b],
                                "picture ({sx}, {sy}) at +({dx}, {dy}), {kind:?} {scale}x"
                            );
                        }
                    }
                }
            }
            // Away from the picture and its shadow: canvas.
            assert_eq!(shot.at(20.0, 400.0), theme.canvas, "{kind:?} {scale}x");
        }
    }

    #[test]
    fn hover_and_hidden_chrome_reach_the_pixels() {
        let palette = Palette::dpaint_default_32();
        let rgba = picture();
        let theme = Theme::dark();
        let render = |ui: &UiState, hover: Option<Hit>| {
            let frame = Frame {
                size: (1280.0, 800.0),
                scale: 1.0,
                theme,
                model: Model {
                    hover,
                    ..Model::new(ui, &palette)
                },
                picture: Image {
                    width: 320,
                    height: 256,
                    rgba: &rgba,
                },
                per_ui_pixel: 1,
            };
            let (w, _, out) = render_window(&frame, &mut Renderer::new());
            (
                Shot {
                    w,
                    scale: 1.0,
                    rgba: out,
                },
                frame.layout(),
            )
        };
        let ui = UiState::default();
        let (shot, l) = render(
            &ui,
            Some(Hit::Tool {
                tool: Tool::Text,
                filled_half: false,
            }),
        );
        let r = l.tool_button(Tool::Text).expect("button");
        assert_eq!(shot.at(r.x + 4.5, r.y + 9.5), theme.hover);
        // F10: no bar, no dock; the canvas reaches the window's corners.
        let bare = UiState {
            title_visible: false,
            toolbox_visible: false,
            ..UiState::default()
        };
        let (shot, l) = render(&bare, None);
        assert!(l.bar.is_none() && l.dock.is_none());
        assert_eq!(shot.at(2.0, 2.0), theme.canvas);
        assert_eq!(shot.at(1277.0, 797.0), theme.canvas);
    }

    fn panel(names: &[(&str, bool)], active: usize) -> dpx_ui::LayerPanel {
        let mut panel = dpx_ui::LayerPanel::default();
        let rows = names
            .iter()
            .map(|&(name, visible)| dpx_ui::LayerRow {
                name: name.into(),
                visible,
            })
            .collect();
        panel.update(rows, active, [true, true, true, false, true, true], 100);
        panel
    }

    /// The layer column and the scrim reach the pixels: the active row's card, the accent
    /// at its edge, a button's ground, and every part of the chrome darker under a requester
    /// while the picture keeps its colours.
    #[test]
    fn the_layer_column_and_the_scrim_reach_the_pixels() {
        let palette = Palette::dpaint_default_32();
        let rgba = picture();
        for (kind, scale) in [(ThemeKind::Dark, 1.0), (ThemeKind::Light, 2.0)] {
            let theme = Theme::of(kind);
            let ui = UiState {
                layer_panel: Some(panel(&[("Background", true), ("Sky", true)], 1)),
                ..UiState::default()
            };
            let render = |dimmed: bool| {
                let frame = Frame {
                    size: (1280.0, 800.0),
                    scale,
                    theme,
                    model: Model {
                        dimmed,
                        ..Model::new(&ui, &palette)
                    },
                    picture: Image {
                        width: 320,
                        height: 256,
                        rgba: &rgba,
                    },
                    per_ui_pixel: 1,
                };
                let (w, _, out) = render_window(&frame, &mut Renderer::new());
                (
                    Shot {
                        w,
                        scale,
                        rgba: out,
                    },
                    frame.layout(),
                )
            };
            let (shot, l) = render(false);
            let column = l.layers.as_ref().expect("the layer column");
            let dock = l.dock.as_ref().expect("dock");
            assert_eq!(column.rect.right(), dock.rect.x);
            assert_eq!(l.viewport.right(), column.rect.x);
            // Row 0 is the top layer, the active one: a raised card with the accent bar.
            let row = column.row(0);
            assert_eq!(
                shot.at(row.right() - 12.0, row.y + row.h / 2.0),
                theme.raised
            );
            assert_eq!(shot.at(row.x + 5.5, row.y + row.h / 2.0), theme.accent);
            // Row 1 is not: the panel shows.
            let below = column.row(1);
            assert_eq!(
                shot.at(below.right() - 12.0, below.y + below.h / 2.0),
                theme.panel
            );
            let new = column.buttons[0];
            assert_eq!(shot.at(new.x + 3.0, new.y + new.h / 2.0), theme.raised);
            assert_eq!(shot.at(column.header.right() - 40.0, 35.0), theme.panel);

            // Under a requester: everything but the picture is veiled.
            let (dim, _) = render(true);
            let veiled = |c: Color| {
                let a = u32::from(theme.scrim.a);
                let ch = |under: u8, over: u8| {
                    ((u32::from(over) * a + u32::from(under) * (255 - a) + 127) / 255) as u8
                };
                Color::rgb(
                    ch(c.r, theme.scrim.r),
                    ch(c.g, theme.scrim.g),
                    ch(c.b, theme.scrim.b),
                )
            };
            let near = |a: Color, b: Color| {
                a.r.abs_diff(b.r) <= 1 && a.g.abs_diff(b.g) <= 1 && a.b.abs_diff(b.b) <= 1
            };
            let bar = l.bar.expect("bar");
            for (what, x, y, under) in [
                ("bar", bar.right() - 3.0, bar.y + 3.0, theme.panel),
                (
                    "dock",
                    dock.rect.x + 3.0,
                    dock.rect.bottom() - 3.0,
                    theme.panel,
                ),
                ("column", column.header.right() - 40.0, 35.0, theme.panel),
                (
                    "active row",
                    row.right() - 12.0,
                    row.y + row.h / 2.0,
                    theme.raised,
                ),
                (
                    "canvas",
                    l.viewport.x + 3.0,
                    l.viewport.y + 3.0,
                    theme.canvas,
                ),
            ] {
                assert_eq!(shot.at(x, y), under, "{what}, {kind:?}");
                let got = dim.at(x, y);
                assert!(
                    near(got, veiled(under)) && got != under,
                    "{what}, {kind:?}: {got:?} is not {under:?} under the scrim"
                );
            }
            let (cx, cy) = l.picture.center();
            assert_eq!(dim.at(cx, cy), shot.at(cx, cy), "the picture is not veiled");
        }
    }
}
