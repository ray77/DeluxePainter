//! Hardware rules of a document's target (ADR-46): what an OCS or AGA Amiga can show in a
//! display mode. They are document rules (CLAUDE.md): exports are checked against them and
//! refused when a rule is broken; nothing is reduced silently.
//!
//! The rules, in the order they are checked:
//! 1. the chipset can show the display mode (OCS has no SuperHiRes);
//! 2. the palette has at most [`DisplayMode::max_colors`] entries (OCS LoRes 32, OCS HiRes
//!    16, AGA 256);
//! 3. the page is within [`Target::page_limits`] ([`validate_picture`] also accepts a page
//!    smaller than the screen, as picture and brush files may be);
//! 4. on OCS every palette entry is a 12-bit colour (each channel a multiple of 0x11);
//! 5. every pixel of every frame is a palette colour.
//!
//! [`format_notes`] and [`iff_notes`] are the honest notes an export gives about screen sizes
//! that are not standard and about what an IFF file cannot carry of the target;
//! [`export_notes`] gives the ones of an export format.

use std::fmt;

use crate::document::Document;
use crate::mode::{Chipset, DisplayMode, Target};
use crate::palette::{Palette, Rgb8};
use crate::quantize::PaletteDepth;

/// The stable error code of a broken hardware rule (also used by MCP).
pub const CODE: &str = "HARDWARE_LIMIT";

/// A broken hardware rule: [`CODE`] plus a message that names the rule and the values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HardwareError {
    pub message: String,
}

impl HardwareError {
    fn new(message: String) -> Self {
        Self { message }
    }

    /// Always [`CODE`].
    pub fn code(&self) -> &'static str {
        CODE
    }
}

impl fmt::Display for HardwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{CODE}: {}", self.message)
    }
}

impl std::error::Error for HardwareError {}

/// The palette precision of a chipset: OCS 12-bit (4 bits per channel), AGA 24-bit.
pub fn depth(chipset: Chipset) -> PaletteDepth {
    match chipset {
        Chipset::Ocs => PaletteDepth::Rgb4,
        Chipset::Aga => PaletteDepth::Rgb8,
    }
}

/// True if `c` is an OCS 12-bit colour (every channel a multiple of 0x11).
pub fn is_12_bit(c: Rgb8) -> bool {
    Rgb8::from_rgb4(c.to_rgb4()) == c
}

/// `c` as `chipset` can show it: the nearest 12-bit colour on OCS, unchanged on AGA.
pub fn snap(chipset: Chipset, c: Rgb8) -> Rgb8 {
    match chipset {
        Chipset::Ocs => Rgb8::from_rgb4(c.to_rgb4()),
        Chipset::Aga => c,
    }
}

/// Snaps every colour to `chipset` ([`snap`]); returns how many changed.
pub fn snap_colors(chipset: Chipset, colors: &mut [Rgb8]) -> usize {
    let mut changed = 0;
    for c in colors {
        let s = snap(chipset, *c);
        if s != *c {
            *c = s;
            changed += 1;
        }
    }
    changed
}

/// The note for `n` colours rounded to 12-bit OCS values (the wording of ADR-9's import note).
pub fn rounded_note(n: usize) -> String {
    if n == 1 {
        "1 colour rounded to a 12-bit OCS value".into()
    } else {
        format!("{n} colours rounded to 12-bit OCS values")
    }
}

/// The first palette entry `chipset` cannot show, with its index (OCS: not 12-bit).
pub fn first_unshowable(chipset: Chipset, colors: &[Rgb8]) -> Option<(usize, Rgb8)> {
    match chipset {
        Chipset::Ocs => colors
            .iter()
            .copied()
            .enumerate()
            .find(|&(_, c)| !is_12_bit(c)),
        Chipset::Aga => None,
    }
}

fn error(message: String) -> HardwareError {
    HardwareError::new(message)
}

