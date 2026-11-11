//! Anim ▸ Move.. (ADR-38): a brush moved and rotated in three dimensions over a number of
//! stamps, one stamp per animation frame (AGA p.252-255, tutorial p.151-163).
//!
//! The pure parts live here: the eased path ([`plan`]), the perspective projection of the
//! brush for one stamp ([`project`], [`outline`]) and the frames the stamps go to. The
//! [`crate::Command::MoveBrush`] command paints a whole move as one undo step through
//! [`validate`] and [`apply`]; the front-ends use the same functions for the wire-frame
//! preview, so the preview shows what Draw paints.
//!
//! Coordinates follow the page: X to the right, Y down, Z away from the viewer (0 = the
//! screen plane, where the brush has its own size). The Move requester's Y distance counts
//! upwards, as in DeluxePaint, so it is negated here. Rotations are in degrees: a positive Z
//! angle turns the brush clockwise on screen, a positive X angle tilts its top away, a
//! positive Y angle turns its right side away. The projection looks from [`EYE`] pixels in
//! front of the screen at the centre of the page.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::brush_ops;
use crate::command::{
    Command, CommandError, MAX_BRUSH_OFFSET, MAX_BRUSH_PIXELS, MAX_COORD, MAX_PAINT_WORK,
};
use crate::document::Document;
use crate::image::{IndexedImage, Rect};
use crate::paint::{self, BrushImage, DrawMode, PaintContext, PaintFx};
use crate::palette::Palette;
use crate::raster::{self, Pen};

/// Most stamps one move may paint (the Count field).
pub const MAX_COUNT: u32 = 999;
/// Most frames of Ease-Out or Ease-In.
pub const MAX_EASE: u32 = 999;
/// Largest distance (pixels) or angle (degrees) of one axis.
pub const MAX_AMOUNT: i32 = 99_999;
/// Distance of the viewer from the screen plane, in page pixels. The manual does not give
/// DeluxePaint's value; one LoRes screen width gives a moderate perspective (a brush 320
/// pixels away shows at half its size). Decided without the owner (ADR-38).
pub const EYE: f64 = 320.0;
/// Corners closer to the viewer than this (in pixels) are not drawn: the brush would be
/// behind the eye or infinitely large.
const NEAR: f64 = 1.0;

/// Direction ▸ Record: the order in which the frames of the move are painted (AGA p.254).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Record {
    /// Stepping forward from the current frame (DeluxePaint's default).
    #[default]
    Forward,
    /// Every stamp on the current frame.
    InPlace,
    /// The same frames, painted from the last one back (matters with Trails).
    Backward,
}

/// The Move requester's settings (AGA p.252-255).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MovePath {
    /// Dist X, Y, Z in pixels: X to the right, Y **up**, Z away from the viewer.
    pub dist: [i32; 3],
    /// Angle X, Y, Z in degrees (Z positive = clockwise).
    pub angle: [i32; 3],
    /// Dist ▸ Brush: move along the brush's own (turning) axes instead of the screen's.
    #[serde(default)]
    pub dist_brush: bool,
    /// Angle ▸ Brush: rotate about the brush's own axes (DeluxePaint's default); off = the
    /// screen axes, Euler order X, Y, Z.
    #[serde(default = "yes")]
    pub angle_brush: bool,
    /// Cyclic: the move completes one stamp after the last, so a looping animation has no
    /// repeated picture (DeluxePaint's default).
    #[serde(default = "yes")]
    pub cyclic: bool,
    /// Ease-Out: frames over which the brush speeds up at the start.
    #[serde(default)]
    pub ease_out: u32,
    /// Ease-In: frames over which the brush slows down at the end.
    #[serde(default)]
    pub ease_in: u32,
    /// Count: how many stamps paint the move (1..=[`MAX_COUNT`]).
    pub count: u32,
    /// Direction ▸ Move: `false` = Go From (the move starts where the brush was stamped),
    /// `true` = Come To (it ends there, on the current frame).
    #[serde(default)]
    pub come_to: bool,
    /// Direction ▸ Record.
    #[serde(default)]
    pub record: Record,
    /// Trails: every frame also gets all stamps painted before it in this move.
    #[serde(default)]
    pub trails: bool,
}

fn yes() -> bool {
    true
}

impl Default for MovePath {
    /// DeluxePaint's defaults: no distance or angle, Dist on the screen axes, Angle on the
    /// brush axes, Cyclic on, no easing, Go From, Forward (AGA p.252-254, tutorial p.154).
    fn default() -> Self {
        Self {
            dist: [0; 3],
            angle: [0; 3],
            dist_brush: false,
            angle_brush: true,
            cyclic: true,
            ease_out: 0,
            ease_in: 0,
            count: 1,
            come_to: false,
            record: Record::Forward,
            trails: false,
        }
    }
}

/// A 3×3 rotation matrix, row-major; it turns brush axes into page axes.
pub type Matrix = [[f64; 3]; 3];

const IDENTITY: Matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Where the brush is at one step of a move: the page position of the centre of its handle
/// pixel (x, y; z = depth) and its orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pos: [f64; 3],
    pub rot: Matrix,
}

impl Pose {
    /// The brush as stamped at page pixel (`x`, `y`) and depth `z`, not rotated.
    pub fn at((x, y, z): (i32, i32, i32)) -> Self {
        Self {
            pos: [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z)],
            rot: IDENTITY,
        }
    }

    /// The page pixel and depth of the handle, rounded (to start the next move from).
    pub fn pixel(&self) -> (i32, i32, i32) {
        let r = |v: f64, bound: f64| v.clamp(-bound, bound) as i32;
        let max = f64::from(MAX_COORD);
        (
            r(self.pos[0].floor(), max),
            r(self.pos[1].floor(), max),
            r(self.pos[2].round(), max),
        )
    }
}

/// One stamp of a move: the frame it is painted on and the brush's pose there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
    pub frame: usize,
    pub pose: Pose,
}

/// A move worked out for a document: the stamps in painting order, the frame that is current
/// afterwards and where the brush ends (the start of the next move, unless Go Back).
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub stamps: Vec<Stamp>,
    pub end_frame: usize,
    pub end: Pose,
}

impl Plan {
    /// The stamps painted on stamp `i`'s frame (painting order): with Trails all stamps up
    /// to `i`, else stamp `i` alone. In Place (one frame) never repeats stamps.
    pub fn layers(&self, path: &MovePath, i: usize) -> std::ops::RangeInclusive<usize> {
        let one_frame = self.stamps.iter().all(|s| s.frame == self.stamps[0].frame);
        if path.trails && !one_frame {
            0..=i
        } else {
            i..=i
        }
    }
}

