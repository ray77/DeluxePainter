//! DeluxePainter core: document model and command engine shared by the GUI, CLI and MCP
//! front-ends.
//!
//! Phase 1: one indexed page with its palette, changed only through [`Engine`] (validated
//! [`Command`]s with multi-level undo/redo). Layers and frames follow (see
//! `docs/ARCHITECTURE.md`).

pub mod command;
pub mod document;
pub mod engine;
pub mod image;
pub mod mode;
pub mod palette;
pub mod quantize;
pub mod raster;
pub mod resample;
pub mod rgba;

pub use command::{Axis, Command, CommandError, ErrorCode};
pub use document::Document;
pub use engine::{Engine, Outcome, Request};
pub use image::{IndexedImage, Rect};
pub use mode::{Chipset, DisplayMode, Resolution, Target, Video};
pub use palette::{Palette, PaletteError, Rgb8};
pub use rgba::{Rgba8, RgbaImage};
