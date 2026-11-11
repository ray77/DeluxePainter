//! Tooltips (ADR-41): a one-line box naming the toolbox button, or the Animation Control Panel
//! button, under the resting pointer, with the paint-screen key that does the same. DeluxePaint
//! IV has no tooltips; this is a DeluxePainter addition, drawn in UI (LoRes) pixels like the
//! rest of the interface: face-coloured, a 1-pixel dark border, square corners.
//!
//! This module holds the texts, where the box goes and how it looks. When it shows is the
//! app's business (`apps/deluxepainter/src/tooltip.rs`), which also checks every key named
//! here against its key handlers, so the texts cannot drift from what the keys do.

use dpx_core::{IndexedImage, Rect};

use crate::anim_panel::{self, PanelButton, PanelHit};
use crate::font::{self, Font};
use crate::pens::Pens;
use crate::screen::{Hit, Screen, UiState};
use crate::toolbox::Tool;

/// Box geometry (UI pixels).
pub mod geom {
    /// Blank columns between the border and the text, left and right.
    pub const PAD_X: i32 = 2;
    /// Text top below the box top: the border and one blank row.
    pub const TEXT_DY: i32 = 2;
    /// Box height: border, blank row, the 8 glyph rows, blank row, border.
    pub const BOX_H: i32 = super::font::GLYPH_H + 4;
    /// Free pixels between the box and the toolbox (tools) or the panel (panel buttons).
    pub const GAP: i32 = 2;
}

/// What a tooltip names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipTarget {
    /// A toolbox button; for split buttons the half under the pointer (`filled` = the
    /// lower-right half). Always `false` for the other tools.
    Tool { tool: Tool, filled: bool },
    /// A button of the Animation Control Panel (not its slider).
    Panel(PanelButton),
}

impl TooltipTarget {
    /// The target under `hit`, if it has a tooltip: toolbox tool buttons and the panel's
    /// buttons. Brushes, palette, colour indicator, title bar, page and the slider have none.
    pub fn of_hit(hit: Hit) -> Option<Self> {
        match hit {
            Hit::Tool { tool, filled_half } => Some(Self::Tool {
                tool,
                filled: filled_half && tool.is_split(),
            }),
            Hit::AnimPanel(PanelHit::Button(b)) => Some(Self::Panel(b)),
            _ => None,
        }
    }

    /// The tooltip text: the name, then two spaces and the key, if there is one.
    pub fn text(self) -> String {
        let (name, key) = match self {
            Self::Tool { tool, filled } => (tool_name(tool, filled), tool_key(tool, filled)),
            Self::Panel(b) => (panel_name(b), panel_key(b)),
        };
        match key {
            Some(key) => format!("{name}  {key}"),
            None => name.to_string(),
        }
    }
}

/// The text naming a toolbox button (half), e.g. "Filled Rectangle  R".
pub fn for_tool(tool: Tool, filled_half: bool) -> String {
    TooltipTarget::Tool {
        tool,
        filled: filled_half && tool.is_split(),
    }
    .text()
}

/// The text naming a Control Panel button, e.g. "Next Frame  2".
pub fn for_panel_button(button: PanelButton) -> String {
    TooltipTarget::Panel(button).text()
}

/// The name of a toolbox button; split buttons name the half (`filled` = lower-right).
pub fn tool_name(tool: Tool, filled: bool) -> &'static str {
    match (tool, filled && tool.is_split()) {
        (Tool::ContinuousFreehand, true) => "Filled Freehand",
        (Tool::Rectangle, true) => "Filled Rectangle",
        (Tool::Circle, true) => "Filled Circle",
        (Tool::Ellipse, true) => "Filled Ellipse",
        (Tool::Polygon, true) => "Filled Polygon",
        (tool, _) => tool.name(),
    }
}

