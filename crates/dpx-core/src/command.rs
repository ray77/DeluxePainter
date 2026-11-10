//! Document commands: the single mutation API shared by the GUI, the CLI and MCP (ADR-2).
//!
//! Commands are plain serialisable values. [`crate::Engine`] validates them, applies them and
//! records undo information.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::image::Rect;
use crate::palette::{Palette, Rgb8};
use crate::raster;

/// Most points one stroke may carry.
pub const MAX_STROKE_POINTS: usize = 100_000;
/// Most pixels a brush may have (DeluxePaint brushes go up to 100×100 when resized).
pub const MAX_BRUSH_PIXELS: usize = 100 * 100;
/// Largest absolute brush offset.
pub const MAX_BRUSH_OFFSET: i32 = 1_000;
/// Largest absolute stroke coordinate; points may lie outside the page but not arbitrarily far.
pub const MAX_COORD: i32 = 1_000_000;

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
}

/// What a command changes; decides what the engine snapshots for undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Footprint {
    /// Page pixels inside the rect (already clipped to the page; may be empty).
    Pixels(Rect),
    Palette,
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
            Command::Clear { color: c } => color(*c),
            Command::Flip { .. } => Ok(()),
            Command::ReplaceColor { from, to } => color(*from).and(color(*to)),
            Command::SwapColors { a, b } => color(*a).and(color(*b)),
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
            Command::Clear { .. }
            | Command::Flip { .. }
            | Command::ReplaceColor { .. }
            | Command::SwapColors { .. } => Footprint::Pixels(page),
            Command::SetPalette { .. } => Footprint::Palette,
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
        }
    }
}

/// Stable error codes, also used by the MCP interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    PaletteRangeInvalid,
    RevisionConflict,
    NothingToUndo,
    NothingToRedo,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ErrorCode::PaletteRangeInvalid => "PALETTE_RANGE_INVALID",
            ErrorCode::RevisionConflict => "REVISION_CONFLICT",
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

    fn invalid(message: String) -> Self {
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
}
