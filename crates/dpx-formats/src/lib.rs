//! Non-Amiga picture formats for DeluxePainter. Phase 2: PNG import/export of indexed
//! pictures. True-colour import (PNG, JPEG) needs colour reduction and follows later.

pub mod png_io;

pub use png_io::{PngError, PngPicture, read_png, write_png};