/// The paint-screen key that does what a click on the button (half) does, as the tooltip
/// shows it: a letter as typed (capitals with Shift), `Shift+`, `Ctrl+`, `Alt+` (Option on
/// macOS) before a key. The app tests press every one of these and compare with the click.
pub fn tool_key(tool: Tool, filled: bool) -> Option<&'static str> {
    Some(match (tool, filled && tool.is_split()) {
        (Tool::DottedFreehand, _) => "s",
        (Tool::ContinuousFreehand, false) => "d",
        (Tool::ContinuousFreehand, true) => "D",
        (Tool::Line, _) => "v",
        (Tool::Curve, _) => "q",
        (Tool::Fill, _) => "f",
        (Tool::Rectangle, false) => "r",
        (Tool::Rectangle, true) => "R",
        (Tool::Circle, false) => "c",
        (Tool::Circle, true) => "C",
        (Tool::Ellipse, false) => "e",
        (Tool::Ellipse, true) => "E",
        (Tool::BrushSelect, _) => "b",
        (Tool::Text, _) => "t",
        (Tool::Grid, _) => "g",
        (Tool::Symmetry, _) => "/",
        (Tool::Magnify, _) => "m",
        (Tool::Zoom, _) => ">",
        (Tool::Undo, _) => "u",
        (Tool::Clear, _) => "K",
        // No key selects them.
        (Tool::Airbrush | Tool::Polygon, _) => return None,
    })
}

/// The name of a Control Panel button (our own words, after what the button does).
pub fn panel_name(button: PanelButton) -> &'static str {
    match button {
        PanelButton::First => "First Frame",
        PanelButton::Last => "Last Frame",
        PanelButton::LoopBackward => "Loop Backward",
        PanelButton::OnceBackward => "Play Once Backward",
        PanelButton::Previous => "Previous Frame",
        PanelButton::Next => "Next Frame",
        PanelButton::OnceForward => "Play Once",
        PanelButton::LoopForward => "Loop",
        PanelButton::PingPong => "Ping-Pong",
        PanelButton::Add => "Add Frame",
        PanelButton::Delete => "Delete Frame",
        // Goes to the frame of the last Go To (the Go to Frame requester before any).
        PanelButton::GoTo => "Repeat Go To",
        PanelButton::LightTable => "Light Table",
        PanelButton::TwoBack => "Light Table: Two Back",
        PanelButton::LightPrevious => "Light Table: Previous",
        PanelButton::LightNext => "Light Table: Next",
        PanelButton::Spare => "Light Table: Spare",
    }
}

/// The paint-screen key that does what a click on a Control Panel button does (see
/// [`tool_key`] for the notation).
pub fn panel_key(button: PanelButton) -> Option<&'static str> {
    Some(match button {
        PanelButton::First => "Shift+1",
        PanelButton::Last => "Shift+2",
        PanelButton::LoopBackward => "Shift+4",
        PanelButton::OnceBackward => "Shift+5",
        PanelButton::Previous => "1",
        PanelButton::Next => "2",
        PanelButton::OnceForward => "5",
        PanelButton::LoopForward => "4",
        PanelButton::PingPong => "6",
        PanelButton::Add => "Alt+=",
        PanelButton::GoTo => "Shift+3",
        PanelButton::LightTable => "l",
        PanelButton::TwoBack => "Ctrl+1",
        PanelButton::LightPrevious => "Ctrl+2",
        PanelButton::LightNext => "Ctrl+3",
        PanelButton::Spare => "Ctrl+4",
        // No key deletes a single frame.
        PanelButton::Delete => return None,
    })
}

/// A tooltip as shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tooltip {
    pub target: TooltipTarget,
    pub text: String,
}

impl Tooltip {
    pub fn new(target: TooltipTarget) -> Self {
        Self {
            target,
            text: target.text(),
        }
    }

    /// Box size (UI pixels) for the text.
    pub fn size(&self) -> (i32, i32) {
        let w = Font::builtin().text_width(&self.text) + 2 * (1 + geom::PAD_X);
        (w, geom::BOX_H)
    }

    /// The button the tooltip names on a `screen_w`×`screen_h` UI screen, if it is shown
    /// (`ui`: the toolbox for tools, the Control Panel for panel buttons).
    pub fn button(&self, screen_w: i32, screen_h: i32, ui: &UiState) -> Option<Rect> {
        match self.target {
            TooltipTarget::Tool { tool, .. } => ui
                .toolbox_visible
                .then(|| Screen::new(screen_w, screen_h).tool_button(tool)),
            TooltipTarget::Panel(b) => ui
                .anim_panel
                .map(|_| anim_panel::Layout::new(screen_w, screen_h, ui.toolbox_visible).button(b)),
        }
    }

