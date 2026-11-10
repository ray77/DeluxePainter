//! DeluxePainter retro UI.
//!
//! The whole visible interface is rendered in Amiga screen pixels into an indexed buffer that
//! shares the picture palette, like the classic Amiga paint programs. The host (eframe) only
//! converts that buffer to RGBA and shows it with nearest-neighbour scaling.

pub mod bitmap;
pub mod file_requester;
pub mod font;
pub mod menu;
pub mod note_requester;
pub mod page_size_requester;
pub mod palette_requester;
pub mod pattern;
pub mod pens;
pub mod range_requester;
pub mod requester;
pub mod screen;
pub mod show_page;
pub mod toolbox;
pub(crate) mod widgets;

use dpx_core::{IndexedImage, Palette};

pub use file_requester::{Browse, Entry, FileKind, FileRequester, FileResponse, PlaceKind};
pub use menu::{Action, MenuState, PixelSize, Pref, Prefs};
pub use note_requester::{NoteRequester, NoteResponse};
pub use page_size_requester::{PageSizeRequester, PageSizeResponse, Preset};
pub use palette_requester::{PaletteRequester, PaletteResponse};
pub use pens::Pens;
pub use range_requester::{RangeData, RangeRequester, RangeResponse};
pub use requester::{ReqEvent, ReqKey, ReqResponse, ScreenFormatRequester};
pub use screen::{Hit, PaintMode, Screen, UiState};
pub use show_page::{ShowPage, ShowPageResponse};
pub use toolbox::Tool;

/// Converts an indexed screen to RGBA8, scaling each pixel by `sx`×`sy` (integer factors).
pub fn to_rgba(img: &IndexedImage, palette: &Palette, sx: usize, sy: usize) -> Vec<u8> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut out = vec![0u8; w * sx * h * sy * 4];
    let lut: Vec<[u8; 4]> = (0..=255u8)
        .map(|i| {
            palette
                .get(i)
                .map_or([0, 0, 0, 255], |c| [c.r, c.g, c.b, 255])
        })
        .collect();
    let row_bytes = w * sx * 4;
    for y in 0..h {
        let line = y * sy * row_bytes;
        for x in 0..w {
            let c = lut[usize::from(img.pixels()[y * w + x])];
            for i in 0..sx {
                let o = line + (x * sx + i) * 4;
                out[o..o + 4].copy_from_slice(&c);
            }
        }
        for j in 1..sy {
            out.copy_within(line..line + row_bytes, line + j * row_bytes);
        }
    }
    out
}
