//! DeluxePainter core: document model and command engine shared by the GUI, CLI and MCP
//! front-ends.
//!
//! A [`Document`] holds animation frames (ADR-28) of one layer stack (ADR-44) with one
//! palette, changed only through [`Engine`] (validated [`Command`]s with multi-level
//! undo/redo).

// Animation frames (ADR-28).
pub mod anim;
mod frame_ops;
// AnimBrush and Metamorph (ADR-36).
pub mod anim_brush;
// Anim ▸ Move.. (ADR-38).
pub mod anim_move;

pub mod brush_ops;
pub mod command;
pub mod document;
pub mod engine;
pub mod gradient;
pub mod image;
// Layers (ADR-44).
pub mod layer;
// Picture ▸ Load's Import requester (ADR-42).
pub mod import;
pub mod mode;
pub mod paint;
pub mod palette;
// Effect > Perspective (ADR-37).
pub mod perspective;
pub mod quantize;
pub mod raster;
pub mod remap;
pub mod resample;
pub mod rgba;
pub mod shapes;
// Area stencils and Use Brush Palette (ADR-39).
pub mod brush_palette;
pub mod stencil;

pub use anim::{LightTable, PlayMode};
pub use anim_brush::{AnimBrush, CelDirection};
pub use command::{Axis, Command, CommandError, ErrorCode};
pub use document::Document;
pub use engine::{Engine, Outcome, Request};
pub use gradient::{Direction as GradientDirection, FillType};
pub use image::{IndexedImage, Rect};
pub use layer::Layer;
pub use mode::{Chipset, DisplayMode, Resolution, Target, Video};
pub use paint::{AntiAlias, BrushImage, DrawMode, PaintContext, PaintFx, ProcessMode};
pub use palette::{Palette, PaletteError, Rgb8};
pub use rgba::{Rgba8, RgbaImage};
pub use shapes::{Shape, Symmetry};
pub use stencil::AreaStencil;
