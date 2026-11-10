//! Document commands: the single mutation API shared by the GUI, the CLI and MCP (ADR-2).
//!
//! Commands are plain serialisable values. [`crate::Engine`] validates them, applies them and
//! records undo information.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::image::{IndexedImage, Rect};
use crate::mode::DisplayMode;
use crate::palette::{Palette, Rgb8};
use crate::raster;
use crate::shapes::{Shape, Symmetry};

/// Most points one stroke may carry.
pub const MAX_STROKE_POINTS: usize = 100_000;
/// Most pixels a brush may have (DeluxePaint brushes go up to 100×100 when resized).
pub const MAX_BRUSH_PIXELS: usize = 100 * 100;
/// Largest absolute brush offset.
pub const MAX_BRUSH_OFFSET: i32 = 1_000;
/// Largest absolute stroke coordinate; points may lie outside the page but not arbitrarily far.
pub const MAX_COORD: i32 = 1_000_000;
/// Largest page width or height.
pub const MAX_PAGE_SIDE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// Mirror left <-> right.
    Horizontal,
    /// Mirror top <-> bottom.
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Command {
    /// Freehand stroke (Dotted / Continuous Freehand): the brush is stamped at every point and,
    /// if `continuous`, along Bresenham lines between consecutive points.
    Stroke {
        /// Brush pixels as offsets from its hotspot.
        brush: Vec<(i32, i32)>,
        color: u8,
        points: Vec<(i32, i32)>,
        continuous: bool,
    },
    /// A drawing-tool shape (ADR-14): the brush stamped along the outline, or with `filled` a
    /// solid shape in `color` (the brush is not used). With `symmetry`, rotated copies too.
    Shape {
        shape: Shape,
        #[serde(default)]
        brush: Vec<(i32, i32)>,
        color: u8,
        #[serde(default)]
        filled: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        symmetry: Option<Symmetry>,
    },
    /// Fill tool: the 4-connected region of the colour at (`x`, `y`) becomes `color`.
    FloodFill { x: i32, y: i32, color: u8 },
    /// Fills the whole page with one colour (CLR).
    Clear { color: u8 },
    /// Mirrors the page (Picture ▸ Flip).
    Flip { axis: Axis },
    /// Recolours every pixel of `from` to `to` (Color ▸ BG -> FG).
    ReplaceColor { from: u8, to: u8 },
    /// Exchanges two colours in every pixel (Color ▸ BG <-> FG).
    SwapColors { a: u8, b: u8 },
    /// Replaces all palette entries; the number of colours must stay the same.
    SetPalette { colors: Vec<Rgb8> },
    /// Picture ▸ Screen Format: display mode, palette size and page size. With `retain` the
    /// picture is kept (top-left aligned, cut off if the page shrinks), otherwise the page is
    /// cleared to colour 0. New palette entries get a grey ramp; dropped entries must be
    /// unused when `retain` is set.
    SetFormat {
        mode: DisplayMode,
        colors: u16,
        width: u32,
        height: u32,
        retain: bool,
        /// Screen size; `None` = the mode's standard size (ADR-11).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        screen: Option<(u32, u32)>,
    },
    /// Picture ▸ Page Size: the picture stays top-left; new area gets `fill` (the BG colour),
    /// a smaller page crops (ADR-10).
    SetPageSize { width: u32, height: u32, fill: u8 },
    /// Replaces the whole page with `pixels` (`width`×`height`, row by row), e.g. Picture ▸
    /// Spare ▸ Delete This Page or CLR on a fixed background (ADR-15). Another size must fit the
    /// target's page limits.
    SetPage {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    /// Picture ▸ Spare ▸ Merge in Front / Back: combines the page with another page
    /// (`width`×`height`, top-left aligned, clipped to the page). `front`: the other page lies
    /// on top and its `transparent` pixels let the page show through; otherwise the page lies
    /// on top and its own `transparent` pixels show the other page (ADR-15).
    MergePage {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        front: bool,
        transparent: u8,
    },
}

/// What a command changes; decides what the engine snapshots for undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Footprint {
    /// Page pixels inside the rect (already clipped to the page; may be empty).
    Pixels(Rect),
    Palette,
    /// Page size, palette and target may all change.
    Document,
}

