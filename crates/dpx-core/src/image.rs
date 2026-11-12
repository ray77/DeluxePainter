//! Indexed (palette-based) pixel buffers.

/// Axis-aligned pixel rectangle; empty when `w <= 0 || h <= 0`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    /// True when (`x`, `y`) lies inside; the right and bottom edges (`x + w`, `y + h`) are outside.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// Smallest rect containing both; an empty operand is ignored (union of two empties is empty).
    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return if other.is_empty() {
                Rect::default()
            } else {
                other
            };
        }
        if other.is_empty() {
            return self;
        }
        let x0 = self.x.min(other.x);
        let y0 = self.y.min(other.y);
        let x1 = (self.x + self.w).max(other.x + other.w);
        let y1 = (self.y + self.h).max(other.y + other.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Overlap of both; returns `Rect::default()` (empty) when they do not overlap.
    pub fn intersect(self, other: Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        if self.is_empty() || other.is_empty() || x1 <= x0 || y1 <= y0 {
            Rect::default()
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    /// Bounding box of the points (inclusive, so one point gives a 1×1 rect); `None` if no points.
    pub fn from_points(points: &[(i32, i32)]) -> Option<Rect> {
        let first = *points.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
        for &(x, y) in &points[1..] {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }
}

/// A rectangular buffer of palette indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl IndexedImage {
    /// Creates an image filled with `fill`.
    pub fn new(width: u32, height: u32, fill: u8) -> Self {
        Self {
            width,
            height,
            pixels: vec![fill; width as usize * height as usize],
        }
    }

    /// Wraps a row-major index buffer; `None` if its length is not `width * height`.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        (pixels.len() == width as usize * height as usize).then_some(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The pixels to change in place (the size stays).
    pub(crate) fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// `Rect::new(0, 0, width, height)`.
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    /// Copies the part of `r` that lies inside the image. The result has the size of
    /// `r.intersect(self.bounds())` (0×0 if they do not overlap).
    pub fn crop(&self, r: Rect) -> IndexedImage {
        let c = r.intersect(self.bounds());
        let mut out = IndexedImage::new(c.w as u32, c.h as u32, 0);
        for y in 0..c.h {
            let src = (c.y + y) as usize * self.width as usize + c.x as usize;
            let dst = y as usize * c.w as usize;
            out.pixels[dst..dst + c.w as usize]
                .copy_from_slice(&self.pixels[src..src + c.w as usize]);
        }
        out
    }

    fn offset(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            None
        } else {
            Some(y as usize * self.width as usize + x as usize)
        }
    }

    pub fn get(&self, x: i32, y: i32) -> Option<u8> {
        self.offset(x, y).map(|o| self.pixels[o])
    }

    /// Sets a pixel; coordinates outside the image are ignored (clipping).
    pub fn set(&mut self, x: i32, y: i32, index: u8) {
        if let Some(o) = self.offset(x, y) {
            self.pixels[o] = index;
        }
    }

    /// Fills a rectangle, clipped to the image.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, index: u8) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width as i32);
        let y1 = (y + h).min(self.height as i32);
        for yy in y0..y1 {
            for xx in x0..x1 {
                let o = yy as usize * self.width as usize + xx as usize;
                self.pixels[o] = index;
            }
        }
    }

    pub fn hline(&mut self, x0: i32, x1: i32, y: i32, index: u8) {
        for x in x0.min(x1)..=x0.max(x1) {
            self.set(x, y, index);
        }
    }

    pub fn vline(&mut self, x: i32, y0: i32, y1: i32, index: u8) {
        for y in y0.min(y1)..=y0.max(y1) {
            self.set(x, y, index);
        }
    }

    /// Mirrors the image about its vertical axis (left <-> right).
    pub fn flip_horizontal(&mut self) {
        let w = self.width as usize;
        if w > 0 {
            for row in self.pixels.chunks_exact_mut(w) {
                row.reverse();
            }
        }
    }

    /// Mirrors the image about its horizontal axis (top <-> bottom).
    pub fn flip_vertical(&mut self) {
        let (w, h) = (self.width as usize, self.height as usize);
        for y in 0..h / 2 {
            let (top, bottom) = self.pixels.split_at_mut((h - 1 - y) * w);
            top[y * w..(y + 1) * w].swap_with_slice(&mut bottom[..w]);
        }
    }

    /// Recolours every pixel of index `from` to `to`.
    pub fn replace_index(&mut self, from: u8, to: u8) {
        for p in &mut self.pixels {
            if *p == from {
                *p = to;
            }
        }
    }

    /// Exchanges the indices `a` and `b` in every pixel.
    pub fn swap_indices(&mut self, a: u8, b: u8) {
        for p in &mut self.pixels {
            if *p == a {
                *p = b;
            } else if *p == b {
                *p = a;
            }
        }
    }

    /// Copies `src` with its top-left corner at (`dx`, `dy`), clipped.
    pub fn blit(&mut self, src: &IndexedImage, dx: i32, dy: i32) {
        for y in 0..src.height as i32 {
            for x in 0..src.width as i32 {
                if let Some(i) = src.get(x, y) {
                    self.set(dx + x, dy + y, i);
                }
            }
        }
    }

    /// Copies the pixels of `r` from `src` to the same place here, clipped to both images.
    pub fn copy_rect_from(&mut self, src: &IndexedImage, r: Rect) {
        let c = r.intersect(self.bounds()).intersect(src.bounds());
        let w = c.w.max(0) as usize;
        for y in c.y..c.y + c.h {
            let from = y as usize * src.width as usize + c.x as usize;
            let to = y as usize * self.width as usize + c.x as usize;
            self.pixels[to..to + w].copy_from_slice(&src.pixels[from..from + w]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping() {
        let mut img = IndexedImage::new(4, 3, 0);
        img.set(-1, 0, 5);
        img.set(4, 0, 5);
        img.fill_rect(-2, -2, 4, 4, 7);
        assert_eq!(img.get(0, 0), Some(7));
        assert_eq!(img.get(1, 1), Some(7));
        assert_eq!(img.get(2, 2), Some(0));
        assert_eq!(img.get(9, 9), None);
    }

    fn numbered(w: u32, h: u32) -> IndexedImage {
        let mut img = IndexedImage::new(w, h, 0);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                img.set(x, y, (y * w as i32 + x) as u8);
            }
        }
        img
    }

    #[test]
    fn flips() {
        let mut img = numbered(3, 3);
        img.flip_horizontal();
        assert_eq!(img.pixels(), &[2, 1, 0, 5, 4, 3, 8, 7, 6]);
        let mut img = numbered(2, 3);
        img.flip_vertical();
        assert_eq!(img.pixels(), &[4, 5, 2, 3, 0, 1]);
        let mut img = numbered(2, 4);
        img.flip_vertical();
        img.flip_vertical();
        assert_eq!(img, numbered(2, 4));
    }

    #[test]
    fn recolouring() {
        let mut img = IndexedImage::new(3, 1, 0);
        img.set(1, 0, 1);
        img.set(2, 0, 2);
        img.swap_indices(0, 1);
        assert_eq!(img.pixels(), &[1, 0, 2]);
        img.replace_index(2, 1);
        assert_eq!(img.pixels(), &[1, 0, 1]);
    }

    #[test]
    fn rect_union_intersect() {
        let a = Rect::new(0, 0, 4, 4);
        let b = Rect::new(2, 2, 4, 4);
        assert_eq!(a.union(b), Rect::new(0, 0, 6, 6));
        assert_eq!(a.intersect(b), Rect::new(2, 2, 2, 2));
        let far = Rect::new(10, 10, 2, 2);
        assert!(a.intersect(far).is_empty());
        assert_eq!(a.intersect(far), Rect::default());
        // touching edges do not overlap
        assert!(a.intersect(Rect::new(4, 0, 2, 2)).is_empty());
        let e = Rect::new(50, 50, 0, 3);
        assert!(e.is_empty());
        assert_eq!(a.union(e), a);
        assert_eq!(e.union(a), a);
        assert_eq!(e.union(Rect::new(1, 1, -1, 1)), Rect::default());
        assert!(a.intersect(e).is_empty());
    }

    #[test]
    fn rect_from_points_and_contains() {
        assert_eq!(Rect::from_points(&[]), None);
        assert_eq!(Rect::from_points(&[(3, -2)]), Some(Rect::new(3, -2, 1, 1)));
        assert_eq!(
            Rect::from_points(&[(3, 1), (-1, 5), (0, 0)]),
            Some(Rect::new(-1, 0, 5, 6))
        );
        let r = Rect::new(2, 3, 4, 5);
        assert!(r.contains(2, 3));
        assert!(r.contains(5, 7));
        assert!(!r.contains(6, 3));
        assert!(!r.contains(2, 8));
        assert!(!r.contains(1, 3));
        assert!(!Rect::new(0, 0, 0, 5).contains(0, 0));
    }

    #[test]
    fn crop_cases() {
        let img = numbered(5, 4);
        assert_eq!(img.bounds(), Rect::new(0, 0, 5, 4));
        let inside = img.crop(Rect::new(1, 1, 3, 2));
        assert_eq!((inside.width(), inside.height()), (3, 2));
        assert_eq!(inside.pixels(), &[6, 7, 8, 11, 12, 13]);
        let partial = img.crop(Rect::new(-2, 2, 4, 5));
        assert_eq!((partial.width(), partial.height()), (2, 2));
        assert_eq!(partial.pixels(), &[10, 11, 15, 16]);
        let outside = img.crop(Rect::new(7, 7, 3, 3));
        assert_eq!((outside.width(), outside.height()), (0, 0));
        assert_eq!(img.crop(img.bounds()), img);
    }

    #[test]
    fn copy_rect_from_copies_in_place_clipped_to_both() {
        let src = numbered(5, 4);
        let mut dst = IndexedImage::new(4, 5, 99);
        dst.copy_rect_from(&src, Rect::new(-1, 2, 3, 9));
        // Rows 2..4 of src (it has 4 rows), columns 0..2.
        assert_eq!(dst.get(0, 2), Some(10));
        assert_eq!(dst.get(1, 3), Some(16));
        assert_eq!(dst.get(2, 2), Some(99), "right of the rectangle");
        assert_eq!(dst.get(0, 4), Some(99), "below src");
        assert_eq!(dst.get(0, 1), Some(99), "above the rectangle");
        dst.copy_rect_from(&src, Rect::new(9, 9, 2, 2));
        dst.copy_rect_from(&src, Rect::new(1, 1, 0, 3));
        assert_eq!(dst.pixels().iter().filter(|&&p| p != 99).count(), 4);
    }
}
