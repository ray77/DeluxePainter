//! The document: one indexed page plus its palette (Phase 1 subset of ADR-1).
//!
//! Layers, frames and hardware targets follow in later phases. Documents are changed only
//! through [`crate::Engine`], which keeps the revision counter and the undo history.

use crate::image::IndexedImage;
use crate::mode::Target;
use crate::palette::Palette;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub(crate) page: IndexedImage,
    pub(crate) palette: Palette,
    pub(crate) target: Target,
    pub(crate) revision: u64,
}

impl Document {
    /// A blank page (colour 0 everywhere) at revision 0 for the default target.
    pub fn new(width: u32, height: u32, palette: Palette) -> Self {
        Self::from_picture(
            IndexedImage::new(width, height, 0),
            palette,
            Target::default(),
        )
    }

    /// A document holding an existing picture (e.g. a loaded file) at revision 0.
    pub fn from_picture(page: IndexedImage, palette: Palette, target: Target) -> Self {
        Self {
            page,
            palette,
            target,
            revision: 0,
        }
    }

    /// Chipset and display mode the document is made for.
    pub fn target(&self) -> Target {
        self.target
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
