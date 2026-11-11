//! Effect ▸ Perspective (ADR-37): a custom brush turned in three dimensions and projected
//! towards the perspective centre, as DeluxePaint IV does it (manual AGA p.126-143 and
//! p.278-281).
//!
//! Space: page pixels, x to the right, y down, z into the screen. The screen is the plane
//! z = 0; the observer looks at it from `distance` pixels in front of the perspective centre,
//! and a point appears where its line of sight crosses the screen. Coordinates are continuous:
//! page pixel (x, y) covers x..x+1 and y..y+1, so its centre is (x + 0.5, y + 0.5). A brush is
//! placed by the position of its handle pixel's centre and an orientation; brush pixel (i, j)
//! lies at (i - handle.x, j - handle.y) along the brush's own x and y axes.
//!
//! Angles are whole degrees. Sines and cosines come from a table and the projection uses only
//! IEEE addition, multiplication, division and `floor`, so the same inputs give the same
//! pixels on every platform. Only the angles shown for a brush turned about its own axes
//! (Type Brush) go through `asin` / `atan2`, rounded to whole degrees.

use crate::brush_ops;
use crate::image::{IndexedImage, Rect};
use crate::paint::BrushImage;

/// A point or direction in page space.
pub type Vec3 = [f64; 3];

/// Points closer to the observer than this (in pixels of depth) are not drawn.
pub const NEAR: f64 = 1.0;
/// The plane of FillScreen ends this many observer distances away, short of the horizon,
/// where its tiles would be far smaller than a pixel.
pub const FAR: f64 = 64.0;
/// Largest absolute page coordinate a projected point is clamped to before it becomes a pixel.
const MAX_COORD: f64 = 1_000_000.0;

/// sin(d°) for d = 0..=90.
const SIN: [f64; 91] = [
    0.0,
    0.01745240643728351,
    0.03489949670250097,
    0.052335956242943835,
    0.0697564737441253,
    0.08715574274765817,
    0.10452846326765347,
    0.12186934340514748,
    0.13917310096006544,
    0.15643446504023087,
    0.17364817766693033,
    0.1908089953765448,
    0.20791169081775934,
    0.224951054343865,
    0.24192189559966773,
    0.25881904510252074,
    0.27563735581699916,
    0.29237170472273677,
    0.3090169943749474,
    0.3255681544571567,
    0.3420201433256687,
    0.35836794954530027,
    0.374606593415912,
    0.39073112848927377,
    0.4067366430758002,
    0.42261826174069944,
    0.43837114678907746,
    0.4539904997395468,
    0.4694715627858908,
    0.48480962024633706,
    0.5,
    0.5150380749100542,
    0.5299192642332049,
    0.5446390350150271,
    0.5591929034707469,
    0.573576436351046,
    0.5877852522924731,
    0.6018150231520483,
    0.6156614753256583,
    0.6293203910498374,
    0.6427876096865393,
    0.6560590289905073,
    0.6691306063588582,
    0.6819983600624985,
    0.6946583704589973,
    std::f64::consts::FRAC_1_SQRT_2,
    0.7193398003386511,
    0.7313537016191705,
    0.7431448254773942,
    0.754709580222772,
    0.766044443118978,
    0.7771459614569709,
    0.788010753606722,
    0.7986355100472928,
    0.8090169943749475,
    0.8191520442889918,
    0.8290375725550417,
    0.838670567945424,
    0.848048096156426,
    0.8571673007021123,
    0.8660254037844386,
    0.8746197071393957,
    0.8829475928589269,
    0.8910065241883679,
    0.8987940462991669,
    0.9063077870366499,
    0.9135454576426009,
    0.9205048534524404,
    0.9271838545667874,
    0.9335804264972017,
    0.9396926207859083,
    0.9455185755993167,
    0.9510565162951535,
    0.9563047559630354,
    0.9612616959383189,
    0.9659258262890683,
    0.9702957262759965,
    0.9743700647852352,
    0.9781476007338056,
    0.981627183447664,
    0.984807753012208,
    0.9876883405951378,
    0.9902680687415704,
    0.992546151641322,
    0.9945218953682733,
    0.9961946980917455,
    0.9975640502598242,
    0.9986295347545738,
    0.9993908270190958,
    0.9998476951563913,
    1.0,
];

/// `(sin, cos)` of a whole number of degrees, from the table (exact for multiples of 90).
pub fn sin_cos(degrees: i32) -> (f64, f64) {
    let d = degrees.rem_euclid(360);
    let (quarter, rest) = (d / 90, (d % 90) as usize);
    let (s, c) = (SIN[rest], SIN[90 - rest]);
    match quarter {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// An angle as shown: -179..=180 degrees.
pub fn normalize(degrees: i32) -> i32 {
    let d = degrees.rem_euclid(360);
    if d > 180 { d - 360 } else { d }
}

fn dot(a: Vec3, b: Vec3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add_scaled(a: Vec3, b: Vec3, k: f64) -> Vec3 {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}

/// One of the three axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis3 {
    X,
    Y,
    Z,
}

impl Axis3 {
    pub const ALL: [Axis3; 3] = [Axis3::X, Axis3::Y, Axis3::Z];

    pub fn index(self) -> usize {
        match self {
            Axis3::X => 0,
            Axis3::Y => 1,
            Axis3::Z => 2,
        }
    }
}

/// A 3×3 matrix, rows first. As an orientation its columns are where the brush's own x, y
/// and z axes point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Mat3 {
    pub const IDENTITY: Mat3 = Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn mul(&self, o: &Mat3) -> Mat3 {
        let (a, b) = (&self.0, &o.0);
        let mut m = [[0.0; 3]; 3];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = a[r][0] * b[0][c] + a[r][1] * b[1][c] + a[r][2] * b[2][c];
            }
        }
        Mat3(m)
    }

    pub fn apply(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
    }

    /// Where the brush's own `axis` points.
    pub fn axis(&self, axis: Axis3) -> Vec3 {
        let i = axis.index();
        [self.0[0][i], self.0[1][i], self.0[2][i]]
    }

    /// A turn of `degrees` about a screen axis. Positive turns go clockwise on the screen
    /// about z, the top towards the observer about x, and the right side towards the
    /// observer about y (keypad 8, 5 and 2; AGA p.128-129).
    pub fn rotation(axis: Axis3, degrees: i32) -> Mat3 {
        let (s, c) = sin_cos(degrees);
        Mat3(match axis {
            Axis3::X => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
            Axis3::Y => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
            Axis3::Z => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
        })
    }

    /// The orientation of the screen (Euler) angles `[x, y, z]`: turned about z first, then
    /// y, then x, all about the screen's axes, so the order in which the angles were changed
    /// does not matter (AGA p.133).
    pub fn euler(angles: [i32; 3]) -> Mat3 {
        Mat3::rotation(Axis3::X, angles[0])
            .mul(&Mat3::rotation(Axis3::Y, angles[1]))
            .mul(&Mat3::rotation(Axis3::Z, angles[2]))
    }

    fn near_identity(&self) -> bool {
        let id = Mat3::IDENTITY.0;
        (0..3).all(|r| (0..3).all(|c| (self.0[r][c] - id[r][c]).abs() < 1e-9))
    }

    /// The screen angles of this orientation, rounded to whole degrees.
    pub fn euler_angles(&self) -> [i32; 3] {
        let m = &self.0;
        let sb = m[0][2].clamp(-1.0, 1.0);
        let (a, c) = if sb.abs() < 1.0 - 1e-9 {
            ((-m[1][2]).atan2(m[2][2]), (-m[0][1]).atan2(m[0][0]))
        } else {
            // Turned a quarter about y: z and x turn about the same line; all on x.
            ((m[1][0] * sb).atan2(m[1][1]), 0.0)
        };
        let deg = |r: f64| normalize(r.to_degrees().round() as i32);
        [deg(a), deg(sb.asin()), deg(c)]
    }
}

