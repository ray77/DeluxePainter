//! Intuition-style pull-down menus with the DeluxePaint IV AGA menu set.
//!
//! While the right mouse button is held on the title bar, the title bar turns into the menu
//! strip; menus drop down below their title, submenus open to the right of their item.
//! Releasing the button over an enabled item selects it; clicking the left button while the
//! menu is held selects an item and keeps the menu open (Intuition multi-select).
//!
//! Geometry is measured on the LoRes reference capture `inapp_gfx_03` (Picture menu open);
//! see `docs/design/AMIGA_GUI_FIDELITY.md`. Values marked "to calibrate" are not visible there.

use std::sync::OnceLock;

use dpx_core::IndexedImage;

use crate::bitmap::{self, Bitmap};
use crate::font::{Font, glyph};
use crate::pens::Pens;
use crate::screen::PaintMode;

/// LoRes menu geometry (screen pixels).
pub mod geom {
    /// The menu strip covers rows 0..STRIP_H in `face` (no shine/dark lines).
    pub const STRIP_H: i32 = 11;
    /// A highlighted menu title is filled in `select` on rows 0..TITLE_HL_H.
    pub const TITLE_HL_H: i32 = 9;
    /// Left edge of the first title slot.
    pub const FIRST_SLOT_X: i32 = 3;
    /// Title text offset inside its slot.
    pub const SLOT_TEXT_DX: i32 = 2;
    /// Slot width = text width + SLOT_PAD (reference: +8 with the narrower original font;
    /// +6 keeps all seven titles inside 320 pixels with our font).
    pub const SLOT_PAD: i32 = 6;
    /// Menu box top (its border overlaps strip rows 9–10).
    pub const BOX_TOP: i32 = 9;
    /// Menu box left edge relative to its title slot.
    pub const BOX_DX: i32 = -3;
    /// `dark` border widths.
    pub const BORDER_X: i32 = 2;
    pub const BORDER_Y: i32 = 1;
    /// First item top relative to the box top (border + 3 rows padding).
    pub const ITEM_DY: i32 = 4;
    pub const ITEM_H: i32 = 10;
    /// Rows below the last item (3 padding + border).
    pub const BOTTOM: i32 = 4;
    /// Item text offset from the box's left edge.
    pub const TEXT_DX: i32 = 6;
    /// Extra text indent in menus with check marks (Intuition LOWCHECKWIDTH; to calibrate).
    pub const CHECK_W: i32 = 13;
    /// Item highlight inset from the box edges.
    pub const HL_DX: i32 = 4;
    /// Shortcut letter column: `box_x + box_w - KEY_FROM_RIGHT`.
    pub const KEY_FROM_RIGHT: i32 = 12;
    /// Amiga-key glyph x relative to the shortcut letter column.
    pub const AMIGA_DX: i32 = -17;
    /// Submenu triangle x relative to the shortcut letter column.
    pub const TRIANGLE_DX: i32 = 3;
    /// Space between an item's text and its shortcut.
    pub const KEY_GAP: i32 = 8;
    /// Space right of the text of items without shortcut.
    pub const RIGHT_MARGIN: i32 = 10;
    /// Submenus overlap their parent box by this many pixels (to calibrate).
    pub const SUB_OVERLAP: i32 = 4;
}

/// Check-mark preferences of the Prefs menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pref {
    Coords,
    FastFb,
    MultiCycle,
    BeSquare,
    Workbench,
    ExclBrush,
    AutoTransp,
    NoIcons,
    AutoGrid,
    OriginUl,
    FastAdjust,
}

impl Pref {
    /// Every preference, in menu order.
    pub const ALL: [Pref; 11] = [
        Pref::Coords,
        Pref::FastFb,
        Pref::MultiCycle,
        Pref::BeSquare,
        Pref::Workbench,
        Pref::ExclBrush,
        Pref::AutoTransp,
        Pref::NoIcons,
        Pref::AutoGrid,
        Pref::OriginUl,
        Pref::FastAdjust,
    ];

    /// Stable name used in the settings file; never changes when the enum is reordered.
    pub fn key(self) -> &'static str {
        match self {
            Pref::Coords => "coords",
            Pref::FastFb => "fast_fb",
            Pref::MultiCycle => "multi_cycle",
            Pref::BeSquare => "be_square",
            Pref::Workbench => "workbench",
            Pref::ExclBrush => "excl_brush",
            Pref::AutoTransp => "auto_transp",
            Pref::NoIcons => "no_icons",
            Pref::AutoGrid => "auto_grid",
            Pref::OriginUl => "origin_ul",
            Pref::FastAdjust => "fast_adjust",
        }
    }

    fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// Set of enabled [`Pref`]s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefs(u16);

impl Default for Prefs {
    /// AGA defaults: Be Square and AutoGrid on (manual AGA p.10, p.284).
    fn default() -> Self {
        Prefs(Pref::BeSquare.bit() | Pref::AutoGrid.bit())
    }
}

impl Prefs {
    pub fn get(self, p: Pref) -> bool {
        self.0 & p.bit() != 0
    }

    pub fn set(&mut self, p: Pref, on: bool) {
        if on {
            self.0 |= p.bit();
        } else {
            self.0 &= !p.bit();
        }
    }

    pub fn toggle(&mut self, p: Pref) {
        self.0 ^= p.bit();
    }
}

/// Size of one interface (LoRes) pixel in half points, a DeluxePainter preference (ADR-13):
/// whole and half steps from 1x (`PixelSize(2)`) to 8x (`PixelSize(16)`). The window snaps to
/// the picture's screen at this size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize(pub u8);

impl PixelSize {
    /// Smallest and largest size, in half points (1x and 8x).
    pub const MIN: u8 = 2;
    pub const MAX: u8 = 16;
}

impl Default for PixelSize {
    /// 3x: the start window shows a 320×256 screen at 960×768 points.
    fn default() -> Self {
        PixelSize(6)
    }
}

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// A document/editor command, by its command-engine name.
    Command(&'static str),
    /// Mode menu: select a paint mode (radio check mark).
    Mode(PaintMode),
    /// Prefs menu: toggle a preference (check mark).
    Pref(Pref),
    /// Prefs ▸ Pixel Size: choose the interface pixel size (radio check mark).
    PixelSize(PixelSize),
}

