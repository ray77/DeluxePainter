//! Builds the display lists of the Modern main window from the shared interface state
//! (ADR-47): the canvas with the picture, the bar, the dock and what lies over them (the
//! menu that is held open, a tooltip). What the lists show is the
//! same state the Classic skin draws; only the look differs.

use dpx_core::Palette;
use dpx_ui::screen::APP_TITLE;
use dpx_ui::toolbox::TOOL_ROWS;
use dpx_ui::{Hit, Pref, Tool, Tooltip, TooltipTarget, UiState};

use crate::font;
use crate::icons::{self, Ink};
use crate::layout::{Dock, MainLayout};
use crate::menu::{self, TitleLook};
use crate::paint::{Color, List, Path, Rect, Weight};
use crate::theme::Theme;

/// Toolbox toggles that stay on while another tool is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Toggles {
    pub grid: bool,
    pub symmetry: bool,
    pub magnify: bool,
}

impl Toggles {
    fn on(self, tool: Tool) -> bool {
        match tool {
            Tool::Grid => self.grid,
            Tool::Symmetry => self.symmetry,
            Tool::Magnify => self.magnify,
            _ => false,
        }
    }
}

/// What the chrome shows.
#[derive(Clone, Copy, Debug)]
pub struct Model<'a> {
    pub ui: &'a UiState,
    /// The palette as displayed (colour cycling included).
    pub palette: &'a Palette,
    /// A custom brush is the brush: no built-in brush is shown selected.
    pub custom_brush: bool,
    pub toggles: Toggles,
    /// The part of the chrome under the pointer, shown raised.
    pub hover: Option<Hit>,
}

impl<'a> Model<'a> {
    /// The model of a resting interface: no toggle on, no pointer.
    pub fn new(ui: &'a UiState, palette: &'a Palette) -> Self {
        Model {
            ui,
            palette,
            custom_brush: false,
            toggles: Toggles::default(),
            hover: None,
        }
    }
}

/// Index of the picture in the images given to the rasteriser.
pub const PICTURE_IMAGE: usize = 0;

/// Corner radius of buttons and cards.
const RADIUS: f32 = 7.0;
/// Corner radius of small parts (brush cells, chips).
const RADIUS_SMALL: f32 = 5.0;
/// Corner radius of palette swatches.
const RADIUS_SWATCH: f32 = 3.0;

fn rgb(palette: &Palette, index: u8) -> Color {
    palette
        .get(index)
        .map_or(Color::rgb(0, 0, 0), |c| Color::rgb(c.r, c.g, c.b))
}

/// The whole window: canvas and picture, dock, bar, and over them the open menu or the
/// tooltip.
pub fn window(layout: &MainLayout, theme: &Theme, model: &Model<'_>) -> List {
    let mut list = canvas(layout, theme);
    list.append(dock(layout, theme, model));
    list.append(title_bar(layout, theme, model));
    if let Some((_, over)) = overlay(layout, theme, model) {
        list.append(over);
    }
    list
}

/// What lies over bar, dock and picture: the cards of the menu that is held open, or the
/// tooltip of the dock button the pointer rests on. With the rectangle the drawing covers
/// (shadows included, cut to the window); `None` when there is nothing to show.
pub fn overlay(layout: &MainLayout, theme: &Theme, model: &Model<'_>) -> Option<(Rect, List)> {
    let ui = model.ui;
    let mut list = List::new();
    let bounds = if ui.menu.active {
        menu::draw(&mut list, layout, theme, &ui.menu_context(), &ui.menu)
    } else {
        ui.tooltip
            .as_ref()
            .and_then(|tip| tooltip(&mut list, layout, theme, tip))
    }?;
    // Whole physical pixels, inside the window.
    let scale = layout.scale;
    let (x0, y0) = (
        ((bounds.x * scale).floor() / scale).max(0.0),
        ((bounds.y * scale).floor() / scale).max(0.0),
    );
    let (x1, y1) = (
        ((bounds.right() * scale).ceil() / scale).min(layout.window.right()),
        ((bounds.bottom() * scale).ceil() / scale).min(layout.window.bottom()),
    );
    (x1 > x0 && y1 > y0).then(|| (Rect::new(x0, y0, x1 - x0, y1 - y0), list))
}

/// Height of a tooltip and the space it keeps to the dock.
const TIP_H: f32 = 28.0;
const TIP_GAP: f32 = 8.0;
/// Free space left and right of a tooltip's content, and between name and key.
const TIP_PAD: f32 = 10.0;
const TIP_KEY_GAP: f32 = 9.0;

