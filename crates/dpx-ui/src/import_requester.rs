//! Picture ▸ Load of a PNG or JPEG picture (ADR-42): the Import Picture requester.
//!
//! It names the file, its size and colours and the size the picture gets on the page, and
//! asks for the Position (Centre / Top left), the Palette (New / Current), the number of
//! Colours (2–256 up to the screen's maximum; ghosted with Current) and the Dither (None /
//! Floyd-Steinberg / Ordered). A thumbnail shows the result. The screen is indexed with one
//! palette, so the thumbnail box is left dark here and the app paints the true-colour
//! thumbnail into the RGBA frame afterwards ([`overlay_rgb`], [`ImportRequester::overlay`]).
//!
//! Drawn in UI (LoRes) pixels with palette-derived pens, square corners, like the other
//! requesters: Return = Ok, Esc = Cancel. Tab moves the keyboard focus from option to option
//! (the focused label is highlighted like a menu item) and the arrow keys change the focused
//! option. The layout follows the screen size: the thumbnail takes the room right of the
//! options, so the requester fits a 320×200 screen and grows on bigger ones.

use dpx_core::import::{COLOR_COUNTS, ImportOptions, PaletteChoice, Position};
use dpx_core::quantize::Dither;
use dpx_core::{IndexedImage, Rect, RgbaImage, resample};

use crate::font::Font;
use crate::form::draw_box;
use crate::pens::Pens;
use crate::requester::{ReqEvent, ReqKey};
use crate::widgets::{Inset, centred, checker, draw_button, frame};

/// Largest requester (UI pixels); smaller screens get a smaller one.
const MAX_W: i32 = 360;
const MAX_H: i32 = 232;
/// Smallest requester: the options and buttons need this much.
const MIN_W: i32 = 288;
const MIN_H: i32 = 150;
const TITLE: &str = "Import Picture";
const LEFT: i32 = 6;
const BOX: i32 = 7;
const ROW_H: i32 = 9;
const BUTTON_W: i32 = 56;
const BUTTON_H: i32 = 13;
const ARROW_W: i32 = 11;
/// Room for the colour count between the arrows.
const COUNT_W: i32 = 28;

mod geom {
    pub const INFO_Y: i32 = 15;
    pub const RESULT_Y: i32 = 25;
    pub const RULE_Y: i32 = 35;
    pub const POSITION_Y: i32 = 39;
    pub const PALETTE_Y: i32 = 61;
    pub const COLORS_Y: i32 = 83;
    pub const DITHER_Y: i32 = 99;
    pub const STEP: i32 = 10;
}

const POSITIONS: [(Position, &str); 2] = [
    (Position::Centre, "Centre"),
    (Position::TopLeft, "Top left"),
];
const PALETTES: [(PaletteChoice, &str); 2] = [
    (PaletteChoice::New, "New"),
    (PaletteChoice::Current, "Current"),
];
const DITHERS: [(Dither, &str); 3] = [
    (Dither::None, "None"),
    (Dither::FloydSteinberg, "Floyd-Steinberg"),
    (Dither::Ordered, "Ordered"),
];
const LABELS: [&str; 4] = ["Position:", "Palette:", "Colours:", "Dither:"];

/// What the requester says about the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportInfo {
    /// The file name (without the drawer).
    pub name: String,
    /// The picture's size.
    pub size: (u32, u32),
    /// Its number of different colours; `None` above 256 ("true colour").
    pub colors: Option<usize>,
    /// Its size on the page after fitting.
    pub fitted: (u32, u32),
    /// The page size.
    pub page: (u32, u32),
}

impl ImportInfo {
    /// "photo.jpg  4032x3024  true colour", without the name.
    fn details(&self) -> String {
        let colours = match self.colors {
            None => "true colour".to_string(),
            Some(1) => "1 colour".to_string(),
            Some(n) => format!("{n} colours"),
        };
        format!("  {}x{}  {colours}", self.size.0, self.size.1)
    }

