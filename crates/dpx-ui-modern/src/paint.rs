//! The display list of the Modern skin: what the layout code builds, what the region cache
//! compares and what most tests assert on. All coordinates are window points (`f32`); the
//! rasteriser multiplies them by the device scale. A list holds no pixels: a picture is
//! referenced by index and handed to the rasteriser separately.

/// A rectangle in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// Whether (`x`, `y`) lies inside: left and top edges belong to the rectangle, right and
    /// bottom edges do not.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The rectangle shrunk by `d` on every side (grown for a negative `d`); never negative
    /// in size.
    pub fn inset(&self, d: f32) -> Rect {
        let (w, h) = ((self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0));
        Rect::new(
            self.x + (self.w - w) / 2.0,
            self.y + (self.h - h) / 2.0,
            w,
            h,
        )
    }

    /// Whether the two rectangles share any area.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether `other` lies completely inside (edges may touch).
    pub fn encloses(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
}

/// A straight (not premultiplied) RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// An opaque colour from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// The colour with opacity `percent` (0 to 100).
    pub const fn with_opacity(self, percent: u32) -> Self {
        Self {
            a: ((percent * 255 + 50) / 100) as u8,
            ..self
        }
    }

    /// The colour `percent` (0 to 100) of the way from `self` to `other`; opaque.
    pub fn mix(self, other: Color, percent: u32) -> Color {
        let ch = |a: u8, b: u8| {
            ((u32::from(a) * (100 - percent) + u32::from(b) * percent + 50) / 100) as u8
        };
        Color::rgb(
            ch(self.r, other.r),
            ch(self.g, other.g),
            ch(self.b, other.b),
        )
    }

    /// Relative luminance (WCAG 2), ignoring alpha.
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// Contrast ratio (WCAG 2) between two opaque colours, from 1 to 21.
    pub fn contrast(self, other: Color) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