impl Command {
    /// Checks the command against the document without changing anything.
    pub(crate) fn validate(&self, doc: &Document) -> Result<(), CommandError> {
        let colors = doc.palette.len();
        let color = |c: u8| {
            if usize::from(c) < colors {
                Ok(())
            } else {
                Err(CommandError::new(
                    ErrorCode::PaletteRangeInvalid,
                    format!("colour {c} is outside the palette (0..{colors})"),
                ))
            }
        };
        match self {
            Command::Stroke {
                brush,
                color: c,
                points,
                ..
            } => {
                color(*c)?;
                if brush.is_empty() || brush.len() > MAX_BRUSH_PIXELS {
                    return Err(CommandError::invalid(format!(
                        "brush must have 1..={MAX_BRUSH_PIXELS} pixels, got {}",
                        brush.len()
                    )));
                }
                if brush
                    .iter()
                    .any(|&(x, y)| x.abs() > MAX_BRUSH_OFFSET || y.abs() > MAX_BRUSH_OFFSET)
                {
                    return Err(CommandError::invalid(format!(
                        "brush offsets must lie within ±{MAX_BRUSH_OFFSET}"
                    )));
                }
                if points.is_empty() || points.len() > MAX_STROKE_POINTS {
                    return Err(CommandError::invalid(format!(
                        "stroke must have 1..={MAX_STROKE_POINTS} points, got {}",
                        points.len()
                    )));
                }
                if points
                    .iter()
                    .any(|&(x, y)| x.abs() > MAX_COORD || y.abs() > MAX_COORD)
                {
                    return Err(CommandError::invalid(format!(
                        "stroke points must lie within ±{MAX_COORD}"
                    )));
                }
                Ok(())
            }
            Command::Shape {
                shape,
                brush,
                color: c,
                filled,
                symmetry,
            } => {
                color(*c)?;
                shape.validate(brush, *filled, symmetry.as_ref())
            }
            Command::FloodFill { x, y, color: c } => {
                color(*c)?;
                if x.abs() > MAX_COORD || y.abs() > MAX_COORD {
                    return Err(CommandError::invalid(format!(
                        "fill point must lie within ±{MAX_COORD}"
                    )));
                }
                Ok(())
            }
            Command::Clear { color: c } => color(*c),
            Command::Flip { .. } => Ok(()),
            Command::ReplaceColor { from, to } => color(*from).and(color(*to)),
            Command::SwapColors { a, b } => color(*a).and(color(*b)),
            Command::SetFormat {
                mode,
                colors: n,
                width,
                height,
                retain,
                screen,
            } => {
                let max = mode.max_colors(doc.target.chipset).ok_or_else(|| {
                    CommandError::new(
                        ErrorCode::UnsupportedMode,
                        format!(
                            "{} is not available on {:?}",
                            mode.name(),
                            doc.target.chipset
                        ),
                    )
                })?;
                if !n.is_power_of_two() || *n < 2 || *n > max {
                    return Err(CommandError::new(
                        ErrorCode::PaletteRangeInvalid,
                        format!(
                            "{n} colours: {} allows 2..={max} (powers of two)",
                            mode.name()
                        ),
                    ));
                }
                if !(1..=MAX_PAGE_SIDE).contains(width) || !(1..=MAX_PAGE_SIDE).contains(height) {
                    return Err(CommandError::invalid(format!(
                        "page {width}x{height} is outside 1..={MAX_PAGE_SIDE}"
                    )));
                }
                let (sw, sh) = screen.unwrap_or(mode.size());
                if !mode.screen_fits(sw, sh) {
                    let ((a, b), (c, d)) = mode.screen_limits();
                    return Err(CommandError::invalid(format!(
                        "screen {sw}x{sh}: {} allows {a}x{b} to {c}x{d} in whole UI pixels",
                        mode.name()
                    )));
                }
                if *width < sw || *height < sh {
                    return Err(CommandError::invalid(format!(
                        "page {width}x{height} is smaller than the {sw}x{sh} screen"
                    )));
                }
                if *retain {
                    let kept = doc
                        .page
                        .crop(Rect::new(0, 0, *width as i32, *height as i32));
                    if let Some(&max_used) = kept.pixels().iter().max()
                        && u16::from(max_used) >= *n
                    {
                        return Err(CommandError::new(
                            ErrorCode::PaletteRangeInvalid,
                            format!(
                                "the picture uses colour {max_used}; keep at least {} colours",
                                (u16::from(max_used) + 1).next_power_of_two()
                            ),
                        ));
                    }
                }
                Ok(())
            }
            Command::SetPageSize {
                width,
                height,
                fill,
            } => {
                color(*fill)?;
                if doc.target.page_fits(*width, *height) {
                    Ok(())
                } else {
                    let ((min_w, min_h), (max_w, max_h)) = doc.target.page_limits();
                    Err(CommandError::invalid(format!(
                        "Page must be {min_w}x{min_h} to {max_w}x{max_h}"
                    )))
                }
            }
            Command::SetPage {
                width,
                height,
                pixels,
            } => {
                other_page(*width, *height, pixels, colors)?;
                let same = (*width, *height) == (doc.page.width(), doc.page.height());
                if same || doc.target.page_fits(*width, *height) {
                    Ok(())
                } else {
                    let ((min_w, min_h), (max_w, max_h)) = doc.target.page_limits();
                    Err(CommandError::invalid(format!(
                        "Page must be {min_w}x{min_h} to {max_w}x{max_h}"
                    )))
                }
            }
            Command::MergePage {
                width,
                height,
                pixels,
                transparent,
                ..
            } => {
                color(*transparent)?;
                other_page(*width, *height, pixels, colors)
            }
            Command::SetPalette { colors: new } => {
                if new.len() == colors {
                    Ok(())
                } else {
                    Err(CommandError::invalid(format!(
                        "palette must keep {colors} colours, got {}",
                        new.len()
                    )))
                }
            }
        }
    }

