//! Picture ▸ Generate.. (ADR-45): the Generate Picture requester, a DeluxePainter addition.
//!
//! It asks what the picture should show (a text field; typing and Cmd/Ctrl+V paste go there),
//! the Resolution (a preset), the Chipset (OCS / AGA), the number of Colours (up to the
//! mode's maximum: OCS LoRes 32, OCS HiRes 16, AGA 256) and the Style. "More.." shows the
//! Palette (New / Current), Dither, Fit (crop to fill / whole picture), Scaler, Background
//! (opaque / transparent), Quality and the Target (Spare page, Picture, Brush, New layer).
//! "Diffusion" is Floyd-Steinberg error diffusion, "Whole" fits the whole picture with
//! margins (ADR-45). A new layer shares the picture's page, format and palette, so with that
//! target Resolution, Chipset, Colours and Palette show the picture's own and are ghosted.
//!
//! The box right of the options says, before anything is generated, where the description
//! goes and what it costs (or that no generator is set up); while a picture is generated it
//! shows the elapsed time; afterwards the result as a true-colour thumbnail. The screen is
//! indexed with one palette, so the box is left dark here and the app paints the thumbnail
//! into the RGBA frame afterwards ([`GenerateRequester::overlay`], as ADR-42's Import
//! requester does). Every option change after a result converts the same picture again
//! (nothing is generated again); "Again" generates a new one and "Apply" takes it over.
//!
//! The requester only holds the choices and draws them; generating, converting and applying
//! are the app's (ADR-45). Drawn in UI (LoRes) pixels with palette-derived pens, square
//! corners: Return = Generate (Apply once there is a result), Esc = Cancel (while
//! generating: stop waiting).

use dpx_core::import::{COLOR_COUNTS, Framing, PaletteChoice, Scaler};
use dpx_core::quantize::Dither;
use dpx_core::{Chipset, DisplayMode, IndexedImage, Rect, Resolution, RgbaImage, Target, Video};

use crate::font::Font;
use crate::note_requester::wrap;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{Inset, TextField, centred, checker, draw_button, draw_field, frame};

const TITLE: &str = "Generate Picture";
const ASK: &str = "What should the picture show?";
/// Largest requester (UI pixels); smaller screens get a smaller one.
const MAX_W: i32 = 360;
const MAX_H: i32 = 232;
/// Smallest requester: every row and the buttons need this much (it fits 320×200).
const MIN_W: i32 = 304;
const MIN_H: i32 = 190;
const LEFT: i32 = 6;
const ROW_STEP: i32 = 10;
const ARROW: i32 = 9;
const BUTTON_H: i32 = 13;
const LINE_STEP: i32 = 9;
/// Most characters of a description.
pub const MAX_PROMPT_CHARS: usize = 1000;

mod geom {
    pub const ASK_Y: i32 = 15;
    pub const PROMPT_Y: i32 = 25;
    pub const PROMPT_H: i32 = 10;
    pub const RESOLUTION_Y: i32 = 38;
    pub const ROWS_Y: i32 = 50;
}

/// A screen the picture is made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolutionPreset {
    pub label: &'static str,
    pub mode: DisplayMode,
    pub screen: (u32, u32),
}

impl ResolutionPreset {
    /// The target on `chipset`: the mode with this screen (the page has the screen's size).
    pub fn target(&self, chipset: Chipset) -> Target {
        Target::new(chipset, self.mode).with_screen(self.screen.0, self.screen.1)
    }
}

// TODO(wave8-integration): use the dpx_core::mode presets of the modes branch (ADR-46)
// instead of this private copy.
/// The Resolution choices: the standard screens and 320×240, a custom PAL LoRes screen (no
/// standard NTSC or PAL size).
pub const RESOLUTIONS: [ResolutionPreset; 7] = [
    ResolutionPreset {
        label: "NTSC LoRes 320x200",
        mode: DisplayMode::new(Video::Ntsc, Resolution::Lo, false),
        screen: (320, 200),
    },
    ResolutionPreset {
        label: "PAL LoRes 320x256",
        mode: DisplayMode::new(Video::Pal, Resolution::Lo, false),
        screen: (320, 256),
    },
    ResolutionPreset {
        label: "Pixel art 320x240 (custom)",
        mode: DisplayMode::new(Video::Pal, Resolution::Lo, false),
        screen: (320, 240),
    },
    ResolutionPreset {
        label: "NTSC HiRes 640x200",
        mode: DisplayMode::new(Video::Ntsc, Resolution::Hi, false),
        screen: (640, 200),
    },
    ResolutionPreset {
        label: "PAL HiRes 640x256",
        mode: DisplayMode::new(Video::Pal, Resolution::Hi, false),
        screen: (640, 256),
    },
    ResolutionPreset {
        label: "NTSC HiRes Laced 640x400",
        mode: DisplayMode::new(Video::Ntsc, Resolution::Hi, true),
        screen: (640, 400),
    },
    ResolutionPreset {
        label: "PAL HiRes Laced 640x512",
        mode: DisplayMode::new(Video::Pal, Resolution::Hi, true),
        screen: (640, 512),
    },
];

/// The default Resolution: 320×240 (the spec's easy-to-find pixel-art size).
pub const DEFAULT_RESOLUTION: usize = 2;

/// The Style choice (the app adds its sentence to the description, ADR-45).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenStyle {
    #[default]
    PixelArt,
    Painted,
    None,
}

/// The Quality choice (the provider's quality; higher costs more).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenQuality {
    Low,
    #[default]
    Medium,
    High,
    Auto,
}

/// Where Apply puts the picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenTarget {
    /// The spare page becomes the picture; the shown page stays untouched.
    #[default]
    Spare,
    /// The picture replaces the shown page (one undo step).
    Picture,
    /// The picture becomes the custom brush, fitted to the picture palette.
    Brush,
    /// A new layer above the active one (ADR-44), in the picture's page size and palette
    /// (one undo step); colour 0 is transparent there, so opaque pixels never take it.
    NewLayer,
}

/// Everything the requester asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerateOptions {
    /// Index into [`RESOLUTIONS`].
    pub resolution: usize,
    pub chipset: Chipset,
    /// Palette entries with a new palette: a power of two up to the mode's maximum.
    pub colors: u16,
    pub style: GenStyle,
    pub palette: PaletteChoice,
    pub dither: Dither,
    pub framing: Framing,
    pub scaler: Scaler,
    /// Background: transparent (colour 0) instead of opaque.
    pub transparent: bool,
    pub quality: GenQuality,
    pub target: GenTarget,
}

impl Default for GenerateOptions {
    /// The spec's default view: 320×240, OCS, 32 colours, classic pixel art; crisp pixel-art
    /// scaling without dither, a new palette, cropped to fill, opaque, onto the spare page.
    fn default() -> Self {
        Self {
            resolution: DEFAULT_RESOLUTION,
            chipset: Chipset::Ocs,
            colors: 32,
            style: GenStyle::PixelArt,
            palette: PaletteChoice::New,
            dither: Dither::None,
            framing: Framing::Crop,
            scaler: Scaler::PixelArt,
            transparent: false,
            quality: GenQuality::Medium,
            target: GenTarget::Spare,
        }
    }
}