    /// "-> 320x240 on 320x256".
    fn result(&self) -> String {
        format!(
            "-> {}x{} on {}x{}",
            self.fitted.0, self.fitted.1, self.page.0, self.page.1
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportResponse {
    Continue,
    /// An option changed: the thumbnail needs to be made again.
    Changed,
    Cancel,
    Ok,
}

/// The requester's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportControl {
    Position(Position),
    Palette(PaletteChoice),
    /// The colour count's ◂ and ▶ buttons.
    Fewer,
    More,
    Dither(Dither),
    Cancel,
    Ok,
}

/// The option rows Tab moves between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRow {
    Position,
    Palette,
    Colors,
    Dither,
}

const ROWS: [ImportRow; 4] = [
    ImportRow::Position,
    ImportRow::Palette,
    ImportRow::Colors,
    ImportRow::Dither,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequester {
    info: ImportInfo,
    options: ImportOptions,
    /// The screen's most colours (OCS LoRes 32, HiRes 16, AGA 256).
    max_colors: u16,
    /// The current palette's size, shown (ghosted) with Current.
    current_colors: u16,
    /// The UI screen (LoRes pixels) the requester is centred on.
    screen: (i32, i32),
    /// The page's pixel shape, for the thumbnail's proportions.
    pixel_shape: (u32, u32),
    focus: Option<ImportRow>,
    held: Option<ImportControl>,
    pointer: (i32, i32),
    thumb: Option<RgbaImage>,
}

impl ImportRequester {
    /// `options.colors` is limited to `max_colors` (whole bitplanes). `screen` is the UI
    /// screen it is shown on; `pixel_shape` the page's.
    pub fn new(
        info: ImportInfo,
        options: ImportOptions,
        max_colors: u16,
        current_colors: u16,
        screen: (i32, i32),
        pixel_shape: (u32, u32),
    ) -> Self {
        let max_colors = dpx_core::import::color_count(max_colors, 256);
        let options = ImportOptions {
            colors: dpx_core::import::color_count(options.colors, max_colors),
            ..options
        };
        Self {
            info,
            options,
            max_colors,
            current_colors,
            screen,
            pixel_shape,
            focus: None,
            held: None,
            pointer: (0, 0),
            thumb: None,
        }
    }

    pub fn options(&self) -> ImportOptions {
        self.options
    }

    pub fn info(&self) -> &ImportInfo {
        &self.info
    }

    pub fn focus(&self) -> Option<ImportRow> {
        self.focus
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
        self.options.palette == PaletteChoice::New
    }

    /// The colour count shown: the chosen one, or the current palette's size.
    pub fn shown_colors(&self) -> u16 {
        if self.colors_enabled() {
            self.options.colors
        } else {
            self.current_colors
        }
    }

    fn radio_x() -> i32 {
        let font = Font::builtin();
        LEFT + LABELS.iter().map(|l| font.text_width(l)).max().unwrap_or(0) + 6
    }

    /// The focus bar behind a row label at `y`.
    fn label_bar(label: &str, y: i32) -> Rect {
        Rect::new(
            LEFT - 1,
            y - 1,
            Font::builtin().text_width(label) + 2,
            ROW_H,
        )
    }

    fn radio_rect(at: (i32, i32), label: &str) -> Rect {
        Rect::new(
            at.0,
            at.1,
            BOX + 4 + Font::builtin().text_width(label),
            ROW_H,
        )
    }

    /// Every control with its rectangle (requester coordinates), in drawing order.
    pub fn controls(&self) -> Vec<(ImportControl, Rect)> {
        let x = Self::radio_x();
        let mut out = Vec::new();
        for (i, (p, label)) in POSITIONS.iter().enumerate() {
            let at = (x, geom::POSITION_Y + i as i32 * geom::STEP);
            out.push((ImportControl::Position(*p), Self::radio_rect(at, label)));
        }
        for (i, (p, label)) in PALETTES.iter().enumerate() {
            let at = (x, geom::PALETTE_Y + i as i32 * geom::STEP);
            out.push((ImportControl::Palette(*p), Self::radio_rect(at, label)));
        }
        let y = geom::COLORS_Y - 2;
        out.push((ImportControl::Fewer, Rect::new(x, y, ARROW_W, ARROW_W)));
        let more_x = x + ARROW_W + COUNT_W;
        out.push((ImportControl::More, Rect::new(more_x, y, ARROW_W, ARROW_W)));
        for (i, (d, label)) in DITHERS.iter().enumerate() {
            let at = (x, geom::DITHER_Y + i as i32 * geom::STEP);
            out.push((ImportControl::Dither(*d), Self::radio_rect(at, label)));
        }
        let (w, h) = self.size();
        let by = h - BUTTON_H - 5;
        out.push((
            ImportControl::Cancel,
            Rect::new(LEFT, by, BUTTON_W, BUTTON_H),
        ));
        out.push((
            ImportControl::Ok,
            Rect::new(w - BUTTON_W - 6, by, BUTTON_W, BUTTON_H),
        ));
        out
    }

    /// Whether a control can be used now.
    pub fn enabled(&self, c: ImportControl) -> bool {
        match c {
            ImportControl::Fewer => self.colors_enabled() && self.options.colors > COLOR_COUNTS[0],
            ImportControl::More => self.colors_enabled() && self.options.colors < self.max_colors,
            _ => true,
        }
    }

    /// The room for the thumbnail (requester coordinates): right of the options, above the
    /// buttons.
    fn thumb_area(&self) -> Rect {
        let font = Font::builtin();
        let widest = DITHERS
            .iter()
            .map(|(_, l)| font.text_width(l))
            .max()
            .unwrap_or(0);
        let x0 = Self::radio_x() + BOX + 4 + widest + 10;
        let (w, h) = self.size();
        let y0 = geom::POSITION_Y - 1;
        let y1 = h - BUTTON_H - 9;
        Rect::new(x0, y0, (w - 6 - x0).max(4), (y1 - y0).max(4))
    }

    /// The thumbnail's size in UI pixels: the page's shape fitted into the room (inside a
    /// one-pixel frame and a one-pixel gap, so a dark page edge stays visible).
    pub fn thumb_size(&self) -> (u32, u32) {
        let area = self.thumb_area().inset(2);
        let (fx, fy) = self.pixel_shape;
        resample::fit_size(
            self.info.page.0 * fx.max(1),
            self.info.page.1 * fy.max(1),
            area.w.max(1) as u32,
            area.h.max(1) as u32,
            (1, 1),
        )
    }

    /// The thumbnail's rectangle in requester coordinates (centred in its room).
    fn thumb_local(&self) -> Rect {
        let area = self.thumb_area();
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

    /// Sets the thumbnail (true colour, [`ImportRequester::thumb_size`]).
    pub fn set_thumbnail(&mut self, thumb: RgbaImage) {
        self.thumb = Some(thumb);
    }

    pub fn thumbnail(&self) -> Option<&RgbaImage> {
        self.thumb.as_ref()
    }

    /// Paints the thumbnail into an RGBA frame of the screen: `frame_w` pixels per row, `sx`×`sy`
    /// frame pixels per UI pixel.
    pub fn overlay(&self, rgba: &mut [u8], frame_w: usize, sx: usize, sy: usize) {
        if let Some(t) = &self.thumb {
            let r = self.thumb_rect();
            overlay_rgb(rgba, frame_w, (r.x, r.y), t, sx, sy);
        }
    }

    fn control_at(&self, x: i32, y: i32) -> Option<ImportControl> {
        self.controls()
            .into_iter()
            .find(|&(c, r)| r.contains(x, y) && self.enabled(c))
            .map(|(c, _)| c)
    }

    fn pressed(&self, c: ImportControl) -> bool {
        self.held == Some(c) && self.control_at(self.pointer.0, self.pointer.1) == Some(c)
    }

    /// Applies a control; `Changed` if an option changed.
    fn apply(&mut self, c: ImportControl) -> ImportResponse {
        let before = self.options;
        match c {
            ImportControl::Position(p) => self.options.position = p,
            ImportControl::Palette(p) => {
                self.options.palette = p;
                // A ghosted row cannot keep the focus: it goes to the palette row.
                if self.focus == Some(ImportRow::Colors) && !self.colors_enabled() {
                    self.focus = Some(ImportRow::Palette);
                }
            }
            ImportControl::Dither(d) => self.options.dither = d,
            ImportControl::Fewer | ImportControl::More if self.enabled(c) => {
                let i = COLOR_COUNTS
                    .iter()
                    .position(|&n| n == self.options.colors)
                    .unwrap_or(0);
                let j = if c == ImportControl::More {
                    i + 1
                } else {
                    i.saturating_sub(1)
                };
                self.options.colors =
                    COLOR_COUNTS[j.min(COLOR_COUNTS.len() - 1)].min(self.max_colors);
            }
            ImportControl::Fewer | ImportControl::More => {}
            ImportControl::Cancel => return ImportResponse::Cancel,
            ImportControl::Ok => return ImportResponse::Ok,
        }
        if self.options != before {
            ImportResponse::Changed
        } else {
            ImportResponse::Continue
        }
    }

    /// Handles one event in requester coordinates (origin: [`ImportRequester::origin`]).
    pub fn handle(&mut self, event: ReqEvent) -> ImportResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.held = self.control_at(x, y);
                // Radios act on press, buttons on release.
                match self.held {
                    Some(
                        c @ (ImportControl::Position(_)
                        | ImportControl::Palette(_)
                        | ImportControl::Dither(_)),
                    ) => {
                        self.held = None;
                        self.apply(c)
                    }
                    _ => ImportResponse::Continue,
                }
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                ImportResponse::Continue
            }
            ReqEvent::Release => match self.held.take() {
                Some(c) if self.control_at(self.pointer.0, self.pointer.1) == Some(c) => {
                    self.apply(c)
                }
                _ => ImportResponse::Continue,
            },
            ReqEvent::Scroll(_) => ImportResponse::Continue,
            ReqEvent::Key(key) => self.key(key),
        }
    }