/// Rule 1: the chipset can show the mode; returns the mode's colour maximum.
fn check_mode(t: Target) -> Result<u16, HardwareError> {
    t.mode
        .max_colors(t.chipset)
        .ok_or_else(|| error(format!("{:?} cannot show {}", t.chipset, t.mode.name())))
}

/// Rule 2: the palette size.
fn check_count(t: Target, max: u16, colors: usize) -> Result<(), HardwareError> {
    if colors > usize::from(max) {
        return Err(error(format!(
            "{colors} colours exceed the {max} of {} on {:?}",
            t.mode.name(),
            t.chipset
        )));
    }
    Ok(())
}

/// Rule 4: OCS palette entries are 12-bit.
fn check_depth(t: Target, palette: &Palette) -> Result<(), HardwareError> {
    match first_unshowable(t.chipset, palette.colors()) {
        Some((i, _)) => Err(error(format!(
            "palette entry {i} is not a 12-bit OCS colour"
        ))),
        None => Ok(()),
    }
}

/// Rules 1, 2 and 4: what `target` can show of `palette`. Used where the page is not the
/// question.
pub fn check_palette(target: Target, palette: &Palette) -> Result<(), HardwareError> {
    check_palette_size(target, palette)?;
    check_depth(target, palette)
}

/// Rules 1 and 2: the chipset shows the mode and the palette has at most its colours. A
/// `.dpxproj` project is read with these (wave-8 integration): its colours themselves are
/// kept as stored, see [`stored_palette_note`].
pub fn check_palette_size(target: Target, palette: &Palette) -> Result<(), HardwareError> {
    let max = check_mode(target)?;
    check_count(target, max, palette.len())
}

/// Every palette entry `chipset` cannot show (OCS: not 12-bit), by index.
pub fn unshowable_entries(chipset: Chipset, colors: &[Rgb8]) -> Vec<usize> {
    match chipset {
        Chipset::Ocs => (0..colors.len())
            .filter(|&i| !is_12_bit(colors[i]))
            .collect(),
        Chipset::Aga => Vec::new(),
    }
}

/// `indices` (ascending) as a short list: runs of three or more as "a-b", e.g. "1, 3, 17-31".
pub fn index_list(indices: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let mut j = i;
        while j + 1 < indices.len() && indices[j + 1] == indices[j] + 1 {
            j += 1;
        }
        match j - i {
            0 => parts.push(indices[i].to_string()),
            1 => parts.extend([indices[i].to_string(), indices[j].to_string()]),
            _ => parts.push(format!("{}-{}", indices[i], indices[j])),
        }
        i = j + 1;
    }
    parts.join(", ")
}

/// The note for a picture whose OCS palette holds colours that are not 12-bit, as a
/// `.dpxproj` project saved before ADR-46 can (wave-8 integration): they are kept exactly as
/// stored, and the exports that check rule 4 refuse the picture until they are 12-bit.
/// `refused_by` names those in the front-end's words: every IFF export in the GUI and the
/// CLI (whose PNG export does not check the colours), every format of MCP's `export_image`
/// (which checks the hardware rules before any format). `None` when the chipset shows every
/// entry. Front-ends add how to fix them in their own words.
pub fn stored_palette_note(target: Target, palette: &Palette, refused_by: &str) -> Option<String> {
    let bad = unshowable_entries(target.chipset, palette.colors());
    let (what, are) = match bad.len() {
        0 => return None,
        1 => ("palette entry", "is not a 12-bit OCS colour"),
        _ => ("palette entries", "are not 12-bit OCS colours"),
    };
    Some(format!(
        "{what} {} {are} (kept as stored); {refused_by} refuses the picture until {} 12-bit",
        index_list(&bad),
        if bad.len() == 1 { "it is" } else { "they are" }
    ))
}

/// Every hardware rule (module docs) for `doc`, in that order. The rules of MCP's
/// `export_image` (`dpx_mcp::pictures::validate_hardware` wraps this).
pub fn validate(doc: &Document) -> Result<(), HardwareError> {
    check(doc, false)
}

