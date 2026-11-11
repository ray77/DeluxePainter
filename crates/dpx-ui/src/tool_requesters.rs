//! The toolbox settings requesters (ADR-20): Gridding, Symmetry, Spacing and Fill Type, plus
//! the Make Stencil requester, behind one [`ToolRequester`] so the UI state holds a single
//! optional modal requester. Each one edits a copy of the settings; the caller reads them
//! back with the `settings` methods after [`ToolReqResponse::Ok`].

use dpx_core::{GradientDirection, IndexedImage, Palette, Rect};

use crate::arrange_requester::ArrangeRequester;
use crate::form::{Form, FormResponse};
use crate::pens::Pens;
use crate::perspective_requester::PerspectiveRequester;
use crate::requester::ReqEvent;
use crate::stencil_requester::StencilRequester;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolReqResponse {
    Continue,
    Cancel,
    Ok,
}

impl From<FormResponse> for ToolReqResponse {
    fn from(r: FormResponse) -> Self {
        match r {
            FormResponse::Continue => ToolReqResponse::Continue,
            FormResponse::Cancel => ToolReqResponse::Cancel,
            FormResponse::Ok => ToolReqResponse::Ok,
        }
    }
}

/// Grid spacing limits in page pixels.
pub const GRID_RANGE: (i32, i32) = (1, 512);
/// Symmetry order limits (DeluxePaint allows up to 40 points).
pub const SYMMETRY_RANGE: (i32, i32) = (2, 40);
/// Spacing count limits (dots, pixels, sprays).
pub const SPACING_RANGE: (i32, i32) = (1, 999);
/// Airbrush radius limits in page pixels.
pub const AIRBRUSH_RANGE: (i32, i32) = (1, 99);

// ---------------------------------------------------------------- Gridding

/// Gridding: the distance between grid points in page pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSettings {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRequester {
    form: Form,
}

impl GridRequester {
    pub fn new(s: GridSettings) -> Self {
        let mut form = Form::new("Gridding", 168, 76);
        let x = Rect::new(76, 19, 36, 9);
        let y = Rect::new(76, 32, 36, 9);
        form.field("X-spacing", (8, 20), x, GRID_RANGE, s.x, None);
        form.field("Y-spacing", (8, 33), y, GRID_RANGE, s.y, None);
        form.text((118, 20), "pixels");
        form.text((118, 33), "pixels");
        Self { form }
    }

    pub fn settings(&self) -> GridSettings {
        GridSettings {
            x: self.form.number(0),
            y: self.form.number(1),
        }
    }
}

// ---------------------------------------------------------------- Symmetry

/// Symmetry: cyclic or mirror, and the number of points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymmetrySettings {
    pub mirror: bool,
    pub order: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymmetryRequester {
    form: Form,
}

impl SymmetryRequester {
    pub fn new(s: SymmetrySettings) -> Self {
        let mut form = Form::new("Symmetry", 168, 96);
        form.text((8, 17), "Point symmetry:");
        form.radio(0, 0, (8, 29), "Cyclic", !s.mirror);
        form.radio(0, 1, (84, 29), "Mirror", s.mirror);
        form.field(
            "Order",
            (8, 44),
            Rect::new(76, 43, 36, 9),
            SYMMETRY_RANGE,
            i32::from(s.order),
            None,
        );
        form.text((118, 44), "points");
        Self { form }
    }

    pub fn settings(&self) -> SymmetrySettings {
        SymmetrySettings {
            mirror: self.form.selected(0) == 1,
            order: self.form.number(0) as u8,
        }
    }
}

// ----------------------------------------------------------------- Spacing

/// How the line, curve, outline and freehand tools stamp the brush along their path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpacingMode {
    /// An unbroken line of stamps.
    #[default]
    Continuous,
    /// Exactly `n` stamps, evenly spread over the path.
    NTotal,
    /// A stamp every `n` pixels along the path.
    EveryN,
    /// `n` random sprays per pixel along the path, within the airbrush radius.
    Airbrush,
}

impl SpacingMode {
    fn index(self) -> u8 {
        match self {
            SpacingMode::Continuous => 0,
            SpacingMode::NTotal => 1,
            SpacingMode::EveryN => 2,
            SpacingMode::Airbrush => 3,
        }
    }

    fn from_index(i: u8) -> Self {
        match i {
            1 => SpacingMode::NTotal,
            2 => SpacingMode::EveryN,
            3 => SpacingMode::Airbrush,
            _ => SpacingMode::Continuous,
        }
    }
}