/// Shortcut shown at the right of an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    None,
    /// Right-Amiga + letter, drawn as the Amiga-key glyph and the letter.
    Amiga(char),
    /// A plain key, drawn in the `select` pen.
    Plain(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub enum Entry {
    Action(Action),
    Submenu(&'static [Item]),
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub label: &'static str,
    pub key: Key,
    pub entry: Entry,
}

#[derive(Debug)]
pub struct Menu {
    pub title: &'static str,
    pub items: &'static [Item],
}

const fn cmd(label: &'static str, id: &'static str) -> Item {
    Item {
        label,
        key: Key::None,
        entry: Entry::Action(Action::Command(id)),
    }
}

const fn cmd_key(label: &'static str, key: Key, id: &'static str) -> Item {
    Item {
        label,
        key,
        entry: Entry::Action(Action::Command(id)),
    }
}

const fn sub(label: &'static str, items: &'static [Item]) -> Item {
    Item {
        label,
        key: Key::None,
        entry: Entry::Submenu(items),
    }
}

const fn mode(label: &'static str, key: Key, m: PaintMode) -> Item {
    Item {
        label,
        key,
        entry: Entry::Action(Action::Mode(m)),
    }
}

const fn pref(label: &'static str, p: Pref) -> Item {
    Item {
        label,
        key: Key::None,
        entry: Entry::Action(Action::Pref(p)),
    }
}

const fn pixel_size(label: &'static str, key: Key, p: PixelSize) -> Item {
    Item {
        label,
        key,
        entry: Entry::Action(Action::PixelSize(p)),
    }
}

const fn plain(k: &'static str) -> Key {
    Key::Plain(k)
}

/// The DeluxePaint IV AGA menu set (item order from the manual's menu figures and the
/// `inapp_gfx_03` capture; see `docs/research/manual-ui-notes.md` §2).
pub static MENUS: [Menu; 7] = [
    Menu {
        title: "Picture",
        items: &[
            cmd_key("Load..", Key::Amiga('L'), "picture.load"),
            cmd_key("Save..", Key::Amiga('S'), "picture.save"),
            cmd("Export..", "picture.export"),
            cmd_key("Delete..", Key::Amiga('D'), "picture.delete"),
            cmd_key("Print..", Key::Amiga('P'), "picture.print"),
            sub(
                "Flip",
                &[
                    cmd("Horiz", "picture.flip_horizontal"),
                    cmd("Vert", "picture.flip_vertical"),
                ],
            ),
            sub(
                "Spare",
                &[
                    cmd_key("Swap", plain("j"), "spare.swap"),
                    cmd("Copy to Spare", "spare.copy"),
                    cmd("Merge in Front", "spare.merge_front"),
                    cmd("Merge in Back", "spare.merge_back"),
                    cmd("Delete This Page", "spare.delete"),
                ],
            ),
            cmd("Page Size..", "picture.page_size"),
            cmd_key("Show Page", plain("S"), "picture.show_page"),
            cmd("Screen Format..", "picture.screen_format"),
            cmd("About..", "app.about"),
            cmd_key("Quit", plain("Q"), "app.quit"),
        ],
    },
    Menu {
        title: "Brush",
        items: &[
            cmd("Load..", "brush.load"),
            cmd("Save..", "brush.save"),
            cmd_key("Restore", plain("B"), "brush.restore"),
            sub(
                "Spare",
                &[
                    cmd("Brush->Spare", "brush.to_spare"),
                    cmd("Brush<->Spare", "brush.swap_spare"),
                    cmd("Metamorph..", "brush.metamorph"),
                ],
            ),
            sub(
                "Size",
                &[
                    cmd_key("Stretch", plain("Z"), "brush.stretch"),
                    cmd_key("Halve", plain("h"), "brush.halve"),
                    cmd_key("Double", plain("H"), "brush.double"),
                    cmd_key("Double Horiz", plain("X"), "brush.double_horizontal"),
                    cmd_key("Double Vert", plain("Y"), "brush.double_vertical"),
                ],
            ),
            sub(
                "Flip",
                &[
                    cmd_key("Horiz", plain("x"), "brush.flip_horizontal"),
                    cmd_key("Vert", plain("y"), "brush.flip_vertical"),
                ],
            ),
            sub(
                "Edge",
                &[
                    cmd_key("Outline", plain("o"), "brush.outline"),
                    cmd_key("Trim", plain("O"), "brush.trim"),
                ],
            ),
            sub(
                "Rotate",
                &[
                    cmd_key("90 Degrees", plain("z"), "brush.rotate_90"),
                    cmd("Any Angle", "brush.rotate_any"),
                    cmd("Shear", "brush.shear"),
                ],
            ),
            sub(
                "Bend",
                &[
                    cmd("Horiz", "brush.bend_horizontal"),
                    cmd("Vert", "brush.bend_vertical"),
                ],
            ),
            sub(
                "Handle",
                &[
                    cmd("Center", "brush.handle_center"),
                    cmd("Corner", "brush.handle_corner"),
                    cmd("Place", "brush.handle_place"),
                ],
            ),
        ],
    },
    Menu {
        title: "Mode",
        items: &[
            mode("Matte", plain("F1"), PaintMode::Matte),
            mode("Color", plain("F2"), PaintMode::Color),
            mode("Replc", plain("F3"), PaintMode::Replace),
            mode("Smear", plain("F4"), PaintMode::Smear),
            mode("Shade", plain("F5"), PaintMode::Shade),
            mode("Blend", plain("F6"), PaintMode::Blend),
            mode("Cycle", plain("F7"), PaintMode::Cycle),
            mode("Smooth", plain("F8"), PaintMode::Smooth),
            mode("Mix", Key::None, PaintMode::Mix),
            mode("HBrite", Key::None, PaintMode::HBrite),
        ],
    },
    Menu {
        title: "Anim",
        items: &[
            sub(
                "AnimBrush",
                &[
                    cmd("Load..", "animbrush.load"),
                    cmd("Save..", "animbrush.save"),
                    cmd("Pick up..", "animbrush.pick_up"),
                    cmd("Settings..", "animbrush.settings"),
                    cmd("Use", "animbrush.use"),
                    cmd("Free", "animbrush.free"),
                ],
            ),
            cmd("Load..", "anim.load"),
            cmd("Save..", "anim.save"),
            cmd_key("Move..", plain("M"), "anim.move"),
            sub(
                "Frames",
                &[
                    cmd("Set #..", "anim.set_frame_count"),
                    cmd("Add Frames..", "anim.add_frames"),
                    cmd("Copy Frames..", "anim.copy_frames"),
                    cmd("Delete Frames..", "anim.delete_frames"),
                ],
            ),
            sub(
                "Control",
                &[
                    cmd("Panel On/Off", "anim.panel"),
                    cmd("Set Rate..", "anim.set_rate"),
                    cmd("Set Range..", "anim.set_range"),
                    cmd_key("Previous", plain("1"), "anim.previous"),
                    cmd_key("Next", plain("2"), "anim.next"),
                    cmd_key("Goto..", plain("3"), "anim.goto"),
                    cmd_key("Play", plain("4"), "anim.play"),
                    cmd_key("Play once", plain("5"), "anim.play_once"),
                    cmd_key("Ping-pong", plain("6"), "anim.ping_pong"),
                ],
            ),
            sub(
                "Method",
                &[
                    cmd("Compressed", "anim.method_compressed"),
                    cmd("Expanded", "anim.method_expanded"),
                ],
            ),
        ],
    },
    Menu {
        title: "Color",
        items: &[
            cmd("Ranges..", "color.ranges"),
            cmd_key("Cycle", plain("TAB"), "color.cycle"),
            sub(
                "Palette",
                &[
                    cmd_key("Mixer On/Off", plain("p"), "palette.mixer"),
                    cmd_key("Arrange", plain("P"), "palette.arrange"),
                    cmd("Use Brush Palette", "palette.use_brush"),
                    cmd("Restore Palette", "palette.restore"),
                    cmd("Default Palette", "palette.default"),
                    cmd("Load..", "palette.load"),
                    cmd("Save..", "palette.save"),
                ],
            ),
            sub(
                "Color Set",
                &[
                    cmd("Load..", "color_set.load"),
                    cmd("Save..", "color_set.save"),
                ],
            ),
            cmd("BG -> FG", "color.bg_to_fg"),
            cmd("BG <-> FG", "color.swap_bg_fg"),
            cmd("Remap", "color.remap"),
            cmd("Recompute", "color.recompute"),
            sub(
                "Brush",
                &[
                    cmd("BG -> FG", "brush.bg_to_fg"),
                    cmd("BG <-> FG", "brush.swap_bg_fg"),
                    cmd("Remap", "brush.remap"),
                    cmd("Change Transparency", "brush.change_transparency"),
                ],
            ),
        ],
    },
    Menu {
        title: "Effect",
        items: &[
            sub(
                "Stencil",
                &[
                    cmd("Show", "stencil.show"),
                    cmd_key("Make..", plain("~"), "stencil.make"),
                    cmd("Remake", "stencil.remake"),
                    cmd("Paint", "stencil.paint"),
                    cmd("Reverse", "stencil.reverse"),
                    cmd_key("On/Off", plain("`"), "stencil.toggle"),
                    cmd("Free", "stencil.free"),
                    cmd("Load..", "stencil.load"),
                    cmd("Save..", "stencil.save"),
                ],
            ),
            sub(
                "LightTable",
                &[
                    cmd_key("On/Off", plain("l"), "light_table.toggle"),
                    cmd("Merge", "light_table.merge"),
                    cmd("Dim", "light_table.dim"),
                    cmd("Two Back", "light_table.two_back"),
                    cmd("Previous", "light_table.previous"),
                    cmd("Next", "light_table.next"),
                    cmd("Spare", "light_table.spare"),
                ],
            ),
            sub(
                "Background",
                &[
                    cmd("Fix", "background.fix"),
                    cmd("Free", "background.free"),
                    cmd("Lock FG", "background.lock_fg"),
                ],
            ),
            sub(
                "AntiAlias",
                &[
                    cmd("None", "antialias.none"),
                    cmd("Low", "antialias.low"),
                    cmd("High", "antialias.high"),
                ],
            ),
            sub(
                "Process",
                &[
                    cmd("On/Off", "process.toggle"),
                    cmd("Tint", "process.tint"),
                    cmd("Hue", "process.hue"),
                    cmd("Value", "process.value"),
                ],
            ),
            sub(
                "Translucency",
                &[
                    cmd("On/Off", "translucency.toggle"),
                    cmd("Settings..", "translucency.settings"),
                ],
            ),
            sub(
                "Perspective",
                &[
                    cmd("Do", "perspective.do"),
                    cmd("FillScreen", "perspective.fill_screen"),
                    cmd("Reset", "perspective.reset"),
                    cmd("Center", "perspective.center"),
                    cmd("Settings..", "perspective.settings"),
                ],
            ),
        ],
    },
    Menu {
        title: "Prefs",
        items: &[
            pref("Coords", Pref::Coords),
            pref("Fast FB", Pref::FastFb),
            pref("MultiCycle", Pref::MultiCycle),
            pref("Be Square", Pref::BeSquare),
            pref("Workbench", Pref::Workbench),
            pref("ExclBrush", Pref::ExclBrush),
            pref("AutoTransp", Pref::AutoTransp),
            pref("No Icons", Pref::NoIcons),
            pref("AutoGrid", Pref::AutoGrid),
            pref("OriginUL", Pref::OriginUl),
            pref("FastAdjust", Pref::FastAdjust),
            sub(
                "Pixel Size",
                &[
                    cmd_key("Largest", Key::Amiga('0'), "view.largest_pixel_size"),
                    pixel_size("1x", Key::None, PixelSize(2)),
                    pixel_size("1.5x", Key::None, PixelSize(3)),
                    pixel_size("2x", Key::None, PixelSize(4)),
                    pixel_size("2.5x", Key::None, PixelSize(5)),
                    pixel_size("3x", Key::None, PixelSize(6)),
                    pixel_size("3.5x", Key::None, PixelSize(7)),
                    pixel_size("4x", Key::None, PixelSize(8)),
                    pixel_size("4.5x", Key::None, PixelSize(9)),
                    pixel_size("5x", Key::None, PixelSize(10)),
                    pixel_size("5.5x", Key::None, PixelSize(11)),
                    pixel_size("6x", Key::None, PixelSize(12)),
                    pixel_size("6.5x", Key::None, PixelSize(13)),
                    pixel_size("7x", Key::None, PixelSize(14)),
                    pixel_size("7.5x", Key::None, PixelSize(15)),
                    pixel_size("8x", Key::None, PixelSize(16)),
                ],
            ),
        ],
    },
];

/// Editor state the menus depend on (check marks, ghosted items).
#[derive(Clone, Copy, Debug)]
pub struct MenuContext {
    pub mode: PaintMode,
    pub prefs: Prefs,
    /// Extra-Half-Brite screen (enables Mode ▸ HBrite).
    pub ehb: bool,
    /// HAM screen (enables Color ▸ Recompute).
    pub ham: bool,
    /// Prefs ▸ Pixel Size.
    pub pixel_size: PixelSize,
    /// A custom brush is current (enables the Brush menu's operations).
    pub has_brush: bool,
    /// There is a picked-up brush to go back to (enables Brush ▸ Restore).
    pub can_restore: bool,
    /// Anim ▸ Method ▸ Expanded is chosen (else Compressed; ADR-38).
    pub anim_expanded: bool,
}

impl Action {
    /// `Some(checked)` for check-mark items, `None` otherwise.
    pub fn checked(self, ctx: &MenuContext) -> Option<bool> {
        match self {
            Action::Mode(m) => Some(ctx.mode == m),
            Action::Pref(p) => Some(ctx.prefs.get(p)),
            Action::PixelSize(p) => Some(ctx.pixel_size == p),
            // Anim ▸ Method: a radio pair (ADR-38).
            Action::Command("anim.method_compressed") => Some(!ctx.anim_expanded),
            Action::Command("anim.method_expanded") => Some(ctx.anim_expanded),
            Action::Command(_) => None,
        }
    }

    /// Commands not in [`IMPLEMENTED`] are ghosted; Mode, Prefs and Pixel Size items always
    /// work (HBrite only on EHB screens).
    pub fn enabled(self, ctx: &MenuContext) -> bool {
        match self {
            Action::Mode(PaintMode::HBrite) => ctx.ehb,
            Action::Command(id) => {
                IMPLEMENTED.contains(&id)
                    && (id != "color.recompute" || ctx.ham)
                    && brush_item_usable(id, ctx)
            }
            _ => true,
        }
    }
}

/// Brush items need a brush: only Load.. works without one and Restore needs a picked-up one.
fn brush_item_usable(id: &str, ctx: &MenuContext) -> bool {
    match id {
        "brush.load" => true,
        "brush.restore" => ctx.can_restore,
        _ if id.starts_with("brush.") => ctx.has_brush,
        _ => true,
    }
}

/// Menu commands that do something; every other command item is ghosted (ADR-15). Later
/// features add their ids here.
pub const IMPLEMENTED: &[&str] = &[
    // Picture
    "picture.load",
    "picture.save",
    "picture.export",
    "picture.delete",
    "picture.flip_horizontal",
    "picture.flip_vertical",
    "spare.swap",
    "spare.copy",
    "spare.merge_front",
    "spare.merge_back",
    "spare.delete",
    "picture.page_size",
    "picture.show_page",
    "picture.screen_format",
    "app.about",
    "app.quit",
    // Color
    "color.cycle",
    "palette.mixer",
    "palette.restore",
    "palette.default",
    "palette.load",
    "palette.save",
    "color.bg_to_fg",
    "color.swap_bg_fg",
    // Effect
    "background.fix",
    "background.free",
    // Effect > Stencil (ADR-20); Show, Paint, Load and Save stay ghosted.
    "stencil.make",
    "stencil.remake",
    "stencil.reverse",
    "stencil.toggle",
    "stencil.free",
    // Palette ops and effects (ADR-31); Use Brush Palette (brushes carry no palette) and
    // Background > Lock FG (needs an area stencil) stay ghosted.
    "palette.arrange",
    "color.remap",
    "color_set.load",
    "color_set.save",
    "stencil.show",
    "stencil.load",
    "stencil.save",
    "antialias.none",
    "antialias.low",
    "antialias.high",
    "process.toggle",
    "process.tint",
    "process.hue",
    "process.value",
    "translucency.toggle",
    "translucency.settings",
    // Prefs
    "view.largest_pixel_size",
    // Anim > Frames and Control, Effect > LightTable (ADR-28); AnimBrush, Move, Panel, Set
    // Range and Method stay ghosted.
    "anim.set_frame_count",
    "anim.add_frames",
    "anim.copy_frames",
    "anim.delete_frames",
    "anim.set_rate",
    "anim.previous",
    "anim.next",
    "anim.goto",
    "anim.play",
    "anim.play_once",
    "anim.ping_pong",
    "light_table.toggle",
    "light_table.merge",
    "light_table.dim",
    "light_table.two_back",
    "light_table.previous",
    "light_table.next",
    "light_table.spare",
    // Anim > Load.. and Save.. (IFF ANIM, ADR-32).
    "anim.load",
    "anim.save",
    // Range requester (ADR-22)
    "color.ranges",
    // Brush menu (ADR-19).
    "brush.load",
    "brush.save",
    "brush.restore",
    "brush.halve",
    "brush.double",
    "brush.double_horizontal",
    "brush.double_vertical",
    "brush.flip_horizontal",
    "brush.flip_vertical",
    "brush.outline",
    "brush.trim",
    "brush.rotate_90",
    "brush.handle_center",
    "brush.handle_corner",
    "brush.bg_to_fg",
    "brush.swap_bg_fg",
    "brush.change_transparency",
    // Brush menu, interactive items and spare brush (ADR-30); Metamorph stays ghosted.
    "brush.stretch",
    "brush.rotate_any",
    "brush.shear",
    "brush.bend_horizontal",
    "brush.bend_vertical",
    "brush.handle_place",
    "brush.to_spare",
    "brush.swap_spare",
    "brush.remap",
    // ADR-38: Anim > Move.., Control > Panel On/Off and Set Range.., Method.
    "anim.move",
    "anim.panel",
    "anim.set_range",
    "anim.method_compressed",
    "anim.method_expanded",
];

impl Item {
    fn action(&self) -> Option<Action> {
        match self.entry {
            Entry::Action(a) => Some(a),
            Entry::Submenu(_) => None,
        }
    }

    /// A submenu item is ghosted when all of its items are.
    fn enabled(&self, ctx: &MenuContext) -> bool {
        match self.entry {
            Entry::Action(a) => a.enabled(ctx),
            Entry::Submenu(items) => items.iter().any(|it| it.enabled(ctx)),
        }
    }
}

/// Whether menu `menu` has at least one enabled item (its title is ghosted otherwise).
pub fn menu_enabled(menu: usize, ctx: &MenuContext) -> bool {
    MENUS[menu].items.iter().any(|it| it.enabled(ctx))
}

struct Glyphs {
    amiga_key: Bitmap,
    submenu: Bitmap,
}

fn glyphs() -> &'static Glyphs {
    static GLYPHS: OnceLock<Glyphs> = OnceLock::new();
    GLYPHS.get_or_init(|| {
        let mut list = bitmap::parse(include_str!("../../../assets/ui/menu-glyphs.txt"))
            .expect("menu glyphs are valid");
        let mut take = |name: &str| {
            let i = list
                .iter()
                .position(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("menu glyph '{name}' missing"));
            list.swap_remove(i).1
        };
        Glyphs {
            amiga_key: take("amiga_key"),
            submenu: take("submenu"),
        }
    })
}

fn draw_bitmap(img: &mut IndexedImage, bmp: &Bitmap, x: i32, y: i32, pen: u8) {
    for by in 0..bmp.height {
        for bx in 0..bmp.width {
            if bmp.get(bx, by) {
                img.set(x + bx as i32, y + by as i32, pen);
            }
        }
    }
}

/// A title slot in the menu strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub x: i32,
    pub w: i32,
}

