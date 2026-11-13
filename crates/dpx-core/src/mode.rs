//! Amiga display modes and the document's hardware target (`docs/HARDWARE_MODES.md`).
//!
//! A mode is video standard × resolution × interlace. [`DisplayMode::size`] is the standard
//! (non-overscan) screen size; a [`Target`] carries the actual screen size, which may be an
//! overscan or custom size within [`DisplayMode::screen_limits`] (ADR-11). Pages may be
//! larger than the screen within [`Target::page_limits`] (ADR-10).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chipset {
    Ocs,
    Aga,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Video {
    Pal,
    Ntsc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// 320 pixels per line.
    Lo,
    /// 640 pixels per line.
    Hi,
    /// 1280 pixels per line (ECS/AGA).
    SuperHi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DisplayMode {
    pub video: Video,
    pub resolution: Resolution,
    pub interlace: bool,
}

/// CAMG / ModeID bits used here (HARDWARE_MODES.md §5).
mod mode_id {
    pub const LACE: u32 = 0x0004;
    pub const SUPER: u32 = 0x0020;
    pub const EHB: u32 = 0x0080;
    pub const DPF: u32 = 0x0400;
    pub const HAM: u32 = 0x0800;
    pub const EXTENDED: u32 = 0x1000;
    pub const HIRES: u32 = 0x8000;
    pub const MONITOR_MASK: u32 = 0xFFFF_1000;
    pub const NTSC_MONITOR: u32 = 0x0001_1000;
    pub const PAL_MONITOR: u32 = 0x0002_1000;
}

impl DisplayMode {
    /// The DeluxePainter default: PAL LoRes, 320×256.
    pub const PAL_LORES: Self = Self::new(Video::Pal, Resolution::Lo, false);

    pub const fn new(video: Video, resolution: Resolution, interlace: bool) -> Self {
        Self {
            video,
            resolution,
            interlace,
        }
    }

    /// All modes in the order of the Choose Display Mode list (sorted by name).
    pub fn all() -> [DisplayMode; 12] {
        let mut out = [Self::PAL_LORES; 12];
        let mut i = 0;
        for video in [Video::Ntsc, Video::Pal] {
            for resolution in [Resolution::Hi, Resolution::Lo, Resolution::SuperHi] {
                for interlace in [false, true] {
                    out[i] = Self::new(video, resolution, interlace);
                    i += 1;
                }
            }
        }
        out
    }

    /// Display database style name, e.g. `PAL:Low Res Laced`.
    pub fn name(self) -> String {
        let video = match self.video {
            Video::Pal => "PAL",
            Video::Ntsc => "NTSC",
        };
        let res = match self.resolution {
            Resolution::Lo => "Low Res",
            Resolution::Hi => "High Res",
            Resolution::SuperHi => "Super-High Res",
        };
        let lace = if self.interlace { " Laced" } else { "" };
        format!("{video}:{res}{lace}")
    }

    /// Standard (non-overscan) screen size in pixels.
    pub fn size(self) -> (u32, u32) {
        let w = match self.resolution {
            Resolution::Lo => 320,
            Resolution::Hi => 640,
            Resolution::SuperHi => 1280,
        };
        let h = match self.video {
            Video::Pal => 256,
            Video::Ntsc => 200,
        };
        (w, if self.interlace { h * 2 } else { h })
    }

    /// Full video ("Overscan") size: 368/736/1472 pixels wide, 241 (NTSC) or 283 (PAL)
    /// lines, laced twice that (HARDWARE_MODES.md §2; the NTSC LoRes value 368×241 also
    /// appears in DeluxePaint's Set Page Size requester). Laced heights are kept even (the
    /// sources quote 483/567, counting a half line); SuperHiRes is derived, not verified.
    /// Used for the Overscan screen size and the Page Size preset.
    pub fn overscan_size(self) -> (u32, u32) {
        let w = match self.resolution {
            Resolution::Lo => 368,
            Resolution::Hi => 736,
            Resolution::SuperHi => 1472,
        };
        let lines = match self.video {
            Video::Ntsc => 241,
            Video::Pal => 283,
        };
        (w, if self.interlace { lines * 2 } else { lines })
    }

    /// Smallest and largest screen (ADR-11): from the standard width with NTSC height (a
    /// PAL screen may be as short as an NTSC one) up to the overscan size.
    pub fn screen_limits(self) -> ((u32, u32), (u32, u32)) {
        let (w, _) = self.size();
        let min_h = if self.interlace { 400 } else { 200 };
        ((w, min_h), self.overscan_size())
    }

    /// True if `width`×`height` is a valid screen for this mode: within the limits and whole
    /// UI pixels (HiRes widths even, SuperHiRes widths multiples of 4, laced heights even).
    pub fn screen_fits(self, width: u32, height: u32) -> bool {
        let ((min_w, min_h), (max_w, max_h)) = self.screen_limits();
        let (ux, uy) = self.ui_scale();
        (min_w..=max_w).contains(&width)
            && (min_h..=max_h).contains(&height)
            && width.is_multiple_of(ux)
            && height.is_multiple_of(uy)
    }

    /// The screen for a picture of `width`×`height` in this mode: the standard size if the
    /// picture fits it, else the picture size (rounded up to whole UI pixels) up to the
    /// overscan size.
    pub fn screen_for_picture(self, width: u32, height: u32) -> (u32, u32) {
        let (sw, sh) = self.size();
        let (_, (max_w, max_h)) = self.screen_limits();
        let (ux, uy) = self.ui_scale();
        let w = width.div_ceil(ux) * ux;
        let h = height.div_ceil(uy) * uy;
        (w.clamp(sw, max_w), h.clamp(sh, max_h))
    }

    /// Shape of one pixel as integer display factors (width, height), reduced: LoRes 1×1,
    /// LoRes Laced 2×1, HiRes 1×2, HiRes Laced 1×1, SuperHiRes 1×4, SuperHiRes Laced 1×2.
    /// Based on pixel clocks (HiRes = ½, SuperHiRes = ¼ LoRes width) and interlace (½ height).
    pub fn pixel_shape(self) -> (u32, u32) {
        let (ux, uy) = self.ui_scale();
        let (w, h) = (4 / ux, 2 / uy * 2);
        let g = gcd(w, h);
        (w / g, h / g)
    }

    /// Screen pixels covered by one LoRes non-laced pixel: (1, 2 or 4) × (1 or 2).
    pub fn ui_scale(self) -> (u32, u32) {
        let x = match self.resolution {
            Resolution::Lo => 1,
            Resolution::Hi => 2,
            Resolution::SuperHi => 4,
        };
        (x, if self.interlace { 2 } else { 1 })
    }

    /// Most indexed colours in this mode, or `None` if the chipset cannot show the mode.
    /// OCS: LoRes 32, HiRes 16, no SuperHiRes; AGA: 256 everywhere (HARDWARE_MODES.md §1).
    pub fn max_colors(self, chipset: Chipset) -> Option<u16> {
        match (chipset, self.resolution) {
            (Chipset::Aga, _) => Some(256),
            (Chipset::Ocs, Resolution::Lo) => Some(32),
            (Chipset::Ocs, Resolution::Hi) => Some(16),
            (Chipset::Ocs, Resolution::SuperHi) => None,
        }
    }

    /// Full ModeID (monitor part | mode key) for CAMG.
    pub fn mode_id(self) -> u32 {
        let monitor = match self.video {
            Video::Pal => mode_id::PAL_MONITOR,
            Video::Ntsc => mode_id::NTSC_MONITOR,
        };
        let key = match self.resolution {
            Resolution::Lo => 0,
            Resolution::Hi => mode_id::HIRES,
            Resolution::SuperHi => mode_id::HIRES | mode_id::SUPER,
        };
        monitor | key | if self.interlace { mode_id::LACE } else { 0 }
    }

    /// BMHD pixel aspect (xAspect, yAspect). Only NTSC LoRes 10:11 is verified; the others
    /// are derived from it and the common PAL LoRes value 44:52 (HARDWARE_MODES.md §3).
    pub fn aspect(self) -> (u8, u8) {
        let (x, y): (u32, u32) = match self.video {
            Video::Ntsc => (10, 11),
            Video::Pal => (44, 52),
        };
        let x = match self.resolution {
            Resolution::Lo => x * 4,
            Resolution::Hi => x * 2,
            Resolution::SuperHi => x,
        };
        let y = if self.interlace { y * 2 } else { y * 4 };
        let g = gcd(x, y);
        let (x, y) = (x / g, y / g);
        // Keep within a byte; all table values fit after reduction.
        (x.min(255) as u8, y.min(255) as u8)
    }

    /// Picks the mode for a loaded picture: from CAMG where it names one, otherwise from the
    /// picture size (≥1280 wide = SuperHiRes, ≥640 = HiRes; more than 283 lines = laced;
    /// up to 200 lines per field = NTSC). HAM/EHB/dual-playfield bits are ignored here.
    pub fn guess(camg: Option<u32>, width: u32, height: u32) -> Self {
        let camg = camg.map(sanitize_camg);
        let resolution = match camg {
            Some(id) if id & mode_id::HIRES != 0 && id & mode_id::SUPER != 0 => Resolution::SuperHi,
            Some(id) if id & mode_id::HIRES != 0 => Resolution::Hi,
            Some(_) => Resolution::Lo,
            None if width >= 1280 => Resolution::SuperHi,
            None if width >= 640 => Resolution::Hi,
            None => Resolution::Lo,
        };
        let interlace = match camg {
            Some(id) => id & mode_id::LACE != 0,
            None => height > 283,
        };
        let lines = if interlace { height / 2 } else { height };
        let video = match camg.map(|id| id & mode_id::MONITOR_MASK) {
            Some(mode_id::PAL_MONITOR) => Video::Pal,
            Some(mode_id::NTSC_MONITOR) => Video::Ntsc,
            _ if lines <= 200 => Video::Ntsc,
            _ => Video::Pal,
        };
        Self::new(video, resolution, interlace)
    }

    /// True if a ModeID asks for something this model does not cover (HAM, EHB, DPF).
    pub fn is_special(camg: u32) -> bool {
        sanitize_camg(camg) & (mode_id::HAM | mode_id::EHB | mode_id::DPF) != 0
    }
}

/// Old-style CAMG values carry junk flags; keep the low word and strip them
/// (HARDWARE_MODES.md §5.3).
fn sanitize_camg(id: u32) -> u32 {
    if id & 0xFFFF_0000 != 0 && id & mode_id::EXTENDED == 0 {
        id & 0xFFFF & 0x8EFD
    } else if id & 0xFFFF_0000 == 0 {
        id & 0x8EFD
    } else {
        id
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// The hardware a document is made for. Its limits are document rules (CLAUDE.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Target {
    pub chipset: Chipset,
    pub mode: DisplayMode,
    /// Screen size in pixels: the mode's standard size, its overscan size or a custom size
    /// between ([`DisplayMode::screen_fits`]; ADR-11).
    pub screen: (u32, u32),
}

impl Target {
    /// A target with the mode's standard screen size.
    pub fn new(chipset: Chipset, mode: DisplayMode) -> Self {
        Self {
            chipset,
            mode,
            screen: mode.size(),
        }
    }

    /// The same target with another screen size.
    pub fn with_screen(self, width: u32, height: u32) -> Self {
        Self {
            screen: (width, height),
            ..self
        }
    }

    /// Smallest and largest page: never smaller than the screen; DeluxePaint IV recognises
    /// pages up to 1008×1024 (AGA manual p.236). Wider or taller overscan pages are allowed up
    /// to the overscan size — a DeluxePainter extension, as the manual names no AGA limit.
    pub fn page_limits(self) -> ((u32, u32), (u32, u32)) {
        let (ow, oh) = self.mode.overscan_size();
        (self.screen, (ow.max(1008), oh.max(1024)))
    }

    /// True if a `width`×`height` page is allowed.
    pub fn page_fits(self, width: u32, height: u32) -> bool {
        let ((min_w, min_h), (max_w, max_h)) = self.page_limits();
        (min_w..=max_w).contains(&width) && (min_h..=max_h).contains(&height)
    }

    /// "Full Page" preset: the screen width and the height that shows the proportions of
    /// DeluxePaint's NTSC LoRes Full Page (320×340 with 10:11 pixels, read from the manual
    /// figure) with the mode's pixel aspect. Other modes are derived, not verified.
    pub fn full_page_size(self) -> (u32, u32) {
        let w = self.screen.0;
        let (ax, ay) = self.mode.aspect();
        // h = w · ax/ay · (340·11)/(320·10), rounded.
        let num = u64::from(w) * u64::from(ax) * 340 * 11;
        let den = u64::from(ay) * 320 * 10;
        let h = ((num + den / 2) / den) as u32;
        let ((min_w, min_h), (max_w, max_h)) = self.page_limits();
        (w.clamp(min_w, max_w), h.clamp(min_h, max_h))
    }

    /// The ILBM CAMG value to write. AGA documents get the full 32-bit mode id with its
    /// monitor (PAL/NTSC) bits; OCS documents get the classic 16-bit view modes, which old
    /// readers understand, so PAL/NTSC is then only implied by the page height.
    pub fn camg(self) -> u32 {
        let id = self.mode.mode_id();
        match self.chipset {
            Chipset::Aga => id,
            Chipset::Ocs => id & !mode_id::MONITOR_MASK,
        }
    }
}

impl Default for Target {
    /// DeluxePaint IV AGA on a PAL machine.
    fn default() -> Self {
        Self::new(Chipset::Aga, DisplayMode::PAL_LORES)
    }
}

/// A named screen format to start a picture from (ADR-46): a display mode with a screen
/// size, or Custom. The Screen Format requester's Preset gadget offers them in this order;
/// they are a DeluxePainter addition (DeluxePaint IV has only the mode list).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    /// Stable identifier, e.g. `pixel_art_320x240` (snapshot flags, MCP, settings).
    pub id: &'static str,
    /// The name shown, e.g. "PAL LoRes 320x256". A screen that is not the mode's standard
    /// size says "(custom)": 320x240 is neither standard PAL nor standard NTSC.
    pub label: &'static str,
    /// The display mode and screen size; `None` for Custom (any mode and screen size the
    /// chipset can show, checked like every format).
    pub format: Option<(DisplayMode, (u32, u32))>,
}

const fn preset(
    id: &'static str,
    label: &'static str,
    video: Video,
    resolution: Resolution,
    interlace: bool,
    screen: (u32, u32),
) -> Preset {
    Preset {
        id,
        label,
        format: Some((DisplayMode::new(video, resolution, interlace), screen)),
    }
}

/// The presets, in the order the requester cycles through them; Custom is last. The one
/// table of screen presets: the Screen Format requester's Preset gadget, the Generate
/// requester's Resolution row (every entry but Custom) and MCP's `preset` argument
/// (`create_document`) all take theirs from here (wave-8 integration).
pub const PRESETS: [Preset; 8] = [
    preset(
        "ntsc_lores",
        "NTSC LoRes 320x200",
        Video::Ntsc,
        Resolution::Lo,
        false,
        (320, 200),
    ),
    preset(
        "pal_lores",
        "PAL LoRes 320x256",
        Video::Pal,
        Resolution::Lo,
        false,
        (320, 256),
    ),
    // PAL:Low Res with a custom screen 16 lines shorter than standard: the variant that
    // keeps its display mode when an OCS ILBM is read back (its CAMG has no monitor bits, so
    // PAL/NTSC comes from the height; NTSC 320x240 would need vertical overscan and read
    // back as PAL).
    preset(
        "pixel_art_320x240",
        "Pixel art 320x240 (custom)",
        Video::Pal,
        Resolution::Lo,
        false,
        (320, 240),
    ),
    preset(
        "ntsc_hires",
        "NTSC HiRes 640x200",
        Video::Ntsc,
        Resolution::Hi,
        false,
        (640, 200),
    ),
    preset(
        "pal_hires",
        "PAL HiRes 640x256",
        Video::Pal,
        Resolution::Hi,
        false,
        (640, 256),
    ),
    preset(
        "ntsc_hires_laced",
        "NTSC HiRes Laced 640x400",
        Video::Ntsc,
        Resolution::Hi,
        true,
        (640, 400),
    ),
    preset(
        "pal_hires_laced",
        "PAL HiRes Laced 640x512",
        Video::Pal,
        Resolution::Hi,
        true,
        (640, 512),
    ),
    Preset {
        id: "custom",
        label: "Custom",
        format: None,
    },
];

impl Preset {
    /// The Custom entry (the last one).
    pub const CUSTOM: Preset = PRESETS[PRESETS.len() - 1];

    /// The preset with this id.
    pub fn by_id(id: &str) -> Option<&'static Preset> {
        PRESETS.iter().find(|p| p.id == id)
    }

    /// The index in [`PRESETS`] of the preset with exactly this mode and screen, else of
    /// Custom.
    pub fn index_of(mode: DisplayMode, screen: (u32, u32)) -> usize {
        PRESETS
            .iter()
            .position(|p| p.format == Some((mode, screen)))
            .unwrap_or(PRESETS.len() - 1)
    }

    /// The preset's target on `chipset`, or `None` for Custom and for a mode the chipset
    /// cannot show.
    pub fn target(&self, chipset: Chipset) -> Option<Target> {
        let (mode, (w, h)) = self.format?;
        mode.max_colors(chipset)?;
        Some(Target::new(chipset, mode).with_screen(w, h))
    }

    /// True if the screen is the mode's standard size (false for Custom).
    pub fn is_standard(&self) -> bool {
        self.format
            .is_some_and(|(mode, screen)| screen == mode.size())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camg_round_trips_through_guess() {
        for mode in DisplayMode::all() {
            let (w, h) = mode.size();
            for chipset in [Chipset::Aga, Chipset::Ocs] {
                let camg = Target::new(chipset, mode).camg();
                assert_eq!(
                    DisplayMode::guess(Some(camg), w, h),
                    mode,
                    "{chipset:?} {camg:#x}"
                );
            }
        }
        let pal_hi_lace = DisplayMode::new(Video::Pal, Resolution::Hi, true);
        let ocs = Target::new(Chipset::Ocs, pal_hi_lace);
        assert_eq!(ocs.camg(), 0x8004);
        let aga = Target::new(Chipset::Aga, pal_hi_lace);
        assert_eq!(aga.camg(), 0x0002_9004);
    }

    #[test]
    fn list_order_and_names() {
        let names: Vec<String> = DisplayMode::all().iter().map(|m| m.name()).collect();
        assert_eq!(names[0], "NTSC:High Res");
        assert_eq!(names[6], "PAL:High Res");
        assert_eq!(names[7], "PAL:High Res Laced");
        assert_eq!(names[8], "PAL:Low Res");
        assert_eq!(names[11], "PAL:Super-High Res Laced");
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "list is sorted by name like the reference");
    }

    #[test]
    fn sizes_shapes_and_ui_scale() {
        for m in DisplayMode::all() {
            let (w, h) = m.size();
            let (ux, uy) = m.ui_scale();
            assert_eq!(w / ux, 320, "{}", m.name());
            assert_eq!(h / uy, if m.video == Video::Pal { 256 } else { 200 });
            // A LoRes pixel has the same displayed shape in every mode.
            let (fx, fy) = m.pixel_shape();
            let lores = (ux * fx, uy * fy);
            assert_eq!(lores.0, lores.1, "{}: LoRes pixel stays square", m.name());
        }
        let m = |r, l| DisplayMode::new(Video::Pal, r, l).pixel_shape();
        assert_eq!(m(Resolution::Lo, false), (1, 1));
        assert_eq!(m(Resolution::Lo, true), (2, 1));
        assert_eq!(m(Resolution::Hi, false), (1, 2));
        assert_eq!(m(Resolution::Hi, true), (1, 1));
        assert_eq!(m(Resolution::SuperHi, false), (1, 4));
        assert_eq!(m(Resolution::SuperHi, true), (1, 2));
        assert_eq!(DisplayMode::PAL_LORES.size(), (320, 256));
        assert_eq!(
            DisplayMode::new(Video::Ntsc, Resolution::Hi, true).size(),
            (640, 400)
        );
    }

    #[test]
    fn colours_mode_ids_and_aspect() {
        let lo = DisplayMode::PAL_LORES;
        let hi = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        let shi = DisplayMode::new(Video::Pal, Resolution::SuperHi, false);
        assert_eq!(lo.max_colors(Chipset::Ocs), Some(32));
        assert_eq!(hi.max_colors(Chipset::Ocs), Some(16));
        assert_eq!(shi.max_colors(Chipset::Ocs), None);
        assert_eq!(shi.max_colors(Chipset::Aga), Some(256));
        assert_eq!(lo.mode_id(), 0x0002_1000);
        assert_eq!(
            DisplayMode::new(Video::Pal, Resolution::Hi, true).mode_id(),
            0x0002_9004
        );
        assert_eq!(shi.mode_id(), 0x0002_9020);
        assert_eq!(
            DisplayMode::new(Video::Ntsc, Resolution::Lo, false).aspect(),
            (10, 11)
        );
        assert_eq!(lo.aspect(), (11, 13));
    }

    #[test]
    fn guess_from_camg_or_size() {
        let ntsc_lo = DisplayMode::new(Video::Ntsc, Resolution::Lo, false);
        // The 320×200 example pictures have no CAMG.
        assert_eq!(DisplayMode::guess(None, 320, 200), ntsc_lo);
        // Old-style CAMG 0x4000 (SPRITES) is junk.
        assert_eq!(DisplayMode::guess(Some(0x4000), 320, 200), ntsc_lo);
        assert_eq!(DisplayMode::guess(None, 320, 256), DisplayMode::PAL_LORES);
        assert_eq!(
            DisplayMode::guess(None, 640, 512),
            DisplayMode::new(Video::Pal, Resolution::Hi, true)
        );
        for m in DisplayMode::all() {
            let (w, h) = m.size();
            assert_eq!(DisplayMode::guess(Some(m.mode_id()), w, h), m);
            assert_eq!(DisplayMode::guess(None, w, h), m, "{}", m.name());
        }
        assert!(DisplayMode::is_special(0x0002_1800));
        assert!(!DisplayMode::is_special(0x0002_9004));
    }

    #[test]
    fn page_presets_and_limits() {
        let ntsc_lo = Target::new(
            Chipset::Aga,
            DisplayMode::new(Video::Ntsc, Resolution::Lo, false),
        );
        // The values in DeluxePaint's Set Page Size figure (NTSC LoRes).
        assert_eq!(ntsc_lo.screen, (320, 200));
        assert_eq!(ntsc_lo.full_page_size(), (320, 340));
        assert_eq!(ntsc_lo.mode.overscan_size(), (368, 241));
        assert_eq!(ntsc_lo.page_limits(), ((320, 200), (1008, 1024)));
        // Derived for the other modes.
        let pal_lo = Target::default();
        assert_eq!(pal_lo.full_page_size(), (320, 316));
        assert_eq!(pal_lo.mode.overscan_size(), (368, 283));
        let hi =
            |video, lace| Target::new(Chipset::Aga, DisplayMode::new(video, Resolution::Hi, lace));
        assert_eq!(hi(Video::Ntsc, false).full_page_size(), (640, 340));
        let ntsc_lo_lace = Target::new(
            Chipset::Aga,
            DisplayMode::new(Video::Ntsc, Resolution::Lo, true),
        );
        assert_eq!(ntsc_lo_lace.full_page_size(), (320, 680));
        let pal_super_lace = Target::new(
            Chipset::Aga,
            DisplayMode::new(Video::Pal, Resolution::SuperHi, true),
        );
        assert_eq!(pal_super_lace.mode.overscan_size(), (1472, 566));
        assert_eq!(pal_super_lace.page_limits(), ((1280, 512), (1472, 1024)));
        // An overscan screen raises the smallest page.
        let over = pal_lo.with_screen(368, 283);
        assert_eq!(over.page_limits().0, (368, 283));
        assert_eq!(over.full_page_size().0, 368);
        assert!(!over.page_fits(320, 256));
        for m in DisplayMode::all() {
            let t = Target::new(Chipset::Aga, m);
            let (w, h) = m.size();
            assert!(t.page_fits(w, h), "{}", m.name());
            let (fw, fh) = t.full_page_size();
            assert!(t.page_fits(fw, fh), "{}", m.name());
            let (ow, oh) = m.overscan_size();
            assert!(t.page_fits(ow, oh), "{}", m.name());
            assert!(!t.page_fits(w - 1, h));
            assert!(!t.page_fits(w, 1025));
        }
    }

    #[test]
    fn presets_table() {
        let ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "ntsc_lores",
                "pal_lores",
                "pixel_art_320x240",
                "ntsc_hires",
                "pal_hires",
                "ntsc_hires_laced",
                "pal_hires_laced",
                "custom"
            ]
        );
        let labels: Vec<&str> = PRESETS.iter().map(|p| p.label).collect();
        assert_eq!(
            labels,
            [
                "NTSC LoRes 320x200",
                "PAL LoRes 320x256",
                "Pixel art 320x240 (custom)",
                "NTSC HiRes 640x200",
                "PAL HiRes 640x256",
                "NTSC HiRes Laced 640x400",
                "PAL HiRes Laced 640x512",
                "Custom"
            ]
        );
        assert_eq!(Preset::CUSTOM.format, None);
        assert_eq!(Preset::by_id("nope"), None);
        for (i, p) in PRESETS.iter().enumerate() {
            assert_eq!(Preset::by_id(p.id), Some(p));
            let Some((mode, (w, h))) = p.format else {
                assert_eq!(i, PRESETS.len() - 1, "Custom is last");
                continue;
            };
            assert!(mode.screen_fits(w, h), "{}", p.label);
            assert_eq!(Preset::index_of(mode, (w, h)), i);
            // Every size is in the label; standard screens are named by their mode.
            assert!(p.label.ends_with(&format!("{w}x{h}")) || p.label.contains("(custom)"));
            if p.is_standard() {
                assert!(!p.label.contains("custom"));
                assert!(p.label.starts_with(if mode.video == Video::Pal {
                    "PAL"
                } else {
                    "NTSC"
                }));
            } else {
                assert!(p.label.contains("(custom)"), "{}", p.label);
            }
            // LoRes and HiRes presets: OCS and AGA can show them all.
            for chipset in [Chipset::Ocs, Chipset::Aga] {
                let t = p.target(chipset).unwrap();
                assert_eq!((t.chipset, t.mode, t.screen), (chipset, mode, (w, h)));
            }
        }
        // 320x240 is never labelled standard PAL or NTSC.
        let px = Preset::by_id("pixel_art_320x240").unwrap();
        assert_eq!(px.format, Some((DisplayMode::PAL_LORES, (320, 240))));
        assert!(!px.is_standard());
        assert!(!px.label.contains("PAL") && !px.label.contains("NTSC"));
        assert!(
            DisplayMode::all().iter().all(|m| m.size() != (320, 240)),
            "no mode has 320x240 as its standard size"
        );
        // It keeps its mode through an OCS ILBM: no monitor bits, 240 lines read as PAL.
        let t = px.target(Chipset::Ocs).unwrap();
        assert_eq!(
            DisplayMode::guess(Some(t.camg()), 320, 240),
            DisplayMode::PAL_LORES
        );
        // Anything else is Custom.
        assert_eq!(
            Preset::index_of(DisplayMode::PAL_LORES, (352, 272)),
            PRESETS.len() - 1
        );
        let shi = DisplayMode::new(Video::Pal, Resolution::SuperHi, false);
        assert_eq!(Preset::index_of(shi, shi.size()), PRESETS.len() - 1);
        assert_eq!(Preset::CUSTOM.target(Chipset::Aga), None);
    }

    #[test]
    fn screen_sizes() {
        let pal_lo = DisplayMode::PAL_LORES;
        assert_eq!(pal_lo.screen_limits(), ((320, 200), (368, 283)));
        assert!(pal_lo.screen_fits(320, 256));
        assert!(pal_lo.screen_fits(352, 272));
        assert!(pal_lo.screen_fits(320, 200), "NTSC height on PAL");
        assert!(!pal_lo.screen_fits(319, 256));
        assert!(!pal_lo.screen_fits(370, 256));
        let hi_lace = DisplayMode::new(Video::Pal, Resolution::Hi, true);
        assert_eq!(hi_lace.screen_limits(), ((640, 400), (736, 566)));
        assert!(!hi_lace.screen_fits(641, 512), "HiRes widths are even");
        assert!(!hi_lace.screen_fits(640, 511), "laced heights are even");
        let shi = DisplayMode::new(Video::Ntsc, Resolution::SuperHi, false);
        assert!(
            !shi.screen_fits(1282, 200),
            "SuperHiRes widths are multiples of 4"
        );
        assert!(shi.screen_fits(1284, 241));
        // Pictures: standard if they fit, else their own size up to the overscan.
        assert_eq!(pal_lo.screen_for_picture(320, 200), (320, 256));
        assert_eq!(pal_lo.screen_for_picture(352, 272), (352, 272));
        assert_eq!(pal_lo.screen_for_picture(400, 300), (368, 283));
        assert_eq!(hi_lace.screen_for_picture(701, 513), (702, 514));
        for m in DisplayMode::all() {
            let (w, h) = m.size();
            assert!(m.screen_fits(w, h), "{}", m.name());
            let (ow, oh) = m.overscan_size();
            assert!(m.screen_fits(ow, oh), "{}", m.name());
        }
    }
}
