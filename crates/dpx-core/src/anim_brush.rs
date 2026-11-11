//! AnimBrush and Metamorph (ADR-36).
//!
//! An AnimBrush is a custom brush with several cels (Anim ▸ AnimBrush ▸ Pick up.. and Load..,
//! Brush ▸ Spare ▸ Metamorph..). Painting with it stamps the cels in turn: a brush of `n`
//! cels with Duration `d` moves `n / d` cels on per stamp, so Duration 20 on 10 cels stamps
//! every cel twice and Duration 5 skips every other cel (manual AGA p.251, p.165-166). The
//! direction plays the cels forward, backward or ping-pong (to the end and back without
//! doubling the first or last cel, so 11 cels make a cycle of 20, AGA p.167).
//!
//! The command engine paints it through [`crate::Command::AnimBrushPaint`]; a front-end keeps
//! the brush and moves its clock on by the stamps of every committed command
//! ([`crate::Command::stamps`]), so the next stroke, on this frame or another, continues where
//! the last one stopped.

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::image::Rect;
use crate::paint::BrushImage;
use crate::palette::{Palette, Rgb8};
use crate::raster;

/// An AnimBrush has at least two cels (a single frame cannot be picked up as one, AGA p.250).
pub const MIN_CELS: usize = 2;
/// Most cels: as many as a document has frames at most.
pub const MAX_CELS: usize = crate::anim::MAX_FRAMES;
/// Longest Duration (stamps for one pass through the cels); the requester field has three
/// digits.
pub const MAX_DURATION: u32 = 999;
/// Most bytes all cels together may hold (one byte per pixel), the frame limit of a document.
pub const MAX_CEL_BYTES: u64 = crate::anim::MAX_FRAME_BYTES;

/// The order the cels play in (the three Direction icons of the AnimBrush Settings
/// requester, AGA p.251).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CelDirection {
    #[default]
    Forward,
    Backward,
    /// To the last cel and back, the first and last cel shown once per pass.
    PingPong,
}

impl CelDirection {
    /// Steps in one cycle through `n` cels.
    pub fn period(self, n: usize) -> u64 {
        let n = n.max(1) as u64;
        match self {
            CelDirection::PingPong if n > 1 => 2 * n - 2,
            _ => n,
        }
    }

    /// The cel shown at step `step` of the cycle through `n` cels.
    pub fn cel(self, step: u64, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        let q = step % self.period(n);
        let n = n as u64;
        (match self {
            CelDirection::Forward => q,
            CelDirection::Backward => n - 1 - q,
            CelDirection::PingPong if q < n => q,
            CelDirection::PingPong => 2 * n - 2 - q,
        }) as usize
    }

    /// The step at which `cel` shows (on the way up for ping-pong).
    pub fn step_of(self, cel: usize, n: usize) -> u64 {
        let cel = cel.min(n.max(1) - 1) as u64;
        match self {
            CelDirection::Backward => n as u64 - 1 - cel,
            _ => cel,
        }
    }
}

/// Where an AnimBrush is in its cycle: the step (see [`CelDirection::cel`]) and the stamps
/// counted towards the next step, in units of `1 / duration` (a Bresenham accumulator, so
/// the steps are spread evenly over the stamps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CelClock {
    pub step: u64,
    #[serde(default)]
    pub sub: u32,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// A brush of several cels with its settings and clock (AGA p.249-252).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimBrush {
    /// The cels; each is a complete brush with its own handle and transparent colour.
    pub cels: Vec<BrushImage>,
    /// Stamps for one pass through all cels (the Settings requester's Duration).
    pub duration: u32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub direction: CelDirection,
    /// Where the next stamp starts.
    #[serde(default, skip_serializing_if = "is_default")]
    pub clock: CelClock,
}

