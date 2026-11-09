//! Indexed (palette-based) pixel buffers.

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

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
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
}
