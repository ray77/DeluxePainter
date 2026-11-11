//! Effect ▸ Perspective ▸ Settings.. (ADR-37): the Perspective requester (manual AGA p.279-280,
//! figure 8.39): Grid X / Y / Z, From Brush, Angle Step, Type Screen / Brush, Display Angle /
//! Pos, Cancel, Ok. A [`Form`] with one extra button.

use dpx_core::{IndexedImage, Rect};

use crate::font::Font;
use crate::form::{Form, FormResponse};
use crate::pens::Pens;
use crate::requester::ReqEvent;
use crate::widgets::{Inset, checker, draw_button};

/// Grid X / Y / Z limits in pixels (the largest page side).
pub const PERSPECTIVE_GRID_RANGE: (i32, i32) = (1, 4096);
/// Angle Step limits in degrees.
pub const ANGLE_STEP_RANGE: (i32, i32) = (1, 360);
/// Angle Step default (AGA p.279).
pub const DEFAULT_ANGLE_STEP: i32 = 90;

/// The Perspective requester's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerspectiveSettings {
    /// Grid X / Y / Z in pixels: the spacing of FillScreen's copies along the brush's x and y
    /// axes and of the perspective grid.
    pub grid: [i32; 3],
    /// Shift with a keypad key turns by this many degrees.
    pub angle_step: i32,
    /// Type Brush: the keypad turns the brush about its own axes (else the screen's).
    pub brush_axes: bool,
    /// Display Pos: the title bar shows the brush's position (else its angles).
    pub show_position: bool,
}

impl PerspectiveSettings {
    /// The defaults for a brush of this size: the grid is the whole brush, Z = Y (AGA p.279).
    pub fn defaults(brush: (u32, u32)) -> Self {
        let side = |v: u32| (v as i32).clamp(PERSPECTIVE_GRID_RANGE.0, PERSPECTIVE_GRID_RANGE.1);
        Self {
            grid: [side(brush.0), side(brush.1), side(brush.1)],
            angle_step: DEFAULT_ANGLE_STEP,
            brush_axes: false,
            show_position: false,
        }
    }
}

const WIDTH: i32 = 196;
const HEIGHT: i32 = 134;
const FROM_BRUSH: &str = "From Brush";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerspectiveRequester {
    pub(crate) form: Form,
    /// The custom brush's size, for From Brush (ghosted without one).
    brush: Option<(u32, u32)>,
    /// From Brush is held down; the pointer is over it.
    held: bool,
    over: bool,
}

