//! True-colour pictures as they come from PNG or JPEG files, before colour reduction
//! (ADR-9). Pages are always indexed; an [`RgbaImage`] never becomes part of a document.

/// One straight (not premultiplied) RGBA pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }
}

/// A rectangular buffer of RGBA pixels, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<Rgba8>,
}

impl RgbaImage {
    /// Creates an image filled with `fill`.
    pub fn new(width: u32, height: u32, fill: Rgba8) -> Self {
        Self {
            width,
            height,
            pixels: vec![fill; width as usize * height as usize],
        }
    }

    /// Wraps a row-major pixel buffer; `None` if its length is not `width * height`.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<Rgba8>) -> Option<Self> {
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

    pub fn pixels(&self) -> &[Rgba8] {
        &self.pixels
    }

    /// The pixel at (`x`, `y`), or `None` outside the image.
    pub fn get(&self, x: u32, y: u32) -> Option<Rgba8> {
        (x < self.width && y < self.height)
            .then(|| self.pixels[y as usize * self.width as usize + x as usize])
    }

    /// Sets the pixel at (`x`, `y`); does nothing outside the image.
    pub fn set(&mut self, x: u32, y: u32, p: Rgba8) {
        if x < self.width && y < self.height {
            self.pixels[y as usize * self.width as usize + x as usize] = p;
        }
    }

    /// The `width`×`height` part whose top-left corner is (`x`, `y`), clipped to the image
    /// (ADR-45: crop-to-fill of generated pictures).
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> RgbaImage {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = width.min(self.width - x);
        let h = height.min(self.height - y);
        let mut pixels = Vec::with_capacity(w as usize * h as usize);
        for row in y..y + h {
            let start = row as usize * self.width as usize + x as usize;
            pixels.extend_from_slice(&self.pixels[start..start + w as usize]);
        }
        RgbaImage {
            width: w,
            height: h,
            pixels,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_set_and_bounds() {
        let mut img = RgbaImage::new(3, 2, Rgba8::default());
        img.set(2, 1, Rgba8::opaque(1, 2, 3));
        img.set(3, 0, Rgba8::opaque(9, 9, 9));
        assert_eq!(img.get(2, 1), Some(Rgba8::opaque(1, 2, 3)));
        assert_eq!(img.get(3, 0), None);
        assert_eq!(img.pixels().len(), 6);
        assert!(RgbaImage::from_pixels(2, 2, vec![Rgba8::default(); 3]).is_none());
    }

    #[test]
    fn crop_takes_the_part_and_clips() {
        let mut img = RgbaImage::new(4, 3, Rgba8::default());
        for y in 0..3 {
            for x in 0..4 {
                img.set(x, y, Rgba8::opaque(x as u8, y as u8, 0));
            }
        }
        let part = img.crop(1, 1, 2, 2);
        assert_eq!((part.width(), part.height()), (2, 2));
        assert_eq!(part.get(0, 0), Some(Rgba8::opaque(1, 1, 0)));
        assert_eq!(part.get(1, 1), Some(Rgba8::opaque(2, 2, 0)));
        let clipped = img.crop(3, 2, 5, 5);
        assert_eq!((clipped.width(), clipped.height()), (1, 1));
        assert_eq!(clipped.get(0, 0), Some(Rgba8::opaque(3, 2, 0)));
        let outside = img.crop(9, 9, 2, 2);
        assert_eq!((outside.width(), outside.height()), (0, 0));
    }
}