    fn key(&mut self, key: ReqKey) -> ImportResponse {
        match key {
            ReqKey::Enter => ImportResponse::Ok,
            ReqKey::Escape => ImportResponse::Cancel,
            ReqKey::Tab => {
                // The colour count is skipped while it is ghosted.
                let mut i = self
                    .focus
                    .and_then(|f| ROWS.iter().position(|&r| r == f))
                    .map_or(0, |i| (i + 1) % ROWS.len());
                if ROWS[i] == ImportRow::Colors && !self.colors_enabled() {
                    i = (i + 1) % ROWS.len();
                }
                self.focus = Some(ROWS[i]);
                ImportResponse::Continue
            }
            ReqKey::Up | ReqKey::Left => self.step(-1),
            ReqKey::Down | ReqKey::Right => self.step(1),
            _ => ImportResponse::Continue,
        }
    }

    /// The previous (`-1`) or next (`1`) choice of the focused option.
    fn step(&mut self, by: i32) -> ImportResponse {
        fn next<T: Copy + PartialEq>(list: &[(T, &str)], cur: T, by: i32) -> T {
            let i = list.iter().position(|(v, _)| *v == cur).unwrap_or(0) as i32;
            list[(i + by).clamp(0, list.len() as i32 - 1) as usize].0
        }
        let c = match self.focus {
            None => return ImportResponse::Continue,
            Some(ImportRow::Position) => {
                ImportControl::Position(next(&POSITIONS, self.options.position, by))
            }
            Some(ImportRow::Palette) => {
                ImportControl::Palette(next(&PALETTES, self.options.palette, by))
            }
            Some(ImportRow::Colors) if by < 0 => ImportControl::Fewer,
            Some(ImportRow::Colors) => ImportControl::More,
            Some(ImportRow::Dither) => {
                ImportControl::Dither(next(&DITHERS, self.options.dither, by))
            }
        };
        self.apply(c)
    }