impl GenerateOptions {
    pub fn preset(&self) -> &'static ResolutionPreset {
        &RESOLUTIONS[self.resolution.min(RESOLUTIONS.len() - 1)]
    }

    /// The target the picture is made for.
    pub fn target(&self) -> Target {
        self.preset().target(self.chipset)
    }

    /// The mode's most colours on the chipset.
    pub fn max_colors(&self) -> u16 {
        self.preset().mode.max_colors(self.chipset).unwrap_or(16)
    }

    /// The options with the resolution and colour count made valid.
    pub fn clamped(mut self) -> Self {
        self.resolution = self.resolution.min(RESOLUTIONS.len() - 1);
        self.colors = dpx_core::import::color_count(self.colors.max(2), self.max_colors());
        self
    }
}

/// The option rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenRow {
    Resolution,
    Chipset,
    Colors,
    Style,
    Palette,
    Dither,
    Fit,
    Scaler,
    Background,
    Quality,
    Target,
}

/// The rows always shown below Resolution.
const BASIC: [GenRow; 3] = [GenRow::Chipset, GenRow::Colors, GenRow::Style];
/// The rows "More.." shows.
const MORE: [GenRow; 7] = [
    GenRow::Palette,
    GenRow::Dither,
    GenRow::Fit,
    GenRow::Scaler,
    GenRow::Background,
    GenRow::Quality,
    GenRow::Target,
];

impl GenRow {
    pub fn label(self) -> &'static str {
        match self {
            GenRow::Resolution => "Resolution:",
            GenRow::Chipset => "Chipset:",
            GenRow::Colors => "Colours:",
            GenRow::Style => "Style:",
            GenRow::Palette => "Palette:",
            GenRow::Dither => "Dither:",
            GenRow::Fit => "Fit:",
            GenRow::Scaler => "Scaler:",
            GenRow::Background => "Background:",
            GenRow::Quality => "Quality:",
            GenRow::Target => "Target:",
        }
    }

    /// The row's choices, in order (Colours: the counts up to the largest maximum).
    fn choices(self) -> Vec<&'static str> {
        match self {
            GenRow::Resolution => RESOLUTIONS.iter().map(|r| r.label).collect(),
            GenRow::Chipset => vec!["OCS", "AGA"],
            GenRow::Colors => vec!["2", "4", "8", "16", "32", "64", "128", "256"],
            GenRow::Style => vec!["Pixel art", "Painted", "None"],
            GenRow::Palette => vec!["New", "Current"],
            GenRow::Dither => vec!["None", "Diffusion", "Ordered"],
            GenRow::Fit => vec!["Crop", "Whole"],
            GenRow::Scaler => vec!["Smooth", "Nearest", "Pixel art"],
            GenRow::Background => vec!["Opaque", "Transparent"],
            GenRow::Quality => vec!["Low", "Medium", "High", "Auto"],
            GenRow::Target => vec!["Spare page", "Picture", "Brush", "New layer"],
        }
    }
}

const CHIPSETS: [Chipset; 2] = [Chipset::Ocs, Chipset::Aga];
const STYLES: [GenStyle; 3] = [GenStyle::PixelArt, GenStyle::Painted, GenStyle::None];
const PALETTES: [PaletteChoice; 2] = [PaletteChoice::New, PaletteChoice::Current];
const DITHERS: [Dither; 3] = [Dither::None, Dither::FloydSteinberg, Dither::Ordered];
const FITS: [Framing; 2] = [Framing::Crop, Framing::Fit];
const SCALERS: [Scaler; 3] = [Scaler::Smooth, Scaler::Nearest, Scaler::PixelArt];
const QUALITIES: [GenQuality; 4] = [
    GenQuality::Low,
    GenQuality::Medium,
    GenQuality::High,
    GenQuality::Auto,
];
const TARGETS: [GenTarget; 4] = [
    GenTarget::Spare,
    GenTarget::Picture,
    GenTarget::Brush,
    GenTarget::NewLayer,
];

fn index_of<T: PartialEq>(list: &[T], v: &T) -> usize {
    list.iter().position(|x| x == v).unwrap_or(0)
}

/// What the requester is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenPhase {
    /// Asking (nothing generated, or the last generation failed or was stopped).
    #[default]
    Ask,
    /// A picture is being generated, for this many seconds.
    Working { seconds: u64 },
    /// A picture was generated; the thumbnail shows it converted with the current options.
    Result,
}

/// What the requester says about the generator (the app fills it in).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderInfo {
    /// A generator is set up and can be used.
    pub ready: bool,
    /// One line under the options, e.g. its name and that it uses the internet.
    pub summary: String,
    /// Shown in the box before anything is generated: where the description goes and what
    /// it costs, or how to set a generator up.
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerateResponse {
    Continue,
    /// An option changed: a result is converted again.
    Changed,
    /// Generate (or Again): start a new generation.
    Generate,
    /// Cancel while generating: stop waiting for the picture.
    CancelJob,
    /// Close the requester without applying anything.
    Cancel,
    /// Take the result over.
    Apply,
    /// Open the Image Generator setup.
    SetUp,
}

/// The requester's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenControl {
    /// The description field.
    Prompt,
    /// A row's ◂ and ▶ buttons.
    Prev(GenRow),
    Next(GenRow),
    More,
    SetUp,
    Cancel,
    /// Generate, or Again once there is a result.
    Generate,
    /// Only with a result.
    Apply,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerateRequester {
    options: GenerateOptions,
    prompt: TextField,
    more: bool,
    /// The UI screen (LoRes pixels) the requester is centred on.
    screen: (i32, i32),
    /// The current document's palette size (shown, ghosted, with Palette: Current).
    doc_colors: u16,
    /// The current document's target and page size (a new layer is made for them).
    document: Option<(Target, (u32, u32))>,
    phase: GenPhase,
    provider: ProviderInfo,
    /// The second line under the options, e.g. what Apply will do.
    message: Option<String>,
    /// Why the last generation or conversion failed: shown in the box instead of the note.
    error: Option<String>,
    thumb: Option<RgbaImage>,
    held: Option<GenControl>,
    pointer: (i32, i32),
}

impl GenerateRequester {
    pub fn new(
        options: GenerateOptions,
        prompt: &str,
        doc_colors: u16,
        screen: (i32, i32),
        provider: ProviderInfo,
    ) -> Self {
        let mut field = TextField::new(MAX_PROMPT_CHARS);
        field.set(prompt);
        Self {
            options: options.clamped(),
            prompt: field,
            more: false,
            screen,
            doc_colors,
            document: None,
            phase: GenPhase::Ask,
            provider,
            message: None,
            error: None,
            thumb: None,
            held: None,
            pointer: (0, 0),
        }
    }

    pub fn options(&self) -> GenerateOptions {
        self.options
    }

    pub fn set_options(&mut self, options: GenerateOptions) {
        self.options = options.clamped();
    }

    /// The description as typed.
    pub fn prompt(&self) -> String {
        self.prompt.text()
    }

    pub fn set_prompt(&mut self, text: &str) {
        self.prompt.set(text);
    }

    pub fn more(&self) -> bool {
        self.more
    }

    pub fn set_more(&mut self, more: bool) {
        self.more = more;
    }

    pub fn phase(&self) -> GenPhase {
        self.phase
    }

    /// The app sets the phase; leaving Result drops the thumbnail.
    pub fn set_phase(&mut self, phase: GenPhase) {
        if phase != GenPhase::Result {
            self.thumb = None;
        }
        self.phase = phase;
    }

    pub fn provider(&self) -> &ProviderInfo {
        &self.provider
    }