/// The rectangle of the tooltip of a dock button: left of the dock, centred on the button,
/// inside the window. `None` for a target the Modern skin does not draw itself (the
/// Animation Control Panel and the layer panel are still shown in the Classic look, with
/// their Classic tooltips) and while the dock is hidden.
pub fn tooltip_rect(layout: &MainLayout, tip: &Tooltip) -> Option<Rect> {
    let TooltipTarget::Tool { tool, .. } = tip.target else {
        return None;
    };
    let button = layout.tool_button(tool)?;
    let dock = layout.dock.as_ref()?;
    let (name, key) = tooltip_parts(tip);
    let size = font::SIZE_PANEL;
    let mut w = 2.0 * TIP_PAD + font::measure(name, size, Weight::Regular);
    if let Some(key) = key {
        w += TIP_KEY_GAP + key_chip_width(key);
    }
    let snap = |v: f32| (v * layout.scale).round() / layout.scale;
    let w = snap(w.ceil());
    let x = (dock.rect.x - TIP_GAP - w).max(0.0);
    let y = (button.center().1 - TIP_H / 2.0)
        .min(layout.window.bottom() - TIP_H - 4.0)
        .max(layout.bar.map_or(4.0, |b| b.bottom() + 4.0));
    Some(Rect::new(snap(x), snap(y), w, snap(TIP_H)))
}

/// The name a tooltip shows and, when the key does the button's job now, its key.
fn tooltip_parts(tip: &Tooltip) -> (&'static str, Option<&'static str>) {
    (
        tip.target.name(),
        tip.target.key().filter(|k| tip.keys.keeps(k)),
    )
}

fn key_chip_width(key: &str) -> f32 {
    (font::measure(key, font::SIZE_PANEL, Weight::Medium) + 10.0).max(18.0)
}

/// Draws the tooltip of a dock button: its name and, on a chip, its key. Returns the
/// rectangle the drawing covers.
fn tooltip(list: &mut List, layout: &MainLayout, theme: &Theme, tip: &Tooltip) -> Option<Rect> {
    let rect = tooltip_rect(layout, tip)?;
    let (name, key) = tooltip_parts(tip);
    menu::card_ground(list, theme, rect, RADIUS);
    let size = font::SIZE_PANEL;
    let mid = rect.y + rect.h / 2.0;
    list.text(
        rect.x + TIP_PAD,
        font::baseline_centered(mid, size, Weight::Regular),
        name,
        size,
        Weight::Regular,
        theme.text,
    );
    if let Some(key) = key {
        let w = key_chip_width(key);
        let chip = Rect::new(rect.right() - TIP_PAD - w, mid - 9.0, w, 18.0);
        list.round_rect(chip, RADIUS_SMALL, theme.panel);
        list.ring(chip, RADIUS_SMALL, theme.hairline, 1.0);
        let text_w = font::measure(key, size, Weight::Medium);
        list.text(
            chip.x + (chip.w - text_w) / 2.0,
            font::baseline_centered(mid, size, Weight::Medium),
            key,
            size,
            Weight::Medium,
            theme.text_dim,
        );
    }
    Some(rect.inset(-menu::SHADOW_REACH))
}

/// The picture viewport: canvas colour, a soft shadow around the picture and the picture
/// itself, clipped to the viewport.
pub fn canvas(layout: &MainLayout, theme: &Theme) -> List {
    let mut list = List::new();
    let view = layout.viewport;
    if view.w <= 0.0 || view.h <= 0.0 {
        return list;
    }
    list.rect(view, theme.canvas);
    list.clip(Path::new().round_rect(view, 0.0));
    // The shadow: three rings of falling strength, wider below than above.
    let step = Color {
        a: theme.shadow.a / 3,
        ..theme.shadow
    };
    for (grow, drop) in [(6.0, 3.0), (3.0, 2.0), (1.0, 1.0)] {
        let r = layout.picture.inset(-grow);
        list.round_rect(Rect::new(r.x, r.y + drop, r.w, r.h), grow + 1.0, step);
    }
    list.image(layout.picture, PICTURE_IMAGE);
    list.unclip();
    list
}