    /// Draws the requester centred on its UI screen.
    pub fn draw(&self, img: &mut IndexedImage, pens: &Pens) {
        let (x, y) = self.origin();
        img.blit(&self.render(pens), x, y);
    }

    /// Renders the requester alone; the thumbnail box is dark (see [`overlay_rgb`]).
    pub fn render(&self, pens: &Pens) -> IndexedImage {
        let font = Font::builtin();
        let (w, h) = self.size();
        let mut img = IndexedImage::new(w as u32, h as u32, pens.face);
        frame(&mut img, Rect::new(0, 0, w, h), pens.dark);
        centred(&mut img, 0, w, 2, TITLE, pens.dark);
        img.hline(1, w - 2, 12, pens.dark);
        // File line: the name is cut to leave room for size and colours.
        let details = self.info.details();
        let room = w - 2 * LEFT - font.text_width(&details);
        let mut x = LEFT;
        for c in self.info.name.chars() {
            if x + font.char_advance(c) - 1 > LEFT + room {
                break;
            }
            x = font.draw_char(&mut img, x, geom::INFO_Y, c, pens.dark);
        }
        font.draw(&mut img, x, geom::INFO_Y, &details, pens.dark);
        font.draw(
            &mut img,
            LEFT,
            geom::RESULT_Y,
            &self.info.result(),
            pens.dark,
        );
        img.hline(1, w - 2, geom::RULE_Y, pens.dark);

        let ghost_text = |img: &mut IndexedImage, x: i32, y: i32, text: &str| {
            font.draw(img, x, y, text, pens.ghost.unwrap_or(pens.dark));
            if pens.ghost.is_none() {
                checker(
                    img,
                    Rect::new(x, y, font.text_width(text), ROW_H),
                    pens.face,
                );
            }
        };
        // Row labels. The focused one is highlighted like a menu item (a bar in the select
        // pen, the text in a pen that differs from it), which shows on every palette, also
        // where the select pen is the dark one (2 and 4 colours). A ghosted row shows no focus.
        let rows_y = [
            geom::POSITION_Y,
            geom::PALETTE_Y,
            geom::COLORS_Y,
            geom::DITHER_Y,
        ];
        for ((row, label), y) in ROWS.iter().zip(LABELS).zip(rows_y) {
            if *row == ImportRow::Colors && !self.colors_enabled() {
                ghost_text(&mut img, LEFT, y, label);
            } else if self.focus == Some(*row) {
                let r = Self::label_bar(label, y);
                img.fill_rect(r.x, r.y, r.w, r.h, pens.select);
                let ink = [pens.shine, pens.face, pens.dark]
                    .into_iter()
                    .find(|&p| p != pens.select)
                    .unwrap_or(pens.dark);
                font.draw(&mut img, LEFT, y, label, ink);
            } else {
                font.draw(&mut img, LEFT, y, label, pens.dark);
            }
        }
        for (c, r) in self.controls() {
            let radio = |img: &mut IndexedImage, on: bool, label: &str| {
                draw_box(img, (r.x, r.y), on, false, pens);
                font.draw(img, r.x + BOX + 4, r.y, label, pens.dark);
            };
            match c {
                ImportControl::Position(p) => {
                    let label = POSITIONS.iter().find(|(v, _)| *v == p).map_or("", |v| v.1);
                    radio(&mut img, self.options.position == p, label);
                }
                ImportControl::Palette(p) => {
                    let label = PALETTES.iter().find(|(v, _)| *v == p).map_or("", |v| v.1);
                    radio(&mut img, self.options.palette == p, label);
                }
                ImportControl::Dither(d) => {
                    let label = DITHERS.iter().find(|(v, _)| *v == d).map_or("", |v| v.1);
                    radio(&mut img, self.options.dither == d, label);
                }
                ImportControl::Fewer | ImportControl::More => {
                    let label = if c == ImportControl::Fewer {
                        "◂"
                    } else {
                        "▶"
                    };
                    draw_button(&mut img, r, label, self.pressed(c), pens);
                    if !self.enabled(c) {
                        checker(&mut img, r.inset(2), pens.face);
                    }
                }
                ImportControl::Cancel => draw_button(&mut img, r, "Cancel", self.pressed(c), pens),
                ImportControl::Ok => draw_button(&mut img, r, "Ok", self.pressed(c), pens),
            }
        }
        // The colour count between its arrows.
        let count = self.shown_colors().to_string();
        let cx = Self::radio_x() + ARROW_W;
        let tx = cx + (COUNT_W - font.text_width(&count)) / 2;
        if self.colors_enabled() {
            font.draw(&mut img, tx, geom::COLORS_Y, &count, pens.dark);
        } else {
            ghost_text(&mut img, tx, geom::COLORS_Y, &count);
        }
        // The thumbnail box: a dark frame, a gap in the face pen (a page edge in a dark BG
        // colour stays visible) and a dark placeholder for the true-colour picture.
        let t = self.thumb_local();
        let outer = Rect::new(t.x - 2, t.y - 2, t.w + 4, t.h + 4);
        frame(&mut img, outer, pens.dark);
        img.fill_rect(t.x, t.y, t.w, t.h, pens.dark);
        img
    }
}