/// `(sin, cos)` of `degrees`, exact for multiples of 90.
fn sin_cos(degrees: f64) -> (f64, f64) {
    let d = degrees.rem_euclid(360.0);
    if d == 0.0 {
        (0.0, 1.0)
    } else if d == 90.0 {
        (1.0, 0.0)
    } else if d == 180.0 {
        (0.0, -1.0)
    } else if d == 270.0 {
        (-1.0, 0.0)
    } else {
        d.to_radians().sin_cos()
    }
}

fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn apply_m(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// Rotation by `deg` = [x, y, z] degrees about the X, then the Y, then the Z axis (in that
/// order about fixed axes), with the sign conventions of the module documentation.
fn rotation(deg: [f64; 3]) -> Matrix {
    let (sx, cx) = sin_cos(deg[0]);
    let (sy, cy) = sin_cos(deg[1]);
    let (sz, cz) = sin_cos(deg[2]);
    // Top (−y) away (+z).
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, sx], [0.0, -sx, cx]];
    // Right (+x) away (+z).
    let ry = [[cy, 0.0, -sy], [0.0, 1.0, 0.0], [sy, 0.0, cy]];
    // Clockwise on the page (y down).
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mul(&rz, &mul(&ry, &rx))
}

/// The eased share of the whole move done after `i` of `n` steps: the brush speeds up over
/// the first `a` steps (Ease-Out) and slows down over the last `b` (Ease-In), with constant
/// acceleration, and moves evenly in between. Ease steps that do not fit are shortened in
/// proportion. The manual gives no formula; this one is the usual trapezoid speed profile.
pub fn progress(i: u32, n: u32, ease_out: u32, ease_in: u32) -> f64 {
    if n == 0 || i == 0 {
        return 0.0;
    }
    if i >= n {
        return 1.0;
    }
    let (i, n) = (f64::from(i), f64::from(n));
    let (mut a, mut b) = (f64::from(ease_out), f64::from(ease_in));
    if a + b > n {
        let s = n / (a + b);
        a *= s;
        b *= s;
    }
    let v = 1.0 / (n - (a + b) / 2.0);
    let p = if i < a {
        v * i * i / (2.0 * a)
    } else if i <= n - b {
        v * (i - a / 2.0)
    } else {
        1.0 - v * (n - i) * (n - i) / (2.0 * b)
    };
    p.clamp(0.0, 1.0)
}

/// The `n + 1` poses of a move of `dist` / `angle` from `start` (step 0) over `n` steps.
fn walk(
    path: &MovePath,
    dist: [f64; 3],
    angle: [f64; 3],
    eases: (u32, u32),
    n: u32,
    start: Pose,
) -> Vec<Pose> {
    let mut poses = Vec::with_capacity(n as usize + 1);
    let mut pose = start;
    poses.push(pose);
    let mut done = 0.0;
    for k in 1..=n {
        let p = progress(k, n, eases.0, eases.1);
        let dp = p - done;
        done = p;
        // Distance: page axes (Y up in the requester), or the brush's axes at the start of
        // this step.
        let step = [dist[0] * dp, -dist[1] * dp, dist[2] * dp];
        let step = if path.dist_brush {
            apply_m(&pose.rot, step)
        } else {
            step
        };
        for (c, s) in pose.pos.iter_mut().zip(step) {
            *c += s;
        }
        pose.rot = if path.angle_brush {
            // About the brush's own axes, turned a little more every step.
            mul(&pose.rot, &rotation(angle.map(|a| a * dp)))
        } else {
            // About the screen axes: the whole angle so far, then the starting orientation.
            mul(&rotation(angle.map(|a| a * p)), &start.rot)
        };
        poses.push(pose);
    }
    poses
}

/// Works out the move `path` for a document of `frames` frames whose frame `current` is
/// shown, the brush stamped at `start` (page x, y and depth z).
///
/// - Count stamps are painted. Without Cyclic the path is split into `count − 1` steps and
///   the last stamp shows the whole move; with Cyclic into `count` steps and the end is not
///   painted (it is where a looping animation starts again, AGA p.254; tutorial p.137-139).
/// - Go From starts at `start` on the current frame; Come To ends at `start` on the current
///   frame and starts `count − 1` frames before it (the brush there has its stamped
///   orientation; with Cyclic the start of the path is the part left out).
/// - Frames wrap around the animation. With one frame, or Record ▸ In Place, every stamp goes
///   to the current frame.
/// - Afterwards the frame of the last stamp painted is current; Forward Go From with Cyclic
///   moves on to the frame after it (AGA p.140 of the tutorial).
pub fn plan(path: &MovePath, start: (i32, i32, i32), frames: usize, current: usize) -> Plan {
    let count = path.count.clamp(1, MAX_COUNT);
    let n = if path.cyclic { count } else { count - 1 };
    let dist = path.dist.map(f64::from);
    let angle = path.angle.map(f64::from);
    let origin = Pose::at(start);
    let poses = if path.come_to {
        // Walk back from the stamped brush with the move reversed, then turn the walk round.
        let neg = |v: [f64; 3]| v.map(|c| -c);
        let mut back = walk(
            path,
            neg(dist),
            neg(angle),
            (path.ease_in, path.ease_out),
            n,
            origin,
        );
        back.reverse();
        back
    } else {
        walk(path, dist, angle, (path.ease_out, path.ease_in), n, origin)
    };
    // The painted part of the path, in path order.
    let painted: &[Pose] = match (path.cyclic, path.come_to) {
        (true, false) => &poses[..n as usize],
        (true, true) => &poses[1..],
        (false, _) => &poses[..],
    };
    let frames = frames.max(1);
    let current = current.min(frames - 1);
    let in_place = frames == 1 || path.record == Record::InPlace;
    let last = painted.len() - 1;
    let frame_of = |j: usize| -> usize {
        if in_place {
            return current;
        }
        let f = frames as i64;
        let offset = if path.come_to {
            j as i64 - last as i64
        } else {
            j as i64
        };
        (current as i64 + offset).rem_euclid(f) as usize
    };
    let mut stamps: Vec<Stamp> = painted
        .iter()
        .enumerate()
        .map(|(j, &pose)| Stamp {
            frame: frame_of(j),
            pose,
        })
        .collect();
    let end_frame = if in_place {
        current
    } else if path.record == Record::Backward {
        stamps.reverse();
        stamps.last().map_or(current, |s| s.frame)
    } else if path.cyclic && !path.come_to {
        (stamps.last().map_or(current, |s| s.frame) + 1) % frames
    } else {
        stamps.last().map_or(current, |s| s.frame)
    };
    Plan {
        stamps,
        end_frame,
        end: *poses.last().expect("at least one pose"),
    }
}

/// The page position of brush point (`u`, `v`) (pixels from the centre of the handle pixel)
/// in `pose`, and its homogeneous weight (`EYE` + depth); `centre` = the point the viewer
/// looks at.
fn corner(pose: &Pose, centre: (f64, f64), u: f64, v: f64) -> ((f64, f64), f64) {
    let q = [0, 1, 2].map(|i| pose.pos[i] + pose.rot[i][0] * u + pose.rot[i][1] * v);
    let w = EYE + q[2];
    let s = (
        centre.0 + EYE * (q[0] - centre.0) / w,
        centre.1 + EYE * (q[1] - centre.1) / w,
    );
    (s, w)
}