/// [`validate`], except that a page smaller than the screen is accepted (rule 3 then only
/// sets the largest page): a picture or brush file may be smaller than the screen it is shown
/// on, as DeluxePaint's brushes are, and `deluxepainter convert` keeps a picture's own size.
/// The GUI's and the CLI's IFF exports use this.
pub fn validate_picture(doc: &Document) -> Result<(), HardwareError> {
    check(doc, true)
}

/// Rule 5 covers every picture of every frame and layer (ADR-44): an export is the visible
/// layers flattened, and every layer must fit the palette, shown or hidden.
fn check(doc: &Document, smaller_pages: bool) -> Result<(), HardwareError> {
    let t = doc.target();
    let (w, h) = (doc.page().width(), doc.page().height());
    let colors = doc.palette().len();
    let max = check_mode(t)?;
    check_count(t, max, colors)?;
    let ((min_w, min_h), (max_w, max_h)) = t.page_limits();
    let fits = if smaller_pages {
        (1..=max_w).contains(&w) && (1..=max_h).contains(&h)
    } else {
        t.page_fits(w, h)
    };
    if !fits {
        let min = if smaller_pages {
            (1, 1)
        } else {
            (min_w, min_h)
        };
        return Err(error(format!(
            "page {w}x{h} is outside the {}x{}..{max_w}x{max_h} limits of {}",
            min.0,
            min.1,
            t.mode.name()
        )));
    }
    check_depth(t, doc.palette())?;
    if let Some(max) = doc.cels().filter_map(|c| c.pixels().iter().max()).max()
        && usize::from(*max) >= colors
    {
        return Err(error(format!(
            "pixel colour {max} is outside the {colors}-colour palette"
        )));
    }
    Ok(())
}

/// Honest notes about the target's screen size for an export (empty for a standard screen):
/// an overscan screen, or a custom size such as 320x240 on PAL:Low Res, which is neither of
/// the standard LoRes sizes 320x200 (NTSC) and 320x256 (PAL). Exact hardware compatibility
/// of such screens is not guaranteed (ADR-46).
pub fn format_notes(target: &Target) -> Vec<String> {
    let mode = target.mode;
    let name = mode.name();
    let (w, h) = target.screen;
    let (sw, sh) = mode.size();
    let (ow, oh) = mode.overscan_size();
    let mut notes = Vec::new();
    if (w, h) == (sw, sh) {
        return notes;
    }
    if (w, h) == (ow, oh) {
        notes.push(format!(
            "{w}x{h} is the full overscan screen of {name} (standard {sw}x{sh}); exact \
             hardware compatibility is not guaranteed"
        ));
        return notes;
    }
    notes.push(format!(
        "{w}x{h} is a custom screen size on {name} (standard {sw}x{sh}); exact hardware \
         compatibility is not guaranteed"
    ));
    let wider = w > sw;
    let taller = h > sh;
    if wider || taller {
        let how = match (wider, taller) {
            (true, true) => "horizontal and vertical",
            (true, false) => "horizontal",
            _ => "vertical",
        };
        notes.push(format!(
            "{w}x{h} uses {how} overscan (at most {ow}x{oh} on {name})"
        ));
    }
    notes
}

/// Notes about what an IFF ILBM or ANIM written from a `page`-sized picture of `target`
/// cannot carry: the file has no chipset field, so DeluxePainter (like DeluxePaint IV AGA,
/// which this program follows) reopens it as AGA; and when the display mode read back from
/// the file (`DisplayMode::guess` on [`Target::camg`] and the page size) differs, e.g. an
/// OCS NTSC picture taller than 200 lines, whose CAMG carries no monitor bits.
pub fn iff_notes(target: &Target, page: (u32, u32)) -> Vec<String> {
    let mut notes = Vec::new();
    if target.chipset == Chipset::Ocs {
        notes.push("IFF files store no chipset: this OCS picture reopens as AGA".into());
    }
    let back = DisplayMode::guess(Some(target.camg()), page.0, page.1);
    if back != target.mode {
        notes.push(format!(
            "the file reopens as {} (an OCS display mode carries no PAL/NTSC monitor)",
            back.name()
        ));
    }
    notes
}