    pub fn set_provider(&mut self, provider: ProviderInfo) {
        self.provider = provider;
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message;
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Why the last generation or conversion failed (shown in the box), or `None`.
    pub fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    pub fn set_doc_colors(&mut self, n: u16) {
        self.doc_colors = n;
    }

    /// The current document's target and page size: the New layer target makes the picture
    /// for them (until this is set, the chosen resolution stands in).
    pub fn set_document(&mut self, target: Target, page: (u32, u32)) {
        self.document = Some((target, page));
    }

    /// The target and page size the picture is made for: the document's for a new layer,
    /// else the chosen resolution on the chosen chipset.
    pub fn format(&self) -> (Target, (u32, u32)) {
        match (self.options.target, self.document) {
            (GenTarget::NewLayer, Some(d)) => d,
            _ => {
                let t = self.options.target();
                (t, t.screen)
            }
        }
    }

    /// The palette the picture is made in: always the current one for a new layer.
    pub fn palette_choice(&self) -> PaletteChoice {
        match self.options.target {
            GenTarget::NewLayer => PaletteChoice::Current,
            _ => self.options.palette,
        }
    }

    /// A row that shows the document's own value and cannot change: Resolution, Chipset,
    /// Colours and Palette for a new layer (it shares the picture's page and palette).
    pub fn row_fixed(&self, row: GenRow) -> bool {
        self.options.target == GenTarget::NewLayer
            && matches!(
                row,
                GenRow::Resolution | GenRow::Chipset | GenRow::Colors | GenRow::Palette
            )
    }

    /// The requester's size on its screen.
    pub fn size(&self) -> (i32, i32) {
        (
            (self.screen.0 - 8).clamp(MIN_W, MAX_W),
            (self.screen.1 - 8).clamp(MIN_H, MAX_H),
        )
    }

    /// Top-left corner on its screen: centred.
    pub fn origin(&self) -> (i32, i32) {
        let (w, h) = self.size();
        (
            ((self.screen.0 - w) / 2).max(0),
            ((self.screen.1 - h) / 2).max(0),
        )
    }

    /// The colour count is ghosted while the palette is the current one.
    pub fn colors_enabled(&self) -> bool {
        self.palette_choice() == PaletteChoice::New
    }

    /// The colour count shown: the chosen one, or the document palette's size.
    pub fn shown_colors(&self) -> u16 {
        if self.colors_enabled() {
            self.options.colors
        } else {
            self.doc_colors
        }
    }

    /// Why nothing can be generated with these options: Palette Current with a document
    /// palette larger than the chosen mode shows (generating would be paid for and then not
    /// convertible). Shown in the box instead of the note; Generate is disabled meanwhile.
    pub fn palette_problem(&self) -> Option<String> {
        let (target, _) = self.format();
        let max = target.mode.max_colors(target.chipset).unwrap_or(16);
        let current = self.palette_choice() == PaletteChoice::Current;
        (current && self.doc_colors > max).then(|| {
            if self.options.target == GenTarget::NewLayer {
                format!(
                    "The picture's palette has {} colours; its mode shows {max} on {}, so a \
                     new layer cannot be made for it. Choose another target.",
                    self.doc_colors,
                    self.value(GenRow::Chipset),
                )
            } else {
                format!(
                    "The current palette has {} colours; this mode shows {max} on {}. Choose \
                     Palette New, AGA or another resolution.",
                    self.doc_colors,
                    self.value(GenRow::Chipset),
                )
            }
        })
    }

    /// The rows shown, below Resolution.
    pub fn rows(&self) -> Vec<GenRow> {
        let mut rows = BASIC.to_vec();
        if self.more {
            rows.extend(MORE);
        }
        rows
    }

    fn label_w() -> i32 {
        let font = Font::builtin();
        let rows = [GenRow::Resolution].into_iter().chain(BASIC).chain(MORE);
        rows.map(|r| font.text_width(r.label())).max().unwrap_or(0) + 3
    }

    /// Width of the value between a row's arrows.
    fn value_w(row: GenRow) -> i32 {
        let font = Font::builtin();
        let widest = |rows: &[GenRow]| {
            rows.iter()
                .flat_map(|r| r.choices())
                .map(|c| font.text_width(c))
                .max()
                .unwrap_or(0)
        };
        match row {
            GenRow::Resolution => widest(&[GenRow::Resolution]) + 4,
            _ => {
                let all: Vec<GenRow> = BASIC.iter().chain(MORE.iter()).copied().collect();
                widest(&all) + 4
            }
        }
    }

    fn row_y(&self, row: GenRow) -> i32 {
        if row == GenRow::Resolution {
            return geom::RESOLUTION_Y;
        }
        let i = self.rows().iter().position(|&r| r == row).unwrap_or(0) as i32;
        geom::ROWS_Y + i * ROW_STEP
    }

    /// The right edge of the option column (requester coordinates).
    fn column_right() -> i32 {
        LEFT + Self::label_w() + ARROW + Self::value_w(GenRow::Chipset) + ARROW
    }

    /// The top of the two lines under the options.
    fn lines_y(&self) -> i32 {
        let (_, h) = self.size();
        h - BUTTON_H - 5 - 2 * LINE_STEP - 3
    }

    /// The box for the note, the progress and the thumbnail (requester coordinates, frame
    /// included): across the requester below the rows, or right of them while "More.." shows
    /// the other rows.
    fn box_rect(&self) -> Rect {
        let (w, _) = self.size();
        let y1 = self.lines_y() - 3;
        let (x0, y0) = if self.more {
            (Self::column_right() + 8, geom::ROWS_Y - 1)
        } else {
            (LEFT, geom::ROWS_Y + BASIC.len() as i32 * ROW_STEP + 1)
        };
        Rect::new(x0, y0, (w - LEFT - x0).max(8), (y1 - y0).max(8))
    }

    /// The thumbnail's size in UI pixels: the result's display proportions fitted into the
    /// box (inside a one-pixel frame and a one-pixel gap).
    pub fn thumb_size(&self) -> (u32, u32) {
        let area = self.box_rect().inset(2);
        let (target, page) = self.format();
        let (fx, fy) = target.mode.pixel_shape();
        dpx_core::resample::fit_size(
            page.0 * fx.max(1),
            page.1 * fy.max(1),
            area.w.max(1) as u32,
            area.h.max(1) as u32,
            (1, 1),
        )
    }

    fn thumb_local(&self) -> Rect {
        let area = self.box_rect().inset(2);
        let (tw, th) = self.thumb_size();
        let (tw, th) = (tw as i32, th as i32);
        Rect::new(
            area.x + (area.w - tw) / 2,
            area.y + (area.h - th) / 2,
            tw,
            th,
        )
    }

    /// The thumbnail's rectangle on the UI screen.
    pub fn thumb_rect(&self) -> Rect {
        let (ox, oy) = self.origin();
        let r = self.thumb_local();
        Rect::new(ox + r.x, oy + r.y, r.w, r.h)
    }

    /// Sets the result's thumbnail (true colour, [`GenerateRequester::thumb_size`]).
    pub fn set_thumbnail(&mut self, thumb: Option<RgbaImage>) {
        self.thumb = thumb;
    }

    pub fn thumbnail(&self) -> Option<&RgbaImage> {
        self.thumb.as_ref()
    }

    /// Paints the thumbnail into an RGBA frame of the screen: `frame_w` pixels per row, `sx`×`sy`
    /// frame pixels per UI pixel. Never outside [`Self::thumb_rect`]: a thumbnail made for
    /// a larger box (before More.. or Less..) is cut to it until the app makes it again.
    pub fn overlay(&self, rgba: &mut [u8], frame_w: usize, sx: usize, sy: usize) {
        if let (GenPhase::Result, Some(t)) = (self.phase, &self.thumb) {
            let r = self.thumb_rect();
            let (w, h) = (r.w.max(0) as u32, r.h.max(0) as u32);
            if t.width() > w || t.height() > h {
                let cut = t.crop(0, 0, t.width().min(w), t.height().min(h));
                crate::overlay_rgb(rgba, frame_w, (r.x, r.y), &cut, sx, sy);
            } else {
                crate::overlay_rgb(rgba, frame_w, (r.x, r.y), t, sx, sy);
            }
        }
    }

    fn prompt_rect(&self) -> Rect {
        let (w, _) = self.size();
        Rect::new(LEFT, geom::PROMPT_Y, w - 2 * LEFT, geom::PROMPT_H)
    }

    /// The bottom buttons, left to right, with their labels.
    fn buttons(&self) -> Vec<(GenControl, &'static str, Rect)> {
        let font = Font::builtin();
        let (w, h) = self.size();
        let y = h - BUTTON_H - 5;
        let width = |label: &str| (font.text_width(label) + 14).max(44);
        let mut out = Vec::new();
        let mut x = LEFT;
        for (c, label) in [
            (GenControl::Cancel, "Cancel"),
            (
                GenControl::More,
                if self.more { "Less.." } else { "More.." },
            ),
        ] {
            let bw = width(label);
            out.push((c, label, Rect::new(x, y, bw, BUTTON_H)));
            x += bw + 4;
        }
        let mut right = vec![(GenControl::SetUp, "Set up..")];
        right.push((
            GenControl::Generate,
            if self.phase == GenPhase::Result {
                "Again"
            } else {
                "Generate"
            },
        ));
        if self.phase == GenPhase::Result {
            right.push((GenControl::Apply, "Apply"));
        }
        let mut x = w - LEFT;
        let mut placed = Vec::new();
        for (c, label) in right.into_iter().rev() {
            let bw = width(label);
            x -= bw;
            placed.push((c, label, Rect::new(x, y, bw, BUTTON_H)));
            x -= 4;
        }
        placed.reverse();
        out.extend(placed);
        out
    }

    /// Every control with its rectangle (requester coordinates), in drawing order.
    pub fn controls(&self) -> Vec<(GenControl, Rect)> {
        let mut out = vec![(GenControl::Prompt, self.prompt_rect())];
        let vx = LEFT + Self::label_w();
        for row in [GenRow::Resolution].into_iter().chain(self.rows()) {
            let y = self.row_y(row) - 1;
            let vw = Self::value_w(row);
            out.push((GenControl::Prev(row), Rect::new(vx, y, ARROW, ARROW)));
            out.push((
                GenControl::Next(row),
                Rect::new(vx + ARROW + vw, y, ARROW, ARROW),
            ));
        }
        out.extend(self.buttons().into_iter().map(|(c, _, r)| (c, r)));
        out
    }

    /// The index of `row`'s current choice and the number of choices it may take now.
    fn position(&self, row: GenRow) -> (usize, usize) {
        let o = &self.options;
        match row {
            GenRow::Resolution => (o.resolution, RESOLUTIONS.len()),
            GenRow::Chipset => (index_of(&CHIPSETS, &o.chipset), 2),
            GenRow::Colors => {
                let max = COLOR_COUNTS
                    .iter()
                    .position(|&n| n == o.max_colors())
                    .unwrap_or(4);
                (index_of(&COLOR_COUNTS, &o.colors), max + 1)
            }
            GenRow::Style => (index_of(&STYLES, &o.style), STYLES.len()),
            GenRow::Palette => (index_of(&PALETTES, &o.palette), 2),
            GenRow::Dither => (index_of(&DITHERS, &o.dither), DITHERS.len()),
            GenRow::Fit => (index_of(&FITS, &o.framing), 2),
            GenRow::Scaler => (index_of(&SCALERS, &o.scaler), SCALERS.len()),
            GenRow::Background => (usize::from(o.transparent), 2),
            GenRow::Quality => (index_of(&QUALITIES, &o.quality), QUALITIES.len()),
            GenRow::Target => (index_of(&TARGETS, &o.target), TARGETS.len()),
        }
    }

    /// The text between `row`'s arrows.
    pub fn value(&self, row: GenRow) -> String {
        if row == GenRow::Colors {
            return self.shown_colors().to_string();
        }
        if self.row_fixed(row) {
            let (target, (w, h)) = self.format();
            match row {
                GenRow::Resolution => return format!("This page {w}x{h}"),
                GenRow::Chipset => {
                    let i = index_of(&CHIPSETS, &target.chipset);
                    return row.choices()[i].to_string();
                }
                GenRow::Palette => return "Current".into(),
                _ => {}
            }
        }
        let (i, _) = self.position(row);
        row.choices()[i].to_string()
    }

    fn set_choice(&mut self, row: GenRow, i: usize) {
        let o = &mut self.options;
        match row {
            GenRow::Resolution => o.resolution = i,
            GenRow::Chipset => o.chipset = CHIPSETS[i],
            GenRow::Colors => o.colors = COLOR_COUNTS[i],
            GenRow::Style => o.style = STYLES[i],
            GenRow::Palette => o.palette = PALETTES[i],
            GenRow::Dither => o.dither = DITHERS[i],
            GenRow::Fit => o.framing = FITS[i],
            GenRow::Scaler => o.scaler = SCALERS[i],
            GenRow::Background => o.transparent = i == 1,
            GenRow::Quality => o.quality = QUALITIES[i],
            GenRow::Target => o.target = TARGETS[i],
        }
        // A smaller maximum (OCS, HiRes) lowers the count.
        self.options = self.options.clamped();
    }

    /// Whether a control can be used now.
    pub fn enabled(&self, c: GenControl) -> bool {
        match c {
            GenControl::Prev(row) | GenControl::Next(row) => {
                if (row == GenRow::Colors && !self.colors_enabled()) || self.row_fixed(row) {
                    return false;
                }
                let (i, n) = self.position(row);
                match c {
                    GenControl::Prev(_) => i > 0,
                    _ => i + 1 < n,
                }
            }
            GenControl::Generate => {
                self.provider.ready
                    && !matches!(self.phase, GenPhase::Working { .. })
                    && !self.prompt.text().trim().is_empty()
                    && self.palette_problem().is_none()
            }
            GenControl::SetUp => !matches!(self.phase, GenPhase::Working { .. }),
            GenControl::Apply => self.phase == GenPhase::Result && self.thumb.is_some(),
            GenControl::Prompt | GenControl::More | GenControl::Cancel => true,
        }
    }

    fn control_at(&self, x: i32, y: i32) -> Option<GenControl> {
        self.controls()
            .into_iter()
            .find(|&(c, r)| r.contains(x, y) && self.enabled(c))
            .map(|(c, _)| c)
    }

    fn pressed(&self, c: GenControl) -> bool {
        self.held == Some(c) && self.control_at(self.pointer.0, self.pointer.1) == Some(c)
    }

    fn apply(&mut self, c: GenControl) -> GenerateResponse {
        match c {
            GenControl::Prev(row) | GenControl::Next(row) => {
                if !self.enabled(c) {
                    return GenerateResponse::Continue;
                }
                let (i, _) = self.position(row);
                let j = if matches!(c, GenControl::Prev(_)) {
                    i - 1
                } else {
                    i + 1
                };
                self.set_choice(row, j);
                GenerateResponse::Changed
            }
            GenControl::More => {
                self.more = !self.more;
                // The box changes its size: a result's thumbnail must be made again for it.
                if self.phase == GenPhase::Result {
                    GenerateResponse::Changed
                } else {
                    GenerateResponse::Continue
                }
            }
            GenControl::Prompt => GenerateResponse::Continue,
            GenControl::SetUp => GenerateResponse::SetUp,
            GenControl::Cancel if matches!(self.phase, GenPhase::Working { .. }) => {
                GenerateResponse::CancelJob
            }
            GenControl::Cancel => GenerateResponse::Cancel,
            GenControl::Generate => GenerateResponse::Generate,
            GenControl::Apply => GenerateResponse::Apply,
        }
    }

    /// Handles one event in requester coordinates (origin: [`GenerateRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> GenerateResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.control_at(x, y);
                GenerateResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                GenerateResponse::Continue
            }
            // Buttons act on release over the button they were pressed on.
            ReqEvent::Release => match self.held.take() {
                Some(c) if self.control_at(self.pointer.0, self.pointer.1) == Some(c) => {
                    self.apply(c)
                }
                _ => GenerateResponse::Continue,
            },
            ReqEvent::Scroll(_) => GenerateResponse::Continue,
            ReqEvent::Key(key) => self.key(key),
        }
    }

    fn key(&mut self, key: ReqKey) -> GenerateResponse {
        match key {
            ReqKey::Enter if self.enabled(GenControl::Apply) => GenerateResponse::Apply,
            ReqKey::Enter if self.enabled(GenControl::Generate) => GenerateResponse::Generate,
            ReqKey::Enter => GenerateResponse::Continue,
            ReqKey::Escape => self.apply(GenControl::Cancel),
            ReqKey::Tab | ReqKey::Up | ReqKey::Down => GenerateResponse::Continue,
            k => {
                self.prompt.key(k);
                GenerateResponse::Continue
            }
        }
    }

    /// Draws the requester centred on its UI screen.
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = self.origin();
        img.blit(&self.render(pens), x, y);
    }

    /// Renders the requester alone; the thumbnail is left dark (see [`Self::overlay`]).
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let (w, h) = self.size();
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, TITLE, pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        font.draw(&mut img, LEFT, geom::ASK_Y, ASK, pens.dark);
        draw_field(&mut img, self.prompt_rect(), &self.prompt, true, pens);

        let ghost_text = |img: &mut IndexedImage, x: i32, y: i32, text: &str| {
            font.draw(img, x, y, text, pens.ghost.unwrap_or(pens.dark));
            if pens.ghost.is_none() {
                checker(
                    img,
                    Rect::new(x, y, font.text_width(text), LINE_STEP),
                    pens.face,
                );
            }
        };
        let vx = LEFT + Self::label_w();
        for row in [GenRow::Resolution].into_iter().chain(self.rows()) {
            let y = self.row_y(row);
            let ghosted = (row == GenRow::Colors && !self.colors_enabled()) || self.row_fixed(row);
            let value = self.value(row);
            let vw = Self::value_w(row);
            let tx = vx + ARROW + (vw - font.text_width(&value)) / 2;
            if ghosted {
                ghost_text(&mut img, LEFT, y, row.label());
                ghost_text(&mut img, tx, y, &value);
            } else {
                font.draw(&mut img, LEFT, y, row.label(), pens.dark);
                font.draw(&mut img, tx, y, &value, pens.dark);
            }
        }
        for (c, r) in self.controls() {
            let label = match c {
                GenControl::Prev(_) => "◂",
                GenControl::Next(_) => "▶",
                _ => continue,
            };
            draw_button(&mut img, r, label, self.pressed(c), pens);
            if !self.enabled(c) {
                checker(&mut img, r.inset(2), pens.face);
            }
        }
        for (c, label, r) in self.buttons() {
            draw_button(&mut img, r, label, self.pressed(c), pens);
            if !self.enabled(c) {
                checker(&mut img, r.inset(2), pens.face);
            }
        }

        // The box: the note, the progress or (painted by the app) the thumbnail.
        let b = self.box_rect();
        frame(&mut img, b, pens.dark);
        let inner = b.inset(2);
        let showing_thumb = self.phase == GenPhase::Result && self.thumb.is_some();
        let text_lines: Vec<String> = match self.phase {
            // A palette the mode cannot show comes first: nothing can be generated until the
            // options change.
            GenPhase::Ask => match (self.palette_problem(), &self.error) {
                (Some(p), _) => wrap(&p, inner.w - 2),
                (None, Some(e)) => wrap(e, inner.w - 2),
                (None, None) => wrap(&self.provider.note, inner.w - 2),
            },
            GenPhase::Result => match &self.error {
                Some(e) => wrap(e, inner.w - 2),
                None => Vec::new(),
            },
            GenPhase::Working { seconds } => {
                let mut lines = vec!["Generating..".to_string(), format!("{seconds} s")];
                lines.push(String::new());
                lines.extend(wrap(
                    "Cancel stops waiting. A request already sent may still be billed.",
                    inner.w - 2,
                ));
                lines
            }
        };
        if showing_thumb {
            let t = self.thumb_local();
            img.fill_rect(t.x, t.y, t.w, t.h, pens.dark);
        } else {
            let max = (inner.h / LINE_STEP).max(0) as usize;
            for (i, line) in text_lines.iter().take(max).enumerate() {
                font.draw(
                    &mut img,
                    inner.x + 1,
                    inner.y + 1 + i as i32 * LINE_STEP,
                    line,
                    pens.dark,
                );
            }
        }

        // The two lines under the options: the generator (on both lines while there is no
        // message), then the message. With a result shown in the box, an error (a new
        // generation that failed or was stopped; the result shown stays) takes both lines.
        let ly = self.lines_y();
        let line_w = w - 2 * LEFT;
        let mut lines = wrap(&self.provider.summary, line_w);
        match (&self.error, &self.message) {
            (Some(e), _) if showing_thumb => {
                lines = wrap(e, line_w);
                lines.truncate(2);
            }
            (_, Some(m)) => {
                lines.truncate(1);
                lines.extend(wrap(m, line_w).into_iter().take(1));
            }
            _ => lines.truncate(2),
        }
        for (i, line) in lines.iter().enumerate() {
            font.draw(&mut img, LEFT, ly + i as i32 * LINE_STEP, line, pens.dark);
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{Palette, Rgba8};

    fn ready() -> ProviderInfo {
        ProviderInfo {
            ready: true,
            summary: "OpenAI Images: uses the internet, billed to your account".into(),
            note: "Sends your description over the internet to OpenAI.".into(),
        }
    }

    fn requester(screen: (i32, i32)) -> GenerateRequester {
        GenerateRequester::new(GenerateOptions::default(), "", 32, screen, ready())
    }

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn rect_of(r: &GenerateRequester, c: GenControl) -> Rect {
        r.controls()
            .into_iter()
            .find(|&(k, _)| k == c)
            .unwrap_or_else(|| panic!("{c:?} not shown"))
            .1
    }

    fn click(r: &mut GenerateRequester, c: GenControl) -> GenerateResponse {
        let rect = rect_of(r, c);
        let (x, y) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
        r.handle(ReqEvent::Press(x, y));
        r.handle(ReqEvent::Release)
    }

    fn typed(r: &mut GenerateRequester, text: &str) {
        for c in text.chars() {
            r.handle(ReqEvent::Key(ReqKey::Char(c)));
        }
    }

    #[test]
    fn layout_fits_every_screen_in_every_state() {
        for screen in [(320, 200), (320, 256), (368, 283), (368, 241), (320, 240)] {
            for more in [false, true] {
                for phase in [
                    GenPhase::Ask,
                    GenPhase::Working { seconds: 3 },
                    GenPhase::Result,
                ] {
                    let mut r = requester(screen);
                    r.set_more(more);
                    r.set_phase(phase);
                    let (w, h) = r.size();
                    let (ox, oy) = r.origin();
                    assert!(ox >= 0 && oy >= 0 && ox + w <= screen.0 && oy + h <= screen.1);
                    let bounds = Rect::new(1, 1, w - 2, h - 2);
                    let controls = r.controls();
                    let b = r.box_rect();
                    for (i, &(c, rect)) in controls.iter().enumerate() {
                        assert_eq!(rect.intersect(bounds), rect, "{c:?} inside {screen:?}");
                        assert!(rect.intersect(b).is_empty(), "{c:?} under the box");
                        for &(d, other) in &controls[i + 1..] {
                            assert!(rect.intersect(other).is_empty(), "{c:?} overlaps {d:?}");
                        }
                    }
                    // The rows end above the two lines, the box too; without "More.." the
                    // box lies below the rows.
                    let last = r.rows().into_iter().map(|row| r.row_y(row)).max().unwrap();
                    assert!(last + LINE_STEP <= r.lines_y(), "{screen:?} more={more}");
                    assert!(b.y + b.h <= r.lines_y());
                    if !more {
                        assert!(last + LINE_STEP <= b.y, "{screen:?}");
                    }
                    let (tw, th) = r.thumb_size();
                    assert!(tw >= 80 && th >= 60, "thumbnail {tw}x{th} on {screen:?}");
                    assert_eq!(
                        controls.iter().any(|&(c, _)| c == GenControl::Apply),
                        phase == GenPhase::Result
                    );
                }
            }
        }
    }

    #[test]
    fn defaults_are_the_specs_default_view() {
        let r = requester((320, 256));
        let o = r.options();
        assert_eq!(r.value(GenRow::Resolution), "Pixel art 320x240 (custom)");
        assert_eq!(o.target().screen, (320, 240));
        assert_eq!(o.target().mode, DisplayMode::PAL_LORES);
        assert_eq!(r.value(GenRow::Chipset), "OCS");
        assert_eq!(r.value(GenRow::Colors), "32");
        assert_eq!(r.value(GenRow::Style), "Pixel art");
        assert_eq!(r.value(GenRow::Target), "Spare page");
        assert!(!r.more());
        assert_eq!(r.rows(), BASIC.to_vec());
        // Every preset is a valid screen and page of its mode on both chipsets.
        for p in RESOLUTIONS {
            for chipset in CHIPSETS {
                let t = p.target(chipset);
                assert!(t.mode.screen_fits(t.screen.0, t.screen.1), "{}", p.label);
                assert!(t.page_fits(t.screen.0, t.screen.1), "{}", p.label);
            }
        }
    }

    #[test]
    fn rows_step_and_colours_follow_the_mode() {
        let mut r = requester((320, 256));
        assert!(
            !r.enabled(GenControl::Next(GenRow::Colors)),
            "32 is OCS LoRes' maximum"
        );
        assert_eq!(
            click(&mut r, GenControl::Next(GenRow::Chipset)),
            GenerateResponse::Changed
        );
        assert_eq!(r.options().chipset, Chipset::Aga);
        for _ in 0..3 {
            click(&mut r, GenControl::Next(GenRow::Colors));
        }
        assert_eq!(r.options().colors, 256);
        // Back to OCS: the count drops to 32; HiRes then to 16.
        click(&mut r, GenControl::Prev(GenRow::Chipset));
        assert_eq!(r.options().colors, 32);
        click(&mut r, GenControl::Next(GenRow::Resolution));
        assert_eq!(r.value(GenRow::Resolution), "NTSC HiRes 640x200");
        assert_eq!(r.options().colors, 16);
        assert_eq!(r.options().max_colors(), 16);
        // The ends of a list ghost their arrow.
        for _ in 0..10 {
            click(&mut r, GenControl::Next(GenRow::Resolution));
        }
        assert_eq!(r.value(GenRow::Resolution), "PAL HiRes Laced 640x512");
        assert!(!r.enabled(GenControl::Next(GenRow::Resolution)));
        assert!(!r.enabled(GenControl::Prev(GenRow::Chipset)));
        // Style cycles.
        click(&mut r, GenControl::Next(GenRow::Style));
        assert_eq!(r.options().style, GenStyle::Painted);
    }

    #[test]
    fn more_shows_the_other_rows_and_every_row_changes_its_option() {
        let mut r = requester((320, 256));
        assert!(
            r.controls()
                .iter()
                .all(|&(c, _)| c != GenControl::Next(GenRow::Target))
        );
        assert_eq!(click(&mut r, GenControl::More), GenerateResponse::Continue);
        assert!(r.more());
        let before = r.options();
        click(&mut r, GenControl::Next(GenRow::Palette));
        click(&mut r, GenControl::Next(GenRow::Dither));
        click(&mut r, GenControl::Next(GenRow::Fit));
        click(&mut r, GenControl::Prev(GenRow::Scaler));
        click(&mut r, GenControl::Next(GenRow::Background));
        click(&mut r, GenControl::Next(GenRow::Quality));
        click(&mut r, GenControl::Next(GenRow::Target));
        let o = r.options();
        assert_ne!(o, before);
        assert_eq!(o.palette, PaletteChoice::Current);
        assert_eq!(o.dither, Dither::FloydSteinberg);
        assert_eq!(o.framing, Framing::Fit);
        assert_eq!(o.scaler, Scaler::Nearest);
        assert!(o.transparent);
        assert_eq!(o.quality, GenQuality::High);
        assert_eq!(o.target, GenTarget::Picture);
        // Current palette: the count shows the document's and is ghosted.
        r.set_doc_colors(12);
        assert_eq!(r.value(GenRow::Colors), "12");
        assert!(!r.enabled(GenControl::Prev(GenRow::Colors)));
        click(&mut r, GenControl::Next(GenRow::Target));
        assert_eq!(r.options().target, GenTarget::Brush);
        assert_eq!(click(&mut r, GenControl::More), GenerateResponse::Continue);
        assert_eq!(r.rows(), BASIC.to_vec());
        let mut expected = o;
        expected.target = GenTarget::Brush;
        assert_eq!(r.options(), expected);
    }

    #[test]
    fn generate_needs_a_ready_generator_and_a_description() {
        let mut r = requester((320, 256));
        assert!(!r.enabled(GenControl::Generate), "empty description");
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            GenerateResponse::Continue
        );
        typed(&mut r, "A castle at dusk");
        assert_eq!(r.prompt(), "A castle at dusk");
        assert_eq!(
            click(&mut r, GenControl::Generate),
            GenerateResponse::Generate
        );
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            GenerateResponse::Generate
        );
        r.set_provider(ProviderInfo::default());
        assert!(!r.enabled(GenControl::Generate), "no generator");
        assert!(r.enabled(GenControl::SetUp));
        assert_eq!(click(&mut r, GenControl::SetUp), GenerateResponse::SetUp);
        assert_eq!(click(&mut r, GenControl::Cancel), GenerateResponse::Cancel);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            GenerateResponse::Cancel
        );
    }

    #[test]
    fn working_cancels_the_job_and_a_result_can_be_applied() {
        let mut r = requester((320, 256));
        typed(&mut r, "x");
        r.set_phase(GenPhase::Working { seconds: 2 });
        assert!(!r.enabled(GenControl::Generate));
        assert!(!r.enabled(GenControl::SetUp));
        assert_eq!(
            click(&mut r, GenControl::Cancel),
            GenerateResponse::CancelJob
        );
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Escape)),
            GenerateResponse::CancelJob
        );
        r.set_phase(GenPhase::Result);
        assert!(!r.enabled(GenControl::Apply), "no thumbnail yet");
        let (tw, th) = r.thumb_size();
        r.set_thumbnail(Some(RgbaImage::new(tw, th, Rgba8::opaque(200, 10, 10))));
        assert_eq!(click(&mut r, GenControl::Apply), GenerateResponse::Apply);
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            GenerateResponse::Apply
        );
        assert_eq!(
            click(&mut r, GenControl::Generate),
            GenerateResponse::Generate
        );
        assert_eq!(click(&mut r, GenControl::Cancel), GenerateResponse::Cancel);
        // Leaving the result drops the thumbnail.
        r.set_phase(GenPhase::Ask);
        assert!(r.thumbnail().is_none());
    }

    #[test]
    fn typing_and_pasting_edit_the_description_only() {
        let mut r = requester((320, 256));
        typed(&mut r, "Burg");
        for k in crate::requester::paste_keys(" im Abendlicht\nmit Türmen") {
            assert_eq!(r.handle(ReqEvent::Key(k)), GenerateResponse::Continue);
        }
        assert_eq!(r.prompt(), "Burg im Abendlicht mit Türmen", "umlauts stay");
        r.handle(ReqEvent::Key(ReqKey::Backspace));
        r.handle(ReqEvent::Key(ReqKey::Home));
        r.handle(ReqEvent::Key(ReqKey::Delete));
        assert_eq!(r.prompt(), "urg im Abendlicht mit Türme");
        // Letters are never shortcuts here.
        let before = r.options();
        typed(&mut r, "mcap");
        assert_eq!(r.options(), before);
        assert!(!r.more());
    }

    #[test]
    fn drawing_shows_the_note_the_progress_and_a_dark_thumbnail_box() {
        let pens = pens();
        let mut r = requester((320, 256));
        let img = r.render(&pens);
        assert_eq!(img.get(0, 0), Some(pens.dark), "square corner");
        // The note is drawn inside the box; with a result the thumbnail area is dark.
        let b = r.box_rect().inset(2);
        let inked = |img: &IndexedImage| {
            (b.y..b.y + b.h)
                .flat_map(|y| (b.x..b.x + b.w).map(move |x| (x, y)))
                .filter(|&(x, y)| img.get(x, y) == Some(pens.dark))
                .count()
        };
        assert!(inked(&img) > 50, "note text");
        r.set_phase(GenPhase::Working { seconds: 7 });
        let working = r.render(&pens);
        assert_ne!(working.pixels(), img.pixels());
        r.set_phase(GenPhase::Result);
        let (tw, th) = r.thumb_size();
        r.set_thumbnail(Some(RgbaImage::new(tw, th, Rgba8::opaque(1, 2, 3))));
        let result = r.render(&pens);
        let t = r.thumb_local();
        for (x, y) in [(t.x, t.y), (t.x + t.w - 1, t.y + t.h - 1)] {
            assert_eq!(result.get(x, y), Some(pens.dark));
        }
        // Overlay writes exactly the thumbnail rectangle.
        let (sw, sh) = (320usize, 256usize);
        let mut rgba = vec![0u8; sw * sh * 4];
        r.overlay(&mut rgba, sw, 1, 1);
        let tr = r.thumb_rect();
        let at = |x: i32, y: i32| {
            let o = (y as usize * sw + x as usize) * 4;
            [rgba[o], rgba[o + 1], rgba[o + 2]]
        };
        assert_eq!(at(tr.x, tr.y), [1, 2, 3]);
        assert_eq!(at(tr.x + tr.w - 1, tr.y + tr.h - 1), [1, 2, 3]);
        assert_eq!(at(tr.x - 1, tr.y), [0, 0, 0]);
        // No overlay outside the result phase.
        r.set_phase(GenPhase::Ask);
        let mut none = vec![0u8; sw * sh * 4];
        r.overlay(&mut none, sw, 1, 1);
        assert!(none.iter().all(|&b| b == 0));
    }

    #[test]
    fn an_error_takes_the_place_of_the_note() {
        let pens = pens();
        let mut r = requester((320, 256));
        let note = r.render(&pens);
        r.set_error(Some("The API key was refused (check or replace it)".into()));
        assert_eq!(
            r.error(),
            Some("The API key was refused (check or replace it)")
        );
        let error = r.render(&pens);
        assert_ne!(note.pixels(), error.pixels());
        // A result that could not be converted shows the error instead of a thumbnail.
        r.set_phase(GenPhase::Result);
        assert!(!r.enabled(GenControl::Apply));
        let failed = r.render(&pens);
        let t = r.thumb_local();
        let dark = (t.y..t.y + t.h)
            .flat_map(|y| (t.x..t.x + t.w).map(move |x| (x, y)))
            .filter(|&(x, y)| failed.get(x, y) == Some(pens.dark))
            .count();
        assert!(dark < (t.w * t.h / 2) as usize, "text, not a dark box");
    }

    #[test]
    fn more_after_a_result_asks_for_a_new_thumbnail_and_never_paints_outside_its_box() {
        let mut r = requester((320, 256));
        typed(&mut r, "x");
        r.set_phase(GenPhase::Result);
        let (tw, th) = r.thumb_size();
        r.set_thumbnail(Some(RgbaImage::new(tw, th, Rgba8::opaque(9, 8, 7))));
        // More.. makes the box smaller: the app must make the thumbnail again.
        assert_eq!(click(&mut r, GenControl::More), GenerateResponse::Changed);
        let small = r.thumb_size();
        assert!(small.0 < tw && small.1 < th, "{small:?} < {tw}x{th}");
        // Until it does, the old (larger) thumbnail stays inside the new box.
        let (ox, _) = r.origin();
        let (w, _) = r.size();
        let (sw, sh) = (360usize, 283usize);
        let mut rgba = vec![0u8; sw * sh * 4];
        r.overlay(&mut rgba, sw, 1, 1);
        let tr = r.thumb_rect();
        for y in 0..sh as i32 {
            for x in 0..sw as i32 {
                let o = (y as usize * sw + x as usize) * 4;
                let painted = rgba[o..o + 3] == [9, 8, 7];
                assert_eq!(painted, tr.contains(x, y), "({x}, {y})");
            }
        }
        assert!(tr.x + tr.w <= ox + w, "inside the requester");
        // Less.. likewise; without a result More.. changes nothing to convert.
        assert_eq!(click(&mut r, GenControl::More), GenerateResponse::Changed);
        r.set_phase(GenPhase::Ask);
        assert_eq!(click(&mut r, GenControl::More), GenerateResponse::Continue);
    }

    #[test]
    fn a_current_palette_the_mode_cannot_show_blocks_generate_and_says_why() {
        let pens = pens();
        let mut r = requester((320, 256));
        typed(&mut r, "A castle");
        let note = r.render(&pens);
        click(&mut r, GenControl::More);
        click(&mut r, GenControl::Next(GenRow::Palette));
        assert_eq!(r.options().palette, PaletteChoice::Current);
        // 32 colours fit OCS PAL LoRes; NTSC HiRes on OCS shows 16 at most.
        assert!(r.palette_problem().is_none());
        assert!(r.enabled(GenControl::Generate));
        click(&mut r, GenControl::Next(GenRow::Resolution));
        assert_eq!(r.value(GenRow::Resolution), "NTSC HiRes 640x200");
        assert_eq!(
            r.palette_problem().as_deref(),
            Some(
                "The current palette has 32 colours; this mode shows 16 on OCS. Choose \
                 Palette New, AGA or another resolution."
            )
        );
        assert!(!r.enabled(GenControl::Generate));
        assert_eq!(
            r.handle(ReqEvent::Key(ReqKey::Enter)),
            GenerateResponse::Continue,
            "Return does not generate either"
        );
        // The reason shows in the box, before any older error.
        r.set_error(Some("No picture: it failed.".into()));
        let blocked = r.render(&pens);
        r.set_error(None);
        assert_eq!(r.render(&pens).pixels(), blocked.pixels());
        assert_ne!(blocked.pixels(), note.pixels());
        // AGA shows 256: fine again; so is a new palette.
        click(&mut r, GenControl::Next(GenRow::Chipset));
        assert!(r.palette_problem().is_none() && r.enabled(GenControl::Generate));
        click(&mut r, GenControl::Prev(GenRow::Chipset));
        click(&mut r, GenControl::Prev(GenRow::Palette));
        assert!(r.palette_problem().is_none() && r.enabled(GenControl::Generate));
        // The longest reason fits the box on the smallest screen.
        let mut small = GenerateRequester::new(
            GenerateOptions {
                palette: PaletteChoice::Current,
                resolution: 5,
                ..GenerateOptions::default()
            },
            "x",
            256,
            (320, 200),
            ready(),
        );
        small.set_more(true);
        let b = small.box_rect().inset(2);
        let lines = wrap(&small.palette_problem().unwrap(), b.w - 2);
        let n = lines.len() as i32;
        assert!(n * LINE_STEP <= b.h, "{n} lines in {}: {lines:?}", b.h);
    }

    #[test]
    fn an_error_beside_a_kept_result_takes_the_two_lines_and_apply_stays() {
        let pens = pens();
        let mut r = requester((320, 256));
        typed(&mut r, "x");
        r.set_phase(GenPhase::Result);
        let (tw, th) = r.thumb_size();
        r.set_thumbnail(Some(RgbaImage::new(tw, th, Rgba8::opaque(1, 2, 3))));
        r.set_message(Some("Apply: onto the spare page (j shows it)".into()));
        let plain = r.render(&pens);
        r.set_error(Some(
            "No new picture: The service did not answer in time.".into(),
        ));
        let failed = r.render(&pens);
        assert!(
            r.enabled(GenControl::Apply),
            "the result shown can be applied"
        );
        // The box still shows the thumbnail (dark here); the lines differ.
        let t = r.thumb_local();
        assert_eq!(failed.crop(t), plain.crop(t));
        let ly = r.lines_y();
        let (w, _) = r.size();
        let lines = Rect::new(LEFT, ly, w - 2 * LEFT, 2 * LINE_STEP);
        assert_ne!(failed.crop(lines), plain.crop(lines));
    }

    #[test]
    fn the_thumbnail_follows_the_resolutions_display_proportions() {
        let mut r = requester((320, 256));
        let (w, h) = r.thumb_size();
        assert!((w as f64 / h as f64 - 4.0 / 3.0).abs() < 0.05, "{w}x{h}");
        // PAL HiRes 640x256 shows as 640x512 (HiRes pixels are twice as tall as wide).
        let mut o = r.options();
        o.resolution = 4;
        r.set_options(o);
        let (w, h) = r.thumb_size();
        assert!((w as f64 / h as f64 - 1.25).abs() < 0.05, "{w}x{h}");
    }

    #[test]
    fn a_new_layer_takes_the_pictures_page_and_palette_and_says_so() {
        let p = pens();
        let mut r = requester((320, 256));
        r.set_doc_colors(16);
        // An AGA HiRes picture of 640x400 pixels.
        let doc = Target::new(
            Chipset::Aga,
            DisplayMode::new(Video::Ntsc, Resolution::Hi, true),
        );
        r.set_document(doc, (640, 400));
        click(&mut r, GenControl::More);
        let chosen = r.options();
        let plain = r.render(&p);
        while r.options().target != GenTarget::NewLayer {
            click(&mut r, GenControl::Next(GenRow::Target));
        }
        assert_eq!(r.value(GenRow::Target), "New layer");
        assert_eq!(r.format(), (doc, (640, 400)));
        assert_eq!(r.palette_choice(), PaletteChoice::Current);
        assert_eq!(r.value(GenRow::Resolution), "This page 640x400");
        assert_eq!(r.value(GenRow::Chipset), "AGA");
        assert_eq!(r.value(GenRow::Colors), "16");
        assert_eq!(r.value(GenRow::Palette), "Current");
        for row in [
            GenRow::Resolution,
            GenRow::Chipset,
            GenRow::Colors,
            GenRow::Palette,
        ] {
            assert!(r.row_fixed(row));
            for c in [GenControl::Prev(row), GenControl::Next(row)] {
                assert!(!r.enabled(c), "{c:?} is ghosted");
                assert_eq!(click(&mut r, c), GenerateResponse::Continue);
            }
        }
        // The other rows still change; the chosen resolution and chipset stay for later.
        assert!(r.enabled(GenControl::Next(GenRow::Dither)));
        assert_eq!(r.options().resolution, chosen.resolution);
        assert_eq!(r.options().chipset, chosen.chipset);
        assert_ne!(r.render(&p), plain);
        // The thumbnail has the page's display proportions (HiRes laced: square pixels).
        let (w, h) = r.thumb_size();
        assert!((w as f64 / h as f64 - 1.6).abs() < 0.05, "{w}x{h}");
        // A palette the picture's own mode cannot show blocks Generate.
        r.set_doc_colors(32);
        r.set_document(
            Target::new(
                Chipset::Ocs,
                DisplayMode::new(Video::Pal, Resolution::Hi, false),
            ),
            (640, 256),
        );
        r.set_prompt("a castle");
        assert!(r.palette_problem().unwrap().contains("new layer cannot"));
        assert!(!r.enabled(GenControl::Generate));
        // Back to the spare page: the chosen rows are back and change again.
        click(&mut r, GenControl::Prev(GenRow::Target));
        assert!(!r.row_fixed(GenRow::Chipset));
        assert_eq!(r.format().0, chosen.target());
        assert!(r.enabled(GenControl::Next(GenRow::Chipset)));
        assert!(r.enabled(GenControl::Generate));
    }
}
