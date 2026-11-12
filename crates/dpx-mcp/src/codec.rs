//! The file encoders behind `save_project` and `export_image` (ADR-43). The headless server
//! uses [`BuiltinCodec`] ([`crate::pictures`]); the GUI passes its own, so a file Claude
//! saves from the running app is byte-for-byte what Picture > Save / Export write (the app's
//! `files.rs`, with its colour ranges, grab point and transparent colour).
//!
//! The tools check the file-access policy, the hardware limits of exports
//! ([`crate::pictures::validate_hardware`]) and overwriting themselves, before and after
//! calling a codec; a codec only turns a document into bytes. It runs on a server thread.

use dpx_core::{Document, IndexedImage};
use dpx_formats::project::ProjectExtras;

use crate::error::ToolError;
use crate::pictures::{self, Format};

pub trait Codec: Send + Sync + std::fmt::Debug {
    /// The `.dpxproj` bytes of `doc` (the shown page) with `extras` and, where the codec
    /// keeps it, the spare page.
    fn encode_project(
        &self,
        doc: &Document,
        extras: &ProjectExtras,
        spare: Option<&IndexedImage>,
    ) -> Result<Vec<u8>, ToolError>;

    /// The bytes of `doc` as IFF ILBM, IFF ANIM or indexed PNG, with notes on what the
    /// format cannot hold. The hardware limits were checked already.
    fn encode_export(
        &self,
        doc: &Document,
        extras: &ProjectExtras,
        format: Format,
    ) -> Result<(Vec<u8>, Vec<String>), ToolError>;
}

/// The encoders of [`crate::pictures`]: the project keeps the spare page; exports carry the
/// display mode but no colour ranges or grab point.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinCodec;

impl Codec for BuiltinCodec {
    fn encode_project(
        &self,
        doc: &Document,
        extras: &ProjectExtras,
        spare: Option<&IndexedImage>,
    ) -> Result<Vec<u8>, ToolError> {
        pictures::encode_project(doc, extras.clone(), spare.cloned())
    }

    fn encode_export(
        &self,
        doc: &Document,
        _extras: &ProjectExtras,
        format: Format,
    ) -> Result<(Vec<u8>, Vec<String>), ToolError> {
        pictures::encode_document(doc, format)
    }
}