/// The bar: the menu titles at the left, the status after them (program name or frame
/// counter, paint mode, effect flags), and at the right the pointer coordinates or the
/// perspective readout and the active layer's name. The content is that of the classic title
/// bar.
pub fn title_bar(layout: &MainLayout, theme: &Theme, model: &Model<'_>) -> List {
    let mut list = List::new();
    let Some(bar) = layout.bar else {
        return list;
    };
    let ui = model.ui;
    list.rect(bar, theme.panel);
    list.rect(
        Rect::new(bar.x, bar.bottom() - 1.0, bar.w, 1.0),
        theme.hairline,
    );
    let (mid, size) = (bar.y + (bar.h - 1.0) / 2.0, font::SIZE_BAR);
    let base = font::baseline_centered(mid, size, Weight::Regular);

    // Menu titles: always visible, subdued; the pull-downs open while the right button is
    // held, as in DeluxePaint. Then the titles are readable and the open one is marked.
    let menu_ctx = ui.menu_context();
    for (i, (rect, entry)) in layout
        .menu_titles
        .iter()
        .zip(dpx_ui::menu::MENUS.iter())
        .enumerate()
    {
        let ink = match menu::title_look(&menu_ctx, &ui.menu, i) {
            TitleLook::Idle => theme.text_dim,
            TitleLook::Ready => theme.text,
            TitleLook::Ghosted => theme.disabled,
            TitleLook::Open => {
                list.round_rect(*rect, RADIUS_SMALL, theme.accent);
                theme.on_accent
            }
        };
        list.text(
            rect.x + crate::layout::TITLE_PAD,
            base,
            entry.title,
            size,
            Weight::Regular,
            ink,
        );
    }
    let mut x = layout.status_x();
    if let Some(last) = layout.menu_titles.last() {
        list.rect(
            Rect::new(last.right() + 6.0, bar.y + 8.0, 1.0, bar.h - 17.0),
            theme.hairline,
        );
    }

    // Status.
    let left = match ui.frame_counter {
        Some((current, count)) => format!("{current}/{count}"),
        None => APP_TITLE.to_string(),
    };
    list.text(x, base, left.as_str(), size, Weight::Medium, theme.text);
    x += font::measure(&left, size, Weight::Medium) + 10.0;
    // The paint mode in a chip.
    let mode = ui.mode.title();
    let chip_size = font::SIZE_PANEL;
    let chip_w = font::measure(mode, chip_size, Weight::Medium) + 14.0;
    let chip = Rect::new(x, mid - 9.5, chip_w, 19.0);
    list.round_rect(chip, RADIUS_SMALL, theme.raised);
    list.text(
        chip.x + 7.0,
        font::baseline_centered(mid, chip_size, Weight::Medium),
        mode,
        chip_size,
        Weight::Medium,
        theme.text,
    );
    x = chip.right() + 10.0;
    if !ui.flags.is_empty() {
        list.text(
            x,
            base,
            ui.flags.as_str(),
            size,
            Weight::Regular,
            theme.text_dim,
        );
        x += font::measure(&ui.flags, size, Weight::Regular) + 10.0;
    }
    let left_end = x;

    // Right side, from the window's edge inwards; what does not fit is left out.
    let right = match ui.coords.filter(|_| ui.prefs.get(Pref::Coords)) {
        Some((cx, cy)) => Some(format!("x {cx}   y {cy}")),
        None if ui.prefs.get(Pref::Coords) => None,
        None => ui.perspective.clone(),
    };
    let mut end = bar.right() - 14.0;
    if let Some(text) = right {
        let w = font::measure(&text, size, Weight::Regular);
        if end - w >= left_end {
            list.text(end - w, base, text, size, Weight::Regular, theme.text_dim);
            end -= w + 16.0;
        }
    }
    if let Some(name) = &ui.layer_label {
        let label = font::fit(
            &format!("Layer: {name}"),
            end - left_end,
            size,
            Weight::Regular,
        );
        if label.chars().count() > "Layer: …".chars().count() {
            let w = font::measure(&label, size, Weight::Regular);
            list.text(end - w, base, label, size, Weight::Regular, theme.text_dim);
        }
    }
    list
}

/// The dock: built-in brushes, the tools, the colour indicator and the palette.
pub fn dock(layout: &MainLayout, theme: &Theme, model: &Model<'_>) -> List {
    let mut list = List::new();
    let Some(dock) = &layout.dock else {
        return list;
    };
    list.rect(dock.rect, theme.panel);
    list.rect(
        Rect::new(dock.rect.x, dock.rect.y, 1.0, dock.rect.h),
        theme.hairline,
    );
    brushes(&mut list, dock, theme, model);
    for (i, rect) in dock.tools.iter().enumerate() {
        tool_button(&mut list, *rect, TOOL_ROWS[i / 2][i % 2], theme, model);
    }
    indicator(&mut list, dock, theme, model);
    palette(&mut list, dock, theme, model);
    list
}

fn brushes(list: &mut List, dock: &Dock, theme: &Theme, model: &Model<'_>) {
    list.round_rect(dock.brush_well, RADIUS, theme.raised);
    for (i, cell) in dock.brushes.iter().enumerate() {
        let selected = !model.custom_brush && model.ui.brush == i;
        let hovered = model.hover == Some(Hit::Brush(i));
        let ink = if selected {
            list.round_rect(*cell, RADIUS_SMALL, theme.accent);
            theme.on_accent
        } else {
            if hovered {
                list.round_rect(*cell, RADIUS_SMALL, theme.hover);
            }
            theme.text
        };
        let (cx, cy) = cell.center();
        if let Some(shape) = icons::brush(i, cx, cy, 1.5) {
            list.fill(shape, ink);
        }
    }
}

