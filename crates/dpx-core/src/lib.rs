//! DeluxePainter core: document model shared by the GUI, CLI and MCP front-ends.
//!
//! Phase 1 starts with palettes and indexed pixel buffers; the command engine, layers and
//! frames follow (see `docs/ARCHITECTURE.md`).

pub mod image;
pub mod palette;
pub mod raster;

pub use image::IndexedImage;
pub use palette::{Palette, PaletteError, Rgb8};