/// One segment of a [`Path`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A vector outline: sub-paths of lines and Bézier curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path(pub Vec<Seg>);

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.0.push(Seg::Move(x, y));
        self
    }

    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.0.push(Seg::Line(x, y));
        self
    }

    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        self.0.push(Seg::Quad(cx, cy, x, y));
        self
    }

    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        self.0.push(Seg::Cubic(c1x, c1y, c2x, c2y, x, y));
        self
    }

    pub fn close(mut self) -> Self {
        self.0.push(Seg::Close);
        self
    }

    /// An open line from (`x0`, `y0`) to (`x1`, `y1`) as its own sub-path.
    pub fn segment(self, x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        self.move_to(x0, y0).line_to(x1, y1)
    }

    /// An elliptical arc around (`cx`, `cy`) from angle `start` over `sweep` degrees. Angles
    /// grow clockwise on screen (y points down), 0 is to the right. The arc starts a new
    /// sub-path when the path is empty or was just closed, and is joined by a line otherwise.
    pub fn arc(mut self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32) -> Self {
        let pieces = (sweep.abs() / 90.0).ceil().max(1.0) as usize;
        let step = (sweep / pieces as f32).to_radians();
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let mut a = start.to_radians();
        let at = |a: f32| (cx + rx * a.cos(), cy + ry * a.sin());
        let (x0, y0) = at(a);
        if matches!(self.0.last(), None | Some(Seg::Close)) {
            self.0.push(Seg::Move(x0, y0));
        } else {
            self.0.push(Seg::Line(x0, y0));
        }
        for _ in 0..pieces {
            let b = a + step;
            let ((x0, y0), (x1, y1)) = (at(a), at(b));
            self.0.push(Seg::Cubic(
                x0 - k * rx * a.sin(),
                y0 + k * ry * a.cos(),
                x1 + k * rx * b.sin(),
                y1 - k * ry * b.cos(),
                x1,
                y1,
            ));
            a = b;
        }
        self
    }

    /// A closed ellipse as its own sub-path.
    pub fn ellipse(mut self, cx: f32, cy: f32, rx: f32, ry: f32) -> Self {
        // Built on its own so that it starts with a move, never joined to an open sub-path.
        let mut arc = Path::new().arc(cx, cy, rx, ry, 0.0, 360.0).close();
        self.0.append(&mut arc.0);
        self
    }

    /// A closed circle as its own sub-path.
    pub fn circle(self, cx: f32, cy: f32, r: f32) -> Self {
        self.ellipse(cx, cy, r, r)
    }

    /// A closed polygon as its own sub-path.
    pub fn polygon(mut self, points: &[(f32, f32)]) -> Self {
        for (i, &(x, y)) in points.iter().enumerate() {
            self.0.push(if i == 0 {
                Seg::Move(x, y)
            } else {
                Seg::Line(x, y)
            });
        }
        if !points.is_empty() {
            self.0.push(Seg::Close);
        }
        self
    }

    /// A closed rectangle with corner radius `r` (0 for square corners) as its own sub-path.
    pub fn round_rect(self, rect: Rect, r: f32) -> Self {
        let r = r.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
        let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
        if r <= 0.0 {
            return self.polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
        }
        let mut corners = Path::new()
            .arc(x1 - r, y0 + r, r, r, -90.0, 90.0)
            .arc(x1 - r, y1 - r, r, r, 0.0, 90.0)
            .arc(x0 + r, y1 - r, r, r, 90.0, 90.0)
            .arc(x0 + r, y0 + r, r, r, 180.0, 90.0)
            .close();
        let mut path = self;
        path.0.append(&mut corners.0);
        path
    }

    /// The path scaled by `scale` and then moved by (`dx`, `dy`).
    pub fn placed(&self, scale: f32, dx: f32, dy: f32) -> Path {
        let p = |x: f32, y: f32| (x * scale + dx, y * scale + dy);
        Path(
            self.0
                .iter()
                .map(|seg| match *seg {
                    Seg::Move(x, y) => {
                        let (x, y) = p(x, y);
                        Seg::Move(x, y)
                    }
                    Seg::Line(x, y) => {
                        let (x, y) = p(x, y);
                        Seg::Line(x, y)
                    }
                    Seg::Quad(cx, cy, x, y) => {
                        let ((cx, cy), (x, y)) = (p(cx, cy), p(x, y));
                        Seg::Quad(cx, cy, x, y)
                    }
                    Seg::Cubic(ax, ay, bx, by, x, y) => {
                        let ((ax, ay), (bx, by), (x, y)) = (p(ax, ay), p(bx, by), p(x, y));
                        Seg::Cubic(ax, ay, bx, by, x, y)
                    }
                    Seg::Close => Seg::Close,
                })
                .collect(),
        )
    }

    /// The rectangle around all end and control points (it encloses the outline; for curves
    /// it may be a little larger). `None` for an empty path.
    pub fn bounds(&self) -> Option<Rect> {
        let mut b: Option<(f32, f32, f32, f32)> = None;
        let mut add = |x: f32, y: f32| {
            b = Some(match b {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        };
        for seg in &self.0 {
            match *seg {
                Seg::Move(x, y) | Seg::Line(x, y) => add(x, y),
                Seg::Quad(cx, cy, x, y) => {
                    add(cx, cy);
                    add(x, y);
                }
                Seg::Cubic(ax, ay, bx, by, x, y) => {
                    add(ax, ay);
                    add(bx, by);
                    add(x, y);
                }
                Seg::Close => {}
            }
        }
        b.map(|(x0, y0, x1, y1)| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// An outline stroke; caps and joins are always round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
}

impl Stroke {
    pub const fn new(color: Color, width: f32) -> Self {
        Self { color, width }
    }
}

/// The two weights of the bundled font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Medium,
}

/// One drawing step.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A rectangle with corner radius `radius`. The stroke lies inside the rectangle, so its
    /// outer edge is the rectangle's edge.
    RoundRect {
        rect: Rect,
        radius: f32,
        fill: Option<Color>,
        stroke: Option<Stroke>,
    },
    /// A straight line with round caps.
    Line {
        from: (f32, f32),
        to: (f32, f32),
        stroke: Stroke,
    },
    /// A filled and/or stroked outline (non-zero winding).
    Path {
        path: Path,
        fill: Option<Color>,
        stroke: Option<Stroke>,
    },
    /// One line of text starting at `x` on the baseline `baseline`; `size` is the em size.
    Text {
        x: f32,
        baseline: f32,
        text: String,
        size: f32,
        weight: Weight,
        color: Color,
    },
    /// Picture number `image` of the pictures given to the rasteriser, drawn into `rect` with
    /// nearest-neighbour sampling.
    Image { rect: Rect, image: usize },
    /// Restricts everything that follows to the inside of the outline; `None` lifts the
    /// restriction. Clips do not nest: a new one replaces the previous one.
    Clip(Option<Path>),
}

/// A display list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct List {
    ops: Vec<Op>,
}

