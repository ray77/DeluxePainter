//! The toolbox settings requesters (ADR-20): right-click on Grid, Symmetry, Fill, Airbrush
//! and the line, curve, outline and freehand tools opens Gridding, Symmetry, Fill Type or
//! Spacing; Effect ▸ Stencil ▸ Make.. opens Make Stencil. The requesters are modal. Ok
//! stores the settings in [`crate::tools::ToolState`] (or the stencil in `PainterApp`), where
//! the tools read them for the next gesture.

use dpx_core::Document;
use dpx_ui::{
    FillRequester, GridRequester, ReqEvent, SpacingRequester, StencilRequester, SymmetryRequester,
    Tool, ToolReqResponse, ToolRequester,
};
use eframe::egui;

use crate::{PainterApp, View};

/// Which requester a toolbox button opens on right-click; `None` for buttons without one.
pub fn requester_for(tool: Tool) -> Option<fn(&PainterApp) -> ToolRequester> {
    Some(match tool {
        Tool::Grid => |a| ToolRequester::Grid(GridRequester::new(a.tools.grid_size)),
        Tool::Symmetry => {
            |a| ToolRequester::Symmetry(SymmetryRequester::new(a.tools.symmetry_settings))
        }
        Tool::Fill => |a| ToolRequester::Fill(FillRequester::new(a.tools.fill)),
        Tool::DottedFreehand
        | Tool::ContinuousFreehand
        | Tool::Line
        | Tool::Curve
        | Tool::Airbrush
        | Tool::Rectangle
        | Tool::Circle
        | Tool::Ellipse
        | Tool::Polygon => |a| ToolRequester::Spacing(SpacingRequester::new(a.tools.spacing)),
        _ => return None,
    })
}

/// Requester for a snapshot option (`--grid-requester`, `--symmetry-requester`,
/// `--spacing-requester`, `--fill-requester`, `--stencil-requester [--protect 1,2,3]`).
pub fn snapshot_requester(args: &[String], doc: &Document) -> Option<ToolRequester> {
    let app = PainterApp::new();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    if has("--grid-requester") {
        return requester_for(Tool::Grid).map(|f| f(&app));
    }
    if has("--symmetry-requester") {
        return requester_for(Tool::Symmetry).map(|f| f(&app));
    }
    if has("--spacing-requester") {
        return requester_for(Tool::Line).map(|f| f(&app));
    }
    if has("--fill-requester") {
        return requester_for(Tool::Fill).map(|f| f(&app));
    }
    if has("--stencil-requester") {
        let protect = args
            .iter()
            .position(|a| a == "--protect")
            .and_then(|i| args.get(i + 1))
            .map(|list| {
                let mut v = vec![false; 256];
                for c in list
                    .split(',')
                    .filter_map(|c| c.trim().parse::<usize>().ok())
                {
                    if c < 256 {
                        v[c] = true;
                    }
                }
                v
            });
        return Some(ToolRequester::Stencil(StencilRequester::new(
            doc.palette().len(),
            protect.as_deref(),
        )));
    }
    if has("--arrange-requester") {
        // `--moves FROM:TO,...` drags swatches first, e.g. `--moves 9:0,31:2`.
        let mut r = dpx_ui::ArrangeRequester::new(doc.palette().len());
        let moves = args
            .iter()
            .position(|a| a == "--moves")
            .and_then(|i| args.get(i + 1));
        for m in moves.iter().flat_map(|m| m.split(',')) {
            let Some((a, b)) = m.split_once(':') else {
                continue;
            };
            if let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>())
                && a < doc.palette().len()
                && b < doc.palette().len()
            {
                let (from, to) = (r.cell_rect(a), r.cell_rect(b));
                r.handle(ReqEvent::Press(from.x + 1, from.y + 1));
                r.handle(ReqEvent::Drag(to.x + 1, to.y + 1));
                r.handle(ReqEvent::Release);
            }
        }
        return Some(ToolRequester::Arrange(r));
    }
    // ADR-37: Effect > Perspective > Settings.. for a 32x24 brush.
    if has("--perspective-requester") {
        return Some(ToolRequester::Perspective(
            dpx_ui::PerspectiveRequester::new(
                dpx_ui::PerspectiveSettings::defaults((32, 24)),
                Some((32, 24)),
            ),
        ));
    }
    if has("--translucency-requester") {
        return Some(ToolRequester::Translucency(
            dpx_ui::TranslucencyRequester::new(50),
        ));
    }
    None
}