/// Perspective requester ▸ Type: what the keypad turns the brush about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AngleType {
    /// The screen's axes (Euler angles, the default): the result depends only on the three
    /// angles.
    #[default]
    Screen,
    /// The brush's own axes as they are now.
    Brush,
}

/// The brush's orientation: the screen angles shown in the title bar and its matrix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
    angles: [i32; 3],
    matrix: Mat3,
}

impl Default for Orientation {
    fn default() -> Self {
        Self {
            angles: [0; 3],
            matrix: Mat3::IDENTITY,
        }
    }
}

impl Orientation {
    pub fn from_angles(angles: [i32; 3]) -> Self {
        let angles = angles.map(normalize);
        Self {
            angles,
            matrix: Mat3::euler(angles),
        }
    }

    /// The screen angles about x, y and z, -179..=180 (also for Type Brush, AGA p.133-134).
    pub fn angles(&self) -> [i32; 3] {
        self.angles
    }

    pub fn matrix(&self) -> Mat3 {
        self.matrix
    }

    /// Not turned at all.
    pub fn is_identity(&self) -> bool {
        self.matrix == Mat3::IDENTITY
    }

    /// Turns the brush by `degrees` about `axis`: a screen axis (Type Screen) or the brush's
    /// own (Type Brush).
    pub fn rotate(&mut self, axis: Axis3, degrees: i32, kind: AngleType) {
        match kind {
            AngleType::Screen => {
                let i = axis.index();
                self.angles[i] = normalize(self.angles[i] + degrees);
                self.matrix = Mat3::euler(self.angles);
            }
            AngleType::Brush => {
                let m = self.matrix.mul(&Mat3::rotation(axis, degrees));
                // Turning back exactly lands on "not turned" despite rounding.
                self.matrix = if m.near_identity() { Mat3::IDENTITY } else { m };
                self.angles = self.matrix.euler_angles();
            }
        }
    }

    /// Sets one screen angle to 0 (keypad 9, 6, 3).
    pub fn reset_axis(&mut self, axis: Axis3) {
        self.angles[axis.index()] = 0;
        self.matrix = Mat3::euler(self.angles);
    }

    /// The orientation of the shown angles exactly (after turning about the brush's axes,
    /// when going back to Type Screen).
    pub fn snap_to_angles(&mut self) {
        self.matrix = Mat3::euler(self.angles);
    }
}

/// The observer: the perspective centre (a continuous page position) and the distance from
/// the screen in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewer {
    pub center: (f64, f64),
    pub distance: f64,
}

impl Viewer {
    /// Centre at the middle of page pixel `center`.
    pub fn at_pixel(center: (i32, i32), distance: f64) -> Self {
        Self {
            center: (f64::from(center.0) + 0.5, f64::from(center.1) + 0.5),
            distance,
        }
    }

    fn eye(&self) -> Vec3 {
        [self.center.0, self.center.1, -self.distance]
    }

    /// Where `p` appears on the screen; `None` closer to the observer than [`NEAR`].
    pub fn project(&self, p: Vec3) -> Option<(f64, f64)> {
        let depth = self.distance + p[2];
        if depth < NEAR {
            return None;
        }
        let k = self.distance / depth;
        let (cx, cy) = self.center;
        Some((cx + (p[0] - cx) * k, cy + (p[1] - cy) * k))
    }

    /// Where the line of sight through the centre of page pixel `pixel` meets the plane
    /// through `origin` square to `normal`; `None` if it does not, or not at a depth from
    /// [`NEAR`] to `max_depth` in front of the observer.
    pub fn hit_plane(
        &self,
        origin: Vec3,
        normal: Vec3,
        pixel: (i32, i32),
        max_depth: f64,
    ) -> Option<Vec3> {
        let (cx, cy) = self.center;
        let ray = [
            f64::from(pixel.0) + 0.5 - cx,
            f64::from(pixel.1) + 0.5 - cy,
            self.distance,
        ];
        let along = dot(ray, normal);
        if along.abs() < 1e-12 {
            return None;
        }
        let eye = self.eye();
        let t = dot(sub(origin, eye), normal) / along;
        // The ray reaches the screen at t = 1, so the depth is t times the distance.
        let depth = t * self.distance;
        (NEAR..=max_depth)
            .contains(&depth)
            .then(|| add_scaled(eye, ray, t))
    }
}

/// A brush in space: its orientation and the position of its handle pixel's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub matrix: Mat3,
    pub position: Vec3,
}