impl List {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    /// Appends all of `other`.
    pub fn append(&mut self, mut other: List) {
        self.ops.append(&mut other.ops);
    }

    /// A filled rectangle with square corners.
    pub fn rect(&mut self, rect: Rect, fill: Color) {
        self.round_rect(rect, 0.0, fill);
    }

    /// A filled rectangle with corner radius `radius`.
    pub fn round_rect(&mut self, rect: Rect, radius: f32, fill: Color) {
        self.push(Op::RoundRect {
            rect,
            radius,
            fill: Some(fill),
            stroke: None,
        });
    }

    /// The outline of a rectangle, `width` wide, lying inside `rect`.
    pub fn ring(&mut self, rect: Rect, radius: f32, color: Color, width: f32) {
        self.push(Op::RoundRect {
            rect,
            radius,
            fill: None,
            stroke: Some(Stroke::new(color, width)),
        });
    }

    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), color: Color, width: f32) {
        self.push(Op::Line {
            from,
            to,
            stroke: Stroke::new(color, width),
        });
    }

    pub fn fill(&mut self, path: Path, color: Color) {
        self.push(Op::Path {
            path,
            fill: Some(color),
            stroke: None,
        });
    }

    pub fn stroke(&mut self, path: Path, color: Color, width: f32) {
        self.push(Op::Path {
            path,
            fill: None,
            stroke: Some(Stroke::new(color, width)),
        });
    }

    pub fn text(
        &mut self,
        x: f32,
        baseline: f32,
        text: impl Into<String>,
        size: f32,
        weight: Weight,
        color: Color,
    ) {
        self.push(Op::Text {
            x,
            baseline,
            text: text.into(),
            size,
            weight,
            color,
        });
    }

    pub fn image(&mut self, rect: Rect, image: usize) {
        self.push(Op::Image { rect, image });
    }

    pub fn clip(&mut self, path: Path) {
        self.push(Op::Clip(Some(path)));
    }

    pub fn unclip(&mut self) {
        self.push(Op::Clip(None));
    }

    /// A 64-bit fingerprint of the list (FNV-1a over every value, floats by their bits): two
    /// equal lists have the same fingerprint on every platform, so a cache can skip
    /// rasterising a region whose fingerprint did not change.
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fnv::new();
        for op in &self.ops {
            match op {
                Op::RoundRect {
                    rect,
                    radius,
                    fill,
                    stroke,
                } => {
                    h.byte(1);
                    h.rect(rect);
                    h.float(*radius);
                    h.color(fill);
                    h.stroke(stroke);
                }
                Op::Line { from, to, stroke } => {
                    h.byte(2);
                    for v in [from.0, from.1, to.0, to.1] {
                        h.float(v);
                    }
                    h.stroke(&Some(*stroke));
                }
                Op::Path { path, fill, stroke } => {
                    h.byte(3);
                    h.path(path);
                    h.color(fill);
                    h.stroke(stroke);
                }
                Op::Text {
                    x,
                    baseline,
                    text,
                    size,
                    weight,
                    color,
                } => {
                    h.byte(4);
                    for v in [*x, *baseline, *size] {
                        h.float(v);
                    }
                    h.word(text.len() as u32);
                    text.bytes().for_each(|b| h.byte(b));
                    h.byte(matches!(weight, Weight::Medium) as u8);
                    h.color(&Some(*color));
                }
                Op::Image { rect, image } => {
                    h.byte(5);
                    h.rect(rect);
                    h.word(*image as u32);
                }
                Op::Clip(None) => h.byte(6),
                Op::Clip(Some(path)) => {
                    h.byte(7);
                    h.path(path);
                }
            }
        }
        h.0
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn byte(&mut self, b: u8) {
        self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn word(&mut self, v: u32) {
        v.to_le_bytes().into_iter().for_each(|b| self.byte(b));
    }

    fn float(&mut self, v: f32) {
        self.word(v.to_bits());
    }

    fn rect(&mut self, r: &Rect) {
        for v in [r.x, r.y, r.w, r.h] {
            self.float(v);
        }
    }

    fn color(&mut self, c: &Option<Color>) {
        match c {
            None => self.byte(0),
            Some(c) => [1, c.r, c.g, c.b, c.a]
                .into_iter()
                .for_each(|b| self.byte(b)),
        }
    }

    fn stroke(&mut self, s: &Option<Stroke>) {
        match s {
            None => self.byte(0),
            Some(s) => {
                self.color(&Some(s.color));
                self.float(s.width);
            }
        }
    }

    fn path(&mut self, path: &Path) {
        self.word(path.0.len() as u32);
        for seg in &path.0 {
            let (tag, values): (u8, [f32; 6]) = match *seg {
                Seg::Move(x, y) => (1, [x, y, 0.0, 0.0, 0.0, 0.0]),
                Seg::Line(x, y) => (2, [x, y, 0.0, 0.0, 0.0, 0.0]),
                Seg::Quad(a, b, x, y) => (3, [a, b, x, y, 0.0, 0.0]),
                Seg::Cubic(a, b, c, d, x, y) => (4, [a, b, c, d, x, y]),
                Seg::Close => (5, [0.0; 6]),
            };
            self.byte(tag);
            values.into_iter().for_each(|v| self.float(v));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_edges_and_inset() {
        let r = Rect::new(10.0, 20.0, 30.0, 40.0);
        assert_eq!((r.right(), r.bottom()), (40.0, 60.0));
        assert_eq!(r.center(), (25.0, 40.0));
        assert!(r.contains(10.0, 20.0));
        assert!(!r.contains(40.0, 20.0), "the right edge is outside");
        assert_eq!(r.inset(5.0), Rect::new(15.0, 25.0, 20.0, 30.0));
        assert_eq!(
            r.inset(100.0).w,
            0.0,
            "an inset never turns a rectangle inside out"
        );
        assert!(r.intersects(&Rect::new(39.0, 59.0, 5.0, 5.0)));
        assert!(
            !r.intersects(&Rect::new(40.0, 20.0, 5.0, 5.0)),
            "touching is not overlapping"
        );
        assert!(r.encloses(&r));
    }

    #[test]
    fn colours_parse_mix_and_contrast() {
        let c = Color::hex(0xFF8A1F);
        assert_eq!((c.r, c.g, c.b, c.a), (255, 138, 31, 255));
        assert_eq!(c.with_opacity(40).a, 102);
        assert_eq!(
            Color::hex(0).mix(Color::hex(0xFFFFFF), 50),
            Color::rgb(128, 128, 128)
        );
        let ratio = Color::hex(0).contrast(Color::hex(0xFFFFFF));
        assert!(
            (ratio - 21.0).abs() < 0.01,
            "black on white is 21:1, got {ratio}"
        );
    }

    #[test]
    fn an_arc_ends_where_its_angles_say() {
        let p = Path::new().arc(10.0, 10.0, 5.0, 3.0, 0.0, 180.0);
        assert_eq!(p.0.first(), Some(&Seg::Move(15.0, 10.0)));
        let Some(Seg::Cubic(.., x, y)) = p.0.last().copied() else {
            panic!("an arc ends with a curve");
        };
        assert!(
            (x - 5.0).abs() < 1e-4 && (y - 10.0).abs() < 1e-4,
            "({x}, {y})"
        );
        // Half a turn is two quarter curves, passing through the bottom of the ellipse.
        assert_eq!(p.0.len(), 3);
        let Seg::Cubic(.., x, y) = p.0[1] else {
            panic!("curve expected");
        };
        assert!(
            (x - 10.0).abs() < 1e-4 && (y - 13.0).abs() < 1e-4,
            "({x}, {y})"
        );
    }

    #[test]
    fn a_round_rect_stays_inside_its_rectangle() {
        let r = Rect::new(2.0, 3.0, 20.0, 10.0);
        let b = Path::new().round_rect(r, 4.0).bounds().expect("not empty");
        assert!(
            (b.x - 2.0).abs() < 1e-4 && (b.y - 3.0).abs() < 1e-4,
            "{b:?}"
        );
        assert!(
            (b.w - 20.0).abs() < 1e-4 && (b.h - 10.0).abs() < 1e-4,
            "{b:?}"
        );
        // A radius larger than half the height is cut down, not drawn outside.
        let b = Path::new().round_rect(r, 99.0).bounds().expect("not empty");
        assert!(r.inset(-1e-3).encloses(&b), "{b:?}");
    }

    #[test]
    fn placing_scales_then_moves() {
        let p = Path::new().move_to(1.0, 2.0).line_to(3.0, 4.0).close();
        assert_eq!(
            p.placed(2.0, 10.0, 20.0).0,
            vec![Seg::Move(12.0, 24.0), Seg::Line(16.0, 28.0), Seg::Close]
        );
    }

    fn sample() -> List {
        let mut l = List::new();
        l.rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::hex(0x202227));
        l.ring(
            Rect::new(1.0, 1.0, 8.0, 8.0),
            2.0,
            Color::hex(0xFF8A1F),
            1.5,
        );
        l.text(
            2.0,
            8.0,
            "Mode",
            13.5,
            Weight::Regular,
            Color::hex(0xE4E6EB),
        );
        l.clip(Path::new().polygon(&[(0.0, 0.0), (5.0, 0.0), (0.0, 5.0)]));
        l.image(Rect::new(0.0, 0.0, 4.0, 4.0), 0);
        l.unclip();
        l
    }

    #[test]
    fn equal_lists_share_a_fingerprint_and_any_change_moves_it() {
        let base = sample();
        assert_eq!(base.fingerprint(), sample().fingerprint());
        assert_eq!(base.len(), 6);
        let mut seen = vec![base.fingerprint(), List::new().fingerprint()];
        // One change each: a coordinate, a colour, a text, a weight, a missing op.
        let mut moved = sample();
        moved.ops[0] = Op::RoundRect {
            rect: Rect::new(0.5, 0.0, 10.0, 10.0),
            radius: 0.0,
            fill: Some(Color::hex(0x202227)),
            stroke: None,
        };
        seen.push(moved.fingerprint());
        let mut recoloured = sample();
        recoloured.ops[1] = Op::RoundRect {
            rect: Rect::new(1.0, 1.0, 8.0, 8.0),
            radius: 2.0,
            fill: None,
            stroke: Some(Stroke::new(Color::hex(0xD66214), 1.5)),
        };
        seen.push(recoloured.fingerprint());
        for (text, weight) in [("Anim", Weight::Regular), ("Mode", Weight::Medium)] {
            let mut other = sample();
            other.ops[2] = Op::Text {
                x: 2.0,
                baseline: 8.0,
                text: text.into(),
                size: 13.5,
                weight,
                color: Color::hex(0xE4E6EB),
            };
            seen.push(other.fingerprint());
        }
        let mut shorter = sample();
        shorter.ops.pop();
        seen.push(shorter.fingerprint());
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "{seen:x?}");
    }
}