/// Title slots of [`MENUS`], left to right.
pub fn title_slots() -> [Slot; 7] {
    let font = Font::builtin();
    let mut x = geom::FIRST_SLOT_X;
    std::array::from_fn(|i| {
        let w = font.text_width(MENUS[i].title) + geom::SLOT_PAD;
        let slot = Slot { x, w };
        x += w;
        slot
    })
}

/// Position and size of a menu or submenu box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxLayout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Item text offset from `x` (larger in menus with check marks).
    pub text_dx: i32,
    pub items: usize,
}

impl BoxLayout {
    fn new(items: &[Item], x: i32, y: i32, screen_w: i32, screen_h: i32) -> Self {
        let font = Font::builtin();
        let checks = items.iter().any(|it| {
            it.action().is_some_and(|a| {
                matches!(a, Action::Mode(_) | Action::Pref(_) | Action::PixelSize(_))
            })
        });
        let text_dx = geom::TEXT_DX + if checks { geom::CHECK_W } else { 0 };
        let w = items
            .iter()
            .map(|it| {
                let end = text_dx + font.text_width(it.label);
                match (it.entry, it.key) {
                    (Entry::Submenu(_), _) => {
                        end + geom::KEY_GAP + geom::KEY_FROM_RIGHT - geom::TRIANGLE_DX
                    }
                    (_, Key::Amiga(_)) => {
                        end + geom::KEY_GAP + geom::KEY_FROM_RIGHT - geom::AMIGA_DX
                    }
                    (_, Key::Plain(k)) => {
                        end + geom::KEY_GAP + geom::KEY_FROM_RIGHT.max(4 + font.text_width(k))
                    }
                    (_, Key::None) => end + geom::RIGHT_MARGIN,
                }
            })
            .max()
            .unwrap_or(0);
        let h = geom::ITEM_DY + items.len() as i32 * geom::ITEM_H + geom::BOTTOM;
        Self {
            x: x.min(screen_w - w).max(0),
            y: y.min(screen_h - h).max(0),
            w,
            h,
            text_dx,
            items: items.len(),
        }
    }