/// The notes an export of `doc` gives about its target: [`format_notes`] for every export
/// format, plus [`iff_notes`] (with the page size) for an IFF ILBM or ANIM file (`iff`). The
/// GUI's and the CLI's exports and MCP's `export_image` all give these (ADR-46).
pub fn export_notes(doc: &Document, iff: bool) -> Vec<String> {
    let t = doc.target();
    let mut notes = format_notes(&t);
    if iff {
        notes.extend(iff_notes(&t, (doc.page().width(), doc.page().height())));
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::IndexedImage;
    use crate::mode::{Resolution, Video};

    fn doc(w: u32, h: u32, palette: Palette, target: Target) -> Document {
        Document::from_picture(IndexedImage::new(w, h, 0), palette, target)
    }

    fn mode(video: Video, resolution: Resolution, interlace: bool) -> DisplayMode {
        DisplayMode::new(video, resolution, interlace)
    }

    #[test]
    fn every_rule_with_its_message() {
        let pal32 = Palette::dpaint_default_32();
        let ocs_lo = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES);
        assert_eq!(validate(&doc(320, 256, pal32.clone(), ocs_lo)), Ok(()));
        // 1. No SuperHiRes on OCS.
        let shi = Target::new(Chipset::Ocs, mode(Video::Pal, Resolution::SuperHi, false));
        let e = validate(&doc(1280, 256, pal32.clone(), shi)).unwrap_err();
        assert_eq!(e.message, "Ocs cannot show PAL:Super-High Res");
        assert_eq!(e.code(), "HARDWARE_LIMIT");
        assert_eq!(
            e.to_string(),
            "HARDWARE_LIMIT: Ocs cannot show PAL:Super-High Res"
        );
        // 2. 32 colours exceed OCS HiRes.
        let hi = Target::new(Chipset::Ocs, mode(Video::Pal, Resolution::Hi, false));
        let e = validate(&doc(640, 256, pal32.clone(), hi)).unwrap_err();
        assert_eq!(e.message, "32 colours exceed the 16 of PAL:High Res on Ocs");
        // 3. The page: smaller than the screen, larger than the largest page.
        let e = validate(&doc(100, 100, pal32.clone(), Target::default())).unwrap_err();
        assert_eq!(
            e.message,
            "page 100x100 is outside the 320x256..1008x1024 limits of PAL:Low Res"
        );
        assert_eq!(
            validate_picture(&doc(100, 100, pal32.clone(), Target::default())),
            Ok(()),
            "a brush-sized picture file"
        );
        let e = validate_picture(&doc(1009, 300, pal32.clone(), Target::default())).unwrap_err();
        assert_eq!(
            e.message,
            "page 1009x300 is outside the 1x1..1008x1024 limits of PAL:Low Res"
        );
        // 4. OCS entries are 12-bit; AGA takes any colour.
        let mut colors = pal32.colors().to_vec();
        colors[5] = Rgb8::new(1, 2, 3);
        let odd = Palette::new(colors).unwrap();
        let e = validate(&doc(320, 256, odd.clone(), ocs_lo)).unwrap_err();
        assert_eq!(e.message, "palette entry 5 is not a 12-bit OCS colour");
        assert_eq!(validate(&doc(320, 256, odd, Target::default())), Ok(()));
        // 5. Pixels outside the palette, in any frame.
        let two = Palette::new(vec![Rgb8::BLACK, Rgb8::WHITE]).unwrap();
        let frames = vec![
            IndexedImage::new(320, 256, 0),
            IndexedImage::new(320, 256, 7),
        ];
        let d = Document::from_frames(frames, 0, 10, two, Target::default()).unwrap();
        let e = validate(&d).unwrap_err();
        assert_eq!(e.message, "pixel colour 7 is outside the 2-colour palette");
        // ... and in any layer, a hidden one above the active bottom layer too (ADR-44).
        let two = Palette::new(vec![Rgb8::BLACK, Rgb8::WHITE]).unwrap();
        let frames = vec![vec![
            IndexedImage::new(320, 256, 1),
            IndexedImage::new(320, 256, 7),
        ]];
        let mut hidden = crate::Layer::new("Top");
        hidden.visible = false;
        let layers = vec![crate::Layer::background(), hidden];
        let d = Document::from_layers(frames, layers, 0, 0, 10, two, Target::default()).unwrap();
        let e = validate(&d).unwrap_err();
        assert_eq!(e.message, "pixel colour 7 is outside the 2-colour palette");
    }

    #[test]
    fn palette_checks_and_snapping() {
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES);
        assert_eq!(check_palette(ocs, &Palette::dpaint_default_32()), Ok(()));
        let grey = Palette::new((0..32).map(|i| Rgb8::new(i, i, i)).collect()).unwrap();
        assert!(check_palette(ocs, &grey).is_err());
        assert_eq!(check_palette(Target::default(), &grey), Ok(()));
        // The size rules alone (a project's palette, wave-8 integration) take the colours.
        assert_eq!(check_palette_size(ocs, &grey), Ok(()));
        // Every entry but 0 and 17 (#111111) is not 12-bit.
        let mut bad: Vec<usize> = (1..32).collect();
        bad.retain(|&i| i != 17);
        assert_eq!(unshowable_entries(Chipset::Ocs, grey.colors()), bad);
        assert!(unshowable_entries(Chipset::Aga, grey.colors()).is_empty());
        assert_eq!(index_list(&bad), "1-16, 18-31");
        assert_eq!(index_list(&[1, 2, 4, 6, 7, 8]), "1, 2, 4, 6-8");
        assert_eq!(index_list(&[]), "");
        assert_eq!(
            stored_palette_note(ocs, &grey, "IFF export").unwrap(),
            "palette entries 1-16, 18-31 are not 12-bit OCS colours (kept as stored); IFF export \
             refuses the picture until they are 12-bit"
        );
        // Each front-end names what refuses it (MCP's export_image checks every format).
        assert!(
            stored_palette_note(ocs, &grey, "export_image (every format, PNG too)")
                .unwrap()
                .ends_with(
                    "(kept as stored); export_image (every format, PNG too) refuses the \
                     picture until they are 12-bit"
                )
        );
        let mut one = Palette::dpaint_default_32().colors().to_vec();
        one[3] = Rgb8::new(1, 2, 3);
        assert_eq!(
            stored_palette_note(ocs, &Palette::new(one).unwrap(), "IFF export").unwrap(),
            "palette entry 3 is not a 12-bit OCS colour (kept as stored); IFF export refuses the \
             picture until it is 12-bit"
        );
        assert_eq!(
            stored_palette_note(ocs, &Palette::dpaint_default_32(), "IFF export"),
            None
        );
        assert_eq!(
            stored_palette_note(Target::default(), &grey, "IFF export"),
            None
        );
        let big = Palette::new(vec![Rgb8::BLACK; 64]).unwrap();
        assert!(check_palette_size(ocs, &big).is_err());
        let big = Palette::new(vec![Rgb8::BLACK; 64]).unwrap();
        assert!(
            check_palette(ocs, &big)
                .unwrap_err()
                .message
                .contains("64 colours")
        );
        assert!(is_12_bit(Rgb8::new(0x11, 0xEE, 0xFF)));
        assert!(!is_12_bit(Rgb8::new(0x10, 0xEE, 0xFF)));
        assert_eq!(
            snap(Chipset::Ocs, Rgb8::new(0x24, 0x24, 0x24)),
            Rgb8::new(0x22, 0x22, 0x22)
        );
        assert_eq!(snap(Chipset::Aga, Rgb8::new(1, 2, 3)), Rgb8::new(1, 2, 3));
        let mut colors = vec![Rgb8::new(0x11, 0x22, 0x33), Rgb8::new(1, 2, 3)];
        assert_eq!(snap_colors(Chipset::Ocs, &mut colors), 1);
        assert_eq!(colors[1], Rgb8::BLACK);
        assert_eq!(
            first_unshowable(Chipset::Ocs, &[Rgb8::WHITE, Rgb8::new(8, 0, 0)]),
            Some((1, Rgb8::new(8, 0, 0)))
        );
        assert_eq!(depth(Chipset::Ocs), PaletteDepth::Rgb4);
        assert_eq!(rounded_note(2), "2 colours rounded to 12-bit OCS values");
    }

    #[test]
    fn format_notes_are_honest_about_custom_and_overscan_screens() {
        let pal_lo = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES);
        assert!(format_notes(&pal_lo).is_empty(), "standard 320x256");
        let notes = format_notes(&pal_lo.with_screen(320, 240));
        assert_eq!(
            notes,
            vec![
                "320x240 is a custom screen size on PAL:Low Res (standard 320x256); exact \
                 hardware compatibility is not guaranteed"
                    .to_string()
            ]
        );
        let ntsc_lo = Target::new(Chipset::Aga, mode(Video::Ntsc, Resolution::Lo, false));
        let notes = format_notes(&ntsc_lo.with_screen(320, 240));
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("custom screen size on NTSC:Low Res (standard 320x200)"));
        assert_eq!(
            notes[1],
            "320x240 uses vertical overscan (at most 368x241 on NTSC:Low Res)"
        );
        let wide = format_notes(&pal_lo.with_screen(352, 272));
        assert!(
            wide[1].contains("horizontal and vertical overscan"),
            "{wide:?}"
        );
        let over = format_notes(&pal_lo.with_screen(368, 283));
        assert_eq!(over.len(), 1);
        assert!(over[0].starts_with("368x283 is the full overscan screen of PAL:Low Res"));
        for m in DisplayMode::all() {
            assert!(format_notes(&Target::new(Chipset::Aga, m)).is_empty());
        }
    }

    #[test]
    fn iff_notes_name_what_a_file_loses() {
        let aga = Target::default();
        assert!(iff_notes(&aga, (320, 256)).is_empty());
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES).with_screen(320, 240);
        assert_eq!(
            iff_notes(&ocs, (320, 240)),
            vec!["IFF files store no chipset: this OCS picture reopens as AGA".to_string()]
        );
        // OCS CAMG has no monitor bits: a 240-line NTSC picture reads back as PAL.
        let ntsc = Target::new(Chipset::Ocs, mode(Video::Ntsc, Resolution::Lo, false))
            .with_screen(320, 240);
        let notes = iff_notes(&ntsc, (320, 240));
        assert_eq!(notes.len(), 2);
        assert!(notes[1].starts_with("the file reopens as PAL:Low Res"));
        // AGA keeps it.
        let aga_ntsc = Target {
            chipset: Chipset::Aga,
            ..ntsc
        };
        assert!(iff_notes(&aga_ntsc, (320, 240)).is_empty());
    }

    #[test]
    fn export_notes_add_the_iff_notes_for_iff_files_only() {
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES).with_screen(320, 240);
        let d = doc(320, 240, Palette::dpaint_default_32(), ocs);
        let png = export_notes(&d, false);
        assert_eq!(png, format_notes(&ocs));
        assert_eq!(png.len(), 1);
        let iff = export_notes(&d, true);
        assert_eq!(iff[..1], png[..]);
        assert_eq!(iff[1..], iff_notes(&ocs, (320, 240))[..]);
        assert_eq!(iff.len(), 2);
        let standard = doc(320, 256, Palette::dpaint_default_32(), Target::default());
        assert!(export_notes(&standard, true).is_empty());
    }
}