/// The Spacing requester's settings, plus the airbrush spray radius.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpacingSettings {
    pub mode: SpacingMode,
    /// Stamps (NTotal), pixels (EveryN) or sprays per pixel (Airbrush).
    pub n: i32,
    /// Spray radius of the Airbrush tool and of the Airbrush spacing, in page pixels.
    pub airbrush_radius: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpacingRequester {
    form: Form,
}

impl SpacingRequester {
    pub fn new(s: SpacingSettings) -> Self {
        let mut form = Form::new("Spacing", 236, 120);
        let rows = [
            (SpacingMode::Continuous, "Continuous", 17),
            (SpacingMode::NTotal, "N Total", 29),
            (SpacingMode::EveryN, "Every Nth dot", 41),
            (SpacingMode::Airbrush, "Airbrush", 53),
        ];
        for (mode, label, y) in rows {
            let r = form.radio(0, mode.index(), (8, y), label, s.mode == mode);
            if mode == SpacingMode::Continuous {
                continue;
            }
            let f = form.field(
                label,
                (-1, 0),
                Rect::new(124, y - 1, 30, 9),
                SPACING_RANGE,
                s.n,
                Some((0, mode.index())),
            );
            form.link(r, f);
        }
        form.text((160, 30), "dots");
        form.text((160, 42), "pixels");
        form.text((160, 54), "per pixel");
        form.rule(68);
        form.field(
            "Airbrush size",
            (8, 76),
            Rect::new(124, 75, 30, 9),
            AIRBRUSH_RANGE,
            s.airbrush_radius,
            None,
        );
        form.text((160, 76), "radius");
        Self { form }
    }

    pub fn settings(&self) -> SpacingSettings {
        // All three count fields show the same value (the last one edited wins): each
        // mode's own field is the one that counts.
        let mode = SpacingMode::from_index(self.form.selected(0));
        let n = match mode {
            SpacingMode::Continuous | SpacingMode::NTotal => self.form.number(0),
            SpacingMode::EveryN => self.form.number(1),
            SpacingMode::Airbrush => self.form.number(2),
        };
        SpacingSettings {
            mode,
            n,
            airbrush_radius: self.form.number(3),
        }
    }
}

// --------------------------------------------------------------- Fill Type

/// Fill Type: solid, or a gradient through the colour range of the FG colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FillSettings {
    /// `None` = solid.
    pub gradient: Option<GradientDirection>,
    pub dither: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FillRequester {
    form: Form,
}

impl FillRequester {
    pub fn new(s: FillSettings) -> Self {
        let mut form = Form::new("Fill Type", 244, 156);
        form.radio(0, 0, (8, 17), "Solid", s.gradient.is_none());
        for (i, d) in GradientDirection::ALL.into_iter().enumerate() {
            form.radio(
                0,
                i as u8 + 1,
                (8, 29 + i as i32 * 12),
                d.label(),
                s.gradient == Some(d),
            );
        }
        // Not implemented yet: shown ghosted like DeluxePaint's unavailable options.
        for (i, label) in ["Brush", "Perspective", "Contour", "Ridge", "Highlight"]
            .into_iter()
            .enumerate()
        {
            form.ghost_radio(0, 10 + i as u8, (140, 17 + i as i32 * 12), label);
        }
        form.rule(82);
        form.check((8, 87), "Dither", s.dither);
        form.text((8, 99), "Range: FG colour range,");
        form.text((8, 108), "else FG to BG");
        Self { form }
    }

    pub fn settings(&self) -> FillSettings {
        let gradient = match self.form.selected(0) {
            i @ 1..=4 => Some(GradientDirection::ALL[usize::from(i) - 1]),
            _ => None,
        };
        FillSettings {
            gradient,
            dither: self.form.checked(0),
        }
    }
}

// ------------------------------------------------------------ Translucency

/// Translucency limits in percent of the picture that stays.
pub const TRANSLUCENCY_RANGE: (i32, i32) = (0, 100);

/// Effect ▸ Translucency ▸ Settings..: the share of the picture that remains under the paint
/// (DeluxePaint uses a slider; this is a number field, ADR-31).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslucencyRequester {
    form: Form,
}

impl TranslucencyRequester {
    pub fn new(percent: u8) -> Self {
        let mut form = Form::new("Translucency", 184, 62);
        form.field(
            "Picture stays",
            (8, 20),
            Rect::new(100, 19, 36, 9),
            TRANSLUCENCY_RANGE,
            i32::from(percent),
            None,
        );
        form.text((142, 20), "%");
        form.text((8, 33), "0 = solid paint, 100 = none");
        Self { form }
    }

    /// Percent of the picture that remains.
    pub fn percent(&self) -> u8 {
        self.form.number(0).clamp(0, 100) as u8
    }
}

// --------------------------------------------------------------- the enum