    /// The part of the document this command changes. Call only after [`Self::validate`].
    pub(crate) fn footprint(&self, doc: &Document) -> Footprint {
        let page = doc.page.bounds();
        match self {
            Command::Stroke { brush, points, .. } => Footprint::Pixels(
                raster::stroke_bounds(brush, points).map_or(Rect::default(), |r| r.intersect(page)),
            ),
            Command::Shape { .. } | Command::FloodFill { .. } => {
                Footprint::Pixels(self.paint_bounds(&doc.page).unwrap_or_default())
            }
            Command::Clear { .. }
            | Command::Flip { .. }
            | Command::ReplaceColor { .. }
            | Command::SwapColors { .. } => Footprint::Pixels(page),
            Command::SetPalette { .. } => Footprint::Palette,
            Command::SetFormat { .. } | Command::SetPageSize { .. } => Footprint::Document,
            Command::SetPage { width, height, .. } => {
                if (*width, *height) == (doc.page.width(), doc.page.height()) {
                    Footprint::Pixels(page)
                } else {
                    Footprint::Document
                }
            }
            Command::MergePage { width, height, .. } => {
                Footprint::Pixels(page.intersect(Rect::new(0, 0, *width as i32, *height as i32)))
            }
        }
    }

    /// Applies the command. Call only after [`Self::validate`].
    pub(crate) fn apply(&self, doc: &mut Document) {
        let page = &mut doc.page;
        match self {
            Command::Stroke {
                brush,
                color,
                points,
                continuous,
            } => raster::stroke(page, brush, *color, points, *continuous),
            Command::Shape { .. } | Command::FloodFill { .. } => {
                self.paint(page);
            }
            Command::Clear { color } => {
                let (w, h) = (page.width() as i32, page.height() as i32);
                page.fill_rect(0, 0, w, h, *color);
            }
            Command::Flip {
                axis: Axis::Horizontal,
            } => page.flip_horizontal(),
            Command::Flip {
                axis: Axis::Vertical,
            } => page.flip_vertical(),
            Command::ReplaceColor { from, to } => page.replace_index(*from, *to),
            Command::SwapColors { a, b } => page.swap_indices(*a, *b),
            Command::SetPalette { colors } => {
                doc.palette = Palette::new(colors.clone()).expect("size checked by validate");
            }
            Command::SetFormat {
                mode,
                colors,
                width,
                height,
                retain,
                screen,
            } => {
                let mut new_page = IndexedImage::new(*width, *height, 0);
                if *retain {
                    new_page.blit(&doc.page, 0, 0);
                }
                doc.page = new_page;
                doc.palette = resized_palette(&doc.palette, usize::from(*colors));
                doc.target.mode = *mode;
                doc.target.screen = screen.unwrap_or(mode.size());
            }
            Command::SetPageSize {
                width,
                height,
                fill,
            } => {
                let mut new_page = IndexedImage::new(*width, *height, *fill);
                new_page.blit(&doc.page, 0, 0);
                doc.page = new_page;
            }
            Command::SetPage {
                width,
                height,
                pixels,
            } => {
                doc.page = IndexedImage::from_pixels(*width, *height, pixels.clone())
                    .expect("size checked by validate");
            }
            Command::MergePage {
                width,
                height,
                pixels,
                front,
                transparent,
            } => {
                let w = (*width).min(page.width()) as usize;
                let h = (*height).min(page.height()) as usize;
                let pw = page.width() as usize;
                let mut merged = page.pixels().to_vec();
                for y in 0..h {
                    let other = &pixels[y * *width as usize..][..w];
                    for (x, &o) in other.iter().enumerate() {
                        let p = &mut merged[y * pw + x];
                        if (*front && o != *transparent) || (!*front && *p == *transparent) {
                            *p = o;
                        }
                    }
                }
                *page = IndexedImage::from_pixels(page.width(), page.height(), merged)
                    .expect("same size");
            }
        }
    }
}