    pub fn item_top(&self, i: usize) -> i32 {
        self.y + geom::ITEM_DY + i as i32 * geom::ITEM_H
    }

    fn key_x(&self) -> i32 {
        self.x + self.w - geom::KEY_FROM_RIGHT
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        (self.x..self.x + self.w).contains(&x) && (self.y..self.y + self.h).contains(&y)
    }

    /// Item under (`x`, `y`), if any.
    pub fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        let inside_x = (self.x + geom::BORDER_X..self.x + self.w - geom::BORDER_X).contains(&x);
        let dy = y - self.item_top(0);
        (inside_x && dy >= 0)
            .then_some((dy / geom::ITEM_H) as usize)
            .filter(|&i| i < self.items)
    }
}

/// Box of top-level menu `menu`.
pub fn menu_box(menu: usize, screen_w: i32, screen_h: i32) -> BoxLayout {
    let slot = title_slots()[menu];
    BoxLayout::new(
        MENUS[menu].items,
        slot.x + geom::BOX_DX,
        geom::BOX_TOP,
        screen_w,
        screen_h,
    )
}

/// Box of the submenu of item `item` in `parent`: right of the parent box with its first item
/// level with `item`; flipped to the left side when it would leave the screen.
pub fn submenu_box(
    parent: &BoxLayout,
    item: usize,
    items: &[Item],
    screen_w: i32,
    screen_h: i32,
) -> BoxLayout {
    let y = parent.item_top(item) - geom::ITEM_DY;
    let right = BoxLayout::new(
        items,
        parent.x + parent.w - geom::SUB_OVERLAP,
        y,
        screen_w,
        screen_h,
    );
    if parent.x + parent.w - geom::SUB_OVERLAP + right.w <= screen_w {
        right
    } else {
        BoxLayout::new(
            items,
            parent.x + geom::SUB_OVERLAP - right.w,
            y,
            screen_w,
            screen_h,
        )
    }
}

