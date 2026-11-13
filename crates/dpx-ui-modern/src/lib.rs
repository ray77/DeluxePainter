//! DeluxePainter's Modern skin (ADR-47): a resolution-independent vector interface with a
//! dark and a light theme, drawn on the CPU. Only the picture is pixels, shown with
//! nearest-neighbour sampling at whole factors.
//!
//! The skin shares the interface state ([`dpx_ui::UiState`]), the hit values
//! ([`dpx_ui::Hit`]) and therefore every action and hotkey with the Classic skin in `dpx-ui`.
//! This crate only decides where things are and how they look:
//!
//! - [`paint`]: the display list, in points; what layout code builds and tests assert on.
//! - [`theme`]: the colour tokens of the dark and the light scheme.
//! - [`font`]: the bundled Go fonts and text measuring.
//! - [`icons`]: our own vector icons for the tools and the built-in brushes.
//! - [`layout`]: the rectangles of the main window.
//! - [`hit`]: what lies under a point.
//! - [`build`]: display lists of the bar, the dock and the picture viewport.
//! - [`raster`]: display list to RGBA8 (`tiny-skia`, `ab_glyph`).
//! - [`frame`]: a whole window in one call, for headless snapshots. The live window
//!   rasterises the same lists region by region ([`Renderer::render_region`]).
//!
//! There is no egui here and no pixel logic: the picture arrives as a finished image.

pub mod build;
pub mod font;
pub mod frame;
pub mod hit;
pub mod icons;
pub mod layout;
pub mod paint;
pub mod raster;
pub mod theme;

pub use build::{Model, Toggles};
pub use frame::{Frame, render_window};
pub use hit::{Spot, hit};
pub use layout::{Chrome, MainLayout, Picture};
pub use raster::{Image, Renderer};
pub use theme::{Theme, ThemeKind};
