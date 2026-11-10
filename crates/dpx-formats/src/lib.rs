//! Non-Amiga picture formats for DeluxePainter: indexed PNG import/export, true-colour PNG
//! and JPEG import (decoded to RGBA, JPEG with its EXIF orientation applied). Colour
//! reduction of true-colour pictures lives in `dpx_core::quantize` (ADR-9).

pub mod jpeg_io;
pub mod png_io;
pub mod project;

pub use jpeg_io::{JpegError, read_jpeg};
pub use png_io::{PngError, PngImage, PngPicture, read_png, write_png};