/// Checks a page passed in a command: `width`×`height` pixels, all inside the palette.
fn other_page(width: u32, height: u32, pixels: &[u8], colors: usize) -> Result<(), CommandError> {
    if !(1..=MAX_PAGE_SIDE).contains(&width) || !(1..=MAX_PAGE_SIDE).contains(&height) {
        return Err(CommandError::invalid(format!(
            "page {width}x{height} is outside 1..={MAX_PAGE_SIDE}"
        )));
    }
    let n = width as usize * height as usize;
    if pixels.len() != n {
        return Err(CommandError::invalid(format!(
            "page {width}x{height} needs {n} pixels, got {}",
            pixels.len()
        )));
    }
    match pixels.iter().max() {
        Some(&max) if usize::from(max) >= colors => Err(CommandError::new(
            ErrorCode::PaletteRangeInvalid,
            format!("colour {max} is outside the palette (0..{colors})"),
        )),
        _ => Ok(()),
    }
}

/// Keeps the first `n` entries; missing ones become a grey ramp from black to white.
fn resized_palette(palette: &Palette, n: usize) -> Palette {
    let mut colors: Vec<Rgb8> = palette.colors().iter().copied().take(n).collect();
    let start = colors.len();
    let extra = n - start;
    for i in 0..extra {
        let v = if extra > 1 {
            (i * 255 / (extra - 1)) as u8
        } else {
            255
        };
        colors.push(Rgb8::new(v, v, v));
    }
    Palette::new(colors).expect("1..=256 entries")
}

