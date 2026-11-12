//! Reading pixels and rendering the page to PNG for the client. Read-only; no pixel logic.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dpx_core::{Document, IndexedImage, Palette, Rect};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ToolError;

/// Most pixels `get_pixels` returns in one call.
pub const MAX_PIXELS: u64 = 65_536;
/// Most pixels of a rendered image (after scaling).
pub const MAX_RENDER_PIXELS: u64 = 4_194_304;
/// The longest side an automatically scaled preview aims for.
pub const AUTO_PREVIEW_SIDE: u32 = 1024;
/// Largest automatic preview scale.
pub const AUTO_PREVIEW_SCALE: u32 = 4;

/// A page rectangle in pixels.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct RectArg {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl From<Rect> for RectArg {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            w: r.w.max(0) as u32,
            h: r.h.max(0) as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PixelEncoding {
    /// `pixels` is an array of rows, each an array of palette indices.
    #[default]
    Rows,
    /// `pixels` is one base64 string of the raw index bytes, row by row (top to bottom).
    Base64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PixelsResult {
    /// The rectangle actually returned: the request clipped to the page.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub encoding: PixelEncoding,
    /// Rows of indices, or a base64 string (see `encoding`).
    pub pixels: serde_json::Value,
    pub revision: u64,
}

/// The rectangle clipped to `page`; error if nothing is left.
pub fn clip(page: Rect, rect: Option<RectArg>) -> Result<Rect, ToolError> {
    let Some(r) = rect else { return Ok(page) };
    if r.w == 0 || r.h == 0 {
        return Err(ToolError::invalid("rect width and height must be positive"));
    }
    const FAR: i32 = 1_000_000;
    if r.x.abs() > FAR || r.y.abs() > FAR || r.w > FAR as u32 || r.h > FAR as u32 {
        return Err(ToolError::invalid(
            "rect coordinates and size must be within 1,000,000",
        ));
    }
    let r = Rect::new(r.x, r.y, r.w as i32, r.h as i32);
    let clipped = r.intersect(page);
    if clipped.is_empty() {
        return Err(ToolError::invalid(format!(
            "rect lies outside the {}x{} page",
            page.w, page.h
        )));
    }
    Ok(clipped)
}

pub fn get_pixels(
    doc: &Document,
    rect: Option<RectArg>,
    encoding: PixelEncoding,
) -> Result<PixelsResult, ToolError> {
    let r = clip(doc.page().bounds(), rect)?;
    if (r.w as u64) * (r.h as u64) > MAX_PIXELS {
        return Err(ToolError::invalid(format!(
            "{}x{} = {} pixels exceed the limit of {MAX_PIXELS} per call; request a smaller rect (use get_preview to look at the whole picture)",
            r.w,
            r.h,
            (r.w as u64) * (r.h as u64)
        )));
    }
    let crop = doc.page().crop(r);
    let pixels = match encoding {
        PixelEncoding::Rows => serde_json::Value::Array(
            crop.pixels()
                .chunks(r.w as usize)
                .map(|row| serde_json::json!(row))
                .collect(),
        ),
        PixelEncoding::Base64 => serde_json::Value::String(STANDARD.encode(crop.pixels())),
    };
    Ok(PixelsResult {
        x: r.x,
        y: r.y,
        w: r.w as u32,
        h: r.h as u32,
        encoding,
        pixels,
        revision: doc.revision(),
    })
}

/// The scale for a preview of `w`×`h` pixels: `scale` if given (1..=8), else the largest of
/// 1..=4 that keeps the longer side within [`AUTO_PREVIEW_SIDE`]. The image may have at most
/// [`MAX_RENDER_PIXELS`].
pub fn preview_scale(w: u32, h: u32, scale: Option<u32>) -> Result<u32, ToolError> {
    let scale = match scale {
        Some(s) if !(1..=8).contains(&s) => {
            return Err(ToolError::invalid("scale must be an integer from 1 to 8"));
        }
        Some(s) => s,
        None => (AUTO_PREVIEW_SIDE / w.max(h).max(1)).clamp(1, AUTO_PREVIEW_SCALE),
    };
    let (ow, oh) = (
        u64::from(w) * u64::from(scale),
        u64::from(h) * u64::from(scale),
    );
    if ow * oh > MAX_RENDER_PIXELS {
        return Err(ToolError::invalid(format!(
            "image {ow}x{oh} exceeds {MAX_RENDER_PIXELS} pixels; lower the scale or request a smaller rect"
        )));
    }
    Ok(scale)
}

/// A rendered RGB PNG.
pub struct Rendered {
    pub png: Vec<u8>,
    pub scale: u32,
    pub width: u32,
    pub height: u32,
}

/// Renders `crop` with `palette` applied, nearest-neighbour scaled by the integer `scale`
/// (checked by [`preview_scale`]). Pixel aspect is not corrected.
pub fn render_png(
    crop: &IndexedImage,
    palette: &Palette,
    scale: u32,
) -> Result<Rendered, ToolError> {
    let (w, h) = (crop.width(), crop.height());
    let (ow, oh) = (w * scale, h * scale);
    let colors = palette.colors();
    let s = scale as usize;
    let mut rgb = Vec::with_capacity(ow as usize * oh as usize * 3);
    for row in crop.pixels().chunks(w.max(1) as usize) {
        let start = rgb.len();
        for &p in row {
            let c = colors.get(usize::from(p)).copied().unwrap_or_default();
            for _ in 0..s {
                rgb.extend_from_slice(&[c.r, c.g, c.b]);
            }
        }
        let line = rgb[start..].to_vec();
        for _ in 1..s {
            rgb.extend_from_slice(&line);
        }
    }
    let mut png_bytes = Vec::new();
    let mut enc = png::Encoder::new(&mut png_bytes, ow, oh);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&rgb))
        .map_err(|e| ToolError::internal(format!("PNG encoding failed: {e}")))?;
    Ok(Rendered {
        png: png_bytes,
        scale,
        width: ow,
        height: oh,
    })
}

pub fn to_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn from_base64(text: &str) -> Result<Vec<u8>, ToolError> {
    let text: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    // Allow a data: URL prefix.
    let text = match text.find(";base64,") {
        Some(i) if text.starts_with("data:") => &text[i + 8..],
        _ => &text[..],
    };
    STANDARD
        .decode(text)
        .map_err(|e| ToolError::invalid(format!("data is not valid base64: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_preview_scale() {
        assert_eq!(preview_scale(320, 256, None).unwrap(), 3);
        assert_eq!(preview_scale(640, 512, None).unwrap(), 1);
        assert_eq!(preview_scale(16, 16, None).unwrap(), 4);
        assert_eq!(
            preview_scale(4096, 4096, None).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
        assert_eq!(
            preview_scale(10, 10, Some(9)).unwrap_err().code,
            "INVALID_ARGUMENT"
        );
        assert_eq!(preview_scale(1280, 512, Some(2)).unwrap(), 2);
    }

    #[test]
    fn base64_with_and_without_data_url() {
        assert_eq!(from_base64("aGk=").unwrap(), b"hi");
        assert_eq!(from_base64("data:image/png;base64,aG\nk=").unwrap(), b"hi");
        assert!(from_base64("%%").is_err());
    }
}