/// Interaction state of the menu strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MenuState {
    /// The strip is shown (right mouse button held).
    pub active: bool,
    pub menu: Option<usize>,
    pub item: Option<usize>,
    pub sub_item: Option<usize>,
}

impl MenuState {
    /// Activates the strip with the pointer at (`x`, `y`).
    pub fn open(&mut self, screen_w: i32, screen_h: i32, x: i32, y: i32) {
        *self = MenuState {
            active: true,
            ..MenuState::default()
        };
        self.pointer(screen_w, screen_h, x, y);
    }

    pub fn close(&mut self) {
        *self = MenuState::default();
    }

    /// Tracks the pointer; returns whether the visible state changed.
    pub fn pointer(&mut self, screen_w: i32, screen_h: i32, x: i32, y: i32) -> bool {
        let before = *self;
        if !self.active {
            return false;
        }
        if y < geom::BOX_TOP {
            if let Some(i) = title_slots()
                .iter()
                .position(|s| (s.x..s.x + s.w).contains(&x))
            {
                self.menu = Some(i);
            }
            self.item = None;
            self.sub_item = None;
            return *self != before;
        }
        let Some(m) = self.menu else {
            return false;
        };
        let mbox = menu_box(m, screen_w, screen_h);
        if let Some(it) = self.item
            && let Entry::Submenu(items) = MENUS[m].items[it].entry
        {
            let sbox = submenu_box(&mbox, it, items, screen_w, screen_h);
            if sbox.contains(x, y) {
                self.sub_item = sbox.item_at(x, y);
                return *self != before;
            }
        }
        if mbox.contains(x, y) {
            let item = mbox.item_at(x, y);
            if item != self.item {
                self.sub_item = None;
            }
            self.item = item;
        } else {
            self.item = None;
            self.sub_item = None;
        }
        *self != before
    }

    /// The enabled action under the pointer, if any.
    pub fn hovered_action(&self, ctx: &MenuContext) -> Option<Action> {
        let item = &MENUS[self.menu?].items[self.item?];
        let target = match item.entry {
            Entry::Action(_) => item,
            Entry::Submenu(items) => &items[self.sub_item?],
        };
        target.action().filter(|a| a.enabled(ctx))
    }
}