/// The two halves of a button: the upper-left and the lower-right triangle.
fn halves(rect: Rect) -> (Path, Path) {
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
    (
        Path::new().polygon(&[(x0, y0), (x1, y0), (x0, y1)]),
        Path::new().polygon(&[(x1, y0), (x1, y1), (x0, y1)]),
    )
}

/// The rectangle of the icon in a button.
fn icon_box(rect: Rect) -> Rect {
    let side = icons::GRID.min(rect.w - 8.0).min(rect.h - 6.0).max(4.0);
    let (cx, cy) = rect.center();
    Rect::new(cx - side / 2.0, cy - side / 2.0, side, side)
}

fn draw_icon(list: &mut List, rect: Rect, tool: Tool, ink: Color) {
    draw_icon_parts(list, rect, tool, ink, true);
}

/// Draws the icon of `tool`; without `fills`, only its stroked parts.
fn draw_icon_parts(list: &mut List, rect: Rect, tool: Tool, ink: Color, fills: bool) {
    let b = icon_box(rect);
    let scale = b.w / icons::GRID;
    for part in icons::tool(tool).parts {
        if part.ink == Ink::Fill && !fills {
            continue;
        }
        let path = part.path.placed(scale, b.x, b.y);
        match part.ink {
            Ink::Fill => list.fill(path, ink),
            Ink::Stroke => list.stroke(path, ink, icons::STROKE * scale),
        }
    }
}

fn tool_button(list: &mut List, rect: Rect, tool: Tool, theme: &Theme, model: &Model<'_>) {
    let ui = model.ui;
    let radius = RADIUS.min(rect.h / 4.0);
    let selected = ui.tool == tool && !tool.is_action();
    let hovered_half = match model.hover {
        Some(Hit::Tool {
            tool: t,
            filled_half,
        }) if t == tool => Some(filled_half),
        _ => None,
    };
    let toggled = model.toggles.on(tool) && !selected;

    list.round_rect(rect, radius, theme.raised);
    if tool.is_split() {
        let (upper, lower) = halves(rect);
        let half = |filled: bool| if filled { lower.clone() } else { upper.clone() };
        // The hovered half, unless it is the selected one.
        if let Some(filled) = hovered_half.filter(|&f| !(selected && f == ui.filled)) {
            list.clip(half(filled));
            list.round_rect(rect, radius, theme.hover);
            list.unclip();
        }
        if selected {
            // The icon in the text colour first, then the chosen half in the accent with its
            // part of the icon in the accent's ink. The filled part of the icon lies in the
            // lower-right half only and ends on the diagonal, so each pass leaves it out
            // where it does not belong: its edge must not shine through along the seam.
            draw_icon_parts(list, rect, tool, theme.text, !ui.filled);
            list.clip(half(ui.filled));
            list.round_rect(rect, radius, theme.accent);
            draw_icon_parts(list, rect, tool, theme.on_accent, ui.filled);
            list.unclip();
        } else {
            draw_icon(list, rect, tool, theme.text);
            // Ticks at the two ends of the dividing diagonal, clear of the icon.
            let (dx, dy) = (rect.w * 0.13, rect.h * 0.13);
            let inset = 3.0;
            list.line(
                (rect.right() - inset, rect.y + inset),
                (rect.right() - inset - dx, rect.y + inset + dy),
                theme.text_dim,
                1.0,
            );
            list.line(
                (rect.x + inset, rect.bottom() - inset),
                (rect.x + inset + dx, rect.bottom() - inset - dy),
                theme.text_dim,
                1.0,
            );
        }
        return;
    }
    if selected {
        list.round_rect(rect, radius, theme.accent);
        draw_icon(list, rect, tool, theme.on_accent);
    } else if toggled {
        // A toggle that is on while another tool is selected: tinted, with an accent ring.
        list.round_rect(rect, radius, theme.raised.mix(theme.accent, 16));
        list.ring(rect, radius, theme.accent, 1.5);
        draw_icon(list, rect, tool, theme.accent);
    } else {
        if hovered_half.is_some() {
            list.round_rect(rect, radius, theme.hover);
        }
        draw_icon(list, rect, tool, theme.text);
    }
}