/// The four page corners of a `width`×`height` brush with its handle at `handle` in `pose`
/// (top-left, top-right, bottom-right, bottom-left), or `None` when a corner is at or
/// behind the viewer. `centre`: where the viewer looks (the page centre).
pub fn outline(
    (width, height): (u32, u32),
    handle: (i32, i32),
    pose: &Pose,
    centre: (f64, f64),
) -> Option<[(f64, f64); 4]> {
    let (hx, hy) = (f64::from(handle.0), f64::from(handle.1));
    let (u0, v0) = (-hx - 0.5, -hy - 0.5);
    let (u1, v1) = (f64::from(width) - hx - 0.5, f64::from(height) - hy - 0.5);
    let mut out = [(0.0, 0.0); 4];
    for (o, (u, v)) in out.iter_mut().zip([(u0, v0), (u1, v0), (u1, v1), (u0, v1)]) {
        let (s, w) = corner(pose, centre, u, v);
        if w < NEAR {
            return None;
        }
        *o = s;
    }
    Some(out)
}

/// The page pixels the brush covers in `pose` (bounding box of [`outline`], clipped to
/// `clip`); empty when nothing shows.
fn cover(
    size: (u32, u32),
    handle: (i32, i32),
    pose: &Pose,
    centre: (f64, f64),
    clip: Rect,
) -> Rect {
    let Some(c) = outline(size, handle, pose, centre) else {
        return Rect::default();
    };
    let lim = |v: f64| v.clamp(-1e9, 1e9);
    let x0 = lim(c.iter().map(|p| p.0).fold(f64::MAX, f64::min)).floor() as i64;
    let y0 = lim(c.iter().map(|p| p.1).fold(f64::MAX, f64::min)).floor() as i64;
    let x1 = lim(c.iter().map(|p| p.0).fold(f64::MIN, f64::max)).ceil() as i64;
    let y1 = lim(c.iter().map(|p| p.1).fold(f64::MIN, f64::max)).ceil() as i64;
    let (cx0, cy0) = (i64::from(clip.x), i64::from(clip.y));
    let (cx1, cy1) = (cx0 + i64::from(clip.w), cy0 + i64::from(clip.h));
    let (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
    if x1 <= x0 || y1 <= y0 {
        return Rect::default();
    }
    Rect::new(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32)
}

fn invert(m: &Matrix) -> Option<Matrix> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-9 {
        return None;
    }
    let c =
        |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    Some([
        [
            c(1, 1, 2, 2) / det,
            -c(0, 1, 2, 2) / det,
            c(0, 1, 1, 2) / det,
        ],
        [
            -c(1, 0, 2, 2) / det,
            c(0, 0, 2, 2) / det,
            -c(0, 0, 1, 2) / det,
        ],
        [
            c(1, 0, 2, 1) / det,
            -c(0, 0, 2, 1) / det,
            c(0, 0, 1, 1) / det,
        ],
    ])
}

/// The brush `b` as it shows in `pose` (perspective, nearest neighbour), cut to the page
/// rectangle `clip`: a brush whose handle is its top-left pixel, to be stamped at the
/// returned page position. Pixels outside the brush get `fill`, which becomes the result's
/// transparent colour: the brush's own transparent colour, or one it does not use
/// ([`brush_ops::fill_colour`]; the caller works it out once per move, as it reads the whole
/// brush). `None` when nothing of it shows (off the page, behind the viewer or seen
/// edge-on).
pub fn project(
    b: &BrushImage,
    pose: &Pose,
    centre: (f64, f64),
    clip: Rect,
    fill: u8,
) -> Option<(BrushImage, (i32, i32))> {
    let area = cover((b.width, b.height), b.handle, pose, centre, clip);
    if area.is_empty() {
        return None;
    }
    // Brush plane point (u, v) → homogeneous page point: a projective map H.
    let (a, bv, p) = (
        [pose.rot[0][0], pose.rot[1][0], pose.rot[2][0]],
        [pose.rot[0][1], pose.rot[1][1], pose.rot[2][1]],
        pose.pos,
    );
    let row = |i: usize, c: f64| {
        [
            EYE * a[i] + c * a[2],
            EYE * bv[i] + c * bv[2],
            EYE * p[i] + c * p[2],
        ]
    };
    let h = [
        row(0, centre.0),
        row(1, centre.1),
        [a[2], bv[2], EYE + p[2]],
    ];
    let inv = invert(&h)?;
    let (w, hgt) = (area.w as u32, area.h as u32);
    let mut pixels = vec![fill; w as usize * hgt as usize];
    let (hx, hy) = (f64::from(b.handle.0), f64::from(b.handle.1));
    for y in 0..area.h {
        let sy = f64::from(area.y + y) + 0.5;
        for x in 0..area.w {
            let sx = f64::from(area.x + x) + 0.5;
            let r = apply_m(&inv, [sx, sy, 1.0]);
            if r[2].abs() < 1e-12 {
                continue;
            }
            let (u, v) = (r[0] / r[2], r[1] / r[2]);
            let (bx, by) = ((u + hx + 0.5).floor(), (v + hy + 0.5).floor());
            if bx < 0.0 || by < 0.0 || bx >= f64::from(b.width) || by >= f64::from(b.height) {
                continue;
            }
            pixels[(y * area.w + x) as usize] =
                b.pixels[by as usize * b.width as usize + bx as usize];
        }
    }
    let out = BrushImage {
        width: w,
        height: hgt,
        pixels,
        handle: (0, 0),
        transparent: Some(fill),
    };
    Some((out, (area.x, area.y)))
}

/// A built-in brush (pixel offsets from its hotspot, painted in `color`) as a custom brush
/// whose other pixels are transparent, so it can be projected. `None` for no offsets.
pub fn brush_of_offsets(offsets: &[(i32, i32)], color: u8) -> Option<BrushImage> {
    let r = Rect::from_points(offsets)?;
    let fill = if color == 0 { 1 } else { 0 };
    let mut pixels = vec![fill; r.w as usize * r.h as usize];
    for &(x, y) in offsets {
        pixels[((y - r.y) * r.w + (x - r.x)) as usize] = color;
    }
    Some(BrushImage {
        width: r.w as u32,
        height: r.h as u32,
        pixels,
        handle: (-r.x, -r.y),
        transparent: Some(fill),
    })
}

/// Draws the wire frame of a projected brush (the Move requester's Preview): the outline
/// through `corners` in `color`, clipped to `img`.
pub fn draw_outline(img: &mut IndexedImage, corners: &[(f64, f64); 4], color: u8) {
    let lim = |v: f64| v.clamp(-1e6, 1e6).floor() as i32;
    let pts = corners.map(|(x, y)| (lim(x), lim(y)));
    for i in 0..4 {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        // Only the part near the image (a far corner would make a long line).
        let reach = img.bounds();
        if let Some((i0, i1)) = raster::clip_steps(a, b, reach) {
            for s in i0..=i1 {
                let (x, y) = raster::line_point(a, b, s);
                img.set(x, y, color);
            }
        }
    }
}