impl PerspectiveRequester {
    pub fn new(s: PerspectiveSettings, brush: Option<(u32, u32)>) -> Self {
        let mut form = Form::new("Perspective", WIDTH, HEIGHT);
        for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            let x = 48 + i as i32 * 48;
            form.text((x + 17, 16), axis);
            form.field(
                &format!("Grid {axis}"),
                (-1, 0),
                Rect::new(x, 27, 40, 9),
                PERSPECTIVE_GRID_RANGE,
                s.grid[i],
                None,
            );
        }
        form.text((8, 28), "Grid:");
        form.rule(58);
        form.field(
            "Angle Step",
            (8, 64),
            Rect::new(96, 63, 30, 9),
            ANGLE_STEP_RANGE,
            s.angle_step,
            None,
        );
        form.text((132, 64), "deg");
        form.text((8, 77), "Type:");
        form.radio(0, 0, (56, 76), "Screen", !s.brush_axes);
        form.radio(0, 1, (130, 76), "Brush", s.brush_axes);
        form.rule(90);
        form.text((8, 96), "Display:");
        form.radio(1, 0, (70, 95), "Angle", !s.show_position);
        form.radio(1, 1, (130, 95), "Pos", s.show_position);
        Self {
            form,
            brush,
            held: false,
            over: false,
        }
    }

    /// The From Brush button.
    pub fn from_brush_rect() -> Rect {
        let w = Font::builtin().text_width(FROM_BRUSH) + 12;
        Rect::new((WIDTH - w) / 2, 40, w, 13)
    }

    pub fn size(&self) -> (i32, i32) {
        self.form.size()
    }

    pub fn settings(&self) -> PerspectiveSettings {
        PerspectiveSettings {
            grid: [0, 1, 2].map(|i| self.form.number(i)),
            angle_step: self.form.number(3),
            brush_axes: self.form.selected(0) == 1,
            show_position: self.form.selected(1) == 1,
        }
    }

    /// Handles one event in requester coordinates. From Brush sets Grid X and Y to the
    /// brush's width and height (Z stays, AGA p.279).
    pub fn handle(&mut self, event: ReqEvent) -> FormResponse {
        let button = Self::from_brush_rect();
        match event {
            ReqEvent::Press(x, y) if button.contains(x, y) => {
                self.held = self.brush.is_some();
                self.over = self.held;
                FormResponse::Continue
            }
            ReqEvent::Drag(x, y) if self.held => {
                self.over = button.contains(x, y);
                FormResponse::Continue
            }
            ReqEvent::Release if self.held => {
                self.held = false;
                if std::mem::take(&mut self.over)
                    && let Some((w, h)) = self.brush
                {
                    let side = |v: u32| {
                        (v as i32).clamp(PERSPECTIVE_GRID_RANGE.0, PERSPECTIVE_GRID_RANGE.1)
                    };
                    self.form.set_number(0, side(w));
                    self.form.set_number(1, side(h));
                }
                FormResponse::Continue
            }
            e => self.form.handle(e),
        }
    }

    /// Renders the requester alone.
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let mut img = self.form.render(pens);
        let r = Self::from_brush_rect();
        draw_button(&mut img, r, FROM_BRUSH, self.held && self.over, pens);
        if self.brush.is_none() {
            checker(&mut img, r.inset(2), pens.face);
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requester::ReqKey;
    use dpx_core::Palette;

    fn key(r: &mut PerspectiveRequester, k: ReqKey) -> FormResponse {
        r.handle(ReqEvent::Key(k))
    }

    fn click(r: &mut PerspectiveRequester, x: i32, y: i32) {
        r.handle(ReqEvent::Press(x, y));
        r.handle(ReqEvent::Release);
    }

    #[test]
    fn defaults_follow_the_brush() {
        let s = PerspectiveSettings::defaults((24, 10));
        assert_eq!(s.grid, [24, 10, 10]);
        assert_eq!(s.angle_step, 90);
        assert!(!s.brush_axes && !s.show_position);
        let r = PerspectiveRequester::new(s, Some((24, 10)));
        assert_eq!(r.settings(), s);
    }

    #[test]
    fn fields_radios_and_from_brush() {
        let s = PerspectiveSettings::defaults((24, 10));
        let mut r = PerspectiveRequester::new(s, Some((7, 5)));
        // Grid X is the first field: type 99, then Tab to Y and Z, Tab to Angle Step: 45.
        for _ in 0..3 {
            key(&mut r, ReqKey::Backspace);
        }
        for c in "99".chars() {
            key(&mut r, ReqKey::Char(c));
        }
        for _ in 0..3 {
            key(&mut r, ReqKey::Tab);
        }
        for _ in 0..3 {
            key(&mut r, ReqKey::Backspace);
        }
        for c in "45".chars() {
            key(&mut r, ReqKey::Char(c));
        }
        // Type Brush, Display Pos.
        click(&mut r, 132, 78);
        click(&mut r, 132, 97);
        let got = r.settings();
        assert_eq!(got.grid, [99, 10, 10]);
        assert_eq!(got.angle_step, 45);
        assert!(got.brush_axes && got.show_position);
        // From Brush: X and Y become the brush size, Z stays.
        let b = PerspectiveRequester::from_brush_rect();
        click(&mut r, b.x + 3, b.y + 3);
        assert_eq!(r.settings().grid, [7, 5, 10]);
        // Released outside the button: nothing.
        let mut r2 = PerspectiveRequester::new(s, Some((7, 5)));
        r2.handle(ReqEvent::Press(b.x + 3, b.y + 3));
        r2.handle(ReqEvent::Drag(2, 2));
        r2.handle(ReqEvent::Release);
        assert_eq!(r2.settings().grid, [24, 10, 10]);
        assert_eq!(key(&mut r, ReqKey::Enter), FormResponse::Ok);
    }

    #[test]
    fn bad_numbers_keep_it_open_and_from_brush_needs_a_brush() {
        let s = PerspectiveSettings::defaults((24, 10));
        let mut r = PerspectiveRequester::new(s, None);
        let b = PerspectiveRequester::from_brush_rect();
        click(&mut r, b.x + 3, b.y + 3);
        assert_eq!(r.settings(), s, "ghosted without a brush");
        for _ in 0..3 {
            key(&mut r, ReqKey::Tab);
        }
        for _ in 0..3 {
            key(&mut r, ReqKey::Backspace);
        }
        key(&mut r, ReqKey::Char('0'));
        assert_eq!(key(&mut r, ReqKey::Enter), FormResponse::Continue);
        assert_eq!(key(&mut r, ReqKey::Escape), FormResponse::Cancel);
    }

    #[test]
    fn renders_at_its_size_with_square_corners() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let r = PerspectiveRequester::new(PerspectiveSettings::defaults((24, 10)), None);
        let img = r.render(&pens);
        assert_eq!((img.width() as i32, img.height() as i32), r.size());
        assert!(r.size().0 <= 320 && r.size().1 <= 200);
        assert_eq!(img.get(0, 0), Some(pens.dark), "square frame corner");
    }
}