/// Writes `thumb` into the RGBA frame `rgba` (`frame_w` pixels per row, 4 bytes per pixel)
/// with its top-left corner at UI pixel `at`, every thumbnail pixel covering `sx`×`sy` frame
/// pixels (the screen's UI scale times the frame's own scale). Pixels outside the frame are
/// left out. The thumbnail is opaque: its alpha is not used.
pub fn overlay_rgb(
    rgba: &mut [u8],
    frame_w: usize,
    at: (i32, i32),
    thumb: &RgbaImage,
    sx: usize,
    sy: usize,
) {
    if frame_w == 0 {
        return;
    }
    let frame_h = rgba.len() / 4 / frame_w;
    for ty in 0..thumb.height() as usize {
        for tx in 0..thumb.width() as usize {
            let p = thumb.pixels()[ty * thumb.width() as usize + tx];
            for dy in 0..sy {
                let y = (at.1 as i64 + ty as i64) * sy as i64 + dy as i64;
                if y < 0 || y >= frame_h as i64 {
                    continue;
                }
                for dx in 0..sx {
                    let x = (at.0 as i64 + tx as i64) * sx as i64 + dx as i64;
                    if x < 0 || x >= frame_w as i64 {
                        continue;
                    }
                    let o = (y as usize * frame_w + x as usize) * 4;
                    rgba[o..o + 4].copy_from_slice(&[p.r, p.g, p.b, 255]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{Palette, Rgba8};

    fn info(page: (u32, u32)) -> ImportInfo {
        ImportInfo {
            name: "photo.jpg".into(),
            size: (4032, 3024),
            colors: None,
            fitted: (320, 240),
            page,
        }
    }

    fn requester(screen: (i32, i32)) -> ImportRequester {
        ImportRequester::new(
            info((screen.0 as u32, screen.1 as u32)),
            ImportOptions::default(),
            256,
            32,
            screen,
            (1, 1),
        )
    }

    fn pens() -> Pens {
        Pens::from_palette(&Palette::dpaint_default_32())
    }

    fn click(r: &mut ImportRequester, rect: Rect) -> ImportResponse {
        let (x, y) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
        let a = r.handle(ReqEvent::Press(x, y));
        let b = r.handle(ReqEvent::Release);
        if a == ImportResponse::Continue { b } else { a }
    }

    fn rect_of(r: &ImportRequester, c: ImportControl) -> Rect {
        r.controls().into_iter().find(|&(k, _)| k == c).unwrap().1
    }

    fn key(r: &mut ImportRequester, k: ReqKey) -> ImportResponse {
        r.handle(ReqEvent::Key(k))
    }

    #[test]
    fn layout_fits_small_and_large_screens() {
        for screen in [
            (320, 200),
            (320, 256),
            (368, 283),
            (320, 512 / 2),
            (368, 241),
        ] {
            let r = requester(screen);
            let (w, h) = r.size();
            let (ox, oy) = r.origin();
            assert!(
                ox >= 0 && oy >= 0 && ox + w <= screen.0 && oy + h <= screen.1,
                "{screen:?}"
            );
            let bounds = Rect::new(0, 0, w, h);
            let controls = r.controls();
            for (i, &(c, rect)) in controls.iter().enumerate() {
                assert_eq!(rect.intersect(bounds), rect, "{c:?} inside on {screen:?}");
                for &(d, other) in &controls[i + 1..] {
                    assert!(rect.intersect(other).is_empty(), "{c:?} overlaps {d:?}");
                }
                let t = r.thumb_local();
                assert!(rect.intersect(t).is_empty(), "{c:?} under the thumbnail");
            }
            let t = r.thumb_local();
            assert_eq!(t.intersect(bounds.inset(1)), t);
            let (tw, th) = r.thumb_size();
            assert!(tw >= 80 && th >= 60, "thumbnail {tw}x{th} on {screen:?}");
            // Proportions of the page.
            let want = screen.0 as f64 / screen.1 as f64;
            assert!((tw as f64 / th as f64 - want).abs() < 0.05, "{screen:?}");
        }
        // The thumbnail grows with the screen.
        assert!(requester((368, 283)).thumb_size().0 > requester((320, 200)).thumb_size().0);
    }

    #[test]
    fn thumbnail_follows_the_pixel_shape() {
        // A HiRes page of 640×256 looks like 320×256 LoRes: same thumbnail proportions.
        let mut r = requester((320, 256));
        let lores = r.thumb_size();
        r.info.page = (640, 256);
        r.pixel_shape = (1, 2);
        assert_eq!(r.thumb_size(), lores);
    }

    #[test]
    fn every_control_is_hit_where_it_is_drawn() {
        let p = pens();
        let mut r = requester((320, 200));
        let plain = r.render(&p);
        for (c, rect) in r.controls() {
            // Something is drawn there (a box, a bevel or a label).
            let drawn = (rect.y..rect.y + rect.h)
                .flat_map(|y| (rect.x..rect.x + rect.w).map(move |x| (x, y)))
                .filter(|&(x, y)| plain.get(x, y) != Some(p.face))
                .count();
            assert!(drawn > 4, "{c:?} is drawn");
            let mut probe = r.clone();
            let got = click(&mut probe, rect);
            match c {
                ImportControl::Position(v) => assert_eq!(probe.options().position, v),
                ImportControl::Palette(v) => assert_eq!(probe.options().palette, v),
                ImportControl::Dither(v) => assert_eq!(probe.options().dither, v),
                ImportControl::Fewer => assert_eq!(probe.options().colors, 16),
                ImportControl::More => assert_eq!(probe.options().colors, 64),
                ImportControl::Cancel => assert_eq!(got, ImportResponse::Cancel),
                ImportControl::Ok => assert_eq!(got, ImportResponse::Ok),
            }
        }
        // Changing an option says so; choosing the same again does not.
        let top_left = rect_of(&r, ImportControl::Position(Position::TopLeft));
        assert_eq!(click(&mut r, top_left), ImportResponse::Changed);
        assert_eq!(click(&mut r, top_left), ImportResponse::Continue);
        // A button released outside does nothing.
        let ok = rect_of(&r, ImportControl::Ok);
        r.handle(ReqEvent::Press(ok.x + 2, ok.y + 2));
        r.handle(ReqEvent::Drag(0, 0));
        assert_eq!(r.handle(ReqEvent::Release), ImportResponse::Continue);
    }

    #[test]
    fn colours_are_ghosted_with_the_current_palette() {
        let p = pens();
        let mut r = requester((320, 256));
        let fewer = rect_of(&r, ImportControl::Fewer);
        let new = r.render(&p);
        let current = rect_of(&r, ImportControl::Palette(PaletteChoice::Current));
        click(&mut r, current);
        assert!(!r.colors_enabled());
        assert_eq!(r.shown_colors(), 32, "the current palette's size");
        let ghosted = r.render(&p);
        assert_ne!(new, ghosted);
        let ghost = p.ghost.unwrap();
        let count_y = geom::COLORS_Y;
        let ghost_pixels = (count_y..count_y + 8)
            .flat_map(|y| (fewer.x..fewer.x + 60).map(move |x| (x, y)))
            .filter(|&(x, y)| ghosted.get(x, y) == Some(ghost))
            .count();
        assert!(ghost_pixels > 0, "the count is grey");
        // The arrows do nothing now.
        let before = r.options();
        assert_eq!(click(&mut r, fewer), ImportResponse::Continue);
        let more = rect_of(&r, ImportControl::More);
        assert_eq!(click(&mut r, more), ImportResponse::Continue);
        assert_eq!(r.options(), before);
        // Tab skips the ghosted count.
        for want in [ImportRow::Position, ImportRow::Palette, ImportRow::Dither] {
            key(&mut r, ReqKey::Tab);
            assert_eq!(r.focus(), Some(want));
        }
    }

    #[test]
    fn colour_count_stops_at_the_screen_maximum() {
        let mut r = ImportRequester::new(
            info((320, 256)),
            ImportOptions {
                colors: 256,
                ..ImportOptions::default()
            },
            32,
            32,
            (320, 256),
            (1, 1),
        );
        assert_eq!(r.options().colors, 32, "clamped to OCS LoRes");
        let more = rect_of(&r, ImportControl::More);
        assert!(!r.enabled(ImportControl::More));
        assert_eq!(click(&mut r, more), ImportResponse::Continue);
        assert_eq!(r.options().colors, 32);
        let fewer = rect_of(&r, ImportControl::Fewer);
        let mut seen = vec![r.options().colors];
        while click(&mut r, fewer) == ImportResponse::Changed {
            seen.push(r.options().colors);
        }
        assert_eq!(seen, [32, 16, 8, 4, 2]);
        assert!(!r.enabled(ImportControl::Fewer));
        // HiRes OCS: 16.
        let r = ImportRequester::new(
            info((640, 256)),
            ImportOptions::default(),
            16,
            16,
            (320, 256),
            (1, 2),
        );
        assert_eq!(r.options().colors, 16);
    }

    #[test]
    fn keyboard_return_esc_tab_and_arrows() {
        let mut r = requester((320, 200));
        assert_eq!(key(&mut r, ReqKey::Enter), ImportResponse::Ok);
        assert_eq!(key(&mut r, ReqKey::Escape), ImportResponse::Cancel);
        // No focus yet: arrows do nothing.
        assert_eq!(key(&mut r, ReqKey::Down), ImportResponse::Continue);
        assert_eq!(r.focus(), None);
        let mut order = Vec::new();
        for _ in 0..5 {
            key(&mut r, ReqKey::Tab);
            order.push(r.focus().unwrap());
        }
        assert_eq!(
            order,
            [
                ImportRow::Position,
                ImportRow::Palette,
                ImportRow::Colors,
                ImportRow::Dither,
                ImportRow::Position
            ]
        );
        assert_eq!(key(&mut r, ReqKey::Down), ImportResponse::Changed);
        assert_eq!(r.options().position, Position::TopLeft);
        assert_eq!(
            key(&mut r, ReqKey::Down),
            ImportResponse::Continue,
            "last choice"
        );
        key(&mut r, ReqKey::Tab);
        key(&mut r, ReqKey::Tab);
        assert_eq!(key(&mut r, ReqKey::Right), ImportResponse::Changed);
        assert_eq!(r.options().colors, 64);
        key(&mut r, ReqKey::Tab);
        assert_eq!(key(&mut r, ReqKey::Up), ImportResponse::Changed);
        assert_eq!(r.options().dither, Dither::None);
        // The focused label is highlighted in the select pen.
        let p = pens();
        let img = r.render(&p);
        let select = (geom::DITHER_Y..geom::DITHER_Y + 8)
            .flat_map(|y| (LEFT..LEFT + 30).map(move |x| (x, y)))
            .filter(|&(x, y)| img.get(x, y) == Some(p.select))
            .count();
        assert!(select > 0);
    }

    /// The default palette cut to its first `n` entries.
    fn first(n: usize) -> Pens {
        let all = Palette::dpaint_default_32();
        Pens::from_palette(&Palette::new(all.colors()[..n].to_vec()).unwrap())
    }

    /// The pixels of a label's focus bar.
    fn label_area(img: &IndexedImage, label: &str, y: i32) -> Vec<Option<u8>> {
        let r = ImportRequester::label_bar(label, y);
        (r.y..r.y + r.h)
            .flat_map(|y| (r.x..r.x + r.w).map(move |x| (x, y)))
            .map(|(x, y)| img.get(x, y))
            .collect()
    }

    #[test]
    fn focus_shows_on_two_and_four_colour_screens() {
        for n in [2, 4, 8, 32] {
            let p = first(n);
            let mut r = requester((320, 256));
            let plain = r.render(&p);
            key(&mut r, ReqKey::Tab);
            assert_eq!(r.focus(), Some(ImportRow::Position));
            let focused = r.render(&p);
            let (a, b) = (
                label_area(&plain, LABELS[0], geom::POSITION_Y),
                label_area(&focused, LABELS[0], geom::POSITION_Y),
            );
            assert_ne!(a, b, "{n} colours: the focus is visible");
            // The label stays readable: its text differs from the bar.
            let distinct: std::collections::HashSet<_> = b.iter().collect();
            assert!(distinct.len() >= 2, "{n} colours");
        }
    }

    #[test]
    fn a_ghosted_colour_row_is_ghosted_whole_and_drops_the_focus() {
        // Two colours: no ghost pen, so the label gets the dot pattern like the count.
        let p = first(2);
        assert_eq!(p.ghost, None);
        let mut r = requester((320, 256));
        let new = r.render(&p);
        let current = rect_of(&r, ImportControl::Palette(PaletteChoice::Current));
        click(&mut r, current);
        let ghosted = r.render(&p);
        assert_ne!(
            label_area(&new, LABELS[2], geom::COLORS_Y),
            label_area(&ghosted, LABELS[2], geom::COLORS_Y),
            "the Colours label is ghosted too"
        );
        // Focus on Colours, then Current with the mouse: the focus moves to Palette, and
        // the ghosted label shows no focus.
        let p = pens();
        let mut r = requester((320, 256));
        for _ in 0..3 {
            key(&mut r, ReqKey::Tab);
        }
        assert_eq!(r.focus(), Some(ImportRow::Colors));
        click(&mut r, current);
        assert_eq!(r.focus(), Some(ImportRow::Palette));
        let img = r.render(&p);
        assert!(
            label_area(&img, LABELS[2], geom::COLORS_Y)
                .iter()
                .all(|&c| c != Some(p.select))
        );
        assert!(label_area(&img, LABELS[1], geom::PALETTE_Y).contains(&Some(p.select)));
    }

    #[test]
    fn renders_square_corners_names_the_file_and_leaves_the_box_dark() {
        let p = pens();
        let r = requester((320, 200));
        let img = r.render(&p);
        assert_eq!((img.width() as i32, img.height() as i32), r.size());
        let (w, h) = r.size();
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(img.get(x, y), Some(p.dark), "square corner {x},{y}");
        }
        let t = r.thumb_local();
        assert!((t.y..t.y + t.h).all(|y| (t.x..t.x + t.w).all(|x| img.get(x, y) == Some(p.dark))));
        assert_eq!(r.info().details(), "  4032x3024  true colour");
        assert_eq!(r.info().result(), "-> 320x240 on 320x200");
        // A very long name is cut, the details stay.
        let mut long = r.clone();
        long.info.name = "a_very_long_file_name_that_goes_on_and_on.png".into();
        let img = long.render(&p);
        let right = (geom::INFO_Y..geom::INFO_Y + 8)
            .filter(|&y| img.get(w - 2 * LEFT, y) == Some(p.dark))
            .count();
        assert!(right <= 8);
        assert_ne!(img, r.render(&p));
    }

    #[test]
    fn overlay_writes_exactly_the_box() {
        let thumb = RgbaImage::from_pixels(
            2,
            2,
            vec![
                Rgba8::opaque(1, 2, 3),
                Rgba8::opaque(4, 5, 6),
                Rgba8::opaque(7, 8, 9),
                Rgba8::opaque(10, 11, 12),
            ],
        )
        .unwrap();
        for (sx, sy) in [(1, 1), (2, 2), (2, 1), (1, 2)] {
            let (fw, fh) = (6 * sx, 5 * sy);
            let mut rgba = vec![200u8; fw * fh * 4];
            overlay_rgb(&mut rgba, fw, (3, 2), &thumb, sx, sy);
            for y in 0..fh {
                for x in 0..fw {
                    let (ux, uy) = (x / sx, y / sy);
                    let o = (y * fw + x) * 4;
                    let inside = (3..5).contains(&ux) && (2..4).contains(&uy);
                    let want = if inside {
                        let p = thumb.get((ux - 3) as u32, (uy - 2) as u32).unwrap();
                        [p.r, p.g, p.b, 255]
                    } else {
                        [200; 4]
                    };
                    assert_eq!(rgba[o..o + 4], want, "{sx}x{sy} at {x},{y}");
                }
            }
        }
        // Clipped at the frame's edge.
        let mut rgba = vec![0u8; 4 * 4 * 4];
        overlay_rgb(&mut rgba, 4, (3, 3), &thumb, 1, 1);
        assert_eq!(rgba[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 3], [1, 2, 3]);
        overlay_rgb(&mut rgba, 4, (-1, -1), &thumb, 1, 1);
        assert_eq!(rgba[0..3], [10, 11, 12]);
    }

    #[test]
    fn requester_overlay_uses_its_box_on_the_screen() {
        let mut r = requester((320, 256));
        let (tw, th) = r.thumb_size();
        r.set_thumbnail(RgbaImage::new(tw, th, Rgba8::opaque(9, 99, 199)));
        // A HiRes screen: 2×1 screen pixels per UI pixel.
        let (fw, fh) = (640usize, 256usize);
        let mut rgba = vec![0u8; fw * fh * 4];
        r.overlay(&mut rgba, fw, 2, 1);
        let rect = r.thumb_rect();
        let count = rgba.chunks(4).filter(|p| p[..3] == [9, 99, 199]).count();
        assert_eq!(count, (rect.w * 2 * rect.h) as usize);
        let o = ((rect.y as usize) * fw + rect.x as usize * 2) * 4;
        assert_eq!(rgba[o..o + 3], [9, 99, 199]);
    }
}
