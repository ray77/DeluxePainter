//! The document: one indexed page plus its palette (Phase 1 subset of ADR-1).
//!
//! Layers, frames and hardware targets follow in later phases. Documents are changed only
//! through [`crate::Engine`], which keeps the revision counter and the undo history.

use crate::image::IndexedImage;
use crate::palette::Palette;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub(crate) page: IndexedImage,
    pub(crate) palette: Palette,
    pub(crate) revision: u64,
}

impl Document {
    /// A blank page (colour 0 everywhere) at revision 0.
    pub fn new(width: u32, height: u32, palette: Palette) -> Self {
        Self {
            page: IndexedImage::new(width, height, 0),
            palette,
            revision: 0,
        }
    }

    /// A document holding an existing picture (e.g. a loaded file) at revision 0.
    pub fn from_picture(page: IndexedImage, palette: Palette) -> Self {
        Self {
            page,
            palette,
            revision: 0,
        }
    }

    pub fn page(&self) -> &IndexedImage {
        &self.page
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Increases by one for every executed, undone or redone command.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