/// Draws the menu strip and the open menu/submenu.
pub fn draw(
    img: &mut IndexedImage,
    pens: &Pens,
    ctx: &MenuContext,
    state: &MenuState,
    screen_w: i32,
    screen_h: i32,
) {
    let font = Font::builtin();
    img.fill_rect(0, 0, screen_w, geom::STRIP_H, pens.face);
    for (i, slot) in title_slots().iter().enumerate() {
        let selected = state.menu == Some(i);
        let enabled = menu_enabled(i, ctx);
        if selected {
            img.fill_rect(slot.x, 0, slot.w, geom::TITLE_HL_H, pens.select);
        }
        let pen = match (selected, enabled) {
            (true, _) => pens.shine,
            (false, true) => pens.dark,
            (false, false) => pens.ghost.unwrap_or(pens.dark),
        };
        font.draw(img, slot.x + geom::SLOT_TEXT_DX, 1, MENUS[i].title, pen);
        if !enabled && !selected && pens.ghost.is_none() {
            let b = BoxLayout {
                x: slot.x - geom::BORDER_X,
                y: 0,
                w: slot.w + 2 * geom::BORDER_X,
                h: geom::ITEM_H,
                text_dx: 0,
                items: 0,
            };
            checker_ghost(img, &b, 0, pens.face);
        }
    }
    let Some(m) = state.menu else {
        return;
    };
    let mbox = menu_box(m, screen_w, screen_h);
    draw_box(img, pens, ctx, &mbox, MENUS[m].items, state.item);
    // A ghosted item opens no submenu (as in Intuition).
    if let Some(it) = state.item
        && let Entry::Submenu(items) = MENUS[m].items[it].entry
        && MENUS[m].items[it].enabled(ctx)
    {
        let sbox = submenu_box(&mbox, it, items, screen_w, screen_h);
        draw_box(img, pens, ctx, &sbox, items, state.sub_item);
    }
}

fn draw_box(
    img: &mut IndexedImage,
    pens: &Pens,
    ctx: &MenuContext,
    b: &BoxLayout,
    items: &[Item],
    highlight: Option<usize>,
) {
    let font = Font::builtin();
    let g = glyphs();
    img.fill_rect(b.x, b.y, b.w, b.h, pens.dark);
    img.fill_rect(
        b.x + geom::BORDER_X,
        b.y + geom::BORDER_Y,
        b.w - 2 * geom::BORDER_X,
        b.h - 2 * geom::BORDER_Y,
        pens.face,
    );
    let key_x = b.key_x();
    for (i, item) in items.iter().enumerate() {
        let top = b.item_top(i);
        let ty = top + 1;
        let enabled = item.enabled(ctx);
        let lit = enabled && highlight == Some(i);
        if lit {
            img.fill_rect(
                b.x + geom::HL_DX,
                top,
                b.w - 2 * geom::HL_DX,
                geom::ITEM_H,
                pens.select,
            );
        }
        let ink = match (enabled, lit) {
            (false, _) => pens.ghost.unwrap_or(pens.dark),
            (true, true) => pens.shine,
            (true, false) => pens.dark,
        };
        if item.action().and_then(|a| a.checked(ctx)) == Some(true) {
            font.draw_code(img, b.x + geom::TEXT_DX, ty, glyph::CHECK, ink);
        }
        font.draw(img, b.x + b.text_dx, ty, item.label, ink);
        match (item.entry, item.key) {
            (Entry::Submenu(_), _) => {
                draw_bitmap(img, &g.submenu, key_x + geom::TRIANGLE_DX, ty, ink);
            }
            (_, Key::Amiga(c)) => {
                draw_bitmap(img, &g.amiga_key, key_x + geom::AMIGA_DX, ty, ink);
                font.draw(img, key_x, ty, c.encode_utf8(&mut [0; 4]), ink);
            }
            (_, Key::Plain(k)) => {
                let x = key_x.min(b.x + b.w - 4 - font.text_width(k));
                let key_ink = match (enabled, lit) {
                    (false, _) => ink,
                    (true, true) => pens.shine,
                    (true, false) => pens.select,
                };
                font.draw(img, x, ty, k, key_ink);
            }
            (_, Key::None) => {}
        }
        if !enabled && pens.ghost.is_none() {
            checker_ghost(img, b, top, pens.face);
        }
    }
}