impl Placement {
    /// The handle on page pixel `pixel` of the screen plane, not turned.
    pub fn at_pixel(pixel: (i32, i32)) -> Self {
        Self {
            matrix: Mat3::IDENTITY,
            position: [f64::from(pixel.0) + 0.5, f64::from(pixel.1) + 0.5, 0.0],
        }
    }

    /// A point of the brush plane, `(u, v)` pixels along the brush's x and y axes from the
    /// handle's centre.
    fn point(&self, (u, v): (f64, f64)) -> Vec3 {
        let p = add_scaled(self.position, self.matrix.axis(Axis3::X), u);
        add_scaled(p, self.matrix.axis(Axis3::Y), v)
    }

    /// The brush is lying flat on the screen, unturned: it looks exactly like itself.
    pub fn on_screen_plane(&self) -> bool {
        self.matrix == Mat3::IDENTITY && self.position[2] == 0.0
    }

    /// The page pixel the handle appears on, and its depth (for the title bar).
    pub fn shown_position(&self) -> (i32, i32, i32) {
        let p = self.position;
        let px = |v: f64| v.floor().clamp(-MAX_COORD, MAX_COORD) as i32;
        (
            px(p[0]),
            px(p[1]),
            p[2].round().clamp(-MAX_COORD, MAX_COORD) as i32,
        )
    }

    /// The brush plane coordinates of the line of sight through `pixel`: pixels along the
    /// brush's x and y axes from the handle centre (also Anim ▸ Move's projection, ADR-38).
    pub fn local(&self, viewer: &Viewer, pixel: (i32, i32), max_depth: f64) -> Option<(f64, f64)> {
        let normal = self.matrix.axis(Axis3::Z);
        let q = viewer.hit_plane(self.position, normal, pixel, max_depth)?;
        let d = sub(q, self.position);
        Some((
            dot(d, self.matrix.axis(Axis3::X)),
            dot(d, self.matrix.axis(Axis3::Y)),
        ))
    }
}

/// The handle position after the pointer moved to page pixel `pixel`: the brush slides on the
/// plane through its handle that holds the two axes other than `fixed` (the mouse never moves
/// the brush along its fixed axis, AGA p.281). `None` when the line of sight does not meet
/// that plane within [`FAR`] observer distances (the brush stays).
pub fn slide(place: &Placement, viewer: &Viewer, fixed: Axis3, pixel: (i32, i32)) -> Option<Vec3> {
    viewer.hit_plane(
        place.position,
        place.matrix.axis(fixed),
        pixel,
        FAR * viewer.distance,
    )
}

/// `position` on the perspective grid: along each of the brush's axes except `keep`, a whole
/// number of `grid` steps from `origin` (Grid X / Y / Z of the Perspective requester).
pub fn snap(
    position: Vec3,
    origin: Vec3,
    matrix: &Mat3,
    grid: [u32; 3],
    keep: Option<Axis3>,
) -> Vec3 {
    let offset = sub(position, origin);
    let mut out = origin;
    for axis in Axis3::ALL {
        let dir = matrix.axis(axis);
        let mut along = dot(offset, dir);
        if Some(axis) != keep {
            let g = f64::from(grid[axis.index()].max(1));
            along = (along / g).round() * g;
        }
        out = add_scaled(out, dir, along);
    }
    out
}

/// Replace mode for a projected stamp, which is painted as Matte (ADR-37): inside the turned
/// brush its transparent pixels are painted in `bg`, outside it nothing.
#[derive(Clone, Copy, Debug)]
pub struct Replace<'a> {
    /// The BG colour.
    pub bg: u8,
    /// The right button: the stamp paints its shape in BG, so every pixel inside the brush
    /// counts and its colours do not matter (DeluxePaint's Replace with the right button paints
    /// the whole brush rectangle in BG).
    pub right: bool,
    /// The picture under the stamp. When the brush's colours and BG use the whole palette, no
    /// colour is left to stand for "nothing" (on 2- and 4-colour screens often): the pixels
    /// outside the brush then repeat the picture and the stamp has no transparent colour.
    pub page: &'a IndexedImage,
}

/// The colour standing for "nothing" in a projected brush. With `replace` it must differ from
/// every colour the stamp paints (BG and, with the left button, the brush's own colours);
/// `None` if every palette colour is taken.
fn nothing_colour(brush: &BrushImage, colors: usize, replace: Option<&Replace>) -> Option<u8> {
    let Some(r) = replace else {
        return Some(brush_ops::fill_colour(brush, colors));
    };
    let mut used = [false; 256];
    used[usize::from(r.bg)] = true;
    if !r.right {
        for &p in &brush.pixels {
            if Some(p) != brush.transparent {
                used[usize::from(p)] = true;
            }
        }
    }
    (0..colors.clamp(1, 256))
        .find(|&c| !used[c])
        .map(|c| c as u8)
}

/// The brush pixel seen at brush plane coordinates `(u, v)`, if the brush covers them.
fn sample(brush: &BrushImage, (u, v): (f64, f64), tile: Option<(i64, i64)>) -> Option<u8> {
    let cell = |c: f64, handle: i32| {
        (c + f64::from(handle) + 0.5)
            .floor()
            .clamp(-MAX_COORD * 64.0, MAX_COORD * 64.0) as i64
    };
    let (mut i, mut j) = (cell(u, brush.handle.0), cell(v, brush.handle.1));
    if let Some((gx, gy)) = tile {
        i = i.rem_euclid(gx);
        j = j.rem_euclid(gy);
    }
    let (w, h) = (i64::from(brush.width), i64::from(brush.height));
    ((0..w).contains(&i) && (0..h).contains(&j)).then(|| brush.pixels[(j * w + i) as usize])
}