    /// Where the box goes on a `screen_w`×`screen_h` UI screen; `None` when its button is
    /// not shown. Tools: left of the toolbox (2 pixels free), top at the button's top. Panel
    /// buttons: above the panel (2 pixels free), left edge at the button's left edge, kept
    /// left of the toolbox. Always inside the screen.
    pub fn layout(&self, screen_w: i32, screen_h: i32, ui: &UiState) -> Option<Rect> {
        let button = self.button(screen_w, screen_h, ui)?;
        let (w, h) = self.size();
        // A text wider than the screen is cut (none of ours is).
        let (w, h) = (w.min(screen_w), h.min(screen_h));
        let (x, y, right) = match self.target {
            TooltipTarget::Tool { .. } => {
                let toolbox_x = Screen::new(screen_w, screen_h).toolbox_x();
                (toolbox_x - geom::GAP - w, button.y, screen_w)
            }
            TooltipTarget::Panel(_) => {
                let panel = anim_panel::Layout::new(screen_w, screen_h, ui.toolbox_visible).panel;
                (button.x, panel.y - geom::GAP - h, panel.x + panel.w)
            }
        };
        let x = x.min(right - w).min(screen_w - w).max(0);
        let y = y.min(screen_h - h).max(0);
        Some(Rect::new(x, y, w, h))
    }