/// The brush a MoveBrush command paints, as a custom brush: its `custom` brush, else its
/// built-in `brush` in `color`.
fn source<'a>(
    custom: Option<&'a BrushImage>,
    brush: &[(i32, i32)],
    color: u8,
) -> Option<Cow<'a, BrushImage>> {
    match custom {
        Some(b) => Some(Cow::Borrowed(b)),
        None => brush_of_offsets(brush, color).map(Cow::Owned),
    }
}

/// Whether painting in `mode` with `fx` reads the page as it was before the stamp
/// (`PaintContext::before`): every mode but plain Matte, Color and Replace, and Smear, which
/// reads the page itself ([`raster::Pen::stamp`]). [`paint::plot_fx`] reads it at the
/// painted pixel only, Smooth also at the eight around it.
fn reads_before(mode: DrawMode, fx: &PaintFx) -> bool {
    match mode {
        DrawMode::Matte | DrawMode::Color | DrawMode::Replace => !fx.is_off(),
        DrawMode::Smear => false,
        DrawMode::Shade
        | DrawMode::Cycle
        | DrawMode::Blend
        | DrawMode::Mix
        | DrawMode::Smooth
        | DrawMode::HBrite => true,
    }
}

/// The page pixels a stamp at `at` of a `w`×`h` projected brush may read in `before`: its
/// rectangle and one pixel around it (Smooth's neighbourhood).
fn reach(at: (i32, i32), (w, h): (u32, u32)) -> Rect {
    Rect::new(at.0 - 1, at.1 - 1, w as i32 + 2, h as i32 + 2)
}

/// The page centre the projection looks at.
pub fn centre_of(page: &IndexedImage) -> (f64, f64) {
    (
        f64::from(page.width()) / 2.0,
        f64::from(page.height()) / 2.0,
    )
}

/// The fields of a MoveBrush command.
struct Fields<'a> {
    start: (i32, i32, i32),
    path: &'a MovePath,
    brush: &'a [(i32, i32)],
    color: u8,
    custom: Option<&'a BrushImage>,
    mode: DrawMode,
    range: Option<(u8, u8)>,
    stencil: Option<&'a [bool]>,
    right: bool,
    fx: PaintFx,
}

fn fields(cmd: &Command) -> Option<Fields<'_>> {
    match cmd {
        Command::MoveBrush {
            start,
            path,
            brush,
            color,
            custom,
            mode,
            range,
            stencil,
            right,
            fx,
        } => Some(Fields {
            start: *start,
            path,
            brush,
            color: *color,
            custom: custom.as_ref(),
            mode: *mode,
            range: *range,
            stencil: stencil.as_deref(),
            right: *right,
            fx: *fx,
        }),
        _ => None,
    }
}

/// Checks a MoveBrush command against `doc` (as [`crate::Engine::execute`] does).
pub(crate) fn validate(cmd: &Command, doc: &Document) -> Result<(), CommandError> {
    let f = fields(cmd).expect("a MoveBrush command");
    let colors = doc.palette().len();
    if usize::from(f.color) >= colors {
        return Err(CommandError::new(
            crate::ErrorCode::PaletteRangeInvalid,
            format!("colour {} is outside the palette (0..{colors})", f.color),
        ));
    }
    crate::command::validate_paint(colors, f.custom, f.range, f.stencil)?;
    if f.custom.is_none() {
        if f.brush.is_empty() || f.brush.len() > MAX_BRUSH_PIXELS {
            return Err(CommandError::invalid(format!(
                "brush must have 1..={MAX_BRUSH_PIXELS} pixels, got {}",
                f.brush.len()
            )));
        }
        if f.brush
            .iter()
            .any(|&(x, y)| x.abs() > MAX_BRUSH_OFFSET || y.abs() > MAX_BRUSH_OFFSET)
        {
            return Err(CommandError::invalid(format!(
                "brush offsets must lie within ±{MAX_BRUSH_OFFSET}"
            )));
        }
    }
    let p = f.path;
    if !(1..=MAX_COUNT).contains(&p.count) {
        return Err(CommandError::invalid(format!(
            "count {}: 1 to {MAX_COUNT} stamps",
            p.count
        )));
    }
    if p.ease_out > MAX_EASE || p.ease_in > MAX_EASE {
        return Err(CommandError::invalid(format!(
            "ease frames must be 0 to {MAX_EASE}"
        )));
    }
    if p.dist.iter().chain(&p.angle).any(|v| v.abs() > MAX_AMOUNT) {
        return Err(CommandError::invalid(format!(
            "distances and angles must lie within ±{MAX_AMOUNT}"
        )));
    }
    let (x, y, z) = f.start;
    if [x, y, z].iter().any(|v| v.abs() > MAX_COORD) {
        return Err(CommandError::invalid(format!(
            "start must lie within ±{MAX_COORD}"
        )));
    }
    let work = work(&f, doc);
    if work > MAX_PAINT_WORK {
        return Err(CommandError::invalid(format!(
            "too much work for one move: at least {} million units, at most {} million (use \
             fewer stamps, a smaller brush or no Trails)",
            work / 1_000_000,
            MAX_PAINT_WORK / 1_000_000
        )));
    }
    Ok(())
}

/// Work units of a move (see [`MAX_PAINT_WORK`]): one per brush pixel (the brush is read
/// once for the colour around the projected brush), then per painted stamp (Trails repeat
/// them) one, plus the page pixels its projection covers, once to project and
/// [`paint::pixel_work`] per pixel painted, plus in the modes that read the page before the
/// stamp the pixels of its [`reach`] (copied for it). Nothing per stamp depends on the page
/// or brush size beyond what the stamp covers.
fn work(f: &Fields, doc: &Document) -> u64 {
    let page = doc.page();
    let Some(b) = source(f.custom, f.brush, f.color) else {
        return 0;
    };
    let plan = plan(f.path, f.start, doc.frame_count(), doc.current_frame());
    let centre = centre_of(page);
    let per_pixel = 1 + paint::pixel_work(f.mode, &f.fx, doc.palette().len());
    let reads = reads_before(f.mode, &f.fx);
    let areas: Vec<u64> = plan
        .stamps
        .iter()
        .map(|s| {
            let r = cover(
                (b.width, b.height),
                b.handle,
                &s.pose,
                centre,
                page.bounds(),
            );
            if r.is_empty() {
                return 1;
            }
            let (w, h) = (r.w as u32, r.h as u32);
            let snapshot = if reads {
                let s = reach((r.x, r.y), (w, h)).intersect(page.bounds());
                s.w as u64 * s.h as u64
            } else {
                0
            };
            u64::from(w) * u64::from(h) * per_pixel + snapshot + 1
        })
        .collect();
    let mut total = b.pixels.len() as u64;
    for i in 0..areas.len() {
        let layers = plan.layers(f.path, i);
        let sum: u64 = areas[layers].iter().sum();
        total = total.saturating_add(sum);
        if total > MAX_PAINT_WORK {
            break;
        }
    }
    total
}