impl AnimBrush {
    /// An AnimBrush of `cels` with the defaults: Duration = number of cels (one cel per
    /// stamp, as the Settings figure shows for 20 cels, AGA p.250), forward, first cel.
    pub fn new(cels: Vec<BrushImage>) -> Self {
        let duration = (cels.len().max(1) as u32).min(MAX_DURATION);
        Self {
            cels,
            duration,
            direction: CelDirection::Forward,
            clock: CelClock::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.cels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cels.is_empty()
    }

    /// The step reached `stamps` stamps after the clock, and the accumulator then.
    fn after(&self, stamps: u64) -> CelClock {
        let n = self.cels.len().max(1) as u128;
        let d = u128::from(self.duration.max(1));
        let total = u128::from(self.clock.sub) + u128::from(stamps) * n;
        let period = u128::from(self.direction.period(self.cels.len()));
        CelClock {
            step: ((u128::from(self.clock.step) + total / d) % period) as u64,
            sub: (total % d) as u32,
        }
    }

    /// The cel that stamp `stamp` (0 = the next one) paints.
    pub fn cel_index_at(&self, stamp: u64) -> usize {
        self.direction.cel(self.after(stamp).step, self.cels.len())
    }

    /// The brush of stamp `stamp` (0 = the next one).
    pub fn cel_at(&self, stamp: u64) -> &BrushImage {
        &self.cels[self.cel_index_at(stamp).min(self.cels.len() - 1)]
    }

    /// The current cel (from 0): the one the next stamp paints.
    pub fn current(&self) -> usize {
        self.cel_index_at(0)
    }

    /// The current cel's brush.
    pub fn current_cel(&self) -> &BrushImage {
        self.cel_at(0)
    }

    /// Moves the clock on by `stamps` stamps.
    pub fn advance(&mut self, stamps: u64) {
        self.clock = self.after(stamps);
    }

    /// Makes `cel` (from 0, clamped) the current cel; the next stamp paints it.
    pub fn set_current(&mut self, cel: usize) {
        self.clock = CelClock {
            step: self.direction.step_of(cel, self.cels.len()),
            sub: 0,
        };
    }

    /// Another direction; the current cel stays.
    pub fn set_direction(&mut self, direction: CelDirection) {
        let c = self.current();
        self.direction = direction;
        self.set_current(c);
    }

    /// Another Duration (1..=[`MAX_DURATION`]); the current cel stays.
    pub fn set_duration(&mut self, duration: u32) {
        let c = self.current();
        self.duration = duration.clamp(1, MAX_DURATION);
        self.set_current(c);
    }

    /// The pixels of all cels relative to their handles; `None` without cels.
    pub fn extent(&self) -> Option<Rect> {
        self.cels
            .iter()
            .filter_map(raster::brush_extent)
            .reduce(Rect::union)
    }

    /// Bytes of pixels over all cels.
    pub fn bytes(&self) -> u64 {
        self.cels.iter().map(|c| c.pixels.len() as u64).sum()
    }

    /// The same brush with `f` applied to every cel (a Brush menu transform; Flip turns every
    /// frame of an AnimBrush, AGA p.244). Settings and clock stay.
    pub fn map(&self, f: impl FnMut(&BrushImage) -> BrushImage) -> AnimBrush {
        AnimBrush {
            cels: self.cels.iter().map(f).collect(),
            ..self.clone()
        }
    }

    /// Like [`Self::map`], but `None` when `f` gives nothing for any cel (Trim of a cel with
    /// nothing left).
    pub fn try_map(&self, f: impl FnMut(&BrushImage) -> Option<BrushImage>) -> Option<AnimBrush> {
        Some(AnimBrush {
            cels: self.cels.iter().map(f).collect::<Option<_>>()?,
            ..self.clone()
        })
    }

    /// Like [`Self::map`], but `None` as soon as the new cels would hold more than
    /// [`MAX_CEL_BYTES`]: a transform that would make the AnimBrush too big leaves it as it is
    /// (the limit is a document rule, never lifted). The cels are made one after the other
    /// and the work stops at the first one over the limit.
    pub fn map_within(&self, f: impl FnMut(&BrushImage) -> BrushImage) -> Option<AnimBrush> {
        self.map_within_bytes(MAX_CEL_BYTES, f)
    }

    fn map_within_bytes(
        &self,
        limit: u64,
        mut f: impl FnMut(&BrushImage) -> BrushImage,
    ) -> Option<AnimBrush> {
        let mut bytes = 0u64;
        let mut cels = Vec::with_capacity(self.cels.len());
        for c in &self.cels {
            let cel = f(c);
            bytes += cel.pixels.len() as u64;
            if bytes > limit {
                return None;
            }
            cels.push(cel);
        }
        Some(AnimBrush {
            cels,
            duration: self.duration,
            direction: self.direction,
            clock: self.clock,
        })
    }

    /// Every cel renumbered for an arranged palette (Color ▸ Palette ▸ Arrange, ADR-31), so
    /// it keeps its look: see [`renumber_cel`]. Settings and clock stay.
    pub fn renumber(&mut self, lut: &[u8; 256]) {
        for cel in &mut self.cels {
            renumber_cel(cel, lut);
        }
    }

    /// Every cel's colours, as palette `from` has them, become the nearest colours of `to`
    /// (transparent colours keep their index; indices whose colour is the same in both stay).
    /// Returns the brush and how many distinct colours changed. Used to fit a loaded AnimBrush
    /// to the picture palette (loading one keeps the picture's palette, AGA p.249) and by
    /// Color ▸ Brush ▸ Remap.
    pub fn remap(&self, from: &Palette, to: &Palette) -> (AnimBrush, usize) {
        let mut map: [Option<u8>; 256] = [None; 256];
        let mut changed = 0;
        let mut out = self.clone();
        for cel in &mut out.cels {
            for p in &mut cel.pixels {
                let old = *p;
                let slot = &mut map[usize::from(old)];
                *p = *slot.get_or_insert_with(|| match from.get(old) {
                    Some(c) if Some(old) != cel.transparent && to.get(old) != Some(c) => {
                        let n = to.nearest(c);
                        changed += usize::from(n != old);
                        n
                    }
                    _ => old,
                });
            }
        }
        (out, changed)
    }

    /// All cels on one canvas: the same size and handle, padded with the transparent colour,
    /// so they can be written as the frames of an IFF ANIM. The canvas covers every cel's
    /// pixels around the handle. The transparent colour is the first cel's, else (when padding
    /// is needed) the lowest colour below `colors` no cel uses. Cels that are on one canvas
    /// already come back unchanged.
    pub fn on_one_canvas(&self, colors: usize) -> AnimBrush {
        let Some(u) = self.extent() else {
            return self.clone();
        };
        let same = self.cels.iter().all(|c| {
            (c.width as i32, c.height as i32, c.handle) == (u.w, u.h, (-u.x, -u.y))
                && c.transparent == self.cels[0].transparent
        });
        if same {
            return self.clone();
        }
        let t = self.cels[0]
            .transparent
            .unwrap_or_else(|| unused_colour(&self.cels, colors));
        self.map(|c| {
            let mut out = BrushImage {
                width: u.w as u32,
                height: u.h as u32,
                pixels: vec![t; u.w as usize * u.h as usize],
                handle: (-u.x, -u.y),
                transparent: Some(t),
            };
            let (ox, oy) = (-c.handle.0 - u.x, -c.handle.1 - u.y);
            for y in 0..c.height as i32 {
                for x in 0..c.width as i32 {
                    if c.is_opaque(x, y) {
                        let i = (oy + y) as usize * u.w as usize + (ox + x) as usize;
                        out.pixels[i] = c.get(x, y).expect("inside the cel");
                    }
                }
            }
            out
        })
    }
}

/// Brush `cel` renumbered for an arranged palette: `lut[old]` is the new index of colour `old`
/// ([`crate::remap::inverse`]), for its pixels and its transparent colour.
pub fn renumber_cel(cel: &mut BrushImage, lut: &[u8; 256]) {
    for p in &mut cel.pixels {
        *p = lut[usize::from(*p)];
    }
    cel.transparent = cel.transparent.map(|t| lut[usize::from(t)]);
}

/// The lowest colour below `colors` that no cel uses, else 0.
fn unused_colour(cels: &[BrushImage], colors: usize) -> u8 {
    let mut used = [false; 256];
    for c in cels {
        for &p in &c.pixels {
            used[usize::from(p)] = true;
        }
    }
    (0..colors.clamp(1, 256))
        .find(|&c| !used[c])
        .map_or(0, |c| c as u8)
}

/// Anim ▸ AnimBrush ▸ Pick up.. (AGA p.250, p.164): the part `rect` of `cels` frames, from the
/// current frame on (after the last frame it goes on with the first), each cel a brush with
/// its handle in the centre and `transparent` as transparent colour. `None` when `cels` is not
/// 2 up to the frame count or `rect` misses the page.
pub fn pick_up(
    doc: &Document,
    rect: Rect,
    cels: usize,
    transparent: Option<u8>,
) -> Option<AnimBrush> {
    let n = doc.frame_count();
    if !(MIN_CELS..=n.min(MAX_CELS)).contains(&cels) {
        return None;
    }
    let r = rect.intersect(doc.page().bounds());
    if r.is_empty() {
        return None;
    }
    let first = doc.current_frame();
    let cels = (0..cels)
        .map(|i| {
            let frame = doc.frame((first + i) % n)?;
            Some(BrushImage::from_image(&frame.crop(r), transparent))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(AnimBrush::new(cels))
}

// ---------------------------------------------------------------------------- Metamorph

/// Far away: farther than any real cell, so every real candidate is nearer.
const NO_SITE: (i32, i32) = (1 << 29, 1 << 29);

fn dist2(p: (i32, i32), s: (i32, i32)) -> i64 {
    let (dx, dy) = (i64::from(p.0 - s.0), i64::from(p.1 - s.1));
    dx * dx + dy * dy
}

/// For every cell of a `w`×`h` grid the nearest cell where `set` is true, by the two-pass
/// 8SSEDT propagation of nearest sites (Euclidean up to rare one-cell errors); [`NO_SITE`]
/// everywhere when no cell is set.
fn nearest_sites(w: usize, h: usize, set: &[bool]) -> Vec<(i32, i32)> {
    let mut site: Vec<(i32, i32)> = (0..w * h)
        .map(|i| {
            if set[i] {
                ((i % w) as i32, (i / w) as i32)
            } else {
                NO_SITE
            }
        })
        .collect();
    let relax = |site: &mut Vec<(i32, i32)>, x: usize, y: usize, dx: i32, dy: i32| {
        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
            return;
        }
        let cand = site[ny as usize * w + nx as usize];
        let p = (x as i32, y as i32);
        if cand != NO_SITE && dist2(p, cand) < dist2(p, site[y * w + x]) {
            site[y * w + x] = cand;
        }
    };
    for y in 0..h {
        for x in 0..w {
            relax(&mut site, x, y, -1, 0);
            relax(&mut site, x, y, 0, -1);
            relax(&mut site, x, y, -1, -1);
            relax(&mut site, x, y, 1, -1);
        }
        for x in (0..w).rev() {
            relax(&mut site, x, y, 1, 0);
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            relax(&mut site, x, y, 1, 0);
            relax(&mut site, x, y, 0, 1);
            relax(&mut site, x, y, 1, 1);
            relax(&mut site, x, y, -1, 1);
        }
        for x in 0..w {
            relax(&mut site, x, y, -1, 0);
        }
    }
    site
}

/// One brush resampled onto the morph grid for one cel.
struct Sample {
    /// Opaque cells.
    mask: Vec<bool>,
    /// The brush colour at each opaque cell.
    colour: Vec<u8>,
    /// For each cell the nearest opaque cell (itself when opaque); [`NO_SITE`] when the
    /// brush has no opaque cell here.
    inside: Vec<(i32, i32)>,
    /// Signed distance to the shape's edge in cells: negative inside, positive outside,
    /// never 0 (half a cell at the edge).
    distance: Vec<f64>,
}

/// The morph grid: the canvas around the handle (`canvas`, relative to the handle) with one
/// empty cell on every side, so shapes touching the canvas edge still have an outside.
struct Grid {
    canvas: Rect,
    w: usize,
    h: usize,
}

impl Grid {
    /// Position relative to the handle of grid cell (`gx`, `gy`).
    fn rel(&self, gx: usize, gy: usize) -> (i32, i32) {
        (self.canvas.x + gx as i32 - 1, self.canvas.y + gy as i32 - 1)
    }

    /// `b` scaled into the rectangle `content` (relative to the handle), sampled at every cell.
    fn sample(&self, b: &BrushImage, content: Rect) -> Sample {
        let n = self.w * self.h;
        let mut mask = vec![false; n];
        let mut colour = vec![0u8; n];
        for gy in 0..self.h {
            for gx in 0..self.w {
                let (rx, ry) = self.rel(gx, gy);
                if !content.contains(rx, ry) {
                    continue;
                }
                let u = (f64::from(rx - content.x) + 0.5) / f64::from(content.w);
                let v = (f64::from(ry - content.y) + 0.5) / f64::from(content.h);
                let sx = ((u * f64::from(b.width)).floor() as i32).clamp(0, b.width as i32 - 1);
                let sy = ((v * f64::from(b.height)).floor() as i32).clamp(0, b.height as i32 - 1);
                if b.is_opaque(sx, sy) {
                    let i = gy * self.w + gx;
                    mask[i] = true;
                    colour[i] = b.get(sx, sy).expect("inside the brush");
                }
            }
        }
        let inside = nearest_sites(self.w, self.h, &mask);
        let empty = !mask.iter().any(|&m| m);
        let outside_mask: Vec<bool> = mask.iter().map(|&m| !m).collect();
        let outside = nearest_sites(self.w, self.h, &outside_mask);
        let far = (self.w + self.h) as f64;
        let distance = (0..n)
            .map(|i| {
                let p = ((i % self.w) as i32, (i / self.w) as i32);
                if empty {
                    far
                } else if mask[i] {
                    -((dist2(p, outside[i]) as f64).sqrt() - 0.5)
                } else {
                    (dist2(p, inside[i]) as f64).sqrt() - 0.5
                }
            })
            .collect();
        Sample {
            mask,
            colour,
            inside,
            distance,
        }
    }

    /// The colour of the brush at its opaque cell nearest to cell `i`.
    fn nearest_colour(&self, s: &Sample, i: usize) -> Option<u8> {
        let (x, y) = s.inside[i];
        (s.inside[i] != NO_SITE).then(|| s.colour[y as usize * self.w + x as usize])
    }
}

/// The canvas every cel of a morph of `from` into `to` covers, relative to the handle: the
/// union of both brushes.
pub fn morph_canvas(from: &BrushImage, to: &BrushImage) -> Rect {
    let ea = raster::brush_extent(from).unwrap_or_default();
    let eb = raster::brush_extent(to).unwrap_or_default();
    ea.union(eb)
}

/// Most cels a morph of `from` into `to` can have: every cel covers the [`morph_canvas`] and
/// all of them together hold at most [`MAX_CEL_BYTES`] (and there are at most [`MAX_CELS`]).
/// Below [`MIN_CELS`] when not even two cels fit.
pub fn max_morph_cels(from: &BrushImage, to: &BrushImage) -> usize {
    let c = morph_canvas(from, to);
    let per_cel = u64::from(c.w.max(1).unsigned_abs()) * u64::from(c.h.max(1).unsigned_abs());
    usize::try_from(MAX_CEL_BYTES / per_cel)
        .unwrap_or(MAX_CELS)
        .min(MAX_CELS)
}

fn lerp(a: i32, b: i32, t: f64) -> i32 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as i32
}

/// Brush ▸ Spare ▸ Metamorph.. (AGA p.243, addendum p.11-12): an AnimBrush of `cels` cels (at
/// least 2) that changes the shape and colours of `from` into those of `to`. DeluxePaint IV
/// AGA morphs from the current brush (first cel) to the spare brush (last cel); Alt-b
/// (Brush<->Spare) before Metamorph turns the order round.
///
/// Every cel lies on one canvas around the handle that holds both brushes. In cel `i` (`t =
/// i / (cels - 1)`) both brushes are scaled into the rectangle between theirs (size and
/// position around the handle interpolated); the shape is where the interpolated signed
/// distances to both outlines are negative, so shapes grow, shrink and flow into each other
/// rather than fading; each pixel's colour is the palette colour nearest to the mix of the two
/// brushes' colours there (outside a brush: its nearest opaque pixel). The first and last cel
/// are the brushes themselves. The transparent colour is `from`'s (else `to`'s, else the lowest
/// unused colour) and painted pixels never take it.
///
/// `None`, before any work, when the cels would hold more than [`MAX_CEL_BYTES`] (more than
/// [`max_morph_cels`]).
pub fn metamorph(
    from: &BrushImage,
    to: &BrushImage,
    cels: usize,
    palette: &Palette,
) -> Option<AnimBrush> {
    let cels = cels.clamp(MIN_CELS, MAX_CELS);
    if cels > max_morph_cels(from, to) {
        return None;
    }
    let ea = raster::brush_extent(from).unwrap_or_default();
    let eb = raster::brush_extent(to).unwrap_or_default();
    let canvas = morph_canvas(from, to);
    let grid = Grid {
        canvas,
        w: canvas.w as usize + 2,
        h: canvas.h as usize + 2,
    };
    let t_colour = from
        .transparent
        .or(to.transparent)
        .unwrap_or_else(|| unused_colour(&[from.clone(), to.clone()], palette.len()));
    let colour_of = |c: u8| palette.get(c).unwrap_or(Rgb8::BLACK);
    let avoid_t = |c: u8| {
        if c == t_colour {
            palette.nearest_excluding(colour_of(c), &[t_colour])
        } else {
            c
        }
    };
    let out = (0..cels)
        .map(|i| {
            let t = i as f64 / (cels - 1) as f64;
            let content = Rect::new(
                lerp(ea.x, eb.x, t),
                lerp(ea.y, eb.y, t),
                lerp(ea.w, eb.w, t).max(1),
                lerp(ea.h, eb.h, t).max(1),
            );
            let a = grid.sample(from, content);
            let b = grid.sample(to, content);
            let mut mixed: Vec<Option<u8>> = vec![None; 256 * 256];
            let mut cel = BrushImage {
                width: canvas.w as u32,
                height: canvas.h as u32,
                pixels: vec![t_colour; (canvas.w * canvas.h) as usize],
                handle: (-canvas.x, -canvas.y),
                transparent: Some(t_colour),
            };
            for gy in 1..grid.h - 1 {
                for gx in 1..grid.w - 1 {
                    let g = gy * grid.w + gx;
                    let d = (1.0 - t) * a.distance[g] + t * b.distance[g];
                    let opaque = if i == 0 {
                        a.mask[g]
                    } else if i == cels - 1 {
                        b.mask[g]
                    } else {
                        d < 0.0
                    };
                    if !opaque {
                        continue;
                    }
                    let (ca, cb) = (grid.nearest_colour(&a, g), grid.nearest_colour(&b, g));
                    let c = match (ca, cb) {
                        (Some(x), _) if i == 0 => x,
                        (_, Some(y)) if i == cels - 1 => y,
                        (Some(x), Some(y)) if x == y => x,
                        (Some(x), Some(y)) => *mixed[usize::from(x) * 256 + usize::from(y)]
                            .get_or_insert_with(|| {
                                let (p, q) = (colour_of(x), colour_of(y));
                                let m = |u: u8, v: u8| lerp(i32::from(u), i32::from(v), t) as u8;
                                let rgb = Rgb8::new(m(p.r, q.r), m(p.g, q.g), m(p.b, q.b));
                                palette.nearest_excluding(rgb, &[t_colour])
                            }),
                        (Some(x), None) | (None, Some(x)) => x,
                        (None, None) => continue,
                    };
                    let px = (gy - 1) * canvas.w as usize + (gx - 1);
                    cel.pixels[px] = avoid_t(c);
                }
            }
            cel
        })
        .collect();
    Some(AnimBrush::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::IndexedImage;

    fn brush(w: u32, h: u32, pixels: &[u8]) -> BrushImage {
        BrushImage {
            width: w,
            height: h,
            pixels: pixels.to_vec(),
            handle: (w as i32 / 2, h as i32 / 2),
            transparent: Some(0),
        }
    }

    fn solid(w: u32, h: u32, c: u8) -> BrushImage {
        brush(w, h, &vec![c; (w * h) as usize])
    }

    fn cels(n: usize) -> AnimBrush {
        AnimBrush::new((0..n).map(|i| solid(1, 1, i as u8 + 1)).collect())
    }

    fn sequence(b: &AnimBrush, stamps: u64) -> Vec<usize> {
        (0..stamps).map(|k| b.cel_index_at(k)).collect()
    }

    #[test]
    fn cels_advance_one_per_stamp_by_default_and_wrap() {
        let b = cels(3);
        assert_eq!(b.duration, 3);
        assert_eq!(sequence(&b, 7), [0, 1, 2, 0, 1, 2, 0]);
        let mut b = b;
        b.advance(4);
        assert_eq!(b.current(), 1);
        assert_eq!(sequence(&b, 3), [1, 2, 0]);
    }

    #[test]
    fn duration_spreads_or_skips_cels() {
        // 10 cels, Duration 20: every cel twice (AGA p.251).
        let mut b = cels(10);
        b.set_duration(20);
        assert_eq!(
            sequence(&b, 20),
            [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9]
        );
        assert_eq!(b.cel_index_at(20), 0, "the cycle takes 20 stamps");
        // Duration 5 on 10 cels skips every other cel.
        b.set_duration(5);
        assert_eq!(sequence(&b, 6), [0, 2, 4, 6, 8, 0]);
        // Duration 3 on 2 cels: spread evenly (Bresenham).
        let mut b = cels(2);
        b.set_duration(3);
        assert_eq!(sequence(&b, 6), [0, 0, 1, 0, 0, 1]);
    }

    #[test]
    fn directions_and_ping_pong_cycle() {
        let mut b = cels(4);
        b.set_direction(CelDirection::Backward);
        assert_eq!(b.current(), 0, "the current cel stays");
        assert_eq!(sequence(&b, 5), [0, 3, 2, 1, 0]);
        // Ping-pong of 11 cels plays over exactly 20 stamps (AGA p.167).
        let mut b = cels(11);
        b.set_direction(CelDirection::PingPong);
        let s = sequence(&b, 21);
        assert_eq!(
            s,
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0
            ]
        );
        assert_eq!(CelDirection::PingPong.period(11), 20);
    }

    #[test]
    fn current_cel_and_clock_survive_settings() {
        let mut b = cels(5);
        b.set_current(3);
        assert_eq!(b.current(), 3);
        assert_eq!(b.current_cel().pixels, [4]);
        b.set_direction(CelDirection::Backward);
        assert_eq!(b.current(), 3);
        assert_eq!(sequence(&b, 3), [3, 2, 1]);
        b.set_direction(CelDirection::PingPong);
        b.set_duration(10);
        assert_eq!(b.current(), 3);
        b.set_current(99);
        assert_eq!(b.current(), 4, "clamped to the last cel");
        // Advancing by a whole cycle comes back to the same cel.
        let before = b.current();
        b.advance(CelDirection::PingPong.period(5) * 10 / 5);
        assert_eq!(b.current(), before);
        // Huge stamp counts do not overflow.
        b.advance(u64::MAX);
        assert!(b.current() < 5);
    }

    #[test]
    fn pick_up_takes_cels_from_the_current_frame_on_and_wraps() {
        let mut doc = Document::new(8, 6, Palette::dpaint_default_32());
        let mut e = crate::Engine::new(doc.clone());
        e.execute(crate::Command::SetFrameCount { count: 4 })
            .unwrap();
        for f in 0..4 {
            e.execute(crate::Command::GotoFrame { frame: f }).unwrap();
            e.execute(crate::Command::Clear { color: f as u8 + 1 })
                .unwrap();
        }
        e.execute(crate::Command::GotoFrame { frame: 2 }).unwrap();
        doc = e.document().clone();
        let r = Rect::new(1, 1, 3, 2);
        let b = pick_up(&doc, r, 3, Some(0)).unwrap();
        assert_eq!(b.len(), 3);
        let firsts: Vec<u8> = b.cels.iter().map(|c| c.pixels[0]).collect();
        assert_eq!(firsts, [3, 4, 1], "frames 3, 4, then 1");
        assert!(
            b.cels
                .iter()
                .all(|c| (c.width, c.height, c.handle) == (3, 2, (1, 1)))
        );
        assert_eq!(b.duration, 3);
        assert!(
            pick_up(&doc, r, 1, None).is_none(),
            "one cel is not allowed"
        );
        assert!(pick_up(&doc, r, 5, None).is_none(), "more cels than frames");
        assert!(pick_up(&doc, Rect::new(20, 20, 2, 2), 2, None).is_none());
        let clipped = pick_up(&doc, Rect::new(6, 4, 9, 9), 2, None).unwrap();
        assert_eq!((clipped.cels[0].width, clipped.cels[0].height), (2, 2));
    }

    #[test]
    fn transforms_map_every_cel_and_remap_fits_colours() {
        let b = AnimBrush::new(vec![brush(2, 1, &[1, 2]), brush(2, 1, &[3, 0])]);
        let f = b.map(crate::brush_ops::flip_horizontal);
        assert_eq!(f.cels[0].pixels, [2, 1]);
        assert_eq!(f.cels[1].pixels, [0, 3]);
        assert!(b.try_map(|_| None).is_none());
        // Colour 2 differs between the palettes: it goes to the nearest colour of the new one.
        let from = Palette::dpaint_default_32();
        let mut to = from.clone();
        to.set(2, Rgb8::new(1, 2, 3));
        let (r, changed) = b.remap(&from, &to);
        assert_eq!(changed, 1);
        assert_ne!(r.cels[0].pixels[1], 2);
        assert_eq!(r.cels[0].pixels[0], 1);
        assert_eq!(r.cels[1].pixels, [3, 0], "transparent colour kept");
    }

    #[test]
    fn transforms_stop_at_the_byte_limit() {
        let mut b = cels(3);
        b.set_direction(CelDirection::PingPong);
        b.set_duration(5);
        b.advance(2);
        let flip = crate::brush_ops::flip_horizontal;
        assert_eq!(b.map_within(flip), Some(b.map(flip)));
        // Each cel grows from 1 to 4 bytes: 12 bytes fit a limit of 12, not one of 7, and the
        // work stops at the cel that crosses the limit.
        let mut made = 0;
        let mut grow = |c: &BrushImage| {
            made += 1;
            solid(2, 2, c.pixels[0])
        };
        let grown = b.map_within_bytes(12, &mut grow).expect("fits");
        assert_eq!(
            (grown.bytes(), grown.clock, grown.duration),
            (12, b.clock, 5)
        );
        assert_eq!(grown.direction, CelDirection::PingPong);
        made = 0;
        let mut grow = |c: &BrushImage| {
            made += 1;
            solid(2, 2, c.pixels[0])
        };
        assert!(b.map_within_bytes(7, &mut grow).is_none());
        assert_eq!(made, 2, "the third cel is never made");
    }

    #[test]
    fn arrange_renumbers_every_cel_and_keeps_its_look() {
        let pal = Palette::dpaint_default_32();
        let mut b = AnimBrush::new(vec![brush(2, 1, &[1, 2]), brush(2, 1, &[3, 0])]);
        b.advance(1);
        let before = b.clone();
        // Colour 2 moves to the front: order[new] = old.
        let mut order = crate::remap::identity(32);
        crate::remap::move_entry(&mut order, 2, 0);
        let arranged = Palette::new(crate::remap::arranged_palette(pal.colors(), &order)).unwrap();
        b.renumber(&crate::remap::inverse(&order));
        assert_eq!(b.cels[0].pixels, [2, 0]);
        assert_eq!(b.cels[1].pixels, [3, 1]);
        assert_eq!(
            b.cels[1].transparent,
            Some(1),
            "the transparent colour follows"
        );
        for (old, new) in before.cels.iter().zip(&b.cels) {
            for (&o, &n) in old.pixels.iter().zip(&new.pixels) {
                assert_eq!(pal.get(o), arranged.get(n));
            }
        }
        assert_eq!((b.clock, b.duration), (before.clock, before.duration));
    }

    #[test]
    fn cels_go_on_one_canvas_around_the_handle() {
        let mut small = brush(1, 1, &[5]);
        small.handle = (0, 0);
        let mut wide = brush(3, 2, &[1, 2, 3, 4, 0, 6]);
        wide.handle = (2, 1);
        let b = AnimBrush::new(vec![small, wide.clone()]);
        let c = b.on_one_canvas(32);
        assert!(
            c.cels
                .iter()
                .all(|k| (k.width, k.height, k.handle) == (3, 2, (2, 1)))
        );
        assert_eq!(c.cels[0].pixels, [0, 0, 0, 0, 0, 5]);
        assert_eq!(c.cels[1], wide);
        assert_eq!(c.on_one_canvas(32), c, "nothing to do the second time");
    }

    fn canvas_image(b: &BrushImage) -> IndexedImage {
        IndexedImage::from_pixels(b.width, b.height, b.pixels.clone()).unwrap()
    }

    #[test]
    fn metamorph_starts_and_ends_with_the_two_brushes() {
        let pal = Palette::dpaint_default_32();
        // A 3x3 square of colour 1 into a 7x7 ring of colour 2.
        let from = solid(3, 3, 1);
        let mut to = solid(7, 7, 2);
        for y in 2..5 {
            for x in 2..5 {
                to.pixels[y * 7 + x] = 0;
            }
        }
        let m = metamorph(&from, &to, 5, &pal).unwrap();
        assert_eq!(m.len(), 5);
        assert!(
            m.cels
                .iter()
                .all(|c| (c.width, c.height, c.handle) == (7, 7, (3, 3)))
        );
        let first = &m.cels[0];
        for y in 0..7 {
            for x in 0..7 {
                let want = if (2..5).contains(&x) && (2..5).contains(&y) {
                    1
                } else {
                    0
                };
                assert_eq!(first.get(x, y), Some(want), "cel 1 at {x},{y}");
            }
        }
        assert_eq!(
            m.cels[4].pixels, to.pixels,
            "the last cel is the spare brush"
        );
        // In between the shape grows: opaque pixel counts rise from 9 to 40.
        let count = |c: &BrushImage| (0..49).filter(|&i| c.pixels[i] != 0).count();
        let counts: Vec<usize> = m.cels.iter().map(count).collect();
        assert_eq!((counts[0], counts[4]), (9, 40));
        assert!(counts[1] > 9 && counts[3] > counts[1], "{counts:?}");
        // The middle cel mixes the colours (light grey 1 and the default palette's colour 2).
        let middle: Vec<u8> = m.cels[2]
            .pixels
            .iter()
            .copied()
            .filter(|&p| p != 0)
            .collect();
        assert!(middle.iter().all(|&p| p != 0));
        assert_eq!(canvas_image(&m.cels[2]).width(), 7);
    }

    #[test]
    fn metamorph_is_deterministic_and_never_paints_the_transparent_colour() {
        let pal = Palette::dpaint_default_32();
        let from = brush(4, 2, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut to = brush(2, 4, &[9, 10, 11, 12, 13, 14, 15, 16]);
        to.transparent = Some(9);
        let a = metamorph(&from, &to, 6, &pal).unwrap();
        assert_eq!(Some(&a), metamorph(&from, &to, 6, &pal).as_ref());
        for c in &a.cels {
            assert_eq!(c.transparent, Some(0));
            // Only padding is transparent: from's colours never include 0 here.
        }
        // The last cel keeps `to`'s shape; its own transparent colour 9 is not painted.
        let last = &a.cels[5];
        let opaque = last.pixels.iter().filter(|&&p| p != 0).count();
        assert_eq!(opaque, 7, "to has 7 opaque pixels");
        // Reversed order (Alt-b before Metamorph) starts with the other brush.
        let r = metamorph(&to, &from, 6, &pal).unwrap();
        assert_eq!(
            r.cels[0].pixels.iter().filter(|&&p| p != 9).count(),
            7,
            "to first"
        );
        // At least two cels.
        assert_eq!(metamorph(&from, &to, 0, &pal).map(|m| m.len()), Some(2));
    }

    #[test]
    fn metamorph_refuses_more_cels_than_the_byte_limit_holds() {
        let pal = Palette::dpaint_default_32();
        let dot = solid(1, 1, 1);
        let mut far = solid(1, 1, 2);
        // The handles 8191 pixels apart: one 8192x8192 canvas, two cels hold the limit.
        far.handle = (-8191, -8191);
        assert_eq!(morph_canvas(&dot, &far), Rect::new(0, 0, 8192, 8192));
        assert_eq!(max_morph_cels(&dot, &far), 2);
        far.handle = (-8192, -8192);
        assert_eq!(max_morph_cels(&dot, &far), 1, "not even two cels fit");
        assert!(
            metamorph(&dot, &far, 2, &pal).is_none(),
            "refused before any work"
        );
        let big = solid(1000, 1000, 1);
        assert_eq!(max_morph_cels(&big, &big), 134);
        assert!(metamorph(&big, &big, 135, &pal).is_none());
        assert_eq!(max_morph_cels(&dot, &dot), MAX_CELS);
    }

    /// A Stroke with a custom brush of cel 0, wrapped for `b`.
    fn wrapped(b: &AnimBrush, paint: crate::Command) -> crate::Command {
        crate::Command::AnimBrushPaint {
            brush: b.clone(),
            paint: Box::new(paint),
        }
    }

    fn stroke(points: Vec<(i32, i32)>, continuous: bool, custom: &BrushImage) -> crate::Command {
        crate::Command::Stroke {
            brush: Vec::new(),
            color: 1,
            points,
            continuous,
            custom: Some(custom.clone()),
            mode: crate::DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    fn dots(
        points: Vec<(i32, i32)>,
        custom: &BrushImage,
        sym: Option<crate::Symmetry>,
    ) -> crate::Command {
        crate::Command::Shape {
            shape: crate::Shape::Dots { points },
            brush: Vec::new(),
            color: 1,
            filled: false,
            symmetry: sym,
            custom: Some(custom.clone()),
            mode: crate::DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        }
    }

    #[test]
    fn anim_brush_paint_is_one_undo_step_and_round_trips_as_json() {
        let doc = Document::new(16, 8, Palette::dpaint_default_32());
        let mut e = crate::Engine::new(doc);
        let b = cels(3);
        let cmd = wrapped(&b, stroke(vec![(1, 1), (6, 1)], true, &b.cels[0]));
        assert_eq!(cmd.stamps(), 6);
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(
            json.starts_with(r#"{"op":"anim_brush_paint","brush":{"cels":["#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<crate::Command>(&json).unwrap(), cmd);
        e.execute(cmd).unwrap();
        let row: Vec<u8> = (1..7)
            .map(|x| e.document().page().get(x, 1).unwrap())
            .collect();
        assert_eq!(row, [1, 2, 3, 1, 2, 3], "one cel per stamp");
        e.undo().unwrap();
        assert!(e.document().page().pixels().iter().all(|&p| p == 0));
        assert!(e.undo().is_err(), "a single undo step");
    }

    #[test]
    fn a_stroke_painted_in_parts_with_the_clock_equals_the_whole() {
        let page = IndexedImage::new(24, 16, 0);
        let pal = Palette::dpaint_default_32();
        let mut b = AnimBrush::new((0..5).map(|i| solid(3, 2, i + 1)).collect());
        b.set_duration(7);
        b.set_direction(CelDirection::PingPong);
        b.set_current(2);
        let pts = vec![(-5, 2), (10, 4), (18, 14), (3, 9), (30, 0)];
        for continuous in [true, false] {
            let whole = wrapped(&b, stroke(pts.clone(), continuous, &b.cels[0]));
            let mut a = page.clone();
            whole.paint(&mut a, &page, &pal);
            let mut parts = page.clone();
            let mut clock = b.clone();
            let mut stamps = 0;
            let first = wrapped(&clock, stroke(vec![pts[0]], continuous, &b.cels[0]));
            if !continuous {
                first.paint(&mut parts, &page, &pal);
                clock.advance(first.stamps());
                stamps += first.stamps();
            }
            for w in pts.windows(2) {
                let part = if continuous {
                    vec![w[0], w[1]]
                } else {
                    vec![w[1]]
                };
                let cmd = wrapped(&clock, stroke(part, continuous, &b.cels[0]));
                cmd.paint(&mut parts, &page, &pal);
                clock.advance(cmd.stamps());
                stamps += cmd.stamps();
            }
            assert_eq!(parts, a, "continuous {continuous}");
            assert_eq!(stamps, whole.stamps());
            let mut moved = b.clone();
            moved.advance(whole.stamps());
            assert_eq!(moved.clock, clock.clock);
        }
    }

    #[test]
    fn symmetric_copies_show_the_cels_of_the_original() {
        let page = IndexedImage::new(21, 21, 0);
        let pal = Palette::dpaint_default_32();
        let b = cels(4);
        let sym = crate::Symmetry {
            center: (10, 10),
            order: 4,
            aspect: (1, 1),
            mirror: false,
        };
        let pts = vec![(14, 10), (16, 10), (18, 10)];
        let cmd = wrapped(&b, dots(pts, &b.cels[0], Some(sym)));
        let mut img = page.clone();
        cmd.paint(&mut img, &page, &pal);
        // Original along +x, copies along +y, -x, -y: cels 1, 2, 3 at each.
        for (k, d) in [4, 6, 8].iter().enumerate() {
            let c = k as u8 + 1;
            for p in [(10 + d, 10), (10, 10 + d), (10 - d, 10), (10, 10 - d)] {
                assert_eq!(img.get(p.0, p.1), Some(c), "{p:?}");
            }
        }
        assert_eq!(cmd.stamps(), 3, "the copies do not count");
    }

    #[test]
    fn stamps_count_path_stamps() {
        let c = solid(1, 1, 1);
        assert_eq!(
            stroke(vec![(0, 0), (4, 2), (4, 5)], true, &c).stamps(),
            5 + 4
        );
        assert_eq!(stroke(vec![(0, 0), (4, 2), (4, 5)], false, &c).stamps(), 3);
        assert_eq!(stroke(vec![(3, 3)], true, &c).stamps(), 1);
        let rect = |filled| crate::Command::Shape {
            shape: crate::Shape::Rect {
                from: (0, 0),
                to: (3, 2),
            },
            brush: vec![(0, 0)],
            color: 1,
            filled,
            symmetry: None,
            custom: None,
            mode: crate::DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: Default::default(),
        };
        assert_eq!(rect(false).stamps(), 4 + 3 + 4 + 3);
        assert_eq!(rect(true).stamps(), 0);
        assert_eq!(crate::Command::Clear { color: 0 }.stamps(), 0);
    }

    #[test]
    fn bad_anim_brushes_are_refused() {
        let doc = Document::new(8, 8, Palette::dpaint_default_32());
        let mut e = crate::Engine::new(doc);
        let good = cels(2);
        let line = stroke(vec![(0, 0), (3, 3)], true, &good.cels[0]);
        let mut zero = good.clone();
        zero.duration = 0;
        let mut empty = good.clone();
        empty.cels.clear();
        let mut outside = good.clone();
        outside.cels[1].pixels[0] = 200;
        for (b, paint) in [
            (zero, line.clone()),
            (empty, line.clone()),
            (outside, line.clone()),
            (good.clone(), crate::Command::Clear { color: 1 }),
        ] {
            let cmd = wrapped(&b, paint);
            assert!(e.execute(cmd).is_err());
        }
        assert_eq!(e.document().revision(), 0);
        // The work estimate counts the largest cel: 3000 stamps of a 1000x1000 cel are too
        // much, of the 1x1 first cel alone they are not.
        let mut e = crate::Engine::new(Document::new(1000, 1000, Palette::dpaint_default_32()));
        let mut big = good.clone();
        big.cels[1] = solid(1000, 1000, 1);
        let long = vec![(0, 0), (999, 0), (0, 1), (999, 1)];
        let alone = stroke(long.clone(), true, &good.cels[0]);
        assert!(
            alone
                .check_paint(e.document().page(), e.document().palette())
                .is_ok()
        );
        let err = e.execute(wrapped(&big, alone)).unwrap_err();
        assert!(err.message.contains("too much work"), "{}", err.message);
    }

    #[test]
    fn nearest_sites_finds_the_nearest_set_cell() {
        let (w, h) = (9, 7);
        let mut set = vec![false; w * h];
        set[0] = true;
        set[w * h - 1] = true;
        set[3 * w + 4] = true;
        let sites = nearest_sites(w, h, &set);
        let seeds = [(0, 0), (8, 6), (4, 3)];
        for y in 0..h {
            for x in 0..w {
                let p = (x as i32, y as i32);
                let best = seeds.iter().map(|&s| dist2(p, s)).min().unwrap();
                assert_eq!(dist2(p, sites[y * w + x]), best, "at {p:?}");
            }
        }
        assert!(
            nearest_sites(3, 3, &[false; 9])
                .iter()
                .all(|&s| s == NO_SITE)
        );
    }
}
