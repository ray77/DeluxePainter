//! Requesters: software-rendered Amiga-style dialogs. The first one is Picture ▸ Screen
//! Format ("Choose Display Mode", DeluxePaint IV AGA): like the original it appears on its
//! own HiRes screen (640×256) with Workbench-style colours. Layout after `Choose_Setup.png`;
//! exact geometry is still to calibrate (AMIGA_GUI_FIDELITY.md).

use std::sync::OnceLock;

use dpx_core::{Chipset, Command, DisplayMode, IndexedImage, Palette, Target};

use crate::bitmap::{self, Bitmap};
use crate::font::{Font, glyph};
use crate::pens::Pens;
use crate::widgets::{TextField, draw_field};

/// Requester screen: PAL HiRes.
pub const SCREEN: DisplayMode =
    DisplayMode::new(dpx_core::Video::Pal, dpx_core::Resolution::Hi, false);

/// Workbench-style colours of the requester screen: grey, black, white, blue.
pub fn palette() -> Palette {
    Palette::from_rgb4(&[0xAAA, 0x000, 0xFFF, 0x68B]).expect("4 colours")
}

/// Version line in the credits panel.
const VERSION: &str = concat!("Version ", env!("CARGO_PKG_VERSION"));

mod geom {
    pub const SCREEN_W: i32 = 640;
    pub const SCREEN_H: i32 = 256;
    pub const TITLE_H: i32 = 11;
    pub const WIN_X: i32 = 16;
    pub const WIN_Y: i32 = 13;
    pub const WIN_W: i32 = 608;
    pub const WIN_H: i32 = 206;
    pub const LEFT_X: i32 = 28;
    pub const LEFT_W: i32 = 344;
    pub const RIGHT_X: i32 = 384;
    pub const RIGHT_W: i32 = 228;
    pub const LIST_Y: i32 = 30;
    pub const ROW_H: i32 = 10;
    pub const ROWS: i32 = 5;
    pub const LIST_H: i32 = ROWS * ROW_H + 2;
    pub const SCROLL_W: i32 = 14;
    pub const MODE_FIELD_Y: i32 = 84;
    pub const FIELD_H: i32 = 12;
    pub const SCREEN_SIZE_Y: i32 = 114;
    pub const PAGE_SIZE_Y: i32 = 144;
    pub const PALETTE_Y: i32 = 174;
    pub const CYCLE_X: i32 = 40;
    pub const CYCLE_W: i32 = 140;
    pub const GADGET_H: i32 = 13;
    pub const NUM_W_X: i32 = 192;
    pub const NUM_H_X: i32 = 248;
    pub const NUM_W: i32 = 48;
    pub const SLIDER_X: i32 = 72;
    pub const SLIDER_W: i32 = 112;
    pub const KNOB_W: i32 = 14;
    pub const CHECK_X: i32 = 200;
    pub const CHECK_W: i32 = 18;
    pub const INFO_Y: i32 = 30;
    pub const INFO_H: i32 = 66;
    pub const CREDITS_Y: i32 = 114;
    pub const CREDITS_H: i32 = 72;
    pub const BUTTON_Y: i32 = 198;
    pub const BUTTON_W: i32 = 64;
    pub const BUTTON_H: i32 = 14;
    pub const USE_X: i32 = 28;
    pub const CANCEL_X: i32 = 548;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageChoice {
    /// The page takes the screen size.
    Screen,
    /// The old page is kept (grown if smaller than the screen).
    KeepSame,
}

/// The Screen Size cycle gadget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenChoice {
    /// The mode's standard size.
    Standard,
    /// The mode's overscan size.
    Overscan,
    /// A size typed into the two number boxes.
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReqKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Tab,
    Enter,
    Escape,
    /// A typed character (text input).
    Char(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReqEvent {
    /// Left button pressed at a requester-screen position.
    Press(i32, i32),
    /// Pointer moved with the button held.
    Drag(i32, i32),
    Release,
    Key(ReqKey),
    /// Mouse wheel: rows to scroll (positive = down).
    Scroll(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReqResponse {
    Continue,
    Use(Command),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    Use,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drag {
    /// Scroll knob held at this offset from its top.
    Scroll(i32),
    Slider,
    Button(Button),
}

/// State of the Screen Format requester.
#[derive(Clone, Debug)]
pub struct ScreenFormatRequester {
    chipset: Chipset,
    modes: Vec<DisplayMode>,
    selected: usize,
    scroll: usize,
    page: PageChoice,
    screen_choice: ScreenChoice,
    /// Custom screen width / height.
    field_w: TextField,
    field_h: TextField,
    /// Active number box (0 = width, 1 = height) while Custom.
    active: Option<usize>,
    /// Palette size as a power of two (1 = 2 colours … 8 = 256).
    colors_log2: u32,
    retain: bool,
    current_page: (u32, u32),
    drag: Option<Drag>,
    pointer: (i32, i32),
    /// Shown in the information panel, e.g. why "Use" was refused.
    message: Option<String>,
}

impl ScreenFormatRequester {
    /// Opens the requester for a document with this target, page size and colour count.
    pub fn new(target: Target, page: (u32, u32), colors: usize) -> Self {
        let modes: Vec<DisplayMode> = DisplayMode::all()
            .into_iter()
            .filter(|m| m.max_colors(target.chipset).is_some())
            .collect();
        let selected = modes.iter().position(|&m| m == target.mode).unwrap_or(0);
        let mut r = Self {
            chipset: target.chipset,
            modes,
            selected,
            scroll: 0,
            page: PageChoice::Screen,
            screen_choice: ScreenChoice::Standard,
            field_w: TextField::new(4),
            field_h: TextField::new(4),
            active: None,
            colors_log2: colors.max(2).next_power_of_two().trailing_zeros(),
            retain: true,
            current_page: page,
            drag: None,
            pointer: (0, 0),
            message: None,
        };
        r.clamp_colors();
        r.reveal_selected();
        let mode = r.mode();
        r.screen_choice = if target.screen == mode.size() {
            ScreenChoice::Standard
        } else if target.screen == mode.overscan_size() {
            ScreenChoice::Overscan
        } else {
            ScreenChoice::Custom
        };
        r.set_fields(target.screen);
        r
    }

    fn set_fields(&mut self, (w, h): (u32, u32)) {
        self.field_w.set(&w.to_string());
        self.field_h.set(&h.to_string());
    }

    pub fn screen_choice(&self) -> ScreenChoice {
        self.screen_choice
    }

    /// The screen size that "Use" would set.
    pub fn screen_size(&self) -> (u32, u32) {
        let std = self.mode().size();
        match self.screen_choice {
            ScreenChoice::Standard => std,
            ScreenChoice::Overscan => self.mode().overscan_size(),
            ScreenChoice::Custom => {
                let parse = |f: &TextField, d: u32| match f.text().parse::<u32>() {
                    Ok(v) if v > 0 => v,
                    _ => d,
                };
                (parse(&self.field_w, std.0), parse(&self.field_h, std.1))
            }
        }
    }

    fn cycle_screen(&mut self) {
        let shown = self.screen_size();
        self.screen_choice = match self.screen_choice {
            ScreenChoice::Standard => ScreenChoice::Overscan,
            ScreenChoice::Overscan => ScreenChoice::Custom,
            ScreenChoice::Custom => ScreenChoice::Standard,
        };
        self.active = None;
        if self.screen_choice == ScreenChoice::Custom {
            self.set_fields(shown);
        }
        self.message = None;
    }

    pub fn mode(&self) -> DisplayMode {
        self.modes[self.selected]
    }

    pub fn colors(&self) -> u16 {
        1 << self.colors_log2
    }

    fn max_log2(&self) -> u32 {
        u32::from(self.mode().max_colors(self.chipset).unwrap_or(2)).trailing_zeros()
    }

    fn clamp_colors(&mut self) {
        self.colors_log2 = self.colors_log2.clamp(1, self.max_log2());
    }

    fn reveal_selected(&mut self) {
        let rows = geom::ROWS as usize;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
        self.scroll = self.scroll.min(self.modes.len().saturating_sub(rows));
    }

    fn select(&mut self, i: usize) {
        self.selected = i.min(self.modes.len() - 1);
        self.clamp_colors();
        self.reveal_selected();
        self.message = None;
        if self.screen_choice == ScreenChoice::Custom {
            let (w, h) = self.screen_size();
            if !self.mode().screen_fits(w, h) {
                let std = self.mode().size();
                self.set_fields(std);
            }
        }
    }

    /// Page size that "Use" would set.
    pub fn page_size(&self) -> (u32, u32) {
        let (w, h) = self.screen_size();
        match self.page {
            PageChoice::Screen => (w, h),
            PageChoice::KeepSame => (self.current_page.0.max(w), self.current_page.1.max(h)),
        }
    }

    /// The command "Use" sends.
    pub fn command(&self) -> Command {
        let (width, height) = self.page_size();
        Command::SetFormat {
            mode: self.mode(),
            colors: self.colors(),
            width,
            height,
            retain: self.retain,
            screen: Some(self.screen_size()),
        }
    }

    /// Shows why the engine refused the format; the requester stays open.
    pub fn set_message(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
    }

    pub fn handle(&mut self, event: ReqEvent) -> ReqResponse {
        match event {
            ReqEvent::Press(x, y) => {
                self.pointer = (x, y);
                self.press(x, y);
                ReqResponse::Continue
            }
            ReqEvent::Drag(x, y) => {
                self.pointer = (x, y);
                match self.drag {
                    Some(Drag::Scroll(offset)) => self.scroll_to_knob(y - offset),
                    Some(Drag::Slider) => self.slider_to(x),
                    _ => {}
                }
                ReqResponse::Continue
            }
            ReqEvent::Release => match self.drag.take() {
                Some(Drag::Button(b)) if button_rect(b).contains(self.pointer) => self.activate(b),
                _ => ReqResponse::Continue,
            },
            ReqEvent::Key(k) => match k {
                ReqKey::Up => {
                    self.select(self.selected.saturating_sub(1));
                    ReqResponse::Continue
                }
                ReqKey::Down => {
                    self.select(self.selected + 1);
                    ReqResponse::Continue
                }
                ReqKey::Enter => self.activate(Button::Use),
                ReqKey::Escape => self.activate(Button::Cancel),
                ReqKey::Tab if self.screen_choice == ScreenChoice::Custom => {
                    self.active = Some(match self.active {
                        Some(0) => 1,
                        _ => 0,
                    });
                    ReqResponse::Continue
                }
                ReqKey::Char(c) if self.active.is_some() && c.is_ascii_digit() => {
                    self.edit(k);
                    ReqResponse::Continue
                }
                ReqKey::Backspace
                | ReqKey::Delete
                | ReqKey::Left
                | ReqKey::Right
                | ReqKey::Home
                | ReqKey::End => {
                    self.edit(k);
                    ReqResponse::Continue
                }
                ReqKey::Char(c) => match c.to_ascii_lowercase() {
                    'u' => self.activate(Button::Use),
                    'c' => self.activate(Button::Cancel),
                    'r' => {
                        self.retain = !self.retain;
                        ReqResponse::Continue
                    }
                    _ => ReqResponse::Continue,
                },
                _ => ReqResponse::Continue,
            },
            ReqEvent::Scroll(rows) => {
                let max = self.modes.len().saturating_sub(geom::ROWS as usize) as i32;
                self.scroll = (self.scroll as i32 + rows).clamp(0, max) as usize;
                ReqResponse::Continue
            }
        }
    }

    fn edit(&mut self, k: ReqKey) {
        if self.screen_choice != ScreenChoice::Custom {
            return;
        }
        match self.active {
            Some(0) => self.field_w.key(k),
            Some(1) => self.field_h.key(k),
            _ => {}
        }
        self.message = None;
    }

    fn activate(&self, b: Button) -> ReqResponse {
        match b {
            Button::Use => ReqResponse::Use(self.command()),
            Button::Cancel => ReqResponse::Cancel,
        }
    }

    fn press(&mut self, x: i32, y: i32) {
        let p = (x, y);
        let list = list_rect();
        let track = scroll_track();
        if track.contains(p) {
            let (ky, kh) = self.knob();
            if y < ky {
                self.scroll = self.scroll.saturating_sub(geom::ROWS as usize);
            } else if y >= ky + kh {
                self.scroll = (self.scroll + geom::ROWS as usize)
                    .min(self.modes.len().saturating_sub(geom::ROWS as usize));
            } else {
                self.drag = Some(Drag::Scroll(y - ky));
            }
        } else if list.contains(p) {
            let row = ((y - list.y - 1) / geom::ROW_H).clamp(0, geom::ROWS - 1) as usize;
            if self.scroll + row < self.modes.len() {
                self.select(self.scroll + row);
            }
        } else if cycle_rect(geom::SCREEN_SIZE_Y).contains(p) {
            self.cycle_screen();
        } else if self.screen_choice == ScreenChoice::Custom
            && number_rect(geom::NUM_W_X, geom::SCREEN_SIZE_Y).contains(p)
        {
            self.active = Some(0);
        } else if self.screen_choice == ScreenChoice::Custom
            && number_rect(geom::NUM_H_X, geom::SCREEN_SIZE_Y).contains(p)
        {
            self.active = Some(1);
        } else if cycle_rect(geom::PAGE_SIZE_Y).contains(p) {
            self.page = match self.page {
                PageChoice::Screen => PageChoice::KeepSame,
                PageChoice::KeepSame => PageChoice::Screen,
            };
        } else if slider_rect().contains(p) {
            self.drag = Some(Drag::Slider);
            self.slider_to(x);
        } else if check_rect().contains(p) || retain_label_rect().contains(p) {
            self.retain = !self.retain;
        } else if button_rect(Button::Use).contains(p) {
            self.drag = Some(Drag::Button(Button::Use));
        } else if button_rect(Button::Cancel).contains(p) {
            self.drag = Some(Drag::Button(Button::Cancel));
        }
    }

    /// Scroll knob top and height inside the track.
    fn knob(&self) -> (i32, i32) {
        let t = scroll_track();
        let total = self.modes.len().max(1) as i32;
        let rows = geom::ROWS.min(total);
        let h = (t.h * rows / total).max(6);
        let range = (total - rows).max(1);
        (t.y + (t.h - h) * self.scroll as i32 / range, h)
    }

    fn scroll_to_knob(&mut self, top: i32) {
        let t = scroll_track();
        let (_, h) = self.knob();
        let total = self.modes.len() as i32;
        let range = (total - geom::ROWS).max(0);
        let free = (t.h - h).max(1);
        let pos = ((top - t.y) * range + free / 2) / free;
        self.scroll = pos.clamp(0, range) as usize;
    }

    fn slider_to(&mut self, x: i32) {
        let s = slider_rect();
        let steps = self.max_log2() as i32; // positions 1..=max
        let free = s.w - geom::KNOB_W;
        let rel = (x - s.x - geom::KNOB_W / 2).clamp(0, free);
        let pos = if steps > 1 {
            1 + (rel * (steps - 1) + free / 2) / free
        } else {
            1
        };
        self.colors_log2 = pos as u32;
        self.message = None;
    }

    fn slider_knob_x(&self) -> i32 {
        let s = slider_rect();
        let steps = self.max_log2() as i32;
        let free = s.w - geom::KNOB_W;
        if steps > 1 {
            s.x + (self.colors_log2 as i32 - 1) * free / (steps - 1)
        } else {
            s.x
        }
    }

    /// Draws the requester screen (use with [`palette`]).
    pub fn render(&self) -> IndexedImage {
        let pal = palette();
        let pens = Pens::from_palette(&pal);
        let font = Font::builtin();
        let mut img = IndexedImage::new(geom::SCREEN_W as u32, geom::SCREEN_H as u32, pens.face);

        // Screen title bar.
        img.fill_rect(0, 0, geom::SCREEN_W, geom::TITLE_H, pens.shine);
        img.hline(0, geom::SCREEN_W - 1, geom::TITLE_H - 1, pens.dark);
        font.draw(&mut img, 4, 1, "DeluxePainter", pens.dark);

        // Window with a raised double frame.
        let win = Rect::new(geom::WIN_X, geom::WIN_Y, geom::WIN_W, geom::WIN_H);
        raised(&mut img, win, &pens);
        raised(&mut img, win.inset(2), &pens);

        // Display mode list.
        let list = list_rect();
        centred(
            &mut img,
            list.x,
            list.w,
            20,
            "Choose Display Mode",
            pens.shine,
        );
        recessed(&mut img, list, &pens);
        for row in 0..geom::ROWS as usize {
            let i = self.scroll + row;
            let Some(&m) = self.modes.get(i) else {
                break;
            };
            let y = list.y + 1 + row as i32 * geom::ROW_H;
            let text_w = list.w - geom::SCROLL_W - 3;
            let ink = if i == self.selected {
                img.fill_rect(list.x + 1, y, text_w, geom::ROW_H, pens.select);
                pens.shine
            } else {
                pens.dark
            };
            font.draw(&mut img, list.x + 4, y + 1, &m.name(), ink);
            let (w, h) = m.size();
            right_text(
                &mut img,
                list.x + text_w - 3,
                y + 1,
                &format!("{w}x{h}"),
                ink,
            );
        }
        let track = scroll_track();
        recessed(&mut img, track, &pens);
        let (ky, kh) = self.knob();
        raised(&mut img, Rect::new(track.x + 1, ky, track.w - 2, kh), &pens);

        let field = Rect::new(list.x, geom::MODE_FIELD_Y, list.w, geom::FIELD_H);
        recessed(&mut img, field, &pens);
        let m = self.mode();
        font.draw(&mut img, field.x + 4, field.y + 2, &m.name(), pens.dark);
        let (w, h) = m.size();
        right_text(
            &mut img,
            field.x + field.w - 6,
            field.y + 2,
            &format!("{w}x{h}"),
            pens.dark,
        );

        // Screen size (standard only for now) and page size.
        let label_w = geom::NUM_H_X + geom::NUM_W - geom::CYCLE_X;
        centred(
            &mut img,
            geom::CYCLE_X,
            label_w,
            geom::SCREEN_SIZE_Y - 10,
            "Screen Size",
            pens.shine,
        );
        let (sw, sh) = self.screen_size();
        let screen_label = match self.screen_choice {
            ScreenChoice::Standard => "Standard",
            ScreenChoice::Overscan => "Overscan",
            ScreenChoice::Custom => "Custom",
        };
        cycle(&mut img, geom::SCREEN_SIZE_Y, screen_label, &pens);
        if self.screen_choice == ScreenChoice::Custom {
            for (i, (x, f)) in [
                (geom::NUM_W_X, &self.field_w),
                (geom::NUM_H_X, &self.field_h),
            ]
            .into_iter()
            .enumerate()
            {
                let r = number_rect(x, geom::SCREEN_SIZE_Y);
                raised(&mut img, r, &pens);
                let core = dpx_core::Rect::new(r.x + 3, r.y + 2, r.w - 6, r.h - 4);
                draw_field(&mut img, core, f, self.active == Some(i), &pens);
            }
        } else {
            number(
                &mut img,
                geom::NUM_W_X,
                geom::SCREEN_SIZE_Y,
                sw,
                false,
                &pens,
            );
            number(
                &mut img,
                geom::NUM_H_X,
                geom::SCREEN_SIZE_Y,
                sh,
                false,
                &pens,
            );
        }
        centred(
            &mut img,
            geom::CYCLE_X,
            label_w,
            geom::PAGE_SIZE_Y - 10,
            "Page Size",
            pens.shine,
        );
        let page_label = match self.page {
            PageChoice::Screen => "Screen",
            PageChoice::KeepSame => "Keep Same",
        };
        cycle(&mut img, geom::PAGE_SIZE_Y, page_label, &pens);
        let (pw, ph) = self.page_size();
        number(&mut img, geom::NUM_W_X, geom::PAGE_SIZE_Y, pw, false, &pens);
        number(&mut img, geom::NUM_H_X, geom::PAGE_SIZE_Y, ph, false, &pens);

        // Palette size slider and Retain Picture.
        centred(
            &mut img,
            geom::CYCLE_X,
            geom::SLIDER_X + geom::SLIDER_W - geom::CYCLE_X,
            geom::PALETTE_Y - 10,
            "Palette Size",
            pens.shine,
        );
        right_text(
            &mut img,
            geom::SLIDER_X - 6,
            geom::PALETTE_Y + 2,
            &self.colors().to_string(),
            pens.dark,
        );
        let s = slider_rect();
        recessed(&mut img, s, &pens);
        raised(
            &mut img,
            Rect::new(self.slider_knob_x(), s.y + 1, geom::KNOB_W, s.h - 2),
            &pens,
        );
        let c = check_rect();
        raised(&mut img, c, &pens);
        if self.retain {
            font.draw_code(&mut img, c.x + 5, c.y + 2, glyph::CHECK, pens.dark);
        }
        let lx = c.x + c.w + 6;
        font.draw(
            &mut img,
            lx,
            geom::PALETTE_Y + 2,
            "Retain Picture",
            pens.dark,
        );
        underline(&mut img, lx, geom::PALETTE_Y + 2, 'R', pens.dark);

        // Display information.
        let info = Rect::new(geom::RIGHT_X, geom::INFO_Y, geom::RIGHT_W, geom::INFO_H);
        centred(
            &mut img,
            info.x,
            info.w,
            20,
            "Display Information",
            pens.shine,
        );
        recessed(&mut img, info, &pens);
        let max = m.max_colors(self.chipset).unwrap_or(0);
        let depth = match self.chipset {
            Chipset::Ocs => "4096",
            Chipset::Aga => "16777216",
        };
        let (ow, oh) = m.overscan_size();
        let (ax, ay) = m.aspect();
        let (fx, fy) = m.pixel_shape();
        let lines = [
            format!(
                "Chipset = {}",
                match self.chipset {
                    Chipset::Ocs => "OCS",
                    Chipset::Aga => "AGA",
                }
            ),
            format!("Max Colors = {max} of {depth}"),
            format!("Size = {sw}x{sh}"),
            format!("Max Size = {ow}x{oh}"),
            format!("Pixel Shape = {fx}:{fy}"),
            format!("Aspect = {ax}:{ay}"),
        ];
        for (i, line) in lines.iter().enumerate() {
            font.draw(
                &mut img,
                info.x + 6,
                info.y + 2 + i as i32 * 9,
                line,
                pens.dark,
            );
        }
        if let Some(msg) = &self.message {
            img.fill_rect(
                info.x + 1,
                info.y + info.h - 12,
                info.w - 2,
                11,
                pens.select,
            );
            font.draw(&mut img, info.x + 6, info.y + info.h - 11, msg, pens.shine);
        }

        // Credits.
        let credits = Rect::new(
            geom::RIGHT_X,
            geom::CREDITS_Y,
            geom::RIGHT_W,
            geom::CREDITS_H,
        );
        centred(
            &mut img,
            credits.x,
            credits.w,
            geom::CREDITS_Y - 10,
            "Credits",
            pens.shine,
        );
        recessed(&mut img, credits, &pens);
        for (i, line) in ["DeluxePainter", VERSION, "", "(c) 2026", "Dominik Schmidt"]
            .iter()
            .enumerate()
        {
            centred(
                &mut img,
                credits.x,
                credits.w,
                credits.y + 6 + i as i32 * 12,
                line,
                pens.dark,
            );
        }

        // Buttons.
        for b in [Button::Use, Button::Cancel] {
            let r = button_rect(b);
            let pressed = self.drag == Some(Drag::Button(b)) && r.contains(self.pointer);
            if pressed {
                recessed(&mut img, r, &pens);
                img.fill_rect(r.x + 1, r.y + 1, r.w - 2, r.h - 2, pens.select);
            } else {
                raised(&mut img, r, &pens);
            }
            let label = match b {
                Button::Use => "Use",
                Button::Cancel => "Cancel",
            };
            let tx = r.x + (r.w - font.text_width(label)) / 2;
            let ink = if pressed { pens.shine } else { pens.dark };
            font.draw(&mut img, tx, r.y + 3, label, ink);
            underline(
                &mut img,
                tx,
                r.y + 3,
                label.chars().next().unwrap_or(' '),
                ink,
            );
        }
        img
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    fn contains(&self, (x, y): (i32, i32)) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    fn inset(&self, d: i32) -> Self {
        Self::new(self.x + d, self.y + d, self.w - 2 * d, self.h - 2 * d)
    }
}

fn list_rect() -> Rect {
    Rect::new(geom::LEFT_X, geom::LIST_Y, geom::LEFT_W, geom::LIST_H)
}

fn scroll_track() -> Rect {
    let l = list_rect();
    Rect::new(
        l.x + l.w - geom::SCROLL_W - 1,
        l.y + 1,
        geom::SCROLL_W,
        l.h - 2,
    )
}

fn cycle_rect(y: i32) -> Rect {
    Rect::new(geom::CYCLE_X, y, geom::CYCLE_W, geom::GADGET_H)
}

fn number_rect(x: i32, y: i32) -> Rect {
    Rect::new(x, y, geom::NUM_W, geom::GADGET_H)
}

fn slider_rect() -> Rect {
    Rect::new(
        geom::SLIDER_X,
        geom::PALETTE_Y,
        geom::SLIDER_W,
        geom::FIELD_H,
    )
}

fn check_rect() -> Rect {
    Rect::new(geom::CHECK_X, geom::PALETTE_Y, geom::CHECK_W, geom::FIELD_H)
}

fn retain_label_rect() -> Rect {
    let c = check_rect();
    Rect::new(c.x + c.w, c.y, 110, c.h)
}

fn button_rect(b: Button) -> Rect {
    let x = match b {
        Button::Use => geom::USE_X,
        Button::Cancel => geom::CANCEL_X,
    };
    Rect::new(x, geom::BUTTON_Y, geom::BUTTON_W, geom::BUTTON_H)
}

/// Raised bevel: shine top/left, dark bottom/right, face inside.
fn raised(img: &mut IndexedImage, r: Rect, pens: &Pens) {
    bevel(img, r, pens.shine, pens.dark, pens);
}

/// Recessed bevel: dark top/left, shine bottom/right.
fn recessed(img: &mut IndexedImage, r: Rect, pens: &Pens) {
    bevel(img, r, pens.dark, pens.shine, pens);
}

fn bevel(img: &mut IndexedImage, r: Rect, top_left: u8, bottom_right: u8, pens: &Pens) {
    img.fill_rect(r.x, r.y, r.w, r.h, pens.face);
    let (x1, y1) = (r.x + r.w - 1, r.y + r.h - 1);
    img.hline(r.x, x1, r.y, top_left);
    img.vline(r.x, r.y, y1, top_left);
    img.hline(r.x, x1, y1, bottom_right);
    img.vline(x1, r.y, y1, bottom_right);
    // HiRes pixels are narrow: vertical edges are two pixels wide, like Workbench gadgets.
    img.vline(r.x + 1, r.y, y1 - 1, top_left);
    img.vline(x1 - 1, r.y + 1, y1, bottom_right);
}

fn centred(img: &mut IndexedImage, x: i32, w: i32, y: i32, text: &str, pen: u8) {
    let font = Font::builtin();
    font.draw(img, x + (w - font.text_width(text)) / 2, y, text, pen);
}

fn right_text(img: &mut IndexedImage, right: i32, y: i32, text: &str, pen: u8) {
    let font = Font::builtin();
    font.draw(img, right - font.text_width(text), y, text, pen);
}

/// Underlines the first occurrence of `c` in a label drawn at (x, y) (keyboard shortcut).
fn underline(img: &mut IndexedImage, x: i32, y: i32, c: char, pen: u8) {
    let font = Font::builtin();
    let w = font.text_width(&c.to_string());
    img.hline(x, x + w - 1, y + 8, pen);
}

fn cycle(img: &mut IndexedImage, y: i32, text: &str, pens: &Pens) {
    let r = cycle_rect(y);
    raised(img, r, pens);
    let g = glyphs();
    draw_bitmap(img, &g.cycle, r.x + 5, r.y + 3, pens.dark);
    img.vline(r.x + 16, r.y + 2, r.y + r.h - 3, pens.dark);
    img.vline(r.x + 17, r.y + 2, r.y + r.h - 3, pens.shine);
    centred(img, r.x + 18, r.w - 18, r.y + 3, text, pens.dark);
}

/// A number field; `editable = false` draws it ghosted (fixed by the current choice).
fn number(img: &mut IndexedImage, x: i32, y: i32, value: u32, editable: bool, pens: &Pens) {
    let r = number_rect(x, y);
    raised(img, r, pens);
    centred(img, r.x, r.w, r.y + 3, &value.to_string(), pens.dark);
    if !editable {
        for yy in r.y + 2..r.y + r.h - 2 {
            for xx in r.x + 2..r.x + r.w - 2 {
                if (xx + yy) % 2 == 0 {
                    img.set(xx, yy, pens.face);
                }
            }
        }
    }
}

struct Glyphs {
    cycle: Bitmap,
}

fn glyphs() -> &'static Glyphs {
    static G: OnceLock<Glyphs> = OnceLock::new();
    G.get_or_init(|| {
        let parsed = bitmap::parse(include_str!("../../../assets/ui/requester-glyphs.txt"))
            .expect("requester glyphs parse");
        let get = |n: &str| {
            parsed
                .iter()
                .find(|(name, _)| name == n)
                .map(|(_, b)| b.clone())
                .unwrap_or_else(|| panic!("missing glyph {n}"))
        };
        Glyphs {
            cycle: get("cycle"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{Resolution, Video};

    fn req() -> ScreenFormatRequester {
        ScreenFormatRequester::new(Target::default(), (320, 256), 32)
    }

    fn row_y(row: i32) -> i32 {
        geom::LIST_Y + 1 + row * geom::ROW_H + 4
    }

    #[test]
    fn opens_on_the_current_mode() {
        let r = req();
        assert_eq!(r.mode(), DisplayMode::PAL_LORES);
        assert_eq!(r.colors(), 32);
        assert_eq!(
            r.command(),
            Command::SetFormat {
                mode: DisplayMode::PAL_LORES,
                colors: 32,
                width: 320,
                height: 256,
                retain: true,
                screen: Some((320, 256)),
            }
        );
        // The selected row is visible.
        assert!(r.selected >= r.scroll && r.selected < r.scroll + geom::ROWS as usize);
    }

    #[test]
    fn choose_mode_page_colours_and_use() {
        let mut r = req();
        // Scroll to the top with the keyboard and pick the first row with the mouse.
        for _ in 0..12 {
            r.handle(ReqEvent::Key(ReqKey::Up));
        }
        assert_eq!(r.scroll, 0);
        r.handle(ReqEvent::Press(geom::LEFT_X + 20, row_y(0)));
        let ntsc_hi = DisplayMode::new(Video::Ntsc, Resolution::Hi, false);
        assert_eq!(r.mode(), ntsc_hi);
        // Keep Same keeps a page larger than the screen in that dimension.
        r.handle(ReqEvent::Press(geom::CYCLE_X + 50, geom::PAGE_SIZE_Y + 5));
        assert_eq!(r.page_size(), (640, 256));
        // Slider to the far right = 256 colours, far left = 2.
        let s = slider_rect();
        r.handle(ReqEvent::Press(s.x + s.w - 1, s.y + 5));
        r.handle(ReqEvent::Release);
        assert_eq!(r.colors(), 256);
        r.handle(ReqEvent::Press(s.x, s.y + 5));
        r.handle(ReqEvent::Drag(s.x - 30, s.y + 5));
        r.handle(ReqEvent::Release);
        assert_eq!(r.colors(), 2);
        // Retain Picture toggles.
        r.handle(ReqEvent::Press(geom::CHECK_X + 4, geom::PALETTE_Y + 4));
        let use_r = button_rect(Button::Use);
        r.handle(ReqEvent::Press(use_r.x + 5, use_r.y + 5));
        assert_eq!(
            r.handle(ReqEvent::Release),
            ReqResponse::Use(Command::SetFormat {
                mode: ntsc_hi,
                colors: 2,
                width: 640,
                height: 256,
                retain: false,
                screen: Some((640, 200)),
            })
        );
    }

    #[test]
    fn buttons_act_on_release_inside_and_keys_work() {
        let mut r = req();
        let c = button_rect(Button::Cancel);
        r.handle(ReqEvent::Press(c.x + 3, c.y + 3));
        r.handle(ReqEvent::Drag(c.x - 40, c.y + 3));
        assert_eq!(
            r.handle(ReqEvent::Release),
            ReqResponse::Continue,
            "slid off"
        );
        r.handle(ReqEvent::Press(c.x + 3, c.y + 3));
        assert_eq!(r.handle(ReqEvent::Release), ReqResponse::Cancel);
        assert_eq!(r.handle(ReqEvent::Key(ReqKey::Escape)), ReqResponse::Cancel);
        assert!(matches!(
            r.handle(ReqEvent::Key(ReqKey::Char('u'))),
            ReqResponse::Use(_)
        ));
    }

    #[test]
    fn ocs_limits_modes_and_colours() {
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES);
        let mut r = ScreenFormatRequester::new(ocs, (320, 256), 32);
        assert_eq!(r.modes.len(), 8, "no SuperHiRes on OCS");
        // PAL:High Res (16 colours max) clamps the palette.
        let hi = r
            .modes
            .iter()
            .position(|m| m.name() == "PAL:High Res")
            .unwrap();
        r.select(hi);
        assert_eq!(r.colors(), 16);
    }

    #[test]
    fn scroll_bar_pages_and_drags() {
        let mut r = req();
        r.scroll = 0;
        let t = scroll_track();
        r.handle(ReqEvent::Press(t.x + 3, t.y + t.h - 2));
        assert_eq!(r.scroll, 5);
        let (ky, _) = r.knob();
        r.handle(ReqEvent::Press(t.x + 3, ky + 1));
        r.handle(ReqEvent::Drag(t.x + 3, t.y + t.h));
        r.handle(ReqEvent::Release);
        assert_eq!(r.scroll, r.modes.len() - geom::ROWS as usize);
    }

    #[test]
    fn renders_selection_and_labels() {
        let r = req();
        let img = r.render();
        assert_eq!((img.width(), img.height()), (640, 256));
        let pens = Pens::from_palette(&palette());
        let sel_row = (r.selected - r.scroll) as i32;
        let y = geom::LIST_Y + 1 + sel_row * geom::ROW_H;
        assert_eq!(img.get(geom::LEFT_X + 2, y + 9), Some(pens.select));
        // Section labels are drawn in the shine pen.
        assert!((20..28).any(|yy| (28..372).any(|xx| img.get(xx, yy) == Some(pens.shine))));
    }

    fn press_cycle(r: &mut ScreenFormatRequester) {
        r.handle(ReqEvent::Press(geom::CYCLE_X + 50, geom::SCREEN_SIZE_Y + 5));
        r.handle(ReqEvent::Release);
    }

    fn type_str(r: &mut ScreenFormatRequester, s: &str) {
        for c in s.chars() {
            r.handle(ReqEvent::Key(ReqKey::Char(c)));
        }
    }

    #[test]
    fn initial_screen_choice() {
        assert_eq!(req().screen_choice(), ScreenChoice::Standard);
        let o = ScreenFormatRequester::new(Target::default().with_screen(368, 283), (320, 256), 32);
        assert_eq!(o.screen_choice(), ScreenChoice::Overscan);
        assert_eq!(o.screen_size(), (368, 283));
        let c = ScreenFormatRequester::new(Target::default().with_screen(352, 272), (320, 256), 32);
        assert_eq!(c.screen_choice(), ScreenChoice::Custom);
        assert_eq!(c.screen_size(), (352, 272));
        assert_eq!(c.field_w.text(), "352");
        assert_eq!(c.field_h.text(), "272");
    }

    #[test]
    fn screen_size_cycles() {
        let mut r = req();
        assert_eq!(r.screen_size(), (320, 256));
        press_cycle(&mut r);
        assert_eq!(r.screen_choice(), ScreenChoice::Overscan);
        assert_eq!(r.screen_size(), (368, 283));
        press_cycle(&mut r);
        assert_eq!(r.screen_choice(), ScreenChoice::Custom);
        assert_eq!(
            r.screen_size(),
            (368, 283),
            "fields start from the overscan size"
        );
        assert_eq!(r.field_w.text(), "368");
        press_cycle(&mut r);
        assert_eq!(r.screen_choice(), ScreenChoice::Standard);
        assert_eq!(r.screen_size(), (320, 256));
    }

    #[test]
    fn typing_a_custom_size() {
        let mut r = req();
        press_cycle(&mut r);
        press_cycle(&mut r);
        r.handle(ReqEvent::Press(geom::NUM_W_X + 10, geom::SCREEN_SIZE_Y + 5));
        for _ in 0..3 {
            r.handle(ReqEvent::Key(ReqKey::Backspace));
        }
        type_str(&mut r, "352");
        r.handle(ReqEvent::Key(ReqKey::Char('x')));
        r.handle(ReqEvent::Key(ReqKey::Tab));
        for _ in 0..3 {
            r.handle(ReqEvent::Key(ReqKey::Backspace));
        }
        type_str(&mut r, "272");
        assert_eq!(r.screen_size(), (352, 272));
        assert_eq!(r.page_size(), (352, 272));
        match r.command() {
            Command::SetFormat {
                screen,
                width,
                height,
                ..
            } => {
                assert_eq!(screen, Some((352, 272)));
                assert_eq!((width, height), (352, 272));
            }
            other => panic!("unexpected {other:?}"),
        }
        // Empty field falls back to the standard size.
        r.handle(ReqEvent::Key(ReqKey::Home));
        for _ in 0..3 {
            r.handle(ReqEvent::Key(ReqKey::Delete));
        }
        assert_eq!(r.screen_size(), (352, 256));
    }

    #[test]
    fn mode_change_while_custom() {
        let mut r =
            ScreenFormatRequester::new(Target::default().with_screen(330, 230), (320, 256), 32);
        assert_eq!(r.screen_choice(), ScreenChoice::Custom);
        // NTSC LoRes (200..241 lines) also takes 330x230.
        let fits = r
            .modes
            .iter()
            .position(|m| m.screen_fits(330, 230) && *m != r.mode())
            .expect("a mode that fits");
        r.select(fits);
        assert_eq!(r.screen_size(), (330, 230));
        let nofit = r
            .modes
            .iter()
            .position(|m| !m.screen_fits(330, 230))
            .expect("a mode that does not fit");
        r.select(nofit);
        assert_eq!(r.screen_size(), r.mode().size());
        assert_eq!(r.field_w.text(), r.mode().size().0.to_string());
    }

    #[test]
    fn keep_same_grows_to_the_overscan_screen() {
        let mut r = req();
        press_cycle(&mut r);
        r.handle(ReqEvent::Press(geom::CYCLE_X + 50, geom::PAGE_SIZE_Y + 5));
        assert_eq!(r.page_size(), (368, 283));
    }

    #[test]
    fn info_panel_shows_size_lines() {
        let a = req();
        let mut b = req();
        press_cycle(&mut b);
        let (ia, ib) = (a.render(), b.render());
        assert_eq!((ib.width(), ib.height()), (640, 256));
        let differs = (geom::INFO_Y..geom::INFO_Y + geom::INFO_H).any(|y| {
            (geom::RIGHT_X..geom::RIGHT_X + geom::RIGHT_W).any(|x| ia.get(x, y) != ib.get(x, y))
        });
        assert!(differs);
        // Custom rendering with an active field also works.
        let mut c = req();
        press_cycle(&mut c);
        press_cycle(&mut c);
        c.handle(ReqEvent::Key(ReqKey::Tab));
        assert_eq!(c.render().width(), 640);
    }
}