/// Stable error codes, also used by the MCP interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    PaletteRangeInvalid,
    RevisionConflict,
    UnsupportedMode,
    NothingToUndo,
    NothingToRedo,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ErrorCode::PaletteRangeInvalid => "PALETTE_RANGE_INVALID",
            ErrorCode::RevisionConflict => "REVISION_CONFLICT",
            ErrorCode::UnsupportedMode => "UNSUPPORTED_MODE",
            ErrorCode::NothingToUndo => "NOTHING_TO_UNDO",
            ErrorCode::NothingToRedo => "NOTHING_TO_REDO",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: String) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CommandError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_is_stable() {
        let cmd = Command::Flip {
            axis: Axis::Horizontal,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, r#"{"op":"flip","axis":"horizontal"}"#);
        let stroke: Command = serde_json::from_str(
            r#"{"op":"stroke","brush":[[0,0]],"color":1,"points":[[3,4],[5,6]],"continuous":true}"#,
        )
        .unwrap();
        assert_eq!(
            stroke,
            Command::Stroke {
                brush: vec![(0, 0)],
                color: 1,
                points: vec![(3, 4), (5, 6)],
                continuous: true,
            }
        );
        let pal = Command::SetPalette {
            colors: vec![Rgb8::new(1, 2, 3)],
        };
        let back: Command = serde_json::from_str(&serde_json::to_string(&pal).unwrap()).unwrap();
        assert_eq!(back, pal);
        assert_eq!(
            serde_json::to_string(&ErrorCode::PaletteRangeInvalid).unwrap(),
            format!("\"{}\"", ErrorCode::PaletteRangeInvalid.as_str())
        );
    }

    fn doc_4x2() -> Document {
        let page = IndexedImage::from_pixels(4, 2, vec![0, 1, 0, 2, 3, 0, 0, 0]).unwrap();
        Document::from_picture(
            page,
            Palette::dpaint_default_32(),
            crate::mode::Target::default(),
        )
    }

    #[test]
    fn merge_page_in_front_and_in_back() {
        // The other page is 3x3: clipped to the 4x2 page, top-left aligned.
        let other = vec![5, 0, 6, 7, 8, 0, 9, 9, 9];
        let merge = |front| Command::MergePage {
            width: 3,
            height: 3,
            pixels: other.clone(),
            front,
            transparent: 0,
        };
        let mut doc = doc_4x2();
        merge(true).validate(&doc).unwrap();
        assert_eq!(
            merge(true).footprint(&doc),
            Footprint::Pixels(Rect::new(0, 0, 3, 2))
        );
        merge(true).apply(&mut doc);
        // In front: the other page's colour-0 pixels let the page show through.
        assert_eq!(doc.page.pixels(), &[5, 1, 6, 2, 7, 8, 0, 0]);
        let mut doc = doc_4x2();
        merge(false).apply(&mut doc);
        // In back: the page's own colour-0 pixels show the other page.
        assert_eq!(doc.page.pixels(), &[5, 1, 6, 2, 3, 8, 0, 0]);
        // Colours outside the palette and wrong sizes are refused.
        let bad = Command::MergePage {
            width: 1,
            height: 1,
            pixels: vec![40],
            front: true,
            transparent: 0,
        };
        assert_eq!(
            bad.validate(&doc).unwrap_err().code,
            ErrorCode::PaletteRangeInvalid
        );
        let short = Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![0; 7],
        };
        assert_eq!(
            short.validate(&doc).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn set_page_replaces_the_pixels() {
        let mut doc = doc_4x2();
        let same = Command::SetPage {
            width: 4,
            height: 2,
            pixels: vec![1; 8],
        };
        same.validate(&doc).unwrap();
        assert_eq!(same.footprint(&doc), Footprint::Pixels(doc.page.bounds()));
        same.apply(&mut doc);
        assert_eq!(doc.page.pixels(), &[1; 8]);
        // Another size must be a valid page for the target (at least the 320x256 screen).
        let small = Command::SetPage {
            width: 2,
            height: 2,
            pixels: vec![0; 4],
        };
        assert!(small.validate(&doc).is_err());
        let big = Command::SetPage {
            width: 320,
            height: 300,
            pixels: vec![2; 320 * 300],
        };
        big.validate(&doc).unwrap();
        assert_eq!(big.footprint(&doc), Footprint::Document);
        let json = serde_json::to_string(&small).unwrap();
        assert!(json.starts_with(r#"{"op":"set_page","width":2"#), "{json}");
    }
}