impl PainterApp {
    /// Opens the requester of `tool` (right-click on its button, or its key). Returns whether
    /// the tool has one.
    pub(crate) fn open_tool_requester(&mut self, tool: Tool) -> bool {
        let Some(make) = requester_for(tool) else {
            return false;
        };
        self.finish_stroke();
        self.ui.tool_requester = Some(make(self));
        self.dirty = true;
        true
    }

    /// Effect ▸ Stencil ▸ Make..: the palette to protect colours in, starting from the
    /// stencil there is (on or off).
    fn open_stencil_requester(&mut self) {
        self.finish_stroke();
        let current = self.stencil.as_ref().or(self.tools.stencil_off.as_ref());
        self.ui.tool_requester = Some(ToolRequester::Stencil(StencilRequester::new(
            self.palette().len(),
            current.map(Vec::as_slice),
        )));
        self.dirty = true;
    }

    /// The Effect ▸ Stencil menu commands. The stencil is colour based: `stencil[c]` protects
    /// colour `c` of the page. On/Off keeps the colours; Free forgets them.
    pub(crate) fn stencil_command(&mut self, id: &str) {
        let colors = self.palette().len();
        match id {
            "stencil.make" => self.open_stencil_requester(),
            // The stencil is by colour, so newly painted pixels of a locked colour are
            // protected already; Remake brings back the last made stencil and cuts it to
            // the palette in use.
            "stencil.remake" => {
                if let Some(mut s) = self
                    .stencil
                    .take()
                    .or_else(|| self.tools.stencil_off.take())
                {
                    s[colors.min(256)..].fill(false);
                    self.stencil = Some(s);
                }
            }
            "stencil.reverse" => {
                if let Some(mut s) = self
                    .stencil
                    .take()
                    .or_else(|| self.tools.stencil_off.take())
                {
                    for p in &mut s[..colors.min(256)] {
                        *p = !*p;
                    }
                    self.stencil = Some(s);
                }
            }
            "stencil.toggle" => match self.stencil.take() {
                Some(s) => self.tools.stencil_off = Some(s),
                None => self.stencil = self.tools.stencil_off.take(),
            },
            "stencil.free" => {
                self.stencil = None;
                self.tools.stencil_off = None;
            }
            _ => {}
        }
        self.dirty = true;
    }

    /// Pointer and keys while a toolbox settings requester is open (it is modal).
    pub(crate) fn tool_requester_input(
        &mut self,
        ctx: &egui::Context,
        origin: egui::Pos2,
        ppp: f32,
        scale: f32,
        view: View,
    ) {
        let Some(req) = &self.ui.tool_requester else {
            return;
        };
        let ui_screen = view.ui_screen();
        let at = req.origin(ui_screen.width, ui_screen.height);
        let (events, _) = self.modal_events(ctx, origin, ppp, scale, view, at);
        for event in events {
            if self.ui.tool_requester.is_none() {
                break;
            }
            self.tool_requester_event(event);
        }
    }

