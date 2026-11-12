//! Reading pixels and rendering the page to PNG for the agent. Read-only; no pixel logic.
//! Both read the picture as it shows: the current frame's visible layers (ADR-44).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use dpx_core::{Document, Rect};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ToolError;

/// Most pixels `get_pixels` returns in one call.
pub const MAX_PIXELS: u64 = 65_536;
/// Most pixels of a rendered image (after scaling).
pub const MAX_RENDER_PIXELS: u64 = 4_194_304;

/// A page rectangle in pixels.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub struct RectArg {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
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

/// The rectangle clipped to the page; error if nothing is left.
fn clip(doc: &Document, rect: Option<RectArg>) -> Result<Rect, ToolError> {
    let page = doc.page().bounds();
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
    let r = clip(doc, rect)?;
    if (r.w as u64) * (r.h as u64) > MAX_PIXELS {
        return Err(ToolError::invalid(format!(
            "{}x{} = {} pixels exceed the limit of {MAX_PIXELS} per call; request a smaller rect",
            r.w,
            r.h,
            (r.w as u64) * (r.h as u64)
        )));
    }
    let crop = doc.composite().crop(r);
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

/// A rendered RGB PNG (base64) with what it shows.
pub struct Rendered {
    pub png: Vec<u8>,
    pub rect: Rect,
    pub scale: u32,
    pub width: u32,
    pub height: u32,
}

/// Renders `rect` (default: the whole page) with the palette applied, nearest-neighbour
/// scaled by the integer `scale` (1..=8). Pixel aspect is not corrected.
pub fn render_png(
    doc: &Document,
    rect: Option<RectArg>,
    scale: Option<u32>,
) -> Result<Rendered, ToolError> {
    let scale = scale.unwrap_or(1);
    if !(1..=8).contains(&scale) {
        return Err(ToolError::invalid("scale must be an integer from 1 to 8"));
    }
    let r = clip(doc, rect)?;
    let (ow, oh) = (r.w as u64 * u64::from(scale), r.h as u64 * u64::from(scale));
    if ow * oh > MAX_RENDER_PIXELS {
        return Err(ToolError::invalid(format!(
            "image {ow}x{oh} exceeds {MAX_RENDER_PIXELS} pixels; lower the scale or request a smaller rect"
        )));
    }
    let (ow, oh) = (ow as u32, oh as u32);
    let crop = doc.composite().crop(r);
    let colors = doc.palette().colors();
    let s = scale as usize;
    let mut rgb = Vec::with_capacity(ow as usize * oh as usize * 3);
    for row in crop.pixels().chunks(r.w as usize) {
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
        .map_err(|e| ToolError::new("INTERNAL_ERROR", format!("PNG encoding failed: {e}")))?;
    Ok(Rendered {
        png: png_bytes,
        rect: r,
        scale,
        width: ow,
        height: oh,
    })
}

pub fn to_base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{Command, Engine, Palette};

    /// ADR-44: what the agent reads and sees is the frame as it shows, all visible layers.
    #[test]
    fn pixels_and_renders_show_the_visible_layers() {
        let mut e = Engine::new(Document::new(8, 4, Palette::dpaint_default_32()));
        e.execute(Command::Clear { color: 4 }).unwrap();
        e.execute(Command::AddLayer { name: None }).unwrap();
        e.execute(Command::SetPage {
            width: 8,
            height: 4,
            pixels: [vec![0, 5, 5, 0, 0, 0, 0, 0], vec![0; 24]].concat(),
        })
        .unwrap();
        let row = |e: &Engine| {
            let r = get_pixels(
                e.document(),
                Some(RectArg {
                    x: 0,
                    y: 0,
                    w: 4,
                    h: 1,
                }),
                PixelEncoding::Rows,
            )
            .unwrap();
            r.pixels[0].clone()
        };
        assert_eq!(row(&e), serde_json::json!([4, 5, 5, 4]));
        e.execute(Command::SetLayerVisible {
            layer: 1,
            visible: false,
        })
        .unwrap();
        assert_eq!(row(&e), serde_json::json!([4, 4, 4, 4]));
        e.undo().unwrap();
        // The render's pixel 0 is colour 4 (orange), pixel 1 colour 5 (yellow).
        let png = render_png(e.document(), None, None).unwrap().png;
        let mut reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut rgb = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut rgb).unwrap();
        let pal = e.document().palette();
        let c = |i: u8| {
            let c = pal.get(i).unwrap();
            vec![c.r, c.g, c.b]
        };
        assert_eq!(rgb[0..3], c(4)[..]);
        assert_eq!(rgb[3..6], c(5)[..]);
    }
}