/// Paints a validated MoveBrush command into `doc`: every stamp on its frame, then the
/// plan's end frame becomes current.
pub(crate) fn apply(cmd: &Command, doc: &mut Document) {
    let f = fields(cmd).expect("a MoveBrush command");
    let Some(b) = source(f.custom, f.brush, f.color) else {
        return;
    };
    let plan = plan(f.path, f.start, doc.frame_count(), doc.current_frame());
    let palette: Palette = doc.palette().clone();
    // The colour around the projected brush, once: finding it reads the whole brush.
    let fill = brush_ops::fill_colour(&b, palette.len());
    let centre = centre_of(doc.page());
    let clip = doc.page().bounds();
    // Modes that read the page before the stamp get it in `before`, one page-sized buffer
    // refreshed for every stamp only where that stamp reads (its reach): the rest of it is
    // stale and never read. With Trails a move paints about count² / 2 stamps, so a stamp
    // must not cost the whole page.
    let reads = reads_before(f.mode, &f.fx);
    let mut before = if reads {
        IndexedImage::new(clip.w as u32, clip.h as u32, 0)
    } else {
        IndexedImage::new(0, 0, 0)
    };
    let mut last_handle: Option<(i32, i32)> = None;
    for (i, stamp) in plan.stamps.iter().enumerate() {
        doc.go_to(stamp.frame);
        for layer in plan.layers(f.path, i) {
            let pose = plan.stamps[layer].pose;
            let Some((img, at)) = project(&b, &pose, centre, clip, fill) else {
                continue;
            };
            let handle = outline((1, 1), (0, 0), &pose, centre)
                .map(|c| (c[0].0.floor() as i32, c[0].1.floor() as i32));
            let motion = match (last_handle, handle) {
                (Some(l), Some(h)) if layer == i => (h.0 - l.0, h.1 - l.1),
                _ => (0, 0),
            };
            if layer == i {
                last_handle = handle;
            }
            if reads {
                before.copy_rect_from(&doc.page, reach(at, (img.width, img.height)));
            }
            let owned: Vec<(i32, i32)>;
            let (custom, offsets) = if f.custom.is_some() {
                (Some(&img), &[][..])
            } else {
                owned = img.opaque_offsets();
                (None, &owned[..])
            };
            let pen = Pen {
                offsets,
                custom,
                color: f.color,
                mode: f.mode,
                ctx: PaintContext {
                    palette: &palette,
                    range: f.range,
                    stencil: f.stencil,
                    before: &before,
                    motion: (0, 0),
                    right: f.right,
                },
                fx: f.fx,
                // Move stamps the brush as it is (an AnimBrush: its current cel).
                cels: None,
            };
            pen.stamp(&mut doc.page, at.0, at.1, motion);
        }
    }
    doc.go_to(plan.end_frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Engine, Palette};

    fn path(count: u32) -> MovePath {
        MovePath {
            count,
            ..MovePath::default()
        }
    }

    fn positions(plan: &Plan) -> Vec<(i32, i32, i32)> {
        plan.stamps.iter().map(|s| s.pose.pixel()).collect()
    }

    fn frames(plan: &Plan) -> Vec<usize> {
        plan.stamps.iter().map(|s| s.frame).collect()
    }

    #[test]
    fn easing_speeds_up_and_slows_down_and_always_arrives() {
        assert_eq!(progress(0, 10, 0, 0), 0.0);
        assert_eq!(progress(5, 10, 0, 0), 0.5);
        assert_eq!(progress(10, 10, 3, 4), 1.0);
        assert_eq!(progress(0, 0, 0, 0), 0.0);
        // Ease-Out 10 of 20: slow start, then even steps of the same speed (tutorial p.141).
        let p: Vec<f64> = (0..=20).map(|i| progress(i, 20, 10, 0)).collect();
        let steps: Vec<f64> = p.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps[0] < steps[5] && steps[5] < steps[9], "{steps:?}");
        for s in &steps[10..] {
            assert!((s - steps[10]).abs() < 1e-12, "even after the ease");
        }
        // Ease-In at the end: slower and slower.
        let q: Vec<f64> = (0..=10).map(|i| progress(i, 10, 0, 5)).collect();
        assert!(q[10] - q[9] < q[6] - q[5]);
        // Too much easing is shortened, still monotone from 0 to 1.
        let r: Vec<f64> = (0..=4).map(|i| progress(i, 4, 9, 9)).collect();
        assert!(r.windows(2).all(|w| w[1] > w[0]), "{r:?}");
    }

    #[test]
    fn go_from_cyclic_leaves_out_the_end_and_moves_past_it() {
        // 20 frames, X 200 over 20 stamps from x = 100 on frame 1 (tutorial p.139).
        let mut p = path(20);
        p.dist = [200, 0, 0];
        let plan = plan(&p, (100, 50, 0), 20, 0);
        assert_eq!(plan.stamps.len(), 20);
        assert_eq!(frames(&plan), (0..20).collect::<Vec<_>>());
        assert_eq!(positions(&plan)[0], (100, 50, 0));
        assert_eq!(positions(&plan)[19], (290, 50, 0), "19/20 of the way");
        assert_eq!(plan.end.pixel(), (300, 50, 0), "remembered: the whole move");
        assert_eq!(plan.end_frame, 0, "the frame after the last, wrapped");
        // Not cyclic: the last stamp arrives, on frame 20, which stays current.
        p.cyclic = false;
        let plan = plan_of(&p, 20, 0);
        assert_eq!(positions(&plan)[19], (300, 50, 0));
        assert_eq!(plan.end_frame, 19);
        // Y counts upwards, Z away.
        p.dist = [0, 30, 0];
        p.count = 2;
        assert_eq!(positions(&plan_of(&p, 1, 0))[1], (100, 20, 0));
    }

    fn plan_of(p: &MovePath, frames: usize, current: usize) -> Plan {
        plan(p, (100, 50, 0), frames, current)
    }

    #[test]
    fn come_to_ends_on_the_stamp_and_the_current_frame() {
        let mut p = path(5);
        p.dist = [0, -40, 0];
        p.come_to = true;
        p.cyclic = false;
        // On frame 5 of 5: frames 1..5, the last stamp where the brush was stamped.
        let plan = plan_of(&p, 5, 4);
        assert_eq!(frames(&plan), vec![0, 1, 2, 3, 4]);
        assert_eq!(positions(&plan)[0], (100, 10, 0), "starts 40 up");
        assert_eq!(positions(&plan)[4], (100, 50, 0));
        assert_eq!(plan.end_frame, 4);
        assert_eq!(plan.end.pixel(), (100, 50, 0));
        // Cyclic: the start is the part left out; still ends on the stamp.
        p.cyclic = true;
        let plan = plan_of(&p, 5, 4);
        assert_eq!(positions(&plan)[0], (100, 18, 0));
        assert_eq!(positions(&plan)[4], (100, 50, 0));
        // Frames wrap: ending on frame 2 of 5 starts on frame 3.
        assert_eq!(frames(&plan_of(&p, 5, 1)), vec![2, 3, 4, 0, 1]);
    }

    #[test]
    fn record_in_place_and_backward() {
        let mut p = path(4);
        p.dist = [30, 0, 0];
        p.cyclic = false;
        p.record = Record::InPlace;
        let plan = plan_of(&p, 10, 3);
        assert_eq!(frames(&plan), vec![3; 4]);
        assert_eq!(plan.end_frame, 3);
        // Backward: the same frames from the last one back; it ends where it started.
        p.record = Record::Backward;
        let plan = plan_of(&p, 10, 3);
        assert_eq!(frames(&plan), vec![6, 5, 4, 3]);
        assert_eq!(positions(&plan)[0], (130, 50, 0));
        assert_eq!(plan.end_frame, 3);
        // One frame: everything in place.
        p.record = Record::Forward;
        assert_eq!(frames(&plan_of(&p, 1, 0)), vec![0; 4]);
        // Trails repeat earlier stamps, except on one frame.
        p.trails = true;
        let plan = plan_of(&p, 10, 0);
        assert_eq!(plan.layers(&p, 2), 0..=2);
        let one = plan_of(&p, 1, 0);
        assert_eq!(one.layers(&p, 2), 2..=2);
    }

    #[test]
    fn rotation_turns_clockwise_and_cyclic_never_shows_the_full_turn() {
        let r = rotation([0.0, 0.0, 90.0]);
        let v = apply_m(&r, [1.0, 0.0, 0.0]);
        assert!(
            (v[0]).abs() < 1e-12 && (v[1] - 1.0).abs() < 1e-12,
            "right turns down"
        );
        let top_away = apply_m(&rotation([90.0, 0.0, 0.0]), [0.0, -1.0, 0.0]);
        assert!((top_away[2] - 1.0).abs() < 1e-12);
        let right_away = apply_m(&rotation([0.0, 90.0, 0.0]), [1.0, 0.0, 0.0]);
        assert!((right_away[2] - 1.0).abs() < 1e-12);
        // 360 degrees over 4 cyclic stamps: 0, 90, 180, 270; the end is upright again.
        let mut p = path(4);
        p.angle = [0, 0, 360];
        let plan = plan_of(&p, 4, 0);
        let x_axis: Vec<(i32, i32)> = plan
            .stamps
            .iter()
            .map(|s| {
                (
                    s.pose.rot[0][0].round() as i32,
                    s.pose.rot[1][0].round() as i32,
                )
            })
            .collect();
        assert_eq!(x_axis, vec![(1, 0), (0, 1), (-1, 0), (0, -1)]);
        assert!((plan.end.rot[0][0] - 1.0).abs() < 1e-9);
        // Screen axes give the same single-axis turn.
        p.angle_brush = false;
        let screen = plan_of(&p, 4, 0);
        for (a, b) in screen.stamps.iter().zip(&plan.stamps) {
            let pairs = a.pose.rot.iter().flatten().zip(b.pose.rot.iter().flatten());
            for (x, y) in pairs {
                assert!((x - y).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn brush_axes_distance_follows_the_turning_brush() {
        // X 100 along the brush while it turns 180 degrees: the path bends, unlike on the
        // screen axes.
        let mut p = path(10);
        p.cyclic = false;
        p.dist = [100, 0, 0];
        p.angle = [0, 0, 180];
        let screen = plan_of(&p, 10, 0);
        p.dist_brush = true;
        let brush = plan_of(&p, 10, 0);
        assert_eq!(screen.end.pixel().1, 50, "straight along the screen");
        assert!(
            brush.end.pixel().1 > 60,
            "curves down as the brush turns clockwise"
        );
        assert!(brush.end.pixel().0 < 200);
    }

    fn brush() -> BrushImage {
        // 3×2: 1 2 3 / 4 5 0 (0 transparent), handle at (1, 0).
        BrushImage {
            width: 3,
            height: 2,
            pixels: vec![1, 2, 3, 4, 5, 0],
            handle: (1, 0),
            transparent: Some(0),
        }
    }

    #[test]
    fn projection_of_the_unmoved_brush_is_the_brush() {
        let b = brush();
        let pose = Pose::at((10, 20, 0));
        let (img, at) = project(&b, &pose, (160.0, 128.0), Rect::new(0, 0, 320, 256), 0).unwrap();
        assert_eq!(at, (9, 20));
        assert_eq!((img.width, img.height), (3, 2));
        assert_eq!(img.pixels, b.pixels);
        assert_eq!(img.transparent, Some(0));
        // Clipped at the page edge.
        let (img, at) = project(
            &b,
            &Pose::at((0, 0, 0)),
            (160.0, 128.0),
            Rect::new(0, 0, 320, 256),
            0,
        )
        .unwrap();
        assert_eq!((at, img.width), ((0, 0), 2));
        assert_eq!(img.pixels, vec![2, 3, 5, 0]);
        // Far off the page or behind the viewer: nothing.
        assert!(
            project(
                &b,
                &Pose::at((-50, 0, 0)),
                (160.0, 128.0),
                Rect::new(0, 0, 320, 256),
                0
            )
            .is_none()
        );
        assert!(
            project(
                &b,
                &Pose::at((10, 10, -400)),
                (160.0, 128.0),
                Rect::new(0, 0, 320, 256),
                0
            )
            .is_none()
        );
    }

    #[test]
    fn depth_shrinks_towards_the_centre_and_a_turn_rotates() {
        let b = BrushImage::from_image(&IndexedImage::new(40, 40, 7), None);
        let clip = Rect::new(0, 0, 320, 256);
        // EYE pixels away: half size, halfway to the centre.
        let far = Pose::at((100, 128, EYE as i32));
        let (img, at) = project(&b, &far, (160.0, 128.0), clip, 0).unwrap();
        assert_eq!((img.width, img.height), (20, 20));
        assert_eq!(at, (120, 118));
        assert!(img.pixels.iter().all(|&p| p == 7));
        // A quarter turn of a 3×1 bar makes it 1×3.
        let bar = BrushImage {
            width: 3,
            height: 1,
            pixels: vec![1, 2, 3],
            handle: (1, 0),
            transparent: None,
        };
        let pose = Pose {
            rot: rotation([0.0, 0.0, 90.0]),
            ..Pose::at((50, 50, 0))
        };
        let (img, at) = project(&bar, &pose, (160.0, 128.0), clip, 0).unwrap();
        assert_eq!((img.width, img.height, at), (1, 3, (50, 49)));
        assert_eq!(img.pixels, vec![1, 2, 3], "clockwise: left end on top");
        // Seen edge-on (its plane runs through the eye): nothing.
        let edge = Pose {
            rot: rotation([0.0, 90.0, 0.0]),
            ..Pose::at((50, 50, 0))
        };
        assert!(project(&bar, &edge, (50.5, 50.5), clip, 0).is_none());
        assert!(
            project(&bar, &edge, (160.0, 128.0), clip, 0).is_some(),
            "seen at an angle"
        );
        // Turned 45 degrees, the corners of the bounding box get the fill colour given,
        // which is the projection's transparent colour.
        let turned = Pose {
            rot: rotation([0.0, 0.0, 45.0]),
            ..Pose::at((100, 100, 0))
        };
        let (img, _) = project(&b, &turned, (160.0, 128.0), clip, 9).unwrap();
        assert_eq!(img.transparent, Some(9));
        assert_eq!(img.pixels[0], 9, "top-left corner");
        let middle = (img.height / 2 * img.width + img.width / 2) as usize;
        assert_eq!(img.pixels[middle], 7, "the middle");
    }

    /// One stamp of `cmd`'s brush and paint fields at `start` on the current frame, as its own
    /// command (Count 1): it reads the whole page as it is before it.
    fn single_stamp(cmd: &Command, start: (i32, i32, i32)) -> Command {
        let mut one = cmd.clone();
        if let Command::MoveBrush {
            start: s, path: p, ..
        } = &mut one
        {
            *s = start;
            *p = MovePath {
                record: Record::InPlace,
                ..path(1)
            };
        }
        one
    }

    /// The modes that read the page before a stamp only get the part of it each stamp of a
    /// move can read refreshed: the result is the same as stamping every layer as a command
    /// of its own, which reads the whole page.
    #[test]
    fn reading_modes_see_the_page_as_before_each_stamp() {
        let pal = Palette::dpaint_default_32();
        let brush = BrushImage {
            width: 3,
            height: 2,
            pixels: vec![4, 0, 6, 7, 8, 9],
            handle: (1, 1),
            transparent: Some(0),
        };
        let translucent = PaintFx {
            translucency: Some(50),
            ..PaintFx::default()
        };
        let cases = [
            (DrawMode::Smooth, PaintFx::default()),
            (DrawMode::Shade, PaintFx::default()),
            (DrawMode::Blend, PaintFx::default()),
            (DrawMode::Cycle, PaintFx::default()),
            (DrawMode::Matte, translucent),
        ];
        for (mode, fx) in cases {
            // A picture to read: stripes of colours 1..=7 over 3 frames.
            let mut e = Engine::new(Document::new(24, 12, pal.clone()));
            e.execute(Command::SetFrameCount { count: 3 }).unwrap();
            for f in 0..3u32 {
                e.execute(Command::GotoFrame { frame: f }).unwrap();
                let pixels = (0..24 * 12).map(|i| ((i % 24 + f) % 7 + 1) as u8).collect();
                e.execute(Command::SetPage {
                    width: 24,
                    height: 12,
                    pixels,
                })
                .unwrap();
            }
            e.execute(Command::GotoFrame { frame: 0 }).unwrap();
            let original = e.document().clone();
            let mut reference = Engine::new(original.clone());
            // Two pixels a stamp over three frames with Trails: the stamps overlap and the
            // later ones read what the earlier ones painted (and the page edge at x 0).
            let mut p = path(3);
            p.cyclic = false;
            p.dist = [4, 0, 0];
            p.trails = true;
            let mut cmd = move_cmd(p, (0, 5, 0));
            if let Command::MoveBrush {
                custom,
                mode: m,
                range,
                fx: x,
                ..
            } = &mut cmd
            {
                *custom = Some(brush.clone());
                *m = mode;
                *range = Some((1, 7));
                *x = fx;
            }
            let plan = plan(&p, (0, 5, 0), 3, 0);
            e.execute(cmd.clone()).unwrap();
            for (i, stamp) in plan.stamps.iter().enumerate() {
                if reference.document().current_frame() != stamp.frame {
                    let frame = stamp.frame as u32;
                    reference.execute(Command::GotoFrame { frame }).unwrap();
                }
                for layer in plan.layers(&p, i) {
                    let at = plan.stamps[layer].pose.pixel();
                    reference.execute(single_stamp(&cmd, at)).unwrap();
                }
            }
            for f in 0..3 {
                let painted = e.document().frame(f).unwrap();
                assert_eq!(
                    painted,
                    reference.document().frame(f).unwrap(),
                    "{mode:?} frame {f}"
                );
                assert_ne!(painted, original.frame(f).unwrap(), "{mode:?} painted");
            }
        }
    }

    #[test]
    fn built_in_brushes_and_outlines() {
        let b = brush_of_offsets(&[(0, 0), (1, 0), (0, 1)], 5).unwrap();
        assert_eq!((b.width, b.height, b.handle), (2, 2, (0, 0)));
        assert_eq!(b.opaque_offsets(), vec![(0, 0), (1, 0), (0, 1)]);
        assert_eq!(brush_of_offsets(&[(0, 0)], 0).unwrap().transparent, Some(1));
        assert!(brush_of_offsets(&[], 1).is_none());
        let c = outline((4, 2), (0, 0), &Pose::at((10, 10, 0)), (0.0, 0.0)).unwrap();
        assert_eq!(c, [(10.0, 10.0), (14.0, 10.0), (14.0, 12.0), (10.0, 12.0)]);
        let mut img = IndexedImage::new(20, 20, 0);
        draw_outline(&mut img, &c, 3);
        assert_eq!(img.get(10, 10), Some(3));
        assert_eq!(img.get(14, 12), Some(3));
        assert_eq!(img.get(12, 11), Some(0), "only the outline");
    }

    fn move_cmd(path: MovePath, start: (i32, i32, i32)) -> Command {
        Command::MoveBrush {
            start,
            path,
            brush: vec![(0, 0)],
            color: 1,
            custom: None,
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: PaintFx::default(),
        }
    }

    fn engine(frames: u32) -> Engine {
        let mut e = Engine::new(Document::new(64, 32, Palette::dpaint_default_32()));
        e.execute(Command::SetFrameCount { count: frames }).unwrap();
        e
    }

    #[test]
    fn draw_paints_one_stamp_per_frame_in_one_undo_step() {
        let mut e = engine(4);
        let mut p = path(4);
        p.dist = [12, 0, 0];
        let revision = e.document().revision();
        e.execute(move_cmd(p, (10, 5, 0))).unwrap();
        let doc = e.document();
        // Cyclic: x 10, 13, 16, 19 on frames 1-4; frame 1 is current again.
        for (f, x) in [10, 13, 16, 19].into_iter().enumerate() {
            let frame = doc.frame(f).unwrap();
            assert_eq!(frame.get(x, 5), Some(1), "frame {f}");
            assert_eq!(frame.pixels().iter().filter(|&&c| c == 1).count(), 1);
        }
        assert_eq!(doc.current_frame(), 0);
        assert_eq!(doc.revision(), revision + 1);
        e.undo().unwrap();
        assert!(
            e.document()
                .frames()
                .all(|f| f.pixels().iter().all(|&c| c == 0))
        );
        e.redo().unwrap();
        assert_eq!(e.document().frame(3).unwrap().get(19, 5), Some(1));
        // JSON name.
        let json = serde_json::to_string(&move_cmd(p, (0, 0, 0))).unwrap();
        assert!(json.contains("\"op\":\"move_brush\""), "{json}");
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, move_cmd(p, (0, 0, 0)));
    }

    #[test]
    fn trails_backward_and_the_stencil_paint_behind() {
        let mut e = engine(3);
        e.execute(Command::GotoFrame { frame: 2 }).unwrap();
        let mut p = path(3);
        p.cyclic = false;
        p.come_to = true;
        p.dist = [4, 0, 0];
        p.record = Record::Backward;
        p.trails = true;
        let mut cmd = move_cmd(p, (20, 5, 0));
        // Protect every colour but 0: later stamps go behind earlier ones (tutorial p.166).
        let mut stencil = vec![true; 256];
        stencil[0] = false;
        if let Command::MoveBrush {
            stencil: s, custom, ..
        } = &mut cmd
        {
            *s = Some(stencil);
            *custom = Some(BrushImage {
                width: 3,
                height: 1,
                pixels: vec![3, 4, 5],
                handle: (0, 0),
                transparent: None,
            });
        }
        e.execute(cmd).unwrap();
        let doc = e.document();
        // Come To from x 16 to 20 over frames 1-3, painted from frame 3 back with trails.
        let row = |f: usize| -> Vec<u8> {
            (14..24)
                .map(|x| doc.frame(f).unwrap().get(x, 5).unwrap())
                .collect()
        };
        assert_eq!(row(2), vec![0, 0, 0, 0, 0, 0, 3, 4, 5, 0]);
        assert_eq!(row(1), vec![0, 0, 0, 0, 3, 4, 3, 4, 5, 0]);
        assert_eq!(row(0), vec![0, 0, 3, 4, 3, 4, 3, 4, 5, 0]);
        assert_eq!(
            doc.current_frame(),
            0,
            "backward ends on the frame painted last"
        );
    }

    #[test]
    fn bad_moves_are_refused() {
        let mut e = engine(2);
        let mut p = path(0);
        assert!(e.execute(move_cmd(p, (0, 0, 0))).is_err());
        p.count = 1000;
        assert!(e.execute(move_cmd(p, (0, 0, 0))).is_err());
        p.count = 3;
        p.dist = [MAX_AMOUNT + 1, 0, 0];
        assert!(e.execute(move_cmd(p, (0, 0, 0))).is_err());
        p.dist = [0; 3];
        p.ease_in = MAX_EASE + 1;
        assert!(e.execute(move_cmd(p, (0, 0, 0))).is_err());
        p.ease_in = 0;
        let mut cmd = move_cmd(p, (0, 0, 0));
        if let Command::MoveBrush { color, .. } = &mut cmd {
            *color = 40;
        }
        assert!(e.execute(cmd).is_err());
        let mut cmd = move_cmd(p, (0, 0, 0));
        if let Command::MoveBrush { brush, .. } = &mut cmd {
            brush.clear();
        }
        assert!(e.execute(cmd).is_err());
        // A huge brush close to the viewer with many trails costs too much.
        let big = BrushImage::from_image(&IndexedImage::new(64, 32, 3), None);
        let mut e = Engine::new(Document::new(1024, 1024, Palette::dpaint_default_32()));
        e.execute(Command::SetFrameCount { count: 128 }).unwrap();
        let mut p = path(999);
        p.trails = true;
        p.dist = [0, 0, -300];
        let mut cmd = move_cmd(p, (512, 512, 0));
        if let Command::MoveBrush { custom, .. } = &mut cmd {
            *custom = Some(big);
        }
        let err = e.execute(cmd).unwrap_err();
        assert!(err.message.contains("too much work"), "{}", err.message);
        assert_eq!(e.document().revision(), 1, "nothing changed");
    }

    /// Runs `cmd` on `e` and returns how long it took.
    fn timed(e: &mut Engine, cmd: Command) -> std::time::Duration {
        let t = std::time::Instant::now();
        e.execute(cmd).unwrap();
        t.elapsed()
    }

    /// With Trails a move paints about count² / 2 layers, so a layer may only cost what it
    /// covers. Shade reads the page before every layer: copying the whole 4096×4096 page
    /// for each of these 20 100 layers took about 19 s in a debug build (review of ADR-38).
    #[test]
    fn trails_in_a_reading_mode_do_not_copy_the_page_per_layer() {
        let mut e = Engine::new(Document::new(4096, 4096, Palette::dpaint_default_32()));
        e.execute(Command::SetFrameCount { count: 2 }).unwrap();
        let mut p = path(200);
        p.trails = true;
        p.dist = [200, 0, 0];
        let mut cmd = move_cmd(p, (100, 100, 0));
        if let Command::MoveBrush { mode, range, .. } = &mut cmd {
            *mode = DrawMode::Shade;
            *range = Some((0, 7));
        }
        let took = timed(&mut e, cmd);
        eprintln!("shade trails on 4096x4096: {took:?}");
        // Stamp 199 (x 299) went on frame 2 (199 is odd), one Shade step above colour 0.
        assert_eq!(e.document().frame(1).unwrap().get(299, 100), Some(1));
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    }

    /// A custom brush without a transparent colour is scanned for a colour it does not use
    /// (the one around the projected brush). Doing that for every layer of a 2048×2048 brush
    /// that shows a few pixels wide took about 15 s for these 820 layers in a debug build.
    #[test]
    fn trails_with_an_opaque_brush_do_not_scan_the_brush_per_layer() {
        let mut e = Engine::new(Document::new(320, 256, Palette::dpaint_default_32()));
        e.execute(Command::SetFrameCount { count: 2 }).unwrap();
        let big = BrushImage::from_image(&IndexedImage::new(2048, 2048, 3), None);
        let mut p = path(40);
        p.trails = true;
        p.dist = [80, 0, 0];
        let mut cmd = move_cmd(p, (100, 100, 200_000));
        if let Command::MoveBrush { custom, .. } = &mut cmd {
            *custom = Some(big);
        }
        let took = timed(&mut e, cmd);
        eprintln!("opaque 2048x2048 brush trails: {took:?}");
        let page = e.document().frame(0).unwrap();
        assert!(page.pixels().contains(&3), "the brush shows");
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    }
}