    /// Feeds one event to the open requester; Ok stores its settings, Cancel drops them.
    pub(crate) fn tool_requester_event(&mut self, event: ReqEvent) {
        let Some(req) = &mut self.ui.tool_requester else {
            return;
        };
        let response = req.handle(event);
        self.dirty = true;
        match response {
            ToolReqResponse::Continue => {}
            ToolReqResponse::Cancel => self.ui.tool_requester = None,
            ToolReqResponse::Ok => {
                let Some(req) = self.ui.tool_requester.take() else {
                    return;
                };
                match req {
                    ToolRequester::Grid(r) => self.tools.grid_size = r.settings(),
                    ToolRequester::Symmetry(r) => self.tools.symmetry_settings = r.settings(),
                    ToolRequester::Spacing(r) => self.tools.spacing = r.settings(),
                    ToolRequester::Fill(r) => self.tools.fill = r.settings(),
                    ToolRequester::Arrange(r) => self.apply_arrange(r.order()),
                    ToolRequester::Translucency(r) => self.set_translucency(&r),
                    ToolRequester::Perspective(r) => self.apply_perspective_settings(&r),
                    ToolRequester::Stencil(r) => {
                        // No colour protected is no stencil.
                        self.tools.stencil_off = None;
                        self.stencil = (r.count() > 0).then(|| r.protected());
                    }
                    ToolRequester::AnimBrush(r) => self.animbrush_requester_ok(r),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{FillType, GradientDirection, IndexedImage};
    use dpx_iff::ColorRange;
    use dpx_ui::{Hit, ReqKey, SpacingMode};

    fn press_tool(app: &mut PainterApp, tool: Tool, right: bool) {
        app.handle_press(
            Hit::Tool {
                tool,
                filled_half: false,
            },
            right,
        );
    }

    fn key(app: &mut PainterApp, k: ReqKey) {
        app.tool_requester_event(ReqEvent::Key(k));
    }

    fn type_text(app: &mut PainterApp, s: &str) {
        for c in s.chars() {
            key(app, ReqKey::Char(c));
        }
    }

    fn click(app: &mut PainterApp, x: i32, y: i32) {
        app.tool_requester_event(ReqEvent::Press(x, y));
        app.tool_requester_event(ReqEvent::Release);
    }

    fn drag_line(app: &mut PainterApp, from: (i32, i32), to: (i32, i32)) {
        app.handle_press(
            Hit::Page {
                x: from.0,
                y: from.1,
            },
            false,
        );
        app.handle_drag(to.0, to.1);
        app.release();
    }

    fn count(app: &PainterApp, color: u8) -> usize {
        app.page().pixels().iter().filter(|&&p| p == color).count()
    }

    #[test]
    fn right_click_opens_the_matching_requester_and_cancel_keeps_the_settings() {
        for (tool, want) in [
            (Tool::Grid, "grid"),
            (Tool::Symmetry, "symmetry"),
            (Tool::Fill, "fill"),
            (Tool::Line, "spacing"),
            (Tool::Airbrush, "spacing"),
            (Tool::DottedFreehand, "spacing"),
        ] {
            let mut app = PainterApp::new();
            press_tool(&mut app, tool, true);
            let kind = match app.ui.tool_requester {
                Some(ToolRequester::Grid(_)) => "grid",
                Some(ToolRequester::Symmetry(_)) => "symmetry",
                Some(ToolRequester::Fill(_)) => "fill",
                Some(ToolRequester::Spacing(_)) => "spacing",
                _ => "none",
            };
            assert_eq!(kind, want, "{tool:?}");
            assert!(
                !app.tools.grid && !app.tools.symmetry,
                "right-click never toggles"
            );
            key(&mut app, ReqKey::Escape);
            assert!(app.ui.tool_requester.is_none());
        }
        // Buttons without a requester do nothing special.
        let mut app = PainterApp::new();
        press_tool(&mut app, Tool::Magnify, true);
        assert!(app.ui.tool_requester.is_none());
    }

    #[test]
    fn gridding_changes_what_the_tools_snap_to() {
        let mut app = PainterApp::new();
        app.select_tool(Tool::Line, false);
        app.tools.grid = true;
        press_tool(&mut app, Tool::Grid, true);
        // X-spacing field is active: 16 -> 10; Tab; Y: 16 -> 5.
        key(&mut app, ReqKey::Backspace);
        key(&mut app, ReqKey::Backspace);
        type_text(&mut app, "10");
        key(&mut app, ReqKey::Tab);
        key(&mut app, ReqKey::Backspace);
        key(&mut app, ReqKey::Backspace);
        type_text(&mut app, "5");
        key(&mut app, ReqKey::Enter);
        assert!(app.ui.tool_requester.is_none());
        assert_eq!((app.tools.grid_size.x, app.tools.grid_size.y), (10, 5));
        assert!(app.tools.grid, "Ok leaves the toggle as it was");
        assert_eq!(app.snap((14, 12)), (10, 10));
        assert_eq!(app.snap((16, 13)), (20, 15));
        app.ui.fg = 2;
        drag_line(&mut app, (14, 12), (47, 38));
        assert_eq!(
            app.page().get(10, 10),
            Some(2),
            "line starts on a grid point"
        );
        assert_eq!(app.page().get(50, 40), Some(2), "and ends on one");
        assert_eq!(app.page().get(14, 12), Some(0));
    }

    #[test]
    fn mirror_symmetry_of_order_two_draws_the_left_right_twin() {
        let mut app = PainterApp::new();
        app.select_tool(Tool::DottedFreehand, false);
        app.ui.fg = 2;
        press_tool(&mut app, Tool::Symmetry, true);
        // Mirror radio, order 2.
        click(&mut app, 86, 31);
        key(&mut app, ReqKey::Backspace);
        type_text(&mut app, "2");
        key(&mut app, ReqKey::Enter);
        assert!(app.tools.symmetry_settings.mirror);
        assert_eq!(app.tools.symmetry_settings.order, 2);
        app.tools.symmetry = true;
        let sym = app.symmetry().expect("symmetry on");
        assert!(sym.mirror);
        let (cx, _) = sym.center;
        app.handle_press(Hit::Page { x: cx + 30, y: 50 }, false);
        app.release();
        assert_eq!(app.page().get(cx + 30, 50), Some(2));
        assert_eq!(
            app.page().get(cx - 30, 50),
            Some(2),
            "mirrored about the centre"
        );
        assert_eq!(app.page().get(cx - 30, 60), Some(0));
        assert_eq!(count(&app, 2), 2, "one brush pixel and its twin");
    }

    #[test]
    fn spacing_every_nth_pixel_stamps_a_dotted_line() {
        let mut app = PainterApp::new();
        app.select_tool(Tool::Line, false);
        app.ui.fg = 2;
        app.tools.spacing.mode = SpacingMode::EveryN;
        app.tools.spacing.n = 4;
        drag_line(&mut app, (10, 10), (50, 10));
        // Pixels 10, 14, 18 ... 50: 11 stamps, one command.
        assert_eq!(count(&app, 2), 11);
        for x in (10..=50).step_by(4) {
            assert_eq!(app.page().get(x, 10), Some(2), "x {x}");
        }
        assert_eq!(app.page().get(12, 10), Some(0));
        assert_eq!(app.engine.document().revision(), 1);
    }

    #[test]
    fn spacing_n_total_stamps_exactly_n_dots_including_both_ends() {
        let mut app = PainterApp::new();
        app.select_tool(Tool::Line, false);
        app.ui.fg = 2;
        app.tools.spacing.mode = SpacingMode::NTotal;
        app.tools.spacing.n = 5;
        drag_line(&mut app, (10, 20), (90, 20));
        assert_eq!(count(&app, 2), 5);
        for x in [10, 30, 50, 70, 90] {
            assert_eq!(app.page().get(x, 20), Some(2), "x {x}");
        }
    }

    #[test]
    fn spacing_applies_to_freehand_while_drawing_and_to_unfilled_shapes() {
        let mut app = PainterApp::new();
        app.select_tool(Tool::ContinuousFreehand, false);
        app.ui.fg = 2;
        app.tools.spacing.mode = SpacingMode::EveryN;
        app.tools.spacing.n = 3;
        app.handle_press(Hit::Page { x: 10, y: 30 }, false);
        for x in 11..=40 {
            app.handle_drag(x, 30);
        }
        let live = count(&app, 2);
        app.release();
        assert_eq!(count(&app, 2), live, "the preview is the committed stroke");
        assert_eq!(live, 11, "x = 10, 13 ... 40");
        assert_eq!(app.engine.document().revision(), 1);
        // An unfilled rectangle is dotted too; a filled one stays solid.
        let mut app = PainterApp::new();
        app.select_tool(Tool::Rectangle, false);
        app.ui.fg = 2;
        app.tools.spacing.mode = SpacingMode::EveryN;
        app.tools.spacing.n = 5;
        drag_line(&mut app, (10, 10), (30, 30));
        let dotted = count(&app, 2);
        assert!(dotted > 8 && dotted < 20, "dotted outline, got {dotted}");
        let mut app = PainterApp::new();
        app.select_tool(Tool::Rectangle, true);
        app.ui.fg = 2;
        app.tools.spacing.mode = SpacingMode::EveryN;
        app.tools.spacing.n = 5;
        drag_line(&mut app, (10, 10), (30, 30));
        assert_eq!(count(&app, 2), 21 * 21);
    }

    #[test]
    fn airbrush_spacing_sprays_inside_the_radius_and_is_repeatable() {
        let run = || {
            let mut app = PainterApp::new();
            app.select_tool(Tool::Line, false);
            app.ui.fg = 2;
            app.tools.spacing.mode = SpacingMode::Airbrush;
            app.tools.spacing.n = 3;
            app.tools.spacing.airbrush_radius = 4;
            drag_line(&mut app, (100, 100), (140, 100));
            app
        };
        let a = run();
        assert_eq!(a.page(), run().page(), "same sprays every time");
        let n = count(&a, 2);
        assert!(n > 40, "sprayed {n} pixels");
        for y in 0..a.page().height() as i32 {
            for x in 0..a.page().width() as i32 {
                if a.page().get(x, y) == Some(2) {
                    assert!(
                        (96..=144).contains(&x) && (96..=104).contains(&y),
                        "({x},{y}) is out of reach"
                    );
                }
            }
        }
    }

    #[test]
    fn airbrush_radius_comes_from_the_requester() {
        let mut app = PainterApp::new();
        press_tool(&mut app, Tool::Airbrush, true);
        // Airbrush size is the fourth field: Tab to it three times from the first.
        for _ in 0..3 {
            key(&mut app, ReqKey::Tab);
        }
        key(&mut app, ReqKey::Backspace);
        key(&mut app, ReqKey::Backspace);
        type_text(&mut app, "3");
        key(&mut app, ReqKey::Enter);
        assert_eq!(app.tools.spacing.airbrush_radius, 3);
        app.select_tool(Tool::Airbrush, false);
        app.ui.fg = 2;
        app.handle_press(Hit::Page { x: 100, y: 100 }, false);
        for _ in 0..30 {
            app.spray(4);
        }
        app.release();
        let mut far = 0;
        for y in 0..256 {
            for x in 0..320 {
                if app.page().get(x, y) == Some(2) && (x - 100).abs().max((y - 100).abs()) > 3 {
                    far += 1;
                }
            }
        }
        assert_eq!(far, 0, "all sprays within radius 3");
        assert!(count(&app, 2) > 10);
    }

    #[test]
    fn gradient_fill_uses_the_range_that_holds_fg_else_fg_to_bg() {
        let mut app = PainterApp::new();
        app.select_tool(Tool::Fill, false);
        app.ui.fg = 4;
        app.ui.bg = 1;
        press_tool(&mut app, Tool::Fill, true);
        // Vertical radio, then Ok.
        click(&mut app, 12, 31);
        key(&mut app, ReqKey::Enter);
        assert_eq!(app.tools.fill.gradient, Some(GradientDirection::Vertical));
        // No range: FG (4) to BG (1).
        assert_eq!(
            app.fill_type(),
            FillType::Gradient {
                direction: GradientDirection::Vertical,
                from: 4,
                to: 1,
                dither: false
            }
        );
        // A picture's colour range holding FG wins.
        app.extras.ranges = vec![ColorRange {
            rate: 0,
            flags: 1,
            low: 3,
            high: 6,
        }];
        let FillType::Gradient { from, to, .. } = app.fill_type() else {
            panic!()
        };
        assert_eq!((from, to), (3, 6));
        // One click on the empty page fills it top to bottom in the four range colours.
        app.handle_press(Hit::Page { x: 5, y: 5 }, false);
        assert_eq!(app.engine.document().revision(), 1, "one command");
        let (h, w) = (app.page().height() as i32, app.page().width() as i32);
        assert_eq!(app.page().get(0, 0), Some(3));
        assert_eq!(app.page().get(w - 1, h - 1), Some(6));
        assert_eq!(app.page().get(0, h / 2 - 1), Some(4));
        // Undo restores the page.
        app.undo();
        assert_eq!(app.page(), &IndexedImage::new(w as u32, h as u32, 0));
        // Back to Solid.
        app.tools.fill.gradient = None;
        assert_eq!(app.fill_type(), FillType::Solid);
    }

    #[test]
    fn fill_keys_open_the_requesters() {
        let mut app = PainterApp::new();
        app.tool_key(crate::tools::ToolKey::Char('F'));
        assert!(matches!(
            app.ui.tool_requester,
            Some(ToolRequester::Fill(_))
        ));
        key(&mut app, ReqKey::Escape);
        app.tool_key(crate::tools::ToolKey::Char('V'));
        assert!(matches!(
            app.ui.tool_requester,
            Some(ToolRequester::Spacing(_))
        ));
    }

    #[test]
    fn stencil_make_toggle_reverse_remake_free() {
        let mut app = PainterApp::new();
        let colors = app.palette().len();
        app.stencil_command("stencil.make");
        let Some(ToolRequester::Stencil(r)) = &app.ui.tool_requester else {
            panic!("requester open")
        };
        let cell = |c| r.cell_rect(c);
        let (c2, c5) = (cell(2), cell(5));
        // Make with nothing protected is no stencil.
        key(&mut app, ReqKey::Enter);
        assert_eq!(app.stencil, None);
        app.stencil_command("stencil.make");
        click(&mut app, c2.x + 1, c2.y + 1);
        click(&mut app, c5.x + 1, c5.y + 1);
        key(&mut app, ReqKey::Enter);
        let s = app.stencil.clone().expect("stencil made");
        assert_eq!(s.len(), 256);
        assert!(s[2] && s[5] && !s[3]);
        // On/Off keeps the colours.
        app.stencil_command("stencil.toggle");
        assert_eq!(app.stencil, None);
        app.stencil_command("stencil.toggle");
        assert_eq!(app.stencil.as_deref(), Some(s.as_slice()));
        // Reverse flips the palette's colours only.
        app.stencil_command("stencil.reverse");
        let r = app.stencil.clone().unwrap();
        assert!(!r[2] && !r[5] && r[3] && r[0]);
        assert!(!r[colors], "beyond the palette stays unprotected");
        // Make starts from the current stencil; Cancel changes nothing.
        app.stencil_command("stencil.make");
        key(&mut app, ReqKey::Escape);
        assert_eq!(app.stencil.as_deref(), Some(r.as_slice()));
        // Remake brings a switched-off stencil back.
        app.stencil_command("stencil.toggle");
        app.stencil_command("stencil.remake");
        assert_eq!(app.stencil.as_deref(), Some(r.as_slice()));
        // Free forgets it, even a switched-off one.
        app.stencil_command("stencil.toggle");
        app.stencil_command("stencil.free");
        app.stencil_command("stencil.toggle");
        assert_eq!(app.stencil, None);
    }

    #[test]
    fn stencil_menu_ids_are_enabled_in_the_menu() {
        use dpx_ui::menu::{Action, IMPLEMENTED};
        for id in [
            "stencil.make",
            "stencil.remake",
            "stencil.reverse",
            "stencil.toggle",
            "stencil.free",
        ] {
            assert!(IMPLEMENTED.contains(&id), "{id}");
            assert!(Action::Command(id).enabled(&PainterApp::new().ui.menu_context()));
        }
        assert!(
            !IMPLEMENTED.contains(&"stencil.paint"),
            "Paint is still open (ADR-31)"
        );
    }

    #[test]
    fn requesters_are_modal_for_the_painting_state() {
        let mut app = PainterApp::new();
        press_tool(&mut app, Tool::Fill, true);
        assert!(app.ui.tool_requester.is_some());
        // The screen shows the requester.
        let with = app.compose_screen();
        key(&mut app, ReqKey::Escape);
        assert_ne!(app.compose_screen(), with);
    }
}
