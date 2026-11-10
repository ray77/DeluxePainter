//! Amiga display modes and the document's hardware target (`docs/HARDWARE_MODES.md`).
//!
//! A mode is video standard × resolution × interlace. Sizes are the standard
//! (non-overscan) screen sizes; overscan and custom sizes follow later.

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
}

impl Target {
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
        Self {
            chipset: Chipset::Aga,
            mode: DisplayMode::PAL_LORES,
        }
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
                let camg = Target { chipset, mode }.camg();
                assert_eq!(
                    DisplayMode::guess(Some(camg), w, h),
                    mode,
                    "{chipset:?} {camg:#x}"
                );
            }
        }
        let pal_hi_lace = DisplayMode::new(Video::Pal, Resolution::Hi, true);
        let ocs = Target {
            chipset: Chipset::Ocs,
            mode: pal_hi_lace,
        };
        assert_eq!(ocs.camg(), 0x8004);
        let aga = Target {
            chipset: Chipset::Aga,
            mode: pal_hi_lace,
        };
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
}
