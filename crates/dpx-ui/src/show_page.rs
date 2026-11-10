//! Picture ▸ Show Page (ADR-10; AGA manual p.51 and p.236): the whole page on the screen,
//! without toolbox and title bar, reduced by whole factors when it is bigger than the screen
//! (only every n-th pixel is shown), with a rectangle around the part the screen shows.
//!
//! - Pressing inside the rectangle grabs it; pressing elsewhere centres it on the pointer.
//!   Dragging moves it; releasing returns to the page with that part in view.
//! - Arrow keys move it one unit (the reduction factor in page pixels), Alt+arrows one screen.
//! - Return keeps the rectangle's position (DeluxePainter addition); any other key returns
//!   without moving the view.
//!
//! Coordinates are screen pixels of the current display mode, not UI pixels.

use dpx_core::{IndexedImage, Rect};

use crate::requester::{ReqEvent, ReqKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowPageResponse {
    Continue,
    /// Back to the page; `Some` is the new view origin (top-left page pixel), `None` keeps it.
    Done(Option<(i32, i32)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowPage {
    page: (i32, i32),
    screen: (i32, i32),
    view: (i32, i32),
    /// Pointer position inside the rectangle while dragging (reduced pixels).
    grab: Option<(i32, i32)>,
}

impl ShowPage {
    /// `view` is the top-left page pixel the screen shows now.
    pub fn new(page: (u32, u32), screen: (u32, u32), view: (i32, i32)) -> Self {
        let mut s = Self {
            page: (page.0 as i32, page.1 as i32),
            screen: (screen.0.max(1) as i32, screen.1.max(1) as i32),
            view,
            grab: None,
        };
        s.set_view(view);
        s
    }

    /// The view origin the rectangle stands for.
    pub fn view(&self) -> (i32, i32) {
        self.view
    }

    /// Reduction per axis: 1 if the page fits, else the smallest whole factor that fits.
    pub fn factor(&self) -> (i32, i32) {
        let f = |p: i32, s: i32| ((p + s - 1) / s).max(1);
        (f(self.page.0, self.screen.0), f(self.page.1, self.screen.1))
    }

    /// The visible part, in screen pixels of the reduced page.
    pub fn rect(&self) -> Rect {
        let (fx, fy) = self.factor();
        let w = self.screen.0.min(self.page.0);
        let h = self.screen.1.min(self.page.1);
        Rect::new(
            self.view.0 / fx,
            self.view.1 / fy,
            (w / fx).max(1),
            (h / fy).max(1),
        )
    }

    fn set_view(&mut self, (x, y): (i32, i32)) {
        let max_x = (self.page.0 - self.screen.0).max(0);
        let max_y = (self.page.1 - self.screen.1).max(0);
        self.view = (x.clamp(0, max_x), y.clamp(0, max_y));
    }

    /// Moves the rectangle so its top-left is at reduced pixel (`x`, `y`).
    fn place(&mut self, x: i32, y: i32) {
        let (fx, fy) = self.factor();
        self.set_view((x * fx, y * fy));
    }

    /// `alt` is the state of the Alt key for arrow keys.
    pub fn handle(&mut self, event: ReqEvent, alt: bool) -> ShowPageResponse {
        match event {
            ReqEvent::Press(x, y) => {
                let r = self.rect();
                if !r.contains(x, y) {
                    self.place(x - r.w / 2, y - r.h / 2);
                }
                let r = self.rect();
                self.grab = Some((x - r.x, y - r.y));
                ShowPageResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                if let Some((gx, gy)) = self.grab {
                    self.place(x - gx, y - gy);
                }
                ShowPageResponse::Continue
            }
            ReqEvent::Release => match self.grab.take() {
                Some(_) => ShowPageResponse::Done(Some(self.view)),
                None => ShowPageResponse::Continue,
            },
            ReqEvent::Key(key) => {
                let (ux, uy) = if alt { self.screen } else { self.factor() };
                let (dx, dy) = match key {
                    ReqKey::Left => (-ux, 0),
                    ReqKey::Right => (ux, 0),
                    ReqKey::Up => (0, -uy),
                    ReqKey::Down => (0, uy),
                    ReqKey::Enter => return ShowPageResponse::Done(Some(self.view)),
                    _ => return ShowPageResponse::Done(None),
                };
                self.set_view((self.view.0 + dx, self.view.1 + dy));
                ShowPageResponse::Continue
            }
            ReqEvent::Scroll(_) => ShowPageResponse::Continue,
        }
    }

    /// The screen image: the reduced page on colour 0, the rectangle drawn in the complement
    /// of the colours under it (like the Amiga's complement drawing mode).
    pub fn render(&self, page: &IndexedImage, palette_len: usize) -> IndexedImage {
        let (fx, fy) = self.factor();
        let (sw, sh) = self.screen;
        let mut img = IndexedImage::new(sw as u32, sh as u32, 0);
        let (pw, ph) = (page.width() as i32, page.height() as i32);
        for y in 0..sh.min((ph + fy - 1) / fy) {
            for x in 0..sw.min((pw + fx - 1) / fx) {
                if let Some(p) = page.get(x * fx, y * fy) {
                    img.set(x, y, p);
                }
            }
        }
        let top = palette_len.clamp(1, 256) - 1;
        let r = self.rect();
        let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
        let mut invert = |x: i32, y: i32| {
            if let Some(p) = img.get(x, y) {
                img.set(x, y, (top - usize::from(p).min(top)) as u8);
            }
        };
        for x in r.x..=x1 {
            invert(x, r.y);
            if y1 != r.y {
                invert(x, y1);
            }
        }
        for y in r.y + 1..y1 {
            invert(r.x, y);
            if x1 != r.x {
                invert(x1, y);
            }
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_and_rectangle_follow_the_manual_example() {
        // 640x400 page on a 320x200 LoRes screen: every other pixel, rectangle 160x100.
        let s = ShowPage::new((640, 400), (320, 200), (100, 50));
        assert_eq!(s.factor(), (2, 2));
        assert_eq!(s.rect(), Rect::new(50, 25, 160, 100));
        // A page that fits is shown 1:1 with the rectangle around the screen.
        let fits = ShowPage::new((320, 256), (320, 256), (0, 0));
        assert_eq!(fits.factor(), (1, 1));
        assert_eq!(fits.rect(), Rect::new(0, 0, 320, 256));
        // Odd sizes round the factor up.
        assert_eq!(
            ShowPage::new((400, 257), (320, 256), (0, 0)).factor(),
            (2, 2)
        );
        // The view is clamped to the page.
        assert_eq!(
            ShowPage::new((640, 400), (320, 200), (999, -5)).view(),
            (320, 0)
        );
    }

    #[test]
    fn dragging_and_keys() {
        let mut s = ShowPage::new((640, 400), (320, 200), (0, 0));
        // Grab inside, drag by 10 reduced pixels, release: the view moves 20 page pixels.
        assert_eq!(
            s.handle(ReqEvent::Press(5, 5), false),
            ShowPageResponse::Continue
        );
        s.handle(ReqEvent::Drag(15, 5), false);
        assert_eq!(
            s.handle(ReqEvent::Release, false),
            ShowPageResponse::Done(Some((20, 0)))
        );
        // Pressing outside centres the rectangle on the pointer.
        let mut s = ShowPage::new((640, 400), (320, 200), (0, 0));
        s.handle(ReqEvent::Press(250, 150), false);
        assert_eq!(
            s.rect(),
            Rect::new(160, 100, 160, 100),
            "clamped to the page"
        );
        // Arrows move one unit, Alt+arrows one screen; Return keeps, other keys cancel.
        let mut s = ShowPage::new((640, 400), (320, 200), (0, 0));
        s.handle(ReqEvent::Key(ReqKey::Right), false);
        s.handle(ReqEvent::Key(ReqKey::Down), false);
        assert_eq!(s.view(), (2, 2));
        s.handle(ReqEvent::Key(ReqKey::Right), true);
        assert_eq!(s.view(), (320, 2));
        s.handle(ReqEvent::Key(ReqKey::Left), true);
        s.handle(ReqEvent::Key(ReqKey::Left), true);
        assert_eq!(s.view(), (0, 2));
        assert_eq!(
            s.handle(ReqEvent::Key(ReqKey::Enter), false),
            ShowPageResponse::Done(Some((0, 2)))
        );
        assert_eq!(
            s.handle(ReqEvent::Key(ReqKey::Char(' ')), false),
            ShowPageResponse::Done(None)
        );
    }

    #[test]
    fn render_reduces_and_inverts_the_rectangle() {
        let mut page = IndexedImage::new(640, 400, 1);
        page.set(2, 2, 5);
        page.set(3, 3, 6); // skipped: odd pixels are not shown at factor 2
        let s = ShowPage::new((640, 400), (320, 200), (320, 200));
        let img = s.render(&page, 32);
        assert_eq!((img.width(), img.height()), (320, 200));
        assert_eq!(img.get(1, 1), Some(5));
        assert!(!img.pixels().contains(&6));
        // Rectangle (160,100)-(319,199) in the complement of colour 1.
        assert_eq!(img.get(160, 100), Some(30));
        assert_eq!(img.get(319, 150), Some(30));
        assert_eq!(img.get(200, 150), Some(1), "inside stays");
    }
}