/// The colour indicator: the background colour as a card, the foreground colour as a disc in
/// its middle, as in the DeluxePaint toolbox.
fn indicator(list: &mut List, dock: &Dock, theme: &Theme, model: &Model<'_>) {
    let r = dock.indicator;
    let (fg, bg) = (
        rgb(model.palette, model.ui.fg),
        rgb(model.palette, model.ui.bg),
    );
    list.round_rect(r, RADIUS, bg);
    list.ring(r, RADIUS, theme.hairline, 1.0);
    if model.hover == Some(Hit::Indicator) {
        list.ring(r, RADIUS, theme.text_dim, 1.0);
    }
    let (cx, cy) = r.center();
    let radius = (r.h / 2.0 - 5.0).max(2.0);
    // A thin rim keeps a foreground colour visible on an equal background.
    list.fill(Path::new().circle(cx, cy, radius + 1.0), theme.hairline);
    list.fill(Path::new().circle(cx, cy, radius), fg);
}

fn palette(list: &mut List, dock: &Dock, theme: &Theme, model: &Model<'_>) {
    for (i, cell) in dock.cells.iter().enumerate() {
        let index = i as u8;
        let colour = rgb(model.palette, index);
        // Cells touch; a swatch keeps a little air unless the cells are very low.
        let air = if cell.h >= 10.0 { 1.0 } else { 0.5 };
        let swatch = Rect::new(
            cell.x + air,
            cell.y + air,
            cell.w - 2.0 * air,
            cell.h - 2.0 * air,
        );
        let radius = RADIUS_SWATCH.min(swatch.h / 3.0);
        let ring = if index == model.ui.fg {
            Some((theme.accent, 2.0))
        } else if index == model.ui.bg {
            Some((theme.text_dim, 1.0))
        } else if model.hover == Some(Hit::PaletteCell(index)) {
            Some((theme.text, 1.0))
        } else {
            None
        };
        match ring.filter(|_| swatch.h >= 8.0) {
            Some((ring_colour, width)) => {
                // Ring, a line of panel colour, then the colour.
                list.round_rect(swatch, radius, ring_colour);
                list.round_rect(swatch.inset(width), (radius - width).max(0.5), theme.panel);
                list.round_rect(
                    swatch.inset(width + 1.0),
                    (radius - width - 1.0).max(0.5),
                    colour,
                );
            }
            None => {
                list.round_rect(swatch, radius, colour);
                // A fine rim, so that a colour close to the panel's keeps its shape.
                let rim = ring.map_or(theme.hairline, |(c, _)| c);
                if swatch.h >= 6.0 {
                    list.ring(swatch, radius, rim, 1.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hit::in_filled_half;
    use crate::layout::Chrome;
    use crate::paint::Op;
    use dpx_ui::{PaintMode, PixelSize};

    fn layout() -> MainLayout {
        MainLayout::new(
            (1280.0, 800.0),
            1.0,
            Chrome::ALL,
            32,
            (320, 256),
            PixelSize::default(),
        )
    }

    fn texts(list: &List) -> Vec<(String, f32, Color)> {
        list.ops()
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, x, color, .. } => Some((text.clone(), *x, *color)),
                _ => None,
            })
            .collect()
    }

    fn fills(list: &List, colour: Color) -> Vec<Rect> {
        list.ops()
            .iter()
            .filter_map(|op| match op {
                Op::RoundRect {
                    rect,
                    fill: Some(c),
                    ..
                } if *c == colour => Some(*rect),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_bar_shows_every_menu_title_then_the_status() {
        let (l, theme, palette) = (layout(), Theme::dark(), Palette::dpaint_default_32());
        let ui = UiState::default();
        let list = title_bar(&l, &theme, &Model::new(&ui, &palette));
        let texts = texts(&list);
        let shown: Vec<&str> = texts.iter().map(|t| t.0.as_str()).collect();
        assert_eq!(
            shown,
            [
                "Picture",
                "Brush",
                "Mode",
                "Anim",
                "Color",
                "Effect",
                "Prefs",
                "DeluxePainter",
                "Color"
            ]
        );
        // Left to right, nothing on top of anything else.
        for pair in texts.windows(2) {
            assert!(pair[0].1 < pair[1].1, "{pair:?}");
        }
        // Titles are subdued, the status is not.
        assert!(texts[..7].iter().all(|t| t.2 == theme.text_dim));
        assert_eq!(texts[7].2, theme.text);
        assert_eq!(texts[7].1, l.status_x());
    }

    #[test]
    fn the_status_follows_the_state_like_the_classic_title_bar() {
        let (l, theme, palette) = (layout(), Theme::light(), Palette::dpaint_default_32());
        let mut ui = UiState {
            mode: PaintMode::Smear,
            frame_counter: Some((3, 12)),
            flags: "AS".into(),
            layer_label: Some("Sky".into()),
            ..UiState::default()
        };
        ui.prefs.set(Pref::Coords, true);
        ui.coords = Some((123, 87));
        let list = title_bar(&l, &theme, &Model::new(&ui, &palette));
        let shown: Vec<String> = texts(&list).into_iter().skip(7).map(|t| t.0).collect();
        assert_eq!(shown, ["3/12", "Smear", "AS", "x 123   y 87", "Layer: Sky"]);
        // Coords on but the pointer off the page: no readout, and no perspective either.
        ui.coords = None;
        ui.perspective = Some("X 10".into());
        let list = title_bar(&l, &theme, &Model::new(&ui, &palette));
        assert!(
            !texts(&list)
                .iter()
                .any(|t| t.0.starts_with("X ") || t.0.starts_with("x "))
        );
        // Coords off: the perspective readout takes the place.
        ui.prefs.set(Pref::Coords, false);
        let list = title_bar(&l, &theme, &Model::new(&ui, &palette));
        assert!(texts(&list).iter().any(|t| t.0 == "X 10"));
    }

    #[test]
    fn a_long_layer_name_is_cut_and_everything_stays_in_the_bar() {
        let theme = Theme::dark();
        let palette = Palette::dpaint_default_32();
        let l = MainLayout::new(
            (720.0, 520.0),
            1.0,
            Chrome::ALL,
            32,
            (320, 256),
            PixelSize::default(),
        );
        let ui = UiState {
            layer_label: Some(
                "A very long layer name that cannot fit into a small bar at all".into(),
            ),
            ..UiState::default()
        };
        let list = title_bar(&l, &theme, &Model::new(&ui, &palette));
        let label = texts(&list)
            .into_iter()
            .find(|t| t.0.starts_with("Layer: "))
            .expect("a cut label");
        assert!(label.0.ends_with('…'), "{}", label.0);
        let end = label.1 + font::measure(&label.0, font::SIZE_BAR, Weight::Regular);
        assert!(end <= 720.0 - 13.9, "{end}");
    }

    #[test]
    fn no_bar_and_no_dock_build_nothing() {
        let theme = Theme::dark();
        let palette = Palette::dpaint_default_32();
        let ui = UiState::default();
        let bare = MainLayout::new(
            (1280.0, 800.0),
            1.0,
            Chrome {
                bar: false,
                dock: false,
            },
            32,
            (320, 256),
            PixelSize::default(),
        );
        let model = Model::new(&ui, &palette);
        assert!(title_bar(&bare, &theme, &model).is_empty());
        assert!(dock(&bare, &theme, &model).is_empty());
        let all = window(&bare, &theme, &model);
        assert!(all.ops().iter().any(|op| matches!(op, Op::Image { .. })));
    }

    #[test]
    fn the_selected_tool_carries_the_accent_and_actions_never_do() {
        let (l, theme, palette) = (layout(), Theme::dark(), Palette::dpaint_default_32());
        let d = l.dock.as_ref().expect("dock");
        for &tool in TOOL_ROWS.iter().flatten() {
            let ui = UiState {
                tool,
                brush: 3,
                ..UiState::default()
            };
            let list = dock(&l, &theme, &Model::new(&ui, &palette));
            // Accent surfaces among the buttons and brush cells (the foreground colour's
            // palette cell has an accent ring of its own).
            let accent: Vec<Rect> = fills(&list, theme.accent)
                .into_iter()
                .filter(|r| d.tools.contains(r) || d.brushes.contains(r))
                .collect();
            let button = l.tool_button(tool).expect("button");
            let mut want = vec![d.brushes[3]];
            if !tool.is_action() {
                want.push(button);
            }
            assert_eq!(accent, want, "{tool:?}");
        }
    }

    #[test]
    fn a_split_tool_shows_only_its_chosen_half_selected() {
        let (l, theme, palette) = (layout(), Theme::dark(), Palette::dpaint_default_32());
        let button = l.tool_button(Tool::Circle).expect("button");
        for filled in [false, true] {
            let ui = UiState {
                tool: Tool::Circle,
                filled,
                ..UiState::default()
            };
            let list = dock(&l, &theme, &Model::new(&ui, &palette));
            // The accent fill of the button comes right after a clip to one triangle.
            let at = list
                .ops()
                .iter()
                .position(|op| {
                    matches!(op, Op::RoundRect { rect, fill: Some(c), .. }
                        if *rect == button && *c == theme.accent)
                })
                .expect("accent fill");
            let Op::Clip(Some(clip)) = &list.ops()[at - 1] else {
                panic!("the accent half is clipped");
            };
            let b = clip.bounds().expect("triangle");
            assert_eq!(b, button, "the triangle spans the button");
            // The triangle's right-angle corner tells the half.
            let corner = if filled {
                (button.right(), button.bottom())
            } else {
                (button.x, button.y)
            };
            let has = clip.0.iter().any(|s| {
                matches!(s, crate::paint::Seg::Move(x, y) | crate::paint::Seg::Line(x, y)
                    if (*x, *y) == corner)
            });
            assert!(has, "filled {filled}: {clip:?}");
            // The hit test agrees: just inside that corner is that half.
            let inward = if filled { -1.0 } else { 1.0 };
            assert_eq!(
                in_filled_half(&button, corner.0 + inward, corner.1 + inward),
                filled
            );
        }
    }

    #[test]
    fn toggles_get_a_ring_and_a_custom_brush_deselects_the_built_in_ones() {
        let (l, theme, palette) = (layout(), Theme::dark(), Palette::dpaint_default_32());
        let ui = UiState::default();
        let rings = |model: &Model<'_>| -> Vec<Rect> {
            dock(&l, &theme, model)
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::RoundRect {
                        rect,
                        stroke: Some(s),
                        ..
                    } if s.color == theme.accent => Some(*rect),
                    _ => None,
                })
                .collect()
        };
        let mut model = Model::new(&ui, &palette);
        assert!(rings(&model).is_empty());
        model.toggles = Toggles {
            grid: true,
            symmetry: false,
            magnify: true,
        };
        assert_eq!(
            rings(&model),
            [
                l.tool_button(Tool::Grid).expect("grid"),
                l.tool_button(Tool::Magnify).expect("magnify")
            ]
        );
        // Built-in brush 0 is selected at the start; not while a custom brush is the brush.
        let d = l.dock.as_ref().expect("dock");
        assert!(fills(&dock(&l, &theme, &model), theme.accent).contains(&d.brushes[0]));
        model.custom_brush = true;
        assert!(!fills(&dock(&l, &theme, &model), theme.accent).contains(&d.brushes[0]));
    }

    #[test]
    fn hover_raises_exactly_the_part_under_the_pointer() {
        let (l, theme, palette) = (layout(), Theme::light(), Palette::dpaint_default_32());
        let ui = UiState::default();
        let d = l.dock.as_ref().expect("dock");
        let mut model = Model::new(&ui, &palette);
        assert!(fills(&dock(&l, &theme, &model), theme.hover).is_empty());
        model.hover = Some(Hit::Tool {
            tool: Tool::Line,
            filled_half: false,
        });
        assert_eq!(
            fills(&dock(&l, &theme, &model), theme.hover),
            [l.tool_button(Tool::Line).expect("line")]
        );
        model.hover = Some(Hit::Brush(4));
        assert_eq!(
            fills(&dock(&l, &theme, &model), theme.hover),
            [d.brushes[4]]
        );
        // The selected brush and the selected tool do not change under the pointer.
        model.hover = Some(Hit::Brush(0));
        assert!(fills(&dock(&l, &theme, &model), theme.hover).is_empty());
        let base = dock(&l, &theme, &Model::new(&ui, &palette)).fingerprint();
        model.hover = Some(Hit::PaletteCell(7));
        assert_ne!(dock(&l, &theme, &model).fingerprint(), base);
        model.hover = Some(Hit::ToolboxBackground);
        assert_eq!(dock(&l, &theme, &model).fingerprint(), base);
    }

    #[test]
    fn the_same_state_builds_the_same_list_and_the_theme_changes_it() {
        let (l, palette) = (layout(), Palette::dpaint_default_32());
        let ui = UiState::default();
        let model = Model::new(&ui, &palette);
        let dark = window(&l, &Theme::dark(), &model);
        assert_eq!(dark, window(&l, &Theme::dark(), &model));
        assert_eq!(
            dark.fingerprint(),
            window(&l, &Theme::dark(), &model).fingerprint()
        );
        assert_ne!(
            dark.fingerprint(),
            window(&l, &Theme::light(), &model).fingerprint()
        );
        // Exactly one picture, inside the clip of the viewport.
        let images: Vec<_> = dark
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Image { rect, image } => Some((*rect, *image)),
                _ => None,
            })
            .collect();
        assert_eq!(images, [(l.picture, PICTURE_IMAGE)]);
    }

    #[test]
    fn the_held_menu_marks_its_title_and_makes_the_others_readable() {
        let (l, theme, palette) = (layout(), Theme::dark(), Palette::dpaint_default_32());
        let mut ui = UiState::default();
        let (x, y) = l.menu_titles[2].center();
        menu::open(&l, &mut ui.menu, x, y);
        let list = title_bar(&l, &theme, &Model::new(&ui, &palette));
        let texts = texts(&list);
        assert_eq!(texts[2].0, "Mode");
        assert_eq!(texts[2].2, theme.on_accent);
        assert_eq!(fills(&list, theme.accent), [l.menu_titles[2]]);
        for i in [0, 1, 3, 4, 5, 6] {
            assert_eq!(texts[i].2, theme.text, "{}", texts[i].0);
        }
        // The status keeps its place and its look.
        assert_eq!(
            (texts[7].0.as_str(), texts[7].1),
            ("DeluxePainter", l.status_x())
        );
    }

    #[test]
    fn the_overlay_is_the_held_menu_or_the_tooltip_with_its_bounds() {
        use dpx_ui::{KeyMode, Tooltip, TooltipTarget};
        let (l, theme, palette) = (layout(), Theme::light(), Palette::dpaint_default_32());
        let mut ui = UiState::default();
        assert!(overlay(&l, &theme, &Model::new(&ui, &palette)).is_none());
        // The whole window has nothing more than its parts then.
        let model = Model::new(&ui, &palette);
        assert_eq!(
            window(&l, &theme, &model).len(),
            canvas(&l, &theme).len()
                + dock(&l, &theme, &model).len()
                + title_bar(&l, &theme, &model).len()
        );

        // A tooltip: name and key beside the dock, level with its button.
        let target = TooltipTarget::Tool {
            tool: Tool::Curve,
            filled: false,
        };
        ui.tooltip = Some(Tooltip::new(target, KeyMode::Paint));
        let tip = ui.tooltip.clone().expect("set");
        let rect = tooltip_rect(&l, &tip).expect("the dock is shown");
        let dock_rect = l.dock.as_ref().expect("dock").rect;
        let button = l.tool_button(Tool::Curve).expect("button");
        assert!(rect.right() <= dock_rect.x - 4.0 && rect.x >= 0.0);
        assert!((rect.center().1 - button.center().1).abs() <= 1.0);
        let (bounds, list) = overlay(&l, &theme, &Model::new(&ui, &palette)).expect("overlay");
        assert!(bounds.encloses(&rect) && l.window.encloses(&bounds));
        let shown: Vec<String> = texts(&list).into_iter().map(|t| t.0).collect();
        assert_eq!(shown, ["Curve", "q"]);
        for (text, x, _) in texts(&list) {
            assert!(x >= rect.x && x < rect.right(), "{text}");
        }
        // While text is typed the key would type: only the name, in a narrower card.
        ui.tooltip = Some(Tooltip::new(target, KeyMode::Typing));
        let typing = ui.tooltip.clone().expect("set");
        assert!(tooltip_rect(&l, &typing).expect("rect").w < rect.w);
        let (_, list) = overlay(&l, &theme, &Model::new(&ui, &palette)).expect("overlay");
        assert_eq!(texts(&list).len(), 1);
        // The last tool row in a low window: the card stays inside the window.
        let low = MainLayout::new(
            crate::layout::MIN_WINDOW,
            1.5,
            Chrome::ALL,
            64,
            (320, 256),
            PixelSize::default(),
        );
        for tool in [Tool::Undo, Tool::Clear, Tool::DottedFreehand] {
            let tip = Tooltip::new(
                TooltipTarget::Tool {
                    tool,
                    filled: false,
                },
                KeyMode::Paint,
            );
            let r = tooltip_rect(&low, &tip).expect("rect");
            assert!(low.window.encloses(&r), "{tool:?}: {r:?}");
            assert!(r.y >= low.bar.expect("bar").bottom());
        }
        // Without the dock (F10) and for a target the Classic look still draws: nothing.
        let bare = MainLayout::new(
            (1280.0, 800.0),
            1.0,
            Chrome {
                bar: true,
                dock: false,
            },
            32,
            (320, 256),
            PixelSize::default(),
        );
        assert!(tooltip_rect(&bare, &tip).is_none());
        let panel = Tooltip::new(
            TooltipTarget::Panel(dpx_ui::PanelButton::First),
            KeyMode::Paint,
        );
        assert!(tooltip_rect(&l, &panel).is_none());

        // The held menu takes the overlay; a tooltip is not drawn under it.
        let (x, y) = l.menu_titles[0].center();
        menu::open(&l, &mut ui.menu, x, y);
        let (bounds, list) = overlay(&l, &theme, &Model::new(&ui, &palette)).expect("overlay");
        let card = menu::menu_card(&l, 0).expect("card");
        assert!(bounds.encloses(&card.rect));
        let shown: Vec<String> = texts(&list).into_iter().map(|t| t.0).collect();
        assert!(shown.contains(&"Load..".to_string()) && !shown.contains(&"Curve".to_string()));
        // The window's list ends with it.
        let all = window(&l, &theme, &Model::new(&ui, &palette));
        assert_eq!(all.ops()[all.len() - list.len()..], *list.ops());
    }
}