/// The one modal requester of the toolbox settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolRequester {
    Grid(GridRequester),
    Symmetry(SymmetryRequester),
    Spacing(SpacingRequester),
    Fill(FillRequester),
    Stencil(StencilRequester),
    /// Color ▸ Palette ▸ Arrange (ADR-31).
    Arrange(ArrangeRequester),
    /// Effect ▸ Translucency ▸ Settings.. (ADR-31).
    Translucency(TranslucencyRequester),
    /// Effect ▸ Perspective ▸ Settings.. (ADR-37).
    Perspective(PerspectiveRequester),
}

impl ToolRequester {
    fn form(&self) -> Option<&Form> {
        match self {
            ToolRequester::Grid(r) => Some(&r.form),
            ToolRequester::Symmetry(r) => Some(&r.form),
            ToolRequester::Spacing(r) => Some(&r.form),
            ToolRequester::Fill(r) => Some(&r.form),
            ToolRequester::Translucency(r) => Some(&r.form),
            ToolRequester::Perspective(r) => Some(&r.form),
            ToolRequester::Stencil(_) | ToolRequester::Arrange(_) => None,
        }
    }

    pub fn size(&self) -> (i32, i32) {
        match (self.form(), self) {
            (Some(f), _) => f.size(),
            (None, ToolRequester::Stencil(r)) => r.size(),
            (None, ToolRequester::Arrange(r)) => r.size(),
            _ => unreachable!("every requester but Stencil and Arrange is a form"),
        }
    }

    /// Top-left corner on a UI screen of this size: centred.
    pub fn origin(&self, screen_w: i32, screen_h: i32) -> (i32, i32) {
        let (w, h) = self.size();
        (((screen_w - w) / 2).max(0), ((screen_h - h) / 2).max(0))
    }