/// Fallback ghosting when the palette has no usable grey: a 50 % `face` checkerboard laid
/// over the disabled item (owner decision 2026-11-10; replaces the Intuition 1-in-4 dots,
/// which tore the 2-px strokes of our font).
fn checker_ghost(img: &mut IndexedImage, b: &BoxLayout, top: i32, face: u8) {
    for y in top..top + geom::ITEM_H {
        for x in b.x + geom::BORDER_X..b.x + b.w - geom::BORDER_X {
            if (x + y).rem_euclid(2) == 0 {
                img.set(x, y, face);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::Palette;

    fn ctx() -> MenuContext {
        MenuContext {
            mode: PaintMode::Color,
            prefs: Prefs::default(),
            ehb: false,
            ham: false,
            pixel_size: PixelSize::default(),
            has_brush: false,
            can_restore: false,
            anim_expanded: false,
        }
    }

    #[test]
    fn titles_fit_a_lores_screen() {
        let last = title_slots()[6];
        assert!(last.x + last.w <= 320, "strip ends at {}", last.x + last.w);
        assert_eq!(title_slots()[0].x, geom::FIRST_SLOT_X);
    }

    #[test]
    fn picture_menu_geometry_matches_reference() {
        let b = menu_box(0, 320, 256);
        // Box at the left edge, top border on row 9. DeluxePaint has 11 items (rows 9..=126);
        // Export.. (ADR-27) adds one: rows 9..=136.
        assert_eq!((b.x, b.y), (0, 9));
        assert_eq!(b.h, 128);
        assert_eq!(b.item_top(0), 13);
        assert_eq!(b.item_top(1), 23);
    }

    #[test]
    fn boxes_stay_on_screen() {
        for (m, menu) in MENUS.iter().enumerate() {
            let b = menu_box(m, 320, 256);
            assert!(b.x >= 0 && b.x + b.w <= 320, "menu {m}");
            for (i, it) in menu.items.iter().enumerate() {
                if let Entry::Submenu(items) = it.entry {
                    let s = submenu_box(&b, i, items, 320, 256);
                    assert!(s.x >= 0 && s.x + s.w <= 320, "submenu {m}/{i}");
                    assert!(s.y + s.h <= 256, "submenu {m}/{i}");
                    // The submenu never hides its parent item's label.
                    let label_end = b.x + b.text_dx + Font::builtin().text_width(it.label);
                    let left = s.x + s.w <= b.x + geom::SUB_OVERLAP;
                    assert!(
                        left || s.x >= label_end,
                        "submenu {m}/{i} covers its parent"
                    );
                }
            }
        }
    }

    #[test]
    fn hover_and_select() {
        let mut st = MenuState::default();
        st.open(320, 256, 10, 4);
        assert_eq!(st.menu, Some(0));
        assert_eq!(st.item, None);
        // Over "Save.." (second item).
        st.pointer(320, 256, 20, 25);
        assert_eq!(st.item, Some(1));
        assert_eq!(
            st.hovered_action(&ctx()),
            Some(Action::Command("picture.save"))
        );
        // Over "Flip": a submenu, nothing to select until a sub item is hovered.
        let b = menu_box(0, 320, 256);
        st.pointer(320, 256, 20, b.item_top(5) + 2);
        assert_eq!(st.item, Some(5));
        assert_eq!(st.hovered_action(&ctx()), None);
        let Entry::Submenu(items) = MENUS[0].items[5].entry else {
            panic!("Flip is a submenu");
        };
        let s = submenu_box(&b, 5, items, 320, 256);
        st.pointer(320, 256, s.x + s.w - 5, s.item_top(1) + 2);
        assert_eq!(st.item, Some(5), "parent item stays highlighted");
        assert_eq!(
            st.hovered_action(&ctx()),
            Some(Action::Command("picture.flip_vertical"))
        );
        // Leaving all boxes clears the selection.
        st.pointer(320, 256, 300, 250);
        assert_eq!(st.hovered_action(&ctx()), None);
    }

    #[test]
    fn pixel_size_submenu_is_last_prefs_item() {
        let prefs = MENUS.last().unwrap();
        assert_eq!(prefs.title, "Prefs");
        let last = prefs.items.last().unwrap();
        assert_eq!(last.label, "Pixel Size");
        let Entry::Submenu(items) = last.entry else {
            panic!("Pixel Size is a submenu");
        };
        let labels: Vec<_> = items.iter().map(|i| i.label).collect();
        assert_eq!(
            labels,
            [
                "Largest", "1x", "1.5x", "2x", "2.5x", "3x", "3.5x", "4x", "4.5x", "5x", "5.5x",
                "6x", "6.5x", "7x", "7.5x", "8x"
            ]
        );
        assert_eq!(items[0].key, Key::Amiga('0'));
        assert_eq!(
            items[0].action(),
            Some(Action::Command("view.largest_pixel_size"))
        );
        // Whole and half steps: item i is i+1 half points.
        for h in PixelSize::MIN..=PixelSize::MAX {
            assert_eq!(
                items[usize::from(h) - 1].action(),
                Some(Action::PixelSize(PixelSize(h)))
            );
        }
    }

    #[test]
    fn pixel_size_checked() {
        let c = MenuContext {
            pixel_size: PixelSize(2),
            ..ctx()
        };
        assert_eq!(Action::PixelSize(PixelSize(2)).checked(&c), Some(true));
        assert_eq!(Action::PixelSize(PixelSize(6)).checked(&c), Some(false));
        assert_eq!(Action::PixelSize(PixelSize(6)).checked(&ctx()), Some(true));
        assert!(Action::PixelSize(PixelSize(8)).enabled(&c));
    }

    #[test]
    fn select_pixel_size_3x_on_lores_screen() {
        let m = MENUS.len() - 1;
        let it = MENUS[m].items.len() - 1;
        let mut st = MenuState::default();
        let mb = menu_box(m, 320, 256);
        st.open(320, 256, title_slots()[m].x + 2, 4);
        assert_eq!(st.menu, Some(m));
        st.pointer(320, 256, mb.x + 5, mb.item_top(it) + 2);
        assert_eq!(st.item, Some(it));
        let Entry::Submenu(items) = MENUS[m].items[it].entry else {
            panic!("submenu");
        };
        let sb = submenu_box(&mb, it, items, 320, 256);
        assert!(sb.x >= 0 && sb.x + sb.w <= 320 && sb.y + sb.h <= 256);
        // Move horizontally into the submenu level with the parent item, then down to "3x".
        st.pointer(320, 256, sb.x + sb.w / 2, sb.item_top(0) + 2);
        st.pointer(320, 256, sb.x + sb.w / 2, sb.item_top(5) + 2);
        assert_eq!(
            st.hovered_action(&ctx()),
            Some(Action::PixelSize(PixelSize(6)))
        );
        // Drawing with the submenu open must not panic.
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let mut img = IndexedImage::new(320, 256, pens.dark);
        draw(&mut img, &pens, &ctx(), &st, 320, 256);
    }

    #[test]
    fn hbrite_is_ghosted_without_ehb() {
        assert!(!Action::Mode(PaintMode::HBrite).enabled(&ctx()));
        let ehb = MenuContext { ehb: true, ..ctx() };
        assert!(Action::Mode(PaintMode::HBrite).enabled(&ehb));
        assert_eq!(Action::Mode(PaintMode::Color).checked(&ctx()), Some(true));
        assert_eq!(Action::Pref(Pref::Coords).checked(&ctx()), Some(false));
        assert_eq!(Action::Pref(Pref::BeSquare).checked(&ctx()), Some(true));
    }

    #[test]
    fn ghosted_item_uses_ghost_pen_or_checkerboard() {
        let mode = MENUS.iter().position(|m| m.title == "Mode").unwrap();
        let items = MENUS[mode].items;
        let hbrite = items.iter().position(|i| i.label == "HBrite").unwrap();
        let no_grey = Palette::from_rgb4(&[0x000, 0xAAA, 0x111, 0x999]).unwrap();
        for (pal, has_grey) in [(Palette::dpaint_default_32(), true), (no_grey, false)] {
            let pens = Pens::from_palette(&pal);
            assert_eq!(pens.ghost.is_some(), has_grey);
            let mut img = IndexedImage::new(320, 256, pens.dark);
            let b = menu_box(mode, 320, 256);
            draw_box(&mut img, &pens, &ctx(), &b, items, None);
            let top = b.item_top(hbrite);
            let ink: Vec<(i32, i32, u8)> = (top..top + geom::ITEM_H)
                .flat_map(|y| {
                    (b.x + geom::BORDER_X..b.x + b.w - geom::BORDER_X).map(move |x| (x, y))
                })
                .map(|(x, y)| (x, y, img.get(x, y).unwrap()))
                .filter(|&(_, _, c)| c != pens.face)
                .collect();
            assert!(!ink.is_empty(), "label is drawn");
            match pens.ghost {
                // Solid label in the ghost pen.
                Some(g) => assert!(ink.iter().all(|&(_, _, c)| c == g)),
                // Dark label thinned by the checkerboard.
                None => assert!(
                    ink.iter()
                        .all(|&(x, y, c)| c == pens.dark && (x + y).rem_euclid(2) == 1)
                ),
            }
        }
    }

    #[test]
    fn draws_highlighted_item_in_select_with_shine_text() {
        let pal = Palette::dpaint_default_32();
        let pens = Pens::from_palette(&pal);
        let mut img = IndexedImage::new(320, 256, 0);
        let mut st = MenuState::default();
        st.open(320, 256, 10, 4);
        st.pointer(320, 256, 20, 15);
        draw(&mut img, &pens, &ctx(), &st, 320, 256);
        let b = menu_box(0, 320, 256);
        // Title highlight, box borders, item highlight.
        assert_eq!(img.get(3, 0), Some(pens.select));
        assert_eq!(img.get(2, 0), Some(pens.face));
        assert_eq!(img.get(0, 60), Some(pens.dark));
        assert_eq!(img.get(1, 60), Some(pens.dark));
        assert_eq!(img.get(2, 60), Some(pens.face));
        assert_eq!(img.get(b.w - 1, 60), Some(pens.dark));
        assert_eq!(img.get(4, 13), Some(pens.select));
        assert_eq!(img.get(3, 13), Some(pens.face));
        assert_eq!(img.get(b.w - 5, 22), Some(pens.select));
        assert_eq!(img.get(b.w - 4, 22), Some(pens.face));
        // The highlighted item's text and shortcut are drawn in shine.
        let row: Vec<u8> = (b.x..b.x + b.w).filter_map(|x| img.get(x, 15)).collect();
        assert!(row.contains(&pens.shine));
        assert_eq!(
            img.get(b.w + 5, 9),
            Some(pens.face),
            "strip row 9 outside the box"
        );
    }

    /// Command ids of all items, submenus included.
    fn command_ids(items: &[Item], out: &mut Vec<&'static str>) {
        for it in items {
            match it.entry {
                Entry::Action(Action::Command(id)) => out.push(id),
                Entry::Action(_) => {}
                Entry::Submenu(sub) => command_ids(sub, out),
            }
        }
    }

    #[test]
    fn unimplemented_commands_are_ghosted() {
        let c = ctx();
        let mut ids = Vec::new();
        for m in &MENUS {
            command_ids(m.items, &mut ids);
        }
        for id in IMPLEMENTED {
            assert!(ids.contains(id), "{id} is no menu item");
        }
        assert!(Action::Command("picture.load").enabled(&c));
        assert!(Action::Command("spare.swap").enabled(&c));
        assert!(!Action::Command("picture.print").enabled(&c));
        assert!(Action::Command("stencil.make").enabled(&c));
        assert!(!Action::Command("stencil.paint").enabled(&c));
        assert!(!Action::Command("color.recompute").enabled(&c));
        // Mode, Prefs and Pixel Size items stay enabled.
        assert!(Action::Mode(PaintMode::Matte).enabled(&c));
        assert!(Action::Pref(Pref::FastFb).enabled(&c));
        assert!(Action::PixelSize(PixelSize(4)).enabled(&c));
        // Menus and submenus with nothing enabled are ghosted as a whole.
        let menu = |t: &str| MENUS.iter().position(|m| m.title == t).unwrap();
        // Brush: only Load.. works without a brush; Restore needs a picked-up one.
        assert!(menu_enabled(menu("Brush"), &c));
        assert!(Action::Command("brush.load").enabled(&c));
        assert!(!Action::Command("brush.flip_horizontal").enabled(&c));
        assert!(!Action::Command("brush.restore").enabled(&c));
        assert!(!Action::Command("brush.bg_to_fg").enabled(&c));
        let with = MenuContext {
            has_brush: true,
            can_restore: true,
            ..c
        };
        assert!(Action::Command("brush.flip_horizontal").enabled(&with));
        assert!(Action::Command("brush.restore").enabled(&with));
        assert!(Action::Command("brush.stretch").enabled(&with));
        assert!(
            !Action::Command("brush.metamorph").enabled(&with),
            "Metamorph: ghosted"
        );
        assert!(menu_enabled(menu("Anim"), &c), "frames work (ADR-28)");
        assert!(
            Action::Command("anim.move").enabled(&c),
            "Move works (ADR-38)"
        );
        assert!(menu_enabled(menu("Picture"), &c));
        let effect = &MENUS[menu("Effect")];
        let item = |label: &str| effect.items.iter().find(|i| i.label == label).unwrap();
        assert!(item("Stencil").enabled(&c), "some Stencil items work");
        assert!(item("LightTable").enabled(&c));
        assert!(!item("Perspective").enabled(&c));
        assert!(item("Background").enabled(&c));
    }

    #[test]
    fn a_ghosted_submenu_does_not_open() {
        let pens = Pens::from_palette(&Palette::dpaint_default_32());
        let m = MENUS.iter().position(|m| m.title == "Effect").unwrap();
        let draw_with = |item: usize| {
            let mut img = IndexedImage::new(320, 256, 200);
            let st = MenuState {
                active: true,
                menu: Some(m),
                item: Some(item),
                sub_item: None,
            };
            draw(&mut img, &pens, &ctx(), &st, 320, 256);
            img
        };
        let b = menu_box(m, 320, 256);
        let sub = |item: usize| {
            let Entry::Submenu(items) = MENUS[m].items[item].entry else {
                panic!("item {item} is a submenu");
            };
            submenu_box(&b, item, items, 320, 256)
        };
        // Perspective (item 6) is ghosted: nothing opens; Background (item 2) opens.
        let s = sub(6);
        assert_eq!(draw_with(6).get(s.x + s.w / 2, s.y + s.h - 1), Some(200));
        let s = sub(2);
        assert_eq!(
            draw_with(2).get(s.x + s.w / 2, s.y + s.h - 1),
            Some(pens.dark)
        );
    }

    #[test]
    fn prefs_toggle() {
        let mut p = Prefs::default();
        assert!(!p.get(Pref::Coords));
        p.toggle(Pref::Coords);
        assert!(p.get(Pref::Coords));
        p.set(Pref::Coords, false);
        assert!(!p.get(Pref::Coords));
    }
}