    /// Draws the box on a `screen_w`×`screen_h` UI screen (nothing when its button is not
    /// shown).
    pub fn draw(
        &self,
        img: &mut IndexedImage,
        pens: &Pens,
        screen_w: i32,
        screen_h: i32,
        ui: &UiState,
    ) {
        let Some(r) = self.layout(screen_w, screen_h, ui) else {
            return;
        };
        img.fill_rect(r.x, r.y, r.w, r.h, pens.face);
        // Square corners: the border lines run through the corner pixels.
        let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
        img.hline(r.x, x1, r.y, pens.dark);
        img.hline(r.x, x1, y1, pens.dark);
        img.vline(r.x, r.y, y1, pens.dark);
        img.vline(x1, r.y, y1, pens.dark);
        // The text, up to the blank columns before the right border.
        let font = Font::builtin();
        let limit = x1 - geom::PAD_X;
        let mut x = r.x + 1 + geom::PAD_X;
        for c in self.text.chars() {
            if x + font.char_advance(c) - font::GAP > limit {
                break;
            }
            x = font.draw_char(img, x, r.y + geom::TEXT_DY, c, pens.dark);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim_panel::{AnimPanel, CONTROLS, TOGGLES};
    use crate::toolbox::TOOL_ROWS;
    use dpx_core::Palette;

    fn all_tools() -> impl Iterator<Item = Tool> {
        TOOL_ROWS.iter().flatten().copied()
    }

    fn all_buttons() -> impl Iterator<Item = PanelButton> {
        [PanelButton::First, PanelButton::Last]
            .into_iter()
            .chain(CONTROLS)
            .chain(TOGGLES)
    }

    /// Every target: each tool (both halves of split buttons) and each panel button.
    fn all_targets() -> Vec<TooltipTarget> {
        let mut targets = Vec::new();
        for tool in all_tools() {
            targets.push(TooltipTarget::Tool {
                tool,
                filled: false,
            });
            if tool.is_split() {
                targets.push(TooltipTarget::Tool { tool, filled: true });
            }
        }
        targets.extend(all_buttons().map(TooltipTarget::Panel));
        targets
    }

    fn panel_ui(toolbox: bool) -> UiState {
        UiState {
            toolbox_visible: toolbox,
            anim_panel: Some(AnimPanel {
                count: 4,
                ..AnimPanel::default()
            }),
            ..UiState::default()
        }
    }

    #[test]
    fn every_tool_names_itself_and_both_halves_of_split_buttons() {
        let expected = [
            (Tool::DottedFreehand, false, "Dotted Freehand  s"),
            (Tool::ContinuousFreehand, false, "Continuous Freehand  d"),
            (Tool::ContinuousFreehand, true, "Filled Freehand  D"),
            (Tool::Line, false, "Straight Line  v"),
            (Tool::Curve, false, "Curve  q"),
            (Tool::Fill, false, "Fill  f"),
            (Tool::Airbrush, false, "Airbrush"),
            (Tool::Rectangle, false, "Rectangle  r"),
            (Tool::Rectangle, true, "Filled Rectangle  R"),
            (Tool::Circle, false, "Circle  c"),
            (Tool::Circle, true, "Filled Circle  C"),
            (Tool::Ellipse, false, "Ellipse  e"),
            (Tool::Ellipse, true, "Filled Ellipse  E"),
            (Tool::Polygon, false, "Polygon"),
            (Tool::Polygon, true, "Filled Polygon"),
            (Tool::BrushSelect, false, "Brush Selector  b"),
            (Tool::Text, false, "Text  t"),
            (Tool::Grid, false, "Grid  g"),
            (Tool::Symmetry, false, "Symmetry  /"),
            (Tool::Magnify, false, "Magnify  m"),
            (Tool::Zoom, false, "Zoom  >"),
            (Tool::Undo, false, "Undo  u"),
            (Tool::Clear, false, "Clear  K"),
        ];
        for (tool, filled, text) in expected {
            assert_eq!(for_tool(tool, filled), text, "{tool:?} filled {filled}");
        }
        // Every target is in the table; unsplit buttons have one text for the whole button.
        let tools: Vec<_> = all_targets()
            .into_iter()
            .filter(|t| matches!(t, TooltipTarget::Tool { .. }))
            .collect();
        assert_eq!(tools.len(), expected.len());
        for tool in all_tools().filter(|t| !t.is_split()) {
            assert_eq!(for_tool(tool, true), for_tool(tool, false), "{tool:?}");
        }
    }

    #[test]
    fn every_panel_button_names_itself() {
        let expected = [
            (PanelButton::First, "First Frame  Shift+1"),
            (PanelButton::Last, "Last Frame  Shift+2"),
            (PanelButton::LoopBackward, "Loop Backward  Shift+4"),
            (PanelButton::OnceBackward, "Play Once Backward  Shift+5"),
            (PanelButton::Previous, "Previous Frame  1"),
            (PanelButton::Next, "Next Frame  2"),
            (PanelButton::OnceForward, "Play Once  5"),
            (PanelButton::LoopForward, "Loop  4"),
            (PanelButton::PingPong, "Ping-Pong  6"),
            (PanelButton::Add, "Add Frame  Alt+="),
            (PanelButton::Delete, "Delete Frame"),
            (PanelButton::GoTo, "Repeat Go To  Shift+3"),
            (PanelButton::LightTable, "Light Table  l"),
            (PanelButton::TwoBack, "Light Table: Two Back  Ctrl+1"),
            (PanelButton::LightPrevious, "Light Table: Previous  Ctrl+2"),
            (PanelButton::LightNext, "Light Table: Next  Ctrl+3"),
            (PanelButton::Spare, "Light Table: Spare  Ctrl+4"),
        ];
        for (b, text) in expected {
            assert_eq!(for_panel_button(b), text, "{b:?}");
        }
        assert_eq!(all_buttons().count(), expected.len());
        // Only characters the font has.
        for t in all_targets() {
            assert!(t.text().chars().all(|c| (' '..='~').contains(&c)), "{t:?}");
        }
    }

    #[test]
    fn targets_come_from_tool_buttons_halves_and_panel_buttons_only() {
        let screen = Screen::new(320, 256);
        let ui = panel_ui(true);
        for tool in all_tools() {
            let b = screen.tool_button(tool);
            // Icon pixel (0, 0) is in the upper-left half, (8, 8) in the lower-right one.
            let upper = screen.hit_test(32, &ui, b.x + 1, b.y + 1);
            let lower = screen.hit_test(32, &ui, b.x + 9, b.y + 9);
            assert_eq!(
                TooltipTarget::of_hit(upper),
                Some(TooltipTarget::Tool {
                    tool,
                    filled: false
                })
            );
            assert_eq!(
                TooltipTarget::of_hit(lower),
                Some(TooltipTarget::Tool {
                    tool,
                    filled: tool.is_split()
                }),
                "{tool:?}"
            );
        }
        let layout = anim_panel::Layout::new(320, 256, true);
        for b in all_buttons() {
            let r = layout.button(b);
            let hit = screen.hit_test(32, &ui, r.x + r.w / 2, r.y + r.h / 2);
            assert_eq!(TooltipTarget::of_hit(hit), Some(TooltipTarget::Panel(b)));
        }
        // No tooltip elsewhere.
        let none = [
            Hit::TitleBar,
            Hit::Brush(0),
            Hit::Indicator,
            Hit::PaletteCell(3),
            Hit::ToolboxBackground,
            Hit::Page { x: 5, y: 5 },
            Hit::AnimPanel(PanelHit::Step(1)),
            Hit::AnimPanel(PanelHit::Knob(2)),
            Hit::AnimPanel(PanelHit::Background),
        ];
        for hit in none {
            assert_eq!(TooltipTarget::of_hit(hit), None, "{hit:?}");
        }
    }

    /// The UI screens the app can show: the smallest (320×200), PAL and NTSC LoRes and the
    /// largest overscan sizes (every display mode has LoRes UI geometry, 320 wide or more).
    const SCREENS: [(i32, i32); 5] = [(320, 200), (320, 256), (368, 241), (368, 283), (352, 272)];

    #[test]
    fn every_box_stays_inside_the_screen_beside_its_button() {
        for (w, h) in SCREENS {
            for toolbox in [true, false] {
                let ui = panel_ui(toolbox);
                let screen = Screen::new(w, h);
                let panel = anim_panel::Layout::new(w, h, toolbox).panel;
                for target in all_targets() {
                    let tip = Tooltip::new(target);
                    let at = format!("{target:?} on {w}x{h}, toolbox {toolbox}");
                    let Some(r) = tip.layout(w, h, &ui) else {
                        assert!(
                            !toolbox && matches!(target, TooltipTarget::Tool { .. }),
                            "{at}"
                        );
                        continue;
                    };
                    let button = tip.button(w, h, &ui).unwrap();
                    assert!(r.x >= 0 && r.y >= 0, "{at}: {r:?}");
                    assert!(r.x + r.w <= w && r.y + r.h <= h, "{at}: {r:?}");
                    assert!(button.intersect(r).is_empty(), "{at}: covers its button");
                    match target {
                        TooltipTarget::Tool { .. } => {
                            assert_eq!(r.x + r.w, screen.toolbox_x() - geom::GAP, "{at}");
                            assert_eq!(r.y, button.y, "{at}: top at the button's top");
                        }
                        TooltipTarget::Panel(_) => {
                            assert_eq!(r.y + r.h, panel.y - geom::GAP, "{at}");
                            assert!(r.x + r.w <= panel.x + panel.w, "{at}: off the toolbox");
                            if button.x + r.w <= panel.x + panel.w {
                                assert_eq!(r.x, button.x, "{at}");
                            }
                        }
                    }
                }
            }
        }
        // Long texts are pushed back inside the screen too; one wider than the screen is cut.
        let ui = panel_ui(true);
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        for n in [30, 60] {
            for target in all_targets() {
                let tip = Tooltip {
                    target,
                    text: "M".repeat(n),
                };
                let (w, h) = (320, 200);
                let r = tip.layout(w, h, &ui).unwrap();
                assert!(r.w > 200, "a long text");
                assert!(
                    r.x >= 0 && r.x + r.w <= w && r.y >= 0 && r.y + r.h <= h,
                    "{r:?}"
                );
                // The right border and the two blank columns before it stay.
                let mut img = IndexedImage::new(w as u32, h as u32, 9);
                tip.draw(&mut img, &pens, w, h, &ui);
                let x1 = r.x + r.w - 1;
                for y in r.y + 1..r.y + r.h - 1 {
                    assert_eq!(img.get(x1, y), Some(pens.dark));
                    assert_eq!(
                        img.get(x1 - 1, y),
                        Some(pens.face),
                        "n {n} ({}, {y})",
                        x1 - 1
                    );
                    assert_eq!(img.get(x1 - 2, y), Some(pens.face));
                }
            }
        }
    }

    #[test]
    fn the_box_has_a_dark_border_square_corners_and_dark_text() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let ui = UiState::default();
        let tip = Tooltip::new(TooltipTarget::Tool {
            tool: Tool::Line,
            filled: false,
        });
        let paper = 9;
        let mut img = IndexedImage::new(320, 256, paper);
        tip.draw(&mut img, &pens, 320, 256, &ui);
        let r = tip.layout(320, 256, &ui).unwrap();
        assert_eq!((r.w, r.h), tip.size());
        assert_eq!(r.h, 12, "compact: one text line");
        let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
        // Square corners: the corner pixels are border, the ones just inside are face, and
        // nothing is drawn outside the box.
        for (cx, cy, ix, iy) in [
            (r.x, r.y, r.x + 1, r.y + 1),
            (x1, r.y, x1 - 1, r.y + 1),
            (r.x, y1, r.x + 1, y1 - 1),
            (x1, y1, x1 - 1, y1 - 1),
        ] {
            assert_eq!(img.get(cx, cy), Some(pens.dark), "corner ({cx}, {cy})");
            assert_eq!(img.get(ix, iy), Some(pens.face), "inside ({ix}, {iy})");
        }
        for x in r.x - 1..=x1 + 1 {
            assert_eq!(img.get(x, r.y - 1), Some(paper));
            assert_eq!(img.get(x, y1 + 1), Some(paper));
        }
        for y in r.y..=y1 {
            assert_eq!(img.get(r.x - 1, y), Some(paper));
            assert_eq!(img.get(x1 + 1, y), Some(paper));
            assert_eq!(img.get(r.x, y), Some(pens.dark), "left border");
            assert_eq!(img.get(x1, y), Some(pens.dark), "right border");
        }
        // Inside: exactly the text as the font draws it, on face, 2 pixels from the border.
        let mut text = IndexedImage::new(320, 256, pens.face);
        let end = Font::builtin().draw(&mut text, r.x + 3, r.y + 2, &tip.text, pens.dark);
        assert_eq!(end - font::GAP, x1 - 2, "2 blank columns right of the text");
        for y in r.y + 1..y1 {
            for x in r.x + 1..x1 {
                assert_eq!(img.get(x, y), text.get(x, y), "({x}, {y})");
            }
        }
        // The text starts in the third column inside the border.
        assert!((r.y + 2..r.y + 10).any(|y| img.get(r.x + 3, y) == Some(pens.dark)));
        assert!((r.y + 1..y1).all(|y| img.get(r.x + 2, y) == Some(pens.face)));
    }

    #[test]
    fn the_box_is_drawn_last_over_page_toolbox_and_panel() {
        let pal = Palette::dpaint_default_32();
        let pens = Pens::from_palette(&pal);
        let page = IndexedImage::new(320, 200, 5);
        let screen = Screen::new(320, 200);
        let rect = TooltipTarget::Tool {
            tool: Tool::Rectangle,
            filled: true,
        };
        // A text as wide as the screen lies over the toolbox and the title-free page.
        let wide = Tooltip {
            target: TooltipTarget::Tool {
                tool: Tool::Line,
                filled: false,
            },
            text: "M".repeat(60),
        };
        for tip in [
            Tooltip::new(rect),
            Tooltip::new(TooltipTarget::Panel(PanelButton::Spare)),
            Tooltip::new(TooltipTarget::Panel(PanelButton::First)),
            wide,
        ] {
            let target = tip.target;
            let mut ui = panel_ui(true);
            let without = screen.render(&page, &pal, &ui);
            ui.tooltip = Some(tip);
            let with = screen.render(&page, &pal, &ui);
            let r = ui.tooltip.as_ref().unwrap().layout(320, 200, &ui).unwrap();
            let mut alone = without.clone();
            ui.tooltip
                .as_ref()
                .unwrap()
                .draw(&mut alone, &pens, 320, 200, &ui);
            assert_eq!(with, alone, "{target:?}: the box over the finished screen");
            assert_ne!(with, without);
            // Outside the box nothing changed.
            for y in 0..200 {
                for x in 0..320 {
                    if !r.contains(x, y) {
                        assert_eq!(with.get(x, y), without.get(x, y));
                    }
                }
            }
            // Scaled screens (HiRes laced) enlarge it like the rest of the UI.
            let big = IndexedImage::new(640, 400, 5);
            let hi = screen.render_scaled(&big, &pal, &ui, 2, 2);
            for (x, y) in [(r.x, r.y), (r.x + 1, r.y + 1), (r.x + 3, r.y + 5)] {
                for (dx, dy) in [(0, 0), (1, 1)] {
                    assert_eq!(hi.get(2 * x + dx, 2 * y + dy), with.get(x, y), "({x},{y})");
                }
            }
        }
    }

    #[test]
    fn nothing_is_drawn_for_a_button_that_is_not_shown() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let blank = IndexedImage::new(320, 256, 9);
        let check = |target, ui: &UiState| {
            let tip = Tooltip::new(target);
            let mut img = blank.clone();
            tip.draw(&mut img, &pens, 320, 256, ui);
            assert_eq!(tip.layout(320, 256, ui), None);
            assert_eq!(img, blank);
        };
        let line = TooltipTarget::Tool {
            tool: Tool::Line,
            filled: false,
        };
        // The toolbox hidden (F10): no tool tooltip; no panel: no panel tooltip.
        check(line, &panel_ui(false));
        check(TooltipTarget::Panel(PanelButton::Next), &UiState::default());
    }
}