    /// Handles one event in requester coordinates (origin: [`ToolRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> ToolReqResponse {
        match self {
            ToolRequester::Grid(r) => r.form.handle(event).into(),
            ToolRequester::Symmetry(r) => r.form.handle(event).into(),
            ToolRequester::Spacing(r) => r.form.handle(event).into(),
            ToolRequester::Fill(r) => r.form.handle(event).into(),
            ToolRequester::Stencil(r) => r.handle(event),
            ToolRequester::Arrange(r) => r.handle(event),
            ToolRequester::Translucency(r) => r.form.handle(event).into(),
            ToolRequester::Perspective(r) => r.handle(event).into(),
        }
    }

    /// Renders the requester alone; `palette` supplies the swatches of the stencil requester.
    pub fn render(&self, pens: &Pens, palette: &Palette) -> IndexedImage {
        match self {
            ToolRequester::Stencil(r) => r.render(pens, palette),
            ToolRequester::Arrange(r) => r.render(pens, palette),
            ToolRequester::Perspective(r) => r.render(pens),
            _ => self.form().expect("a form requester").render(pens),
        }
    }

    /// Draws the requester centred on a UI screen.
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens, palette: &Palette) {
        let (x, y) = self.origin(img.width() as i32, img.height() as i32);
        img.blit(&self.render(pens, palette), x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requester::ReqKey;

    fn key(r: &mut ToolRequester, k: ReqKey) -> ToolReqResponse {
        r.handle(ReqEvent::Key(k))
    }

    fn type_digits(r: &mut ToolRequester, s: &str) {
        for c in s.chars() {
            key(r, ReqKey::Char(c));
        }
    }

    /// Selects the whole active field's text away.
    fn clear(r: &mut ToolRequester, n: usize) {
        for _ in 0..n {
            key(r, ReqKey::Backspace);
        }
    }

    #[test]
    fn grid_edits_both_spacings_and_rejects_zero() {
        let mut r = ToolRequester::Grid(GridRequester::new(GridSettings { x: 16, y: 16 }));
        clear(&mut r, 2);
        type_digits(&mut r, "8");
        key(&mut r, ReqKey::Tab);
        clear(&mut r, 2);
        type_digits(&mut r, "0");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Continue);
        clear(&mut r, 1);
        type_digits(&mut r, "24");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        let ToolRequester::Grid(g) = &r else { panic!() };
        assert_eq!(g.settings(), GridSettings { x: 8, y: 24 });
    }

    #[test]
    fn symmetry_switches_to_mirror_and_limits_order_to_forty() {
        let mut r = ToolRequester::Symmetry(SymmetryRequester::new(SymmetrySettings {
            mirror: false,
            order: 6,
        }));
        // Click the Mirror radio.
        let (x, y) = (86, 31);
        r.handle(ReqEvent::Press(x, y));
        r.handle(ReqEvent::Release);
        clear(&mut r, 1);
        type_digits(&mut r, "41");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Continue);
        clear(&mut r, 1);
        type_digits(&mut r, "0");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        let ToolRequester::Symmetry(s) = &r else {
            panic!()
        };
        assert_eq!(
            s.settings(),
            SymmetrySettings {
                mirror: true,
                order: 40
            }
        );
        // Order 1 is no symmetry: refused.
        let mut r = ToolRequester::Symmetry(SymmetryRequester::new(SymmetrySettings {
            mirror: false,
            order: 1,
        }));
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Continue);
    }

    #[test]
    fn spacing_reads_the_count_of_the_chosen_mode() {
        let s = SpacingSettings {
            mode: SpacingMode::Continuous,
            n: 5,
            airbrush_radius: 10,
        };
        let mut r = ToolRequester::Spacing(SpacingRequester::new(s));
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        let ToolRequester::Spacing(q) = &r else {
            panic!()
        };
        assert_eq!(q.settings().mode, SpacingMode::Continuous);
        // Choose "Every Nth dot" (its field is activated), type 12.
        let mut r = ToolRequester::Spacing(SpacingRequester::new(s));
        r.handle(ReqEvent::Press(10, 43));
        r.handle(ReqEvent::Release);
        clear(&mut r, 1);
        type_digits(&mut r, "12");
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        let ToolRequester::Spacing(q) = &r else {
            panic!()
        };
        assert_eq!(
            q.settings(),
            SpacingSettings {
                mode: SpacingMode::EveryN,
                n: 12,
                airbrush_radius: 10
            }
        );
    }

    #[test]
    fn spacing_ignores_a_bad_count_of_a_mode_that_is_not_chosen() {
        let s = SpacingSettings {
            mode: SpacingMode::Continuous,
            n: 5,
            airbrush_radius: 10,
        };
        let mut r = ToolRequester::Spacing(SpacingRequester::new(s));
        // Empty the N Total field (the first field is active), then Ok with Continuous.
        clear(&mut r, 1);
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        // With Airbrush chosen an empty airbrush count keeps the requester open.
        let mut r = ToolRequester::Spacing(SpacingRequester::new(SpacingSettings {
            mode: SpacingMode::Airbrush,
            ..s
        }));
        r.handle(ReqEvent::Press(10, 55));
        r.handle(ReqEvent::Release);
        clear(&mut r, 1);
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Continue);
    }

    #[test]
    fn fill_type_chooses_gradient_direction_and_dither() {
        let mut r = ToolRequester::Fill(FillRequester::new(FillSettings {
            gradient: None,
            dither: false,
        }));
        // "Horizontal" is the third radio (y 41), "Dither" the check box at y 87.
        r.handle(ReqEvent::Press(12, 43));
        r.handle(ReqEvent::Release);
        r.handle(ReqEvent::Press(12, 89));
        r.handle(ReqEvent::Release);
        // A ghosted option does not take.
        r.handle(ReqEvent::Press(144, 18));
        r.handle(ReqEvent::Release);
        assert_eq!(key(&mut r, ReqKey::Enter), ToolReqResponse::Ok);
        let ToolRequester::Fill(f) = &r else { panic!() };
        assert_eq!(
            f.settings(),
            FillSettings {
                gradient: Some(GradientDirection::Horizontal),
                dither: true
            }
        );
        // Escape cancels.
        assert_eq!(key(&mut r, ReqKey::Escape), ToolReqResponse::Cancel);
    }

    #[test]
    fn requesters_centre_and_render_at_their_size() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let pal = Palette::dpaint_default_32();
        for r in [
            ToolRequester::Grid(GridRequester::new(GridSettings { x: 16, y: 16 })),
            ToolRequester::Symmetry(SymmetryRequester::new(SymmetrySettings {
                mirror: false,
                order: 6,
            })),
            ToolRequester::Spacing(SpacingRequester::new(SpacingSettings {
                mode: SpacingMode::NTotal,
                n: 9,
                airbrush_radius: 10,
            })),
            ToolRequester::Fill(FillRequester::new(FillSettings {
                gradient: Some(GradientDirection::Vertical),
                dither: false,
            })),
        ] {
            let (w, h) = r.size();
            let img = r.render(&pens, &pal);
            assert_eq!((img.width() as i32, img.height() as i32), (w, h));
            assert!(w <= 320 && h <= 200, "fits the smallest UI screen");
            let (x, y) = r.origin(320, 200);
            assert_eq!((x, y), ((320 - w) / 2, (200 - h) / 2));
            let mut screen = IndexedImage::new(320, 200, 9);
            r.draw(&mut screen, &pens, &pal);
            assert_eq!(screen.get(x, y), Some(pens.dark), "frame corner");
        }
    }
}