/// What the brush plane shows through each page pixel of `area`, as a brush image with its
/// handle at the top-left, cut to the pixels that show something, and where that top-left
/// lies; `None` if nothing of the brush shows.
#[allow(clippy::too_many_arguments)]
fn paint_area(
    brush: &BrushImage,
    place: &Placement,
    viewer: &Viewer,
    area: Rect,
    colors: usize,
    replace: Option<&Replace>,
    tile: Option<(i64, i64)>,
    max_depth: f64,
) -> Option<(BrushImage, (i32, i32))> {
    if area.is_empty() {
        return None;
    }
    let nothing = nothing_colour(brush, colors, replace);
    // What each pixel paints; `None` = nothing.
    let mut pixels = vec![None; area.w as usize * area.h as usize];
    // Bounds of the pixels that show something: x0..x1, y0..y1 (exclusive ends).
    let (mut x0, mut y0, mut x1, mut y1) = (area.w, area.h, 0, 0);
    for y in 0..area.h {
        for x in 0..area.w {
            let pixel = (area.x + x, area.y + y);
            let Some(p) = place
                .local(viewer, pixel, max_depth)
                .and_then(|uv| sample(brush, uv, tile))
            else {
                continue;
            };
            let p = match replace {
                Some(r) if r.right || Some(p) == brush.transparent => r.bg,
                Some(_) => p,
                // The brush's own transparent pixels (or, without one, those in the colour
                // standing for nothing, see `brush_ops::fill_colour`) paint nothing.
                None if Some(p) == nothing => continue,
                None => p,
            };
            pixels[(y * area.w + x) as usize] = Some(p);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    if x0 >= x1 {
        return None;
    }
    let shown = Rect::new(x0, y0, x1 - x0, y1 - y0);
    let mut out = BrushImage {
        width: shown.w as u32,
        height: shown.h as u32,
        pixels: Vec::with_capacity(shown.w as usize * shown.h as usize),
        handle: (0, 0),
        transparent: nothing,
    };
    for y in shown.y..shown.y + shown.h {
        for x in shown.x..shown.x + shown.w {
            let p = pixels[(y * area.w + x) as usize]
                .or(nothing)
                .unwrap_or_else(|| {
                    // No colour is free: the picture itself, painted over unchanged.
                    replace
                        .and_then(|r| r.page.get(area.x + x, area.y + y))
                        .unwrap_or(0)
                });
            out.pixels.push(p);
        }
    }
    Some((out, (area.x + shown.x, area.y + shown.y)))
}

/// The four corners of the brush in brush plane coordinates, `grow` pixels outside its edge
/// pixels' centres (0 = through the centres of its outermost pixels).
fn corners(brush: (u32, u32), handle: (i32, i32), grow: f64) -> [(f64, f64); 4] {
    let (w, h) = (f64::from(brush.0), f64::from(brush.1));
    let (hx, hy) = (f64::from(handle.0), f64::from(handle.1));
    let (l, t) = (-hx - grow, -hy - grow);
    let (r, b) = (w - 1.0 - hx + grow, h - 1.0 - hy + grow);
    [(l, t), (r, t), (r, b), (l, b)]
}

fn to_pixel(v: f64) -> i32 {
    v.floor().clamp(-MAX_COORD, MAX_COORD) as i32
}

/// The page pixels the turned brush can cover, cut to `clip`; all of `clip` when part of it
/// comes too close to the observer.
fn footprint(brush: &BrushImage, place: &Placement, viewer: &Viewer, clip: Rect) -> Rect {
    let mut pts = Vec::with_capacity(4);
    for c in corners((brush.width, brush.height), brush.handle, 0.5) {
        match viewer.project(place.point(c)) {
            Some(p) => pts.push(p),
            None => return clip,
        }
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in pts {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let (x0, y0) = (to_pixel(x0) - 1, to_pixel(y0) - 1);
    let (x1, y1) = (to_pixel(x1) + 1, to_pixel(y1) + 1);
    Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1).intersect(clip)
}

/// The brush turned and placed in space, as it appears on the page, cut to `clip` (the page):
/// a brush image with its handle at the top-left and where that top-left lies. Pixels the
/// brush does not cover are transparent (the brush's transparent colour, else an unused one,
/// see [`brush_ops::fill_colour`]). With `replace` the brush's own transparent pixels come
/// out in BG (Replace mode, painted as Matte, see [`Replace`]). `None` if nothing shows in
/// `clip`. Unturned on the screen plane it is exactly the brush.
pub fn project_brush(
    brush: &BrushImage,
    place: &Placement,
    viewer: &Viewer,
    clip: Rect,
    colors: usize,
    replace: Option<Replace>,
) -> Option<(BrushImage, (i32, i32))> {
    let area = footprint(brush, place, viewer, clip);
    paint_area(
        brush,
        place,
        viewer,
        area,
        colors,
        replace.as_ref(),
        None,
        f64::INFINITY,
    )
}

/// Perspective ▸ FillScreen: the brush plane covered with copies of the brush, `grid` pixels
/// apart along its x and y axes (Grid X and Y; the whole brush size by default, AGA p.278),
/// one of them where the brush is, as it appears in `area` (the screen), up to [`FAR`]
/// observer distances away. Like [`project_brush`] otherwise.
pub fn fill_plane(
    brush: &BrushImage,
    place: &Placement,
    viewer: &Viewer,
    grid: (u32, u32),
    area: Rect,
    colors: usize,
    replace: Option<Replace>,
) -> Option<(BrushImage, (i32, i32))> {
    let tile = (i64::from(grid.0.max(1)), i64::from(grid.1.max(1)));
    paint_area(
        brush,
        place,
        viewer,
        area,
        colors,
        replace.as_ref(),
        Some(tile),
        FAR * viewer.distance,
    )
}

/// Page pixels of the line from `a` to `b` that lie in `clip`.
fn line_into(out: &mut Vec<(i32, i32)>, a: (f64, f64), b: (f64, f64), clip: Rect) {
    let (a, b) = (
        (to_pixel(a.0), to_pixel(a.1)),
        (to_pixel(b.0), to_pixel(b.1)),
    );
    if let Some((i0, i1)) = crate::raster::clip_steps(a, b, clip) {
        for i in i0..=i1 {
            let p = crate::raster::line_point(a, b, i);
            if clip.contains(p.0, p.1) {
                out.push(p);
            }
        }
    }
}

/// The brush's wire frame as page pixels in `clip`: a frame one pixel outside the brush and
/// the cross through its handle along its axes (DeluxePaint's "four-cell matrix", AGA
/// p.278). Parts too close to the observer are left out.
pub fn wireframe(
    size: (u32, u32),
    handle: (i32, i32),
    place: &Placement,
    viewer: &Viewer,
    clip: Rect,
) -> Vec<(i32, i32)> {
    let [tl, tr, br, bl] = corners(size, handle, 1.0);
    let segments = [
        (tl, tr),
        (tr, br),
        (br, bl),
        (bl, tl),
        ((tl.0, 0.0), (tr.0, 0.0)),
        ((0.0, tl.1), (0.0, bl.1)),
    ];
    let mut out = Vec::new();
    for (a, b) in segments {
        let (mut a, mut b) = (place.point(a), place.point(b));
        // Cut the segment where it comes closer to the observer than NEAR.
        let depth = |p: Vec3| viewer.distance + p[2];
        let (da, db) = (depth(a), depth(b));
        if da < NEAR && db < NEAR {
            continue;
        }
        if da < NEAR || db < NEAR {
            let t = (NEAR - da) / (db - da);
            let cut = add_scaled(a, sub(b, a), t);
            if da < NEAR {
                a = cut;
            } else {
                b = cut;
            }
        }
        if let (Some(pa), Some(pb)) = (viewer.project(a), viewer.project(b)) {
            line_into(&mut out, pa, pb, clip);
        }
    }
    out
}

/// A cross hair at page pixel `at` with arms `arm` pixels long, cut to `clip` (the small one
/// marks the perspective centre; a long one is the pointer while the centre is placed).
pub fn cross(at: (i32, i32), arm: i32, clip: Rect) -> Vec<(i32, i32)> {
    let arm = arm.max(0);
    let mut out = Vec::with_capacity(4 * arm as usize + 1);
    let mut push = |p: (i32, i32)| {
        if clip.contains(p.0, p.1) {
            out.push(p);
        }
    };
    // The centre once, then both arms: every pixel once, in time linear in `arm` (the large
    // cross hair spans the page and is made on every redraw while the centre is placed).
    push(at);
    for d in (1..=arm).flat_map(|d| [-d, d]) {
        push((at.0 + d, at.1));
        push((at.0, at.1 + d));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush(w: u32, h: u32) -> BrushImage {
        // Every pixel its own colour (1..), colour 0 transparent at the top-left corner.
        let mut pixels: Vec<u8> = (0..w * h).map(|i| (i % 250 + 1) as u8).collect();
        pixels[0] = 0;
        BrushImage {
            width: w,
            height: h,
            pixels,
            handle: (w as i32 / 2, h as i32 / 2),
            transparent: Some(0),
        }
    }

    fn page() -> Rect {
        Rect::new(0, 0, 320, 256)
    }

    fn viewer() -> Viewer {
        Viewer::at_pixel((160, 128), 320.0)
    }

    /// The page pixels a projected brush paints (its non-transparent pixels).
    fn painted(p: &(BrushImage, (i32, i32))) -> Vec<((i32, i32), u8)> {
        let (b, (x0, y0)) = p;
        let mut out = Vec::new();
        for y in 0..b.height as i32 {
            for x in 0..b.width as i32 {
                if b.is_opaque(x, y) {
                    out.push(((x0 + x, y0 + y), b.get(x, y).unwrap()));
                }
            }
        }
        out
    }

    #[test]
    fn the_sine_table_is_exact_at_quarters_and_close_everywhere() {
        assert_eq!(sin_cos(0), (0.0, 1.0));
        assert_eq!(sin_cos(90), (1.0, 0.0));
        assert_eq!(sin_cos(180), (0.0, -1.0));
        assert_eq!(sin_cos(-90), (-1.0, 0.0));
        assert_eq!(sin_cos(30).0, 0.5);
        for d in -720..=720 {
            let (s, c) = sin_cos(d);
            let r = f64::from(d.rem_euclid(360)).to_radians();
            assert!((s - r.sin()).abs() < 1e-14, "sin {d}");
            assert!((c - r.cos()).abs() < 1e-14, "cos {d}");
            assert_eq!(sin_cos(-d).0, -s, "odd {d}");
        }
        assert_eq!(normalize(-180), 180);
        assert_eq!(normalize(181), -179);
        assert_eq!(normalize(-45), -45);
        assert_eq!(normalize(720), 0);
    }

    #[test]
    fn screen_angles_do_not_depend_on_the_order() {
        // AGA p.132-133: Shift-2 (+45 on z), then Shift-7 (-45 on x).
        let mut a = Orientation::default();
        a.rotate(Axis3::Z, 45, AngleType::Screen);
        a.rotate(Axis3::X, -45, AngleType::Screen);
        let mut b = Orientation::default();
        b.rotate(Axis3::X, -45, AngleType::Screen);
        b.rotate(Axis3::Z, 45, AngleType::Screen);
        assert_eq!(a, b);
        assert_eq!(a.angles(), [-45, 0, 45]);
        // The brush's x axis lies diagonally and still tilts straight back: its top edge
        // (towards -y) goes into the screen (+z).
        let up = a.matrix().apply([0.0, -1.0, 0.0]);
        assert!(up[2] > 0.0, "{up:?}");
        // Back to nothing.
        a.rotate(Axis3::Z, -45, AngleType::Screen);
        a.rotate(Axis3::X, 45, AngleType::Screen);
        assert!(a.is_identity());
    }

    #[test]
    fn brush_angles_turn_about_the_brush_and_show_screen_angles() {
        let mut o = Orientation::default();
        o.rotate(Axis3::Z, 45, AngleType::Brush);
        o.rotate(Axis3::X, -45, AngleType::Brush);
        // About the brush's own (diagonal) x axis: not the screen result (AGA p.134) ...
        assert_ne!(o.angles(), [-45, 0, 45]);
        // ... but the title bar shows screen angles that give the same orientation.
        let again = Mat3::euler(o.angles());
        for (r, row) in again.0.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!((v - o.matrix().0[r][c]).abs() < 0.02, "{again:?} {o:?}");
            }
        }
        // Turning back exactly is "not turned" again.
        o.rotate(Axis3::X, 45, AngleType::Brush);
        o.rotate(Axis3::Z, -45, AngleType::Brush);
        assert!(o.is_identity());
        assert_eq!(o.angles(), [0, 0, 0]);
        // A quarter turn about y (gimbal lock) still shows sensible angles.
        let mut o = Orientation::default();
        o.rotate(Axis3::Y, 90, AngleType::Brush);
        o.rotate(Axis3::Z, 30, AngleType::Brush);
        let again = Mat3::euler(o.angles());
        for (r, row) in again.0.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!((v - o.matrix().0[r][c]).abs() < 0.02, "{o:?}");
            }
        }
    }

    #[test]
    fn reset_axis_zeroes_one_angle() {
        let mut o = Orientation::from_angles([10, 20, 30]);
        o.reset_axis(Axis3::Y);
        assert_eq!(o.angles(), [10, 0, 30]);
        assert_eq!(o.matrix(), Mat3::euler([10, 0, 30]));
        o.reset_axis(Axis3::X);
        o.reset_axis(Axis3::Z);
        assert!(o.is_identity());
    }

    #[test]
    fn unturned_on_the_screen_it_is_exactly_the_brush() {
        let b = brush(7, 5);
        let place = Placement::at_pixel((100, 60));
        assert!(place.on_screen_plane());
        let p = project_brush(&b, &place, &viewer(), page(), 256, None).unwrap();
        assert_eq!(p.1, (100 - 3, 60 - 2));
        let mut want = Vec::new();
        for j in 0..5 {
            for i in 0..7 {
                if (i, j) != (0, 0) {
                    want.push(((97 + i, 58 + j), b.get(i, j).unwrap()));
                }
            }
        }
        assert_eq!(painted(&p), want);
        // A brush without a transparent colour: every pixel shows, the rest is "nothing".
        let mut solid = b.clone();
        solid.transparent = None;
        let p = project_brush(&solid, &place, &viewer(), page(), 256, None).unwrap();
        assert_eq!(painted(&p).len(), 35);
        assert_ne!(p.0.transparent, Some(0));
    }

    #[test]
    fn a_quarter_turn_about_z_is_clockwise_on_the_screen() {
        let b = brush(5, 3);
        let mut place = Placement::at_pixel((50, 50));
        place.matrix = Mat3::rotation(Axis3::Z, 90);
        let p = project_brush(&b, &place, &viewer(), page(), 256, None).unwrap();
        let got = painted(&p);
        // Brush pixel (u, v) from the handle appears at (-v, u).
        for j in 0..3 {
            for i in 0..5 {
                if (i, j) == (0, 0) {
                    continue;
                }
                let (u, v) = (i - 2, j - 1);
                let at = (50 - v, 50 + u);
                assert!(got.contains(&(at, b.get(i, j).unwrap())), "{i},{j}");
            }
        }
        assert_eq!(got.len(), 14);
    }

    #[test]
    fn farther_away_is_smaller_towards_the_centre() {
        let b = BrushImage {
            width: 8,
            height: 8,
            pixels: vec![5; 64],
            handle: (4, 4),
            transparent: Some(0),
        };
        let v = viewer();
        // Handle on the perspective centre, one observer distance behind the screen: half size.
        let mut place = Placement::at_pixel((160, 128));
        place.position[2] = 320.0;
        let p = project_brush(&b, &place, &v, page(), 32, None).unwrap();
        let got = painted(&p);
        assert_eq!(got.len(), 16, "{got:?}");
        let xs: Vec<i32> = got.iter().map(|((x, _), _)| *x).collect();
        assert_eq!((xs.iter().min(), xs.iter().max()), (Some(&158), Some(&161)));
        // Off the centre it also moves towards the centre.
        let mut place = Placement::at_pixel((260, 128));
        place.position[2] = 320.0;
        assert_eq!(place.shown_position(), (260, 128, 320));
        let p = project_brush(&b, &place, &v, page(), 32, None).unwrap();
        let xs: Vec<i32> = painted(&p).iter().map(|((x, _), _)| *x).collect();
        let mid = (xs.iter().min().unwrap() + xs.iter().max().unwrap()) / 2;
        assert!((209..=211).contains(&mid), "{mid}");
    }

    /// The colour a projected brush paints on page pixel (`x`, `y`), if any.
    fn shows(p: &(BrushImage, (i32, i32)), x: i32, y: i32) -> Option<u8> {
        let (b, (x0, y0)) = p;
        b.is_opaque(x - x0, y - y0)
            .then(|| b.get(x - x0, y - y0).unwrap())
    }

    #[test]
    fn a_floor_reaches_the_horizon_and_no_further() {
        // AGA p.137: the brush at the bottom, turned -90 on x, fills a floor up to the
        // horizon at the centre's height.
        let b = brush(16, 16);
        let v = viewer();
        let mut place = Placement::at_pixel((160, 240));
        place.matrix = Mat3::rotation(Axis3::X, -90);
        let plane = fill_plane(&b, &place, &v, (16, 16), page(), 256, None).unwrap();
        let row_painted = |y: i32| (0..320).any(|x| shows(&plane, x, y).is_some());
        assert!(!row_painted(100), "above the horizon");
        assert!(!row_painted(128), "the horizon itself");
        assert!(row_painted(140) && row_painted(200) && row_painted(255));
        assert_eq!((plane.1, plane.0.width), ((0, 130), 320), "up to FAR");
        // The row under the handle shows the brush's middle row at full size there.
        let mid: Vec<Option<u8>> = (152..168).map(|x| shows(&plane, x, 240)).collect();
        let want: Vec<Option<u8>> = (0..16).map(|i| b.get(i, 8)).collect();
        assert_eq!(mid, want);
        // The same pixels every time.
        let again = fill_plane(&b, &place, &v, (16, 16), page(), 256, None).unwrap();
        assert_eq!(again, plane);
    }

    #[test]
    fn fill_plane_leaves_gaps_when_the_grid_is_wider_than_the_brush() {
        let b = brush(4, 4);
        let place = Placement::at_pixel((10, 10));
        let plane = fill_plane(&b, &place, &viewer(), (6, 4), page(), 256, None).unwrap();
        // Unturned on the screen: a flat pattern; columns 4 and 5 of every 6 stay empty.
        for x in 0..24 {
            let empty = !(0..4).any(|y| shows(&plane, x, 12 + y).is_some());
            let col = (x - 8).rem_euclid(6);
            assert_eq!(empty, col >= 4, "column {x}");
        }
    }

    #[test]
    fn replace_paints_the_brush_transparent_pixels_in_bg_only_inside_the_brush() {
        let b = brush(3, 3);
        let mut place = Placement::at_pixel((20, 20));
        place.matrix = Mat3::rotation(Axis3::Z, 45);
        let picture = IndexedImage::new(320, 256, 0);
        let replace = Replace {
            bg: 9,
            right: false,
            page: &picture,
        };
        let (img, at) = project_brush(&b, &place, &viewer(), page(), 32, Some(replace)).unwrap();
        let nothing = img.transparent.unwrap();
        assert_ne!(nothing, 9);
        assert!(
            b.pixels
                .iter()
                .all(|&c| Some(c) == b.transparent || c != nothing),
            "no brush colour is lost"
        );
        assert!(
            img.pixels.contains(&9),
            "the transparent corner shows in BG"
        );
        // Outside the turned square nothing is painted (the corners of the box).
        assert_eq!(img.get(0, 0), Some(nothing), "{at:?}");
    }

    /// A picture in every colour of a `colors`-colour palette, as a striped pattern.
    fn picture(colors: u8) -> IndexedImage {
        let mut p = IndexedImage::new(64, 48, 0);
        for y in 0..48 {
            for x in 0..64 {
                p.set(x, y, ((x * 3 + y * 5) % i32::from(colors)) as u8);
            }
        }
        p
    }

    /// `stamp` painted on `picture` the way the app stamps in Replace mode: one Matte
    /// `Stroke` (with `right` the shape in BG) through the engine, on a palette of `rgb`.
    fn stamped(
        picture: &IndexedImage,
        rgb: &[u32],
        (img, at): (BrushImage, (i32, i32)),
        bg: u8,
        right: bool,
        fx: crate::PaintFx,
    ) -> IndexedImage {
        let colors = rgb
            .iter()
            .map(|&c| crate::Rgb8::new((c >> 16) as u8, (c >> 8) as u8, c as u8))
            .collect();
        let palette = crate::Palette::new(colors).unwrap();
        let (w, h) = (picture.width(), picture.height());
        let mut engine = crate::Engine::new(crate::Document::new(w, h, palette));
        let pixels = picture.pixels().to_vec();
        let set = crate::Command::SetPage {
            width: w,
            height: h,
            pixels,
        };
        engine.execute(set).unwrap();
        let stamp = crate::Command::Stroke {
            brush: Vec::new(),
            color: bg,
            points: vec![at],
            continuous: false,
            custom: Some(img),
            mode: crate::DrawMode::Matte,
            range: None,
            stencil: None,
            right,
            fx,
        };
        engine.execute(stamp).unwrap();
        engine.document().page().clone()
    }

    /// What Replace should leave: the reference projection (on a palette with colours to
    /// spare, so it has a transparent colour) painted over `picture` pixel by pixel.
    fn replaced(
        picture: &IndexedImage,
        b: &BrushImage,
        place: &Placement,
        bg: u8,
        right: bool,
    ) -> IndexedImage {
        let replace = Replace {
            bg,
            right,
            page: picture,
        };
        let (img, at) =
            project_brush(b, place, &viewer(), picture.bounds(), 256, Some(replace)).unwrap();
        assert!(img.transparent.is_some());
        let mut want = picture.clone();
        let mut outside = 0;
        for y in 0..img.height as i32 {
            for x in 0..img.width as i32 {
                if img.is_opaque(x, y) {
                    let c = if right { bg } else { img.get(x, y).unwrap() };
                    want.set(at.0 + x, at.1 + y, c);
                } else {
                    outside += 1;
                }
            }
        }
        assert!(outside > 0, "the turned brush leaves corners of its box");
        want
    }

    #[test]
    fn replace_works_when_the_brush_and_bg_use_the_whole_palette() {
        let mut place = Placement::at_pixel((30, 22));
        place.matrix = Orientation::from_angles([-30, 0, 30]).matrix();
        let rgb4 = [0x000000, 0xff2020, 0x20ff20, 0xffffff];
        // 4 colours: a brush in all of them (0 transparent), BG 0. No colour is left to stand
        // for nothing, so the stamp repeats the picture around the brush.
        let b4 = BrushImage {
            width: 8,
            height: 6,
            pixels: (0..48u8).map(|i| (i * 5 + i / 8) % 4).collect(),
            handle: (4, 3),
            transparent: Some(0),
        };
        let pic4 = picture(4);
        let replace = Replace {
            bg: 0,
            right: false,
            page: &pic4,
        };
        let stamp = project_brush(&b4, &place, &viewer(), pic4.bounds(), 4, Some(replace));
        let stamp = stamp.unwrap();
        assert_eq!(stamp.0.transparent, None, "no colour is free");
        let plain = crate::PaintFx::default();
        let got = stamped(&pic4, &rgb4, stamp.clone(), 0, false, plain);
        assert_eq!(got, replaced(&pic4, &b4, &place, 0, false));
        assert_ne!(got, pic4);
        // Translucency, Process and AntiAlias leave the picture around the brush as it was.
        let fx = crate::PaintFx {
            translucency: Some(50),
            process: Some(crate::ProcessMode::Tint),
            antialias: crate::AntiAlias::High,
        };
        let soft = stamped(&pic4, &rgb4, stamp, 0, false, fx);
        let want = replaced(&pic4, &b4, &place, 0, false);
        for y in 0..48 {
            for x in 0..64 {
                if want.get(x, y) == pic4.get(x, y) {
                    assert_eq!(soft.get(x, y), pic4.get(x, y), "({x}, {y})");
                }
            }
        }
        // 2 colours: any brush with BG 0 uses both.
        let b2 = BrushImage {
            width: 7,
            height: 5,
            pixels: (0..35u8).map(|i| u8::from(i % 3 != 0)).collect(),
            handle: (3, 2),
            transparent: Some(0),
        };
        let pic2 = picture(2);
        let rgb2 = [0x000000, 0xffffff];
        for right in [false, true] {
            let replace = Replace {
                bg: 0,
                right,
                page: &pic2,
            };
            let stamp = project_brush(&b2, &place, &viewer(), pic2.bounds(), 2, Some(replace));
            let plain = crate::PaintFx::default();
            let got = stamped(&pic2, &rgb2, stamp.unwrap(), 0, right, plain);
            let want = replaced(&pic2, &b2, &place, 0, right);
            assert_eq!(got, want, "right {right}");
        }
    }

    #[test]
    fn sliding_follows_the_pointer_on_the_brush_plane() {
        let v = viewer();
        let place = Placement::at_pixel((100, 200));
        // Fixed z, unturned: exactly under the pointer.
        let p = slide(&place, &v, Axis3::Z, (30, 40)).unwrap();
        assert_eq!(p, [30.5, 40.5, 0.0]);
        // Fixed y (Ctrl): the pointer moving up towards the horizon moves the brush away.
        let near = slide(&place, &v, Axis3::Y, (100, 200)).unwrap();
        let far = slide(&place, &v, Axis3::Y, (100, 150)).unwrap();
        assert_eq!(near[2], 0.0);
        assert!(far[2] > 100.0, "{far:?}");
        assert_eq!(far[1], 200.5, "stays on its height");
        // Above the horizon the floor cannot be reached: the brush stays.
        assert_eq!(slide(&place, &v, Axis3::Y, (100, 100)), None);
    }

    #[test]
    fn snapping_keeps_the_fixed_axis() {
        let m = Mat3::IDENTITY;
        let origin = [0.5, 0.5, 0.0];
        let p = snap([37.5, 22.5, 13.0], origin, &m, [16, 8, 10], Some(Axis3::Z));
        assert_eq!(p, [32.5, 24.5, 13.0]);
        let p = snap([37.5, 22.5, 13.0], origin, &m, [16, 8, 10], None);
        assert_eq!(p, [32.5, 24.5, 10.0]);
    }

    #[test]
    fn the_wire_frame_surrounds_the_brush_and_crosses_its_handle() {
        let place = Placement::at_pixel((50, 40));
        let px = wireframe((5, 3), (2, 1), &place, &viewer(), page());
        // Frame one pixel outside: x 47..=53, y 38..=42.
        for x in 47..=53 {
            assert!(px.contains(&(x, 38)) && px.contains(&(x, 42)), "{x}");
        }
        for y in 38..=42 {
            assert!(px.contains(&(47, y)) && px.contains(&(53, y)), "{y}");
        }
        // Cross through the handle.
        assert!(px.contains(&(50, 39)) && px.contains(&(49, 40)) && px.contains(&(51, 40)));
        assert!(!px.contains(&(49, 39)), "cells stay open");
        // Turned so that it reaches behind the observer: no panic, only what is in front.
        let mut place = Placement::at_pixel((160, 250));
        place.matrix = Mat3::rotation(Axis3::X, 90);
        let far = wireframe((400, 400), (200, 200), &place, &viewer(), page());
        assert!(far.iter().all(|&(x, y)| page().contains(x, y)));
    }

    #[test]
    fn a_brush_partly_behind_the_observer_draws_only_its_front() {
        let b = BrushImage {
            width: 64,
            height: 640,
            pixels: vec![3; 64 * 640],
            handle: (32, 320),
            transparent: Some(0),
        };
        let mut place = Placement::at_pixel((160, 250));
        place.matrix = Mat3::rotation(Axis3::X, -90);
        let (img, at) = project_brush(&b, &place, &viewer(), page(), 32, None).unwrap();
        assert!(at.1 >= 128, "below the horizon only: {at:?}");
        assert!(img.pixels.contains(&3));
        // Entirely off the page: nothing.
        let off = Placement::at_pixel((-500, -500));
        assert!(project_brush(&b, &off, &viewer(), page(), 32, None).is_none());
    }

    #[test]
    fn the_cross_hair_is_cut_to_the_page() {
        let c = cross((1, 1), 3, page());
        assert!(c.contains(&(1, 1)) && c.contains(&(4, 1)) && c.contains(&(1, 4)));
        assert!(c.contains(&(0, 1)) && !c.iter().any(|&(x, y)| x < 0 || y < 0));
        assert_eq!(c.len(), 9);
        let mut sorted = c.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), c.len(), "every pixel once");
    }

    /// The large cross hair is made on every redraw while the centre is placed: across the
    /// largest page it takes well under a frame (the old duplicate check made it quadratic,
    /// about 0.2 s here).
    #[test]
    fn the_large_cross_hair_is_quick_on_the_largest_page() {
        let side = crate::command::MAX_PAGE_SIDE as i32;
        let clip = Rect::new(0, 0, side, side);
        let start = std::time::Instant::now();
        let c = cross((side / 2, side / 2), side, clip);
        let took = start.elapsed();
        assert_eq!(c.len(), 2 * side as usize - 1);
        assert!(took < std::time::Duration::from_millis(20), "{took:?}");
    }

    /// Pins the pixels of one turned, distant stamp and one plane, so any change of the
    /// arithmetic (or a platform that computes differently) shows.
    #[test]
    fn projected_pixels_are_pinned() {
        let b = brush(12, 9);
        let mut place = Placement::at_pixel((90, 200));
        let mut o = Orientation::default();
        o.rotate(Axis3::X, -60, AngleType::Screen);
        o.rotate(Axis3::Z, 17, AngleType::Screen);
        place.matrix = o.matrix();
        place.position[2] = 37.0;
        let v = Viewer::at_pixel((150, 90), 280.0);
        let stamp = project_brush(&b, &place, &v, page(), 256, None).unwrap();
        let plane = fill_plane(&b, &place, &v, (14, 10), page(), 256, None).unwrap();
        let fnv = |bytes: &[u8]| {
            bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            })
        };
        let got = (
            stamp.1,
            stamp.0.width,
            stamp.0.height,
            fnv(&stamp.0.pixels),
            fnv(&plane.0.pixels),
            plane.1,
            (plane.0.width, plane.0.height),
        );
        assert_eq!(
            got,
            (
                (91, 184),
                12,
                8,
                4_816_778_729_831_156_136,
                17_624_507_543_087_497_336,
                (0, 0),
                (320, 256)
            )
        );
    }
}
