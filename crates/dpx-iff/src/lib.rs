//! IFF (EA Interchange File Format) support for DeluxePainter: the chunk container,
//! ByteRun1 compression and ILBM pictures and ANIM animations. See `docs/IFF_SUPPORT_MATRIX.md`.

pub mod anim;
pub mod anim_doc;
pub mod byterun1;
pub mod iff;
pub mod ilbm;

pub use ilbm::{ColorRange, Compression, Ilbm, IlbmError, Masking, SpecialMode};
