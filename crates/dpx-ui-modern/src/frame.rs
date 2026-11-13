//! A whole Modern main window as one image, for headless snapshots and tests. The live
//! window uses the same layout, lists and rasteriser.

use crate::build::{self, Model};
use crate::layout::{Chrome, MainLayout};
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
}

impl Frame<'_> {
    /// The layout of this frame: the bar and the dock follow the shared visibility flags
    /// (F9, F10), the picture's enlargement follows Prefs ▸ Pixel Size.
    pub fn layout(&self) -> MainLayout {
        let ui = self.model.ui;
        MainLayout::new(
            self.size,
            self.scale,
            Chrome {
                bar: ui.title_visible,
                dock: ui.toolbox_visible,
            },
            self.model.palette.len(),
            (self.picture.width, self.picture.height),
            ui.pixel_size,
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
}
