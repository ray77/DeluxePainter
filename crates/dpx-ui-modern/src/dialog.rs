//! The dialog kit of the Modern skin (ADR-47, slice 4a): a requester drawn as a card over
//! the dimmed window.
//!
//! A requester describes itself as a [`Dialog`] (`dpx_ui::dialog`): title, columns of items,
//! buttons, and for every part that takes the pointer a [`Control`] with the rectangle in
//! which the Classic requester takes its press. [`build`] lays that description out in
//! points, draws it and returns, beside the display list, a [`Spot`] for every control: its
//! rectangle in the window. A pointer position over a spot becomes the matching position
//! inside the control's Classic rectangle ([`Built::press`], [`Built::drag`]); the app feeds
//! it to the requester's own `handle` function as the Classic skin does. So the requester
//! keeps its state, its events, its keys and its answers, and the kit holds no dialog logic.
//!
//! The kit's parts: the card (title, rounded corners, shadow), buttons (default with the
//! accent, cancel, plain; under the pointer, held, ghosted), text and number fields with a
//! caret, cycle gadgets (a stepper, or two segments for a gadget with two values), check
//! boxes, sliders, radio rows, a list with a scroll bar, labels, headings, wrapped text
//! with bullets, a panel of facts with a message, credits, rules. Since slice 4b: radio
//! buttons inside a line, buttons and icon buttons inside a line (plain or latched), and the
//! palette as a grid of swatches or as a strip, with marks.

use dpx_ui::dialog::{
    Button, Check, Choice, Control, ControlId, Cycle, Dialog, Field, IconButton, Item, Kind,
    ListView, Mark, Part, Push, Radio, Role, Size, Slider, Swatches, Thumbnail, Tone,
};

use crate::font;
use crate::icons;
use crate::paint::{Color, List, Path, Pixels, Rect, Weight};
use crate::theme::Theme;

/// Free space between the card's edge and its content.
pub const PAD: f32 = 20.0;
/// Corner radius of the card.
pub const RADIUS: f32 = 12.0;
/// The least space the card keeps to the window's edge.
pub const MARGIN: f32 = 12.0;
/// How far the card's shadow reaches beyond it.
pub const SHADOW_REACH: f32 = 44.0;
/// Em size of the title.
pub const TITLE_SIZE: f32 = 15.0;
/// Em size of text, labels and buttons.
pub const SIZE: f32 = font::SIZE_BAR;
/// Em size of headings.
pub const SMALL: f32 = font::SIZE_PANEL;
/// Height of a button.
pub const BUTTON_H: f32 = 30.0;
/// Least width of a button.
pub const BUTTON_MIN_W: f32 = 84.0;
/// Space between two buttons.
pub const BUTTON_GAP: f32 = 8.0;
/// The position a pointer has for the Classic requester while it is over no control: outside
/// every requester's own rectangle.
pub const NOWHERE: (i32, i32) = (-1, -1);

/// How many faint cards make the shadow.
const SHADOW_STEPS: usize = 10;
const LINE_H: f32 = 19.0;
const CONTROL_RADIUS: f32 = 7.0;
const PART_GAP: f32 = 8.0;
const LABEL_GAP: f32 = 12.0;
const COLUMN_GAP: f32 = 24.0;
const CHECK: f32 = 18.0;
const KNOB_R: f32 = 8.0;
const SCROLL_W: f32 = 14.0;
const STEPPER_W: f32 = 116.0;
/// Space between two buttons that stand side by side in a line.
const GROUP_GAP: f32 = 6.0;
/// The least width a radio button takes inside a line, so that the radio buttons of
/// neighbouring lines stand under each other.
const RADIO_MIN_W: f32 = 96.0;
/// Width of an icon button, and the least width of a button in a line.
const ICON_W: f32 = 38.0;
const PUSH_MIN_W: f32 = 34.0;
/// The two inks of a mark on a swatch: together they show on every colour.
const MARK_DARK: Color = Color::rgb(20, 20, 24);
const MARK_LIGHT: Color = Color::rgb(255, 255, 255);

/// Sizes that give way in a low window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    /// Space between two items of a column.
    gap: f32,
    /// Height of a form line and of its controls.
    line: f32,
    control: f32,
    /// Height of a list row, a radio row and a row of facts.
    list_row: f32,
    choice_row: f32,
    fact_row: f32,
    /// Height of the title block and the space over the buttons.
    title: f32,
    footer_gap: f32,
    /// Side of a swatch of a swatch grid.
    swatch: f32,
    /// The most height a picture takes.
    picture: f32,
}

const ROOMY: Metrics = Metrics {
    gap: 10.0,
    line: 30.0,
    control: 28.0,
    list_row: 26.0,
    choice_row: 30.0,
    fact_row: 21.0,
    title: 50.0,
    footer_gap: 18.0,
    swatch: 24.0,
    picture: 220.0,
};

const COMPACT: Metrics = Metrics {
    gap: 6.0,
    line: 28.0,
    control: 26.0,
    list_row: 22.0,
    choice_row: 26.0,
    fact_row: 18.0,
    title: 44.0,
    footer_gap: 12.0,
    swatch: 19.0,
    picture: 150.0,
};

/// A control as it lies in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub control: Control,
    /// Where the control takes the pointer, in window points.
    pub rect: Rect,
    /// For a slider: its knob's centre runs from `rect.x + inset` to `rect.right() - inset`
    /// while the Classic knob (`knob_w` wide) runs through the control's area.
    slider: Option<(f32, i32)>,
}

impl Spot {
    /// The position in the Classic requester's coordinates that the window point (`x`, `y`)
    /// stands for. A point inside the spot always gives a point inside the control's area; a
    /// point outside (the pointer dragged away with the button held) gives one outside, in
    /// the same direction. A slider takes any point: beyond its ends it stays at the end.
    pub fn to_classic(&self, x: f32, y: f32) -> (i32, i32) {
        let area = self.control.area;
        if let Some((inset, knob_w)) = self.slider {
            let run = (self.rect.w - 2.0 * inset).max(1.0);
            let t = ((x - self.rect.x - inset) / run).clamp(0.0, 1.0);
            let free = (area.w - knob_w).max(0) as f32;
            return (
                area.x + knob_w / 2 + (t * free).round() as i32,
                area.y + area.h / 2,
            );
        }
        let along = |p: f32, from: f32, len: f32, to: i32, size: i32| {
            let t = (p - from) / len.max(f32::MIN_POSITIVE);
            let v = (t * size as f32).floor() as i32;
            // Inside stays inside (the float product may reach the far edge).
            let v = if (0.0..1.0).contains(&t) {
                v.clamp(0, size - 1)
            } else {
                v
            };
            to + v
        };
        (
            along(x, self.rect.x, self.rect.w, area.x, area.w),
            along(y, self.rect.y, self.rect.h, area.y, area.h),
        )
    }
}

/// One piece of text of a dialog with the box it has to fit in (for tests).
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    pub text: String,
    /// The rectangle the glyphs take: advance width, and the line around the capitals.
    pub ink: Rect,
    /// The part of the dialog the text belongs to.
    pub within: Rect,
    pub color: Color,
    /// The surface under the text.
    pub ground: Color,
}

/// The pointer over a dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pointer {
    /// In window points; `None` outside the window.
    pub at: Option<(f32, f32)>,
    /// The left button is held.
    pub down: bool,
    /// The control the button went down on.
    pub captured: Option<ControlId>,
}

/// A dialog laid out and drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    /// Shadow and card, in window points.
    pub list: List,
    pub card: Rect,
    /// What the drawing covers (the card and its shadow), cut to the window.
    pub bounds: Rect,
    /// The controls from back to front.
    pub spots: Vec<Spot>,
    pub texts: Vec<TextBox>,
    /// The low-window sizes were used.
    pub compact: bool,
    /// For a text field with caret stops: the window x of each caret position it shows,
    /// with the Classic x that puts the caret there.
    carets: Vec<(ControlId, Vec<(f32, i32)>)>,
}

impl Built {
    /// The Classic position the window point stands for on `spot`. In a text field with
    /// caret stops it is the stop nearest to the pointer; elsewhere [`Spot::to_classic`].
    fn classic(&self, spot: &Spot, x: f32, y: f32) -> (i32, i32) {
        let stops = self
            .carets
            .iter()
            .find(|(id, _)| *id == spot.control.id)
            .map(|(_, stops)| stops);
        let nearest = stops.and_then(|stops| {
            stops
                .iter()
                .min_by(|a, b| (a.0 - x).abs().total_cmp(&(b.0 - x).abs()))
        });
        match nearest {
            Some(&(_, cx)) if spot.rect.contains(x, y) => {
                (cx, spot.control.area.y + spot.control.area.h / 2)
            }
            _ => spot.to_classic(x, y),
        }
    }

    /// The control under the window point.
    pub fn spot_at(&self, x: f32, y: f32) -> Option<&Spot> {
        self.spots.iter().rev().find(|s| s.rect.contains(x, y))
    }

    /// The spot of control `id`.
    pub fn spot(&self, id: ControlId) -> Option<&Spot> {
        self.spots.iter().find(|s| s.control.id == id)
    }

    /// The left button goes down at the window point: the control that takes it (to be kept
    /// as [`Pointer::captured`] until the button is up) and the position for the Classic
    /// requester's `Press` event.
    pub fn press(&self, x: f32, y: f32) -> (Option<ControlId>, (i32, i32)) {
        match self.spot_at(x, y) {
            Some(spot) => (Some(spot.control.id), self.classic(spot, x, y)),
            None => (None, NOWHERE),
        }
    }

    /// The pointer moves to the window point with the button held since a press on
    /// `captured`: the position for the Classic requester's `Drag` event.
    pub fn drag(&self, captured: Option<ControlId>, x: f32, y: f32) -> (i32, i32) {
        captured
            .and_then(|id| self.spot(id))
            .map_or(NOWHERE, |spot| self.classic(spot, x, y))
    }
}

/// `text` wrapped at spaces into lines at most `width` wide; a word wider than a line is
/// broken by characters.
pub fn wrap(text: &str, width: f32, size: f32, weight: Weight) -> Vec<String> {
    let fits = |s: &str| font::measure(s, size, weight) <= width;
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let joined = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if fits(&joined) {
            cur = joined;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        for c in word.chars() {
            let mut next = cur.clone();
            next.push(c);
            if !cur.is_empty() && !fits(&next) {
                lines.push(std::mem::take(&mut cur));
                cur.push(c);
            } else {
                cur = next;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// The fill of a plain button: at rest, under the pointer, held.
pub fn button_fill(theme: &Theme, hover: bool, pressed: bool) -> Color {
    match (pressed, hover) {
        (true, _) => theme.hover.mix(theme.text, 16),
        (false, true) => theme.hover.mix(theme.text, 8),
        (false, false) => theme.hover,
    }
}

/// The fill of the default button: at rest and held the accent, lighter under the pointer.
pub fn default_fill(theme: &Theme, hover: bool, pressed: bool) -> Color {
    if hover && !pressed {
        theme.accent.mix(Color::rgb(255, 255, 255), 14)
    } else {
        theme.accent
    }
}

/// The width of the card a dialog of `size` asks for.
fn card_width(size: Size) -> f32 {
    match size {
        Size::Small => 400.0,
        Size::Medium => 480.0,
        Size::Large => 680.0,
    }
}

struct Draw<'a> {
    list: List,
    spots: Vec<Spot>,
    texts: Vec<TextBox>,
    carets: Vec<(ControlId, Vec<(f32, i32)>)>,
    theme: &'a Theme,
    pointer: &'a Pointer,
    m: Metrics,
    scale: f32,
}

impl Draw<'_> {
    fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }

    /// One line of text with its middle on `mid`; recorded with the box it belongs to.
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        x: f32,
        mid: f32,
        text: &str,
        size: f32,
        weight: Weight,
        color: Color,
        within: Rect,
        ground: Color,
    ) -> f32 {
        let w = font::measure(text, size, weight);
        if text.is_empty() {
            return 0.0;
        }
        self.list.text(
            x,
            font::baseline_centered(mid, size, weight),
            text,
            size,
            weight,
            color,
        );
        let cap = font::cap_height(size, weight);
        self.texts.push(TextBox {
            text: text.to_string(),
            ink: Rect::new(x, mid - cap / 2.0, w, cap),
            within,
            color,
            ground,
        });
        w
    }

    /// A line under the first `key` of `text` drawn at `x` (the letter is the part's key).
    #[allow(clippy::too_many_arguments)]
    fn underline(
        &mut self,
        x: f32,
        mid: f32,
        text: &str,
        key: Option<char>,
        size: f32,
        weight: Weight,
        color: Color,
    ) {
        let Some(at) = key.and_then(|k| text.find(k)) else {
            return;
        };
        let before = font::measure(&text[..at], size, weight);
        let letter = &text[at..at + key.map_or(1, char::len_utf8)];
        let w = font::measure(letter, size, weight);
        let y = self.snap(mid + font::cap_height(size, weight) / 2.0 + 2.5);
        self.list.rect(Rect::new(x + before, y, w, 1.0), color);
    }

    fn hovered(&self, id: ControlId, rect: &Rect) -> bool {
        let over = self.pointer.at.is_some_and(|(x, y)| rect.contains(x, y));
        match (self.pointer.down, self.pointer.captured) {
            (true, captured) => over && captured == Some(id),
            (false, _) => over,
        }
    }

    fn spot(&mut self, control: Control, rect: Rect) {
        self.spots.push(Spot {
            control,
            rect,
            slider: None,
        });
    }

    fn ink(&self, tone: Tone) -> Color {
        match tone {
            Tone::Normal | Tone::Alert => self.theme.text,
            Tone::Dim => self.theme.text_dim,
        }
    }

    /// A wrapped paragraph; returns its height.
    #[allow(clippy::too_many_arguments)]
    fn paragraph(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        text: &str,
        tone: Tone,
        bullet: bool,
        centred: bool,
    ) -> f32 {
        let theme = self.theme;
        let weight = if tone == Tone::Alert {
            Weight::Medium
        } else {
            Weight::Regular
        };
        let indent = if bullet || tone == Tone::Alert {
            14.0
        } else {
            0.0
        };
        let lines = wrap(text, w - indent, SIZE, weight);
        let h = lines.len().max(1) as f32 * LINE_H;
        let within = Rect::new(x, y, w, h);
        if bullet {
            self.list.fill(
                Path::new().circle(x + 4.0, y + LINE_H / 2.0, 2.2),
                theme.accent,
            );
        }
        if tone == Tone::Alert {
            // An accent bar marks the message; its text keeps the full contrast of text.
            self.list
                .round_rect(Rect::new(x, y + 2.0, 3.0, h - 4.0), 1.5, theme.accent);
        }
        let ink = self.ink(tone);
        for (i, line) in lines.iter().enumerate() {
            let mid = y + LINE_H * (i as f32 + 0.5);
            let lx = if centred {
                x + (w - font::measure(line, SIZE, weight)) / 2.0
            } else {
                x + indent
            };
            self.text(lx, mid, line, SIZE, weight, ink, within, theme.raised);
        }
        h
    }

    fn button(&mut self, b: &Button, rect: Rect) {
        let theme = self.theme;
        let hover = b.enabled && self.hovered(b.control.id, &rect);
        let pressed = b.enabled && b.pressed;
        let default = b.role == Role::Default;
        let (fill, ink) = match (b.enabled, default) {
            (false, _) => (theme.panel, theme.disabled),
            (true, true) => (default_fill(theme, hover, pressed), theme.on_accent),
            (true, false) => (button_fill(theme, hover, pressed), theme.text),
        };
        self.list.round_rect(rect, CONTROL_RADIUS, fill);
        if default && b.enabled {
            if pressed {
                // Held: a dark rim inside, the ink keeps its contrast.
                self.list
                    .ring(rect, CONTROL_RADIUS, theme.on_accent.with_opacity(45), 2.0);
            }
        } else {
            self.list.ring(rect, CONTROL_RADIUS, theme.hairline, 1.0);
        }
        let label = font::fit(&b.label, rect.w - 12.0, SIZE, Weight::Medium);
        let w = font::measure(&label, SIZE, Weight::Medium);
        let (x, mid) = (rect.x + (rect.w - w) / 2.0, rect.y + rect.h / 2.0);
        self.text(x, mid, &label, SIZE, Weight::Medium, ink, rect, fill);
        self.underline(x, mid, &label, b.key, SIZE, Weight::Medium, ink);
        self.spot(b.control, rect);
    }

    fn field_width(f: &Field) -> f32 {
        let digit = font::measure("0", SIZE, Weight::Regular);
        let wide = font::measure("M", SIZE, Weight::Regular);
        let per = if f.numeric { digit } else { wide };
        (f.max_chars.clamp(3, 32) as f32 * per + 22.0).ceil()
    }

    fn field(&mut self, f: &Field, rect: Rect) {
        let theme = self.theme;
        let active = f.caret.is_some() && f.enabled;
        self.list.round_rect(rect, CONTROL_RADIUS, theme.panel);
        if active {
            self.list.ring(rect, CONTROL_RADIUS, theme.accent, 1.5);
        } else {
            self.list.ring(rect, CONTROL_RADIUS, theme.hairline, 1.0);
        }
        let ink = if f.enabled {
            theme.text
        } else {
            theme.disabled
        };
        let (pad, mid) = (10.0, rect.y + rect.h / 2.0);
        let room = rect.w - 2.0 * pad;
        let chars: Vec<char> = f.text.chars().collect();
        let caret = f.caret.map_or(chars.len(), |c| c.min(chars.len()));
        let width = |from: usize, to: usize| {
            font::measure(
                &chars[from..to].iter().collect::<String>(),
                SIZE,
                Weight::Regular,
            )
        };
        // The first shown character: the caret stays in view.
        let mut start = 0;
        while start < caret && width(start, caret) > room - 2.0 {
            start += 1;
        }
        let mut end = chars.len();
        while end > caret.max(start) && width(start, end) > room {
            end -= 1;
        }
        let shown: String = chars[start..end].iter().collect();
        self.text(
            rect.x + pad,
            mid,
            &shown,
            SIZE,
            Weight::Regular,
            ink,
            rect,
            theme.panel,
        );
        if active {
            let x = self.snap(rect.x + pad + width(start, caret));
            self.list
                .rect(Rect::new(x, mid - 9.0, 1.5, 18.0), theme.text);
        }
        if f.enabled {
            self.spot(f.control, rect);
            // The caret positions that are shown here and can be reached there.
            let shown: Vec<(f32, i32)> = f
                .stops
                .iter()
                .filter(|(i, _)| (start..=end).contains(i))
                .map(|&(i, cx)| (rect.x + pad + width(start, i), cx))
                .collect();
            if !shown.is_empty() {
                self.carets.push((f.control.id, shown));
            }
        }
    }

    /// The natural width of a cycle gadget; `None` for a stepper, which takes what is left.
    fn segments_width(c: &Cycle) -> Option<f32> {
        (c.options.len() == 2).then(|| {
            let widest = c
                .options
                .iter()
                .map(|o| font::measure(o, SIZE, Weight::Medium))
                .fold(0.0, f32::max);
            2.0 * (widest + 24.0).ceil() + 4.0
        })
    }

    fn cycle(&mut self, c: &Cycle, rect: Rect) {
        let theme = self.theme;
        let mid = rect.y + rect.h / 2.0;
        if c.options.len() == 2 {
            // Two values side by side; a press on the other one switches.
            self.list.round_rect(rect, CONTROL_RADIUS, theme.panel);
            self.list.ring(rect, CONTROL_RADIUS, theme.hairline, 1.0);
            let w = (rect.w - 4.0) / 2.0;
            for (i, option) in c.options.iter().enumerate() {
                let seg = Rect::new(rect.x + 2.0 + i as f32 * w, rect.y + 2.0, w, rect.h - 4.0);
                let selected = i == c.selected;
                let hover = !selected && self.hovered(c.control.id, &seg);
                let (fill, ink, weight) = match (selected, c.enabled) {
                    (true, true) => (Some(theme.accent), theme.on_accent, Weight::Medium),
                    (true, false) => (Some(theme.hover), theme.text_dim, Weight::Medium),
                    (false, true) if hover => (Some(theme.hover), theme.text, Weight::Regular),
                    (false, true) => (None, theme.text_dim, Weight::Regular),
                    (false, false) => (None, theme.disabled, Weight::Regular),
                };
                if let Some(fill) = fill {
                    self.list.round_rect(seg, CONTROL_RADIUS - 2.0, fill);
                }
                let label = font::fit(option, seg.w - 8.0, SIZE, weight);
                let tw = font::measure(&label, SIZE, weight);
                self.text(
                    seg.x + (seg.w - tw) / 2.0,
                    mid,
                    &label,
                    SIZE,
                    weight,
                    ink,
                    seg,
                    fill.unwrap_or(theme.panel),
                );
                if !selected {
                    self.spot(c.control, seg);
                }
            }
            return;
        }
        // A stepper: the shown value, and a chevron for "next".
        let hover = c.enabled && self.hovered(c.control.id, &rect);
        let fill = button_fill(theme, hover, false);
        self.list.round_rect(rect, CONTROL_RADIUS, fill);
        self.list.ring(rect, CONTROL_RADIUS, theme.hairline, 1.0);
        let (ink, mark) = if c.enabled {
            (theme.text, theme.text_dim)
        } else {
            (theme.disabled, theme.disabled)
        };
        let value = c.options.get(c.selected).map_or("", String::as_str);
        let label = font::fit(value, rect.w - 38.0, SIZE, Weight::Regular);
        self.text(
            rect.x + 12.0,
            mid,
            &label,
            SIZE,
            Weight::Regular,
            ink,
            rect,
            fill,
        );
        self.list.stroke(
            icons::chevron(rect.right() - 14.0, mid),
            mark,
            icons::MARK_STROKE,
        );
        self.spot(c.control, rect);
    }

    fn check_width(c: &Check) -> f32 {
        CHECK + 8.0 + font::measure(&c.label, SIZE, Weight::Regular)
    }

    fn check(&mut self, c: &Check, rect: Rect) {
        let theme = self.theme;
        let mid = rect.y + rect.h / 2.0;
        let boxed = Rect::new(rect.x, self.snap(mid - CHECK / 2.0), CHECK, CHECK);
        let hover = c.enabled && self.hovered(c.control.id, &rect);
        if c.on && c.enabled {
            self.list.round_rect(boxed, 5.0, theme.accent);
            self.list.stroke(
                icons::check_mark(boxed.x + CHECK / 2.0, boxed.y + CHECK / 2.0),
                theme.on_accent,
                icons::MARK_STROKE,
            );
        } else {
            let fill = if hover { theme.hover } else { theme.panel };
            self.list.round_rect(boxed, 5.0, fill);
            let rim = if c.enabled {
                theme.text_dim
            } else {
                theme.disabled
            };
            self.list.ring(boxed, 5.0, rim, 1.5);
            if c.on {
                self.list.stroke(
                    icons::check_mark(boxed.x + CHECK / 2.0, boxed.y + CHECK / 2.0),
                    theme.disabled,
                    icons::MARK_STROKE,
                );
            }
        }
        let ink = if c.enabled {
            theme.text
        } else {
            theme.disabled
        };
        let x = rect.x + CHECK + 8.0;
        self.text(
            x,
            mid,
            &c.label,
            SIZE,
            Weight::Regular,
            ink,
            rect,
            theme.raised,
        );
        self.underline(x, mid, &c.label, c.key, SIZE, Weight::Regular, ink);
        self.spot(c.control, rect);
    }

    fn slider(&mut self, s: &Slider, rect: Rect) {
        let theme = self.theme;
        let mid = rect.y + rect.h / 2.0;
        // The value in words has a fixed place at the right, so the track does not jump.
        let value_w = font::measure("0000", SIZE, Weight::Regular).ceil();
        let track = Rect::new(rect.x, rect.y, rect.w - value_w - 10.0, rect.h);
        let free = (s.control.area.w - s.knob.w).max(1) as f32;
        let t = ((s.knob.x - s.control.area.x) as f32 / free).clamp(0.0, 1.0);
        let run = track.w - 2.0 * KNOB_R;
        let cx = track.x + KNOB_R + t * run;
        let groove = Rect::new(track.x, self.snap(mid - 2.0), track.w, 4.0);
        self.list.round_rect(groove, 2.0, theme.hairline);
        if !s.enabled {
            // Nothing to set yet: the groove alone, the value faint, no pointer.
            let w = font::measure(&s.value, SIZE, Weight::Regular);
            self.text(
                rect.right() - w,
                mid,
                &s.value,
                SIZE,
                Weight::Regular,
                theme.disabled,
                rect,
                theme.raised,
            );
            return;
        }
        self.list.round_rect(
            Rect::new(groove.x, groove.y, (cx - groove.x).max(4.0), 4.0),
            2.0,
            theme.accent,
        );
        let hover = self.hovered(s.control.id, &track);
        let r = if hover { KNOB_R + 1.0 } else { KNOB_R };
        self.list.fill(Path::new().circle(cx, mid, r), theme.accent);
        self.list
            .fill(Path::new().circle(cx, mid, r - 2.5), theme.raised);
        let w = font::measure(&s.value, SIZE, Weight::Regular);
        self.text(
            rect.right() - w,
            mid,
            &s.value,
            SIZE,
            Weight::Regular,
            theme.text,
            rect,
            theme.raised,
        );
        self.spots.push(Spot {
            control: s.control,
            rect: track,
            slider: Some((KNOB_R, s.knob.w)),
        });
    }

    fn radio_width(r: &Radio) -> f32 {
        (CHECK + 8.0 + font::measure(&r.label, SIZE, Weight::Regular)).ceil()
    }

    /// A radio button inside a line: a ring, with the accent and a dot when chosen.
    fn radio(&mut self, r: &Radio, rect: Rect) {
        let theme = self.theme;
        let mid = rect.y + rect.h / 2.0;
        let (cx, radius) = (rect.x + CHECK / 2.0, CHECK / 2.0 - 1.5);
        let hover = r.enabled && !r.on && self.hovered(r.control.id, &rect);
        let (rim, ink) = match (r.enabled, r.on) {
            (false, _) => (theme.disabled, theme.disabled),
            (true, true) => (theme.accent, theme.text),
            (true, false) => (theme.text_dim, theme.text),
        };
        let well = if hover { theme.hover } else { theme.panel };
        self.list.fill(Path::new().circle(cx, mid, radius), well);
        self.list
            .stroke(Path::new().circle(cx, mid, radius - 0.75), rim, 1.5);
        if r.on {
            self.list.fill(Path::new().circle(cx, mid, 3.5), rim);
        }
        self.text(
            rect.x + CHECK + 8.0,
            mid,
            &r.label,
            SIZE,
            Weight::Regular,
            ink,
            rect,
            theme.raised,
        );
        if r.enabled {
            self.spot(r.control, rect);
        }
    }

    /// The fill and the ink of a button in a line: latched on the accent, else quiet.
    fn push_look(&self, on: bool, enabled: bool, hover: bool, pressed: bool) -> (Color, Color) {
        let theme = self.theme;
        match (enabled, on) {
            (false, true) => (theme.hover, theme.text_dim),
            (false, false) => (theme.panel, theme.disabled),
            (true, true) => (default_fill(theme, hover, pressed), theme.on_accent),
            (true, false) => (button_fill(theme, hover, pressed), theme.text),
        }
    }

    fn push_width(b: &Push) -> f32 {
        (font::measure(&b.label, SIZE, Weight::Medium) + 24.0)
            .ceil()
            .max(PUSH_MIN_W)
    }

    /// A button inside a line.
    fn push(&mut self, b: &Push, rect: Rect) {
        let theme = self.theme;
        let hover = b.enabled && self.hovered(b.control.id, &rect);
        let pressed = b.enabled && b.pressed;
        let (fill, ink) = self.push_look(b.on, b.enabled, hover, pressed);
        self.list.round_rect(rect, CONTROL_RADIUS, fill);
        if !(b.on && b.enabled) {
            self.list.ring(rect, CONTROL_RADIUS, theme.hairline, 1.0);
        } else if pressed {
            self.list
                .ring(rect, CONTROL_RADIUS, theme.on_accent.with_opacity(45), 2.0);
        }
        let label = font::fit(&b.label, rect.w - 8.0, SIZE, Weight::Medium);
        let w = font::measure(&label, SIZE, Weight::Medium);
        self.text(
            rect.x + (rect.w - w) / 2.0,
            rect.y + rect.h / 2.0,
            &label,
            SIZE,
            Weight::Medium,
            ink,
            rect,
            fill,
        );
        if b.enabled {
            self.spot(b.control, rect);
        }
    }

    /// A button with a pictogram inside a line.
    fn icon(&mut self, b: &IconButton, rect: Rect) {
        let theme = self.theme;
        let hover = b.enabled && self.hovered(b.control.id, &rect);
        let pressed = b.enabled && b.pressed;
        let (fill, ink) = self.push_look(b.on, b.enabled, hover, pressed);
        self.list.round_rect(rect, CONTROL_RADIUS, fill);
        if !(b.on && b.enabled) {
            self.list.ring(rect, CONTROL_RADIUS, theme.hairline, 1.0);
        }
        let (cx, cy) = rect.center();
        let (filled, stroked) = icons::glyph(b.glyph, cx, cy);
        if !filled.is_empty() {
            self.list.fill(filled, ink);
        }
        if !stroked.is_empty() {
            self.list.stroke(stroked, ink, icons::MARK_STROKE);
        }
        if b.enabled {
            self.spot(b.control, rect);
        }
    }

    /// The palette as swatches; returns the height. The whole grid is one control, laid
    /// out with equal cells like the Classic one, so a point on a swatch is a point on the
    /// same swatch there.
    fn swatches(&mut self, g: &Swatches, x: f32, y: f32, w: f32) -> f32 {
        let theme = self.theme;
        let (cols, rows) = (g.cols.max(1), g.rows());
        let (cw, ch, x0) = if g.strip {
            let ch = (self.m.swatch * 2.0 / rows as f32).clamp(9.0, self.m.swatch + 4.0);
            (w / cols as f32, self.snap(ch), x)
        } else {
            // A few rows use the column's width; many rows keep the card low.
            let most = if rows <= 4 { 32.0 } else { self.m.swatch };
            let side = ((w / cols as f32).min(most) * self.scale).floor() / self.scale;
            (side, side, self.snap(x + (w - side * cols as f32) / 2.0))
        };
        let frame = Rect::new(x0, y, cw * cols as f32, ch * rows as f32);
        if g.strip {
            self.list.rect(frame, theme.panel);
        }
        for (i, cell) in g.cells.iter().enumerate() {
            let (col, row) = ((i % cols) as f32, (i / cols) as f32);
            let (left, right) = (self.snap(x0 + col * cw), self.snap(x0 + (col + 1.0) * cw));
            let outer = Rect::new(left, y + row * ch, right - left, ch);
            let fill = Color::rgb(cell.rgb[0], cell.rgb[1], cell.rgb[2]);
            let (cx, cy) = outer.center();
            let small = outer.w.min(outer.h);
            if g.strip {
                self.list.rect(outer, fill);
            } else {
                let face = outer.inset(1.5);
                self.list.round_rect(face, 3.0, fill);
                self.list.ring(face, 3.0, theme.hairline, 1.0);
            }
            match cell.mark {
                Mark::None => {}
                Mark::Chosen if g.strip => {
                    let r = (small / 2.0 - 2.0).clamp(2.0, 4.5);
                    self.list.fill(Path::new().circle(cx, cy, r), MARK_DARK);
                    self.list
                        .fill(Path::new().circle(cx, cy, (r - 1.5).max(1.0)), MARK_LIGHT);
                }
                Mark::Chosen => {
                    self.list
                        .fill(Path::new().circle(cx, cy, small * 0.32), MARK_DARK);
                    let k = (small / 24.0).min(1.0);
                    self.list.stroke(
                        icons::check_mark(0.0, 0.0).placed(k, cx, cy),
                        MARK_LIGHT,
                        icons::MARK_STROKE,
                    );
                }
                Mark::Picked => {
                    self.list.ring(outer.inset(0.5), 4.0, theme.accent, 2.0);
                    self.list.fill(Path::new().circle(cx, cy, 4.0), MARK_DARK);
                    self.list.fill(Path::new().circle(cx, cy, 2.5), MARK_LIGHT);
                }
                Mark::Target => {
                    self.list.ring(outer.inset(0.5), 4.0, theme.text, 2.0);
                    self.list.ring(outer.inset(2.5), 2.0, theme.raised, 1.0);
                }
            }
        }
        if g.strip {
            self.list.ring(frame, 0.0, theme.hairline, 1.0);
        }
        self.spot(g.control, frame);
        frame.h
    }

    /// A true-colour picture in a frame, centred in the column; returns the height. One
    /// picture pixel is a whole number of points where the column and the low-window limit
    /// allow it, else the picture is fitted into them.
    fn picture(
        &mut self,
        width: u32,
        height: u32,
        picture: Option<&Thumbnail>,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let theme = self.theme;
        let (pw, ph) = (width.max(1) as f32, height.max(1) as f32);
        // The frame: a hairline and a gap around the picture.
        let rim = self.snap(5.0);
        let room = ((w - 2.0 * rim).max(8.0), self.m.picture);
        let fit = (room.0 / pw).min(room.1 / ph);
        let k = if fit >= 1.0 {
            fit.floor().min(3.0)
        } else {
            fit
        };
        let (iw, ih) = (self.snap(pw * k).max(1.0), self.snap(ph * k).max(1.0));
        let frame = Rect::new(
            self.snap(x + (w - iw) / 2.0 - rim),
            y,
            iw + 2.0 * rim,
            ih + 2.0 * rim,
        );
        self.list.round_rect(frame, CONTROL_RADIUS, theme.panel);
        self.list.ring(frame, CONTROL_RADIUS, theme.hairline, 1.0);
        let inner = Rect::new(frame.x + rim, frame.y + rim, iw, ih);
        match picture {
            Some(p) if p.rgba.len() as u64 >= u64::from(p.width) * u64::from(p.height) * 4 => {
                self.list.pixels(
                    inner,
                    Pixels {
                        width: p.width,
                        height: p.height,
                        rgba: p.rgba.clone(),
                    },
                );
            }
            // Not made yet: the empty well.
            _ => self.list.rect(inner, theme.canvas),
        }
        frame.h
    }

    /// The width a part asks for; `None` for a stepper or a slider, which take what is left.
    fn natural(p: &Part) -> Option<f32> {
        match p {
            // A field for long text (a file name, a prompt) takes what the line has left.
            Part::Field(f) if !f.numeric && f.max_chars > 32 => None,
            Part::Field(f) => Some(Self::field_width(f)),
            Part::Cycle(c) => Self::segments_width(c),
            Part::Check(c) => Some(Self::check_width(c)),
            Part::Slider(_) => None,
            Part::Text(t, _) => Some(font::measure(t, SIZE, Weight::Regular).ceil()),
            Part::Radio(r) => Some(Self::radio_width(r)),
            Part::Button(b) => Some(Self::push_width(b)),
            Part::Icon(_) => Some(ICON_W),
        }
    }

    /// The parts of a form line from `x` on, in a line `w` wide. `pad_radios`: radio
    /// buttons take at least [`RADIO_MIN_W`].
    fn parts(&mut self, parts: &[Part], x: f32, y: f32, w: f32, pad_radios: bool) {
        let theme = self.theme;
        let (line, ch) = (self.m.line, self.m.control);
        let top = self.snap(y + (line - ch) / 2.0);
        let mid = y + line / 2.0;
        // Radio buttons give up their common width in a line too narrow for it.
        let plain: f32 = parts.iter().filter_map(Self::natural).sum::<f32>()
            + PART_GAP * parts.len().saturating_sub(1) as f32;
        let padded: f32 = parts
            .iter()
            .filter_map(|p| match p {
                Part::Radio(r) => Some(Self::radio_width(r).max(RADIO_MIN_W)),
                p => Self::natural(p),
            })
            .sum::<f32>()
            + PART_GAP * parts.len().saturating_sub(1) as f32;
        let pad_radios = pad_radios && (padded <= w || plain > w);
        let natural = |p: &Part| -> Option<f32> {
            match p {
                Part::Radio(r) if pad_radios => Some(Self::radio_width(r).max(RADIO_MIN_W)),
                p => Self::natural(p),
            }
        };
        // Buttons side by side stand closer together than other parts.
        let gap_after = |i: usize| -> f32 {
            match (&parts[i], parts.get(i + 1)) {
                (_, None) => 0.0,
                (Part::Button(_), Some(Part::Button(_))) | (Part::Icon(_), Some(Part::Icon(_))) => {
                    GROUP_GAP
                }
                _ => PART_GAP,
            }
        };
        // The last stretchy part (a stepper or a slider) takes what the line has left; a
        // stepper before it has a fixed width.
        let last_stretchy = parts.iter().rposition(|p| natural(p).is_none());
        let fixed: f32 = parts
            .iter()
            .enumerate()
            .map(|(i, p)| match natural(p) {
                Some(w) => w,
                None if Some(i) == last_stretchy => 0.0,
                None => STEPPER_W,
            })
            .sum::<f32>()
            + (0..parts.len()).map(gap_after).sum::<f32>();
        let rest = (w - fixed).max(60.0);
        let mut px = x;
        for (i, part) in parts.iter().enumerate() {
            let pw = match natural(part) {
                Some(w) => w,
                None if Some(i) == last_stretchy => rest,
                None => STEPPER_W,
            };
            let mut rect = Rect::new(self.snap(px), top, self.snap(pw), ch);
            if matches!(part, Part::Field(_)) && natural(part).is_none() {
                // A field that takes the rest of the line ends on a pixel inside it.
                rect.w = ((px + pw) * self.scale).floor() / self.scale - rect.x;
            }
            match part {
                Part::Field(f) => self.field(f, rect),
                Part::Cycle(c) => self.cycle(c, rect),
                Part::Check(c) => self.check(c, rect),
                Part::Slider(s) => self.slider(s, rect),
                Part::Radio(r) => self.radio(r, rect),
                Part::Button(b) => self.push(b, rect),
                Part::Icon(b) => self.icon(b, rect),
                Part::Text(t, tone) => {
                    let ink = self.ink(*tone);
                    // A text that would leave the line ends in an ellipsis.
                    let t = &font::fit(t, (x + w - rect.x).max(0.0), SIZE, Weight::Regular);
                    let rect = Rect::new(rect.x, rect.y, rect.w.min(x + w - rect.x), rect.h);
                    self.text(
                        rect.x,
                        mid,
                        t,
                        SIZE,
                        Weight::Regular,
                        ink,
                        rect,
                        theme.raised,
                    );
                }
            }
            px += pw + gap_after(i);
        }
    }

    fn choices(&mut self, rows: &[Choice], x: f32, y: f32, w: f32) -> f32 {
        let theme = self.theme;
        let row_h = self.m.choice_row;
        for (i, row) in rows.iter().enumerate() {
            let rect = Rect::new(x, self.snap(y + i as f32 * row_h), w, row_h);
            let hover = row.enabled && self.hovered(row.control.id, &rect);
            let ground = if row.pressed {
                Some(theme.hover.mix(theme.text, 8))
            } else if hover {
                Some(theme.hover)
            } else if row.selected {
                Some(theme.panel)
            } else {
                None
            };
            let face = rect.inset(1.0);
            if let Some(fill) = ground {
                self.list.round_rect(face, CONTROL_RADIUS, fill);
            }
            let mid = rect.y + rect.h / 2.0;
            let (cx, r) = (rect.x + 17.0, 7.0);
            let (rim, ink, dim) = match (row.enabled, row.selected) {
                (false, _) => (theme.disabled, theme.disabled, theme.disabled),
                (true, true) => (theme.accent, theme.text, theme.text_dim),
                (true, false) => (theme.text_dim, theme.text, theme.text_dim),
            };
            self.list
                .stroke(Path::new().circle(cx, mid, r - 0.75), rim, 1.5);
            if row.selected {
                self.list.fill(Path::new().circle(cx, mid, 3.5), rim);
            }
            let under = ground.unwrap_or(theme.raised);
            let detail_w = font::measure(&row.detail, SIZE, Weight::Regular);
            let label = font::fit(
                &row.label,
                rect.w - 34.0 - detail_w - 24.0,
                SIZE,
                Weight::Regular,
            );
            self.text(
                rect.x + 34.0,
                mid,
                &label,
                SIZE,
                Weight::Regular,
                ink,
                rect,
                under,
            );
            self.text(
                rect.right() - 12.0 - detail_w,
                mid,
                &row.detail,
                SIZE,
                Weight::Regular,
                dim,
                rect,
                under,
            );
            self.spot(row.control, rect);
        }
        rows.len() as f32 * row_h
    }

    fn list_view(&mut self, view: &ListView, x: f32, y: f32, w: f32) -> f32 {
        let theme = self.theme;
        let row_h = self.m.list_row;
        let h = view.capacity as f32 * row_h + 8.0;
        let frame = Rect::new(x, y, w, h);
        self.list
            .round_rect(frame, CONTROL_RADIUS + 1.0, theme.panel);
        self.list
            .ring(frame, CONTROL_RADIUS + 1.0, theme.hairline, 1.0);
        let bar_w = if view.scroll.is_some() { SCROLL_W } else { 0.0 };
        for (i, row) in view.rows.iter().enumerate() {
            let rect = Rect::new(
                x + 4.0,
                self.snap(y + 4.0 + i as f32 * row_h),
                w - 8.0 - bar_w,
                row_h,
            );
            let hover = self.hovered(row.control.id, &rect);
            let (fill, ink, dim) = if row.selected {
                (Some(theme.accent), theme.on_accent, theme.on_accent)
            } else if hover {
                (Some(theme.hover), theme.text, theme.text_dim)
            } else {
                (None, theme.text, theme.text_dim)
            };
            if let Some(fill) = fill {
                self.list.round_rect(rect, 5.0, fill);
            }
            let under = fill.unwrap_or(theme.panel);
            let mid = rect.y + rect.h / 2.0;
            let detail_w = font::measure(&row.detail, SIZE, Weight::Regular);
            let label = font::fit(
                &row.label,
                rect.w - 20.0 - detail_w - 12.0,
                SIZE,
                Weight::Regular,
            );
            self.text(
                rect.x + 10.0,
                mid,
                &label,
                SIZE,
                Weight::Regular,
                ink,
                rect,
                under,
            );
            self.text(
                rect.right() - 10.0 - detail_w,
                mid,
                &row.detail,
                SIZE,
                Weight::Regular,
                dim,
                rect,
                under,
            );
            self.spot(row.control, rect);
        }
        if let Some(bar) = &view.scroll {
            let track = Rect::new(frame.right() - 4.0 - 10.0, y + 4.0, 10.0, h - 8.0);
            let area = bar.control.area;
            let span = area.h.max(1) as f32;
            let top = track.y + (bar.knob.y - area.y) as f32 / span * track.h;
            let len = (bar.knob.h as f32 / span * track.h).max(8.0);
            let knob = Rect::new(track.x + 2.0, top, 6.0, len.min(track.bottom() - top));
            let hover = self.hovered(bar.control.id, &track);
            let ink = if hover {
                theme.text_dim
            } else {
                theme.disabled
            };
            self.list.round_rect(knob, 3.0, ink);
            self.spot(bar.control, track);
        }
        h
    }

    fn facts(
        &mut self,
        rows: &[(String, String)],
        message: Option<&str>,
        x: f32,
        y: f32,
        w: f32,
    ) -> f32 {
        let theme = self.theme;
        let row_h = self.m.fact_row;
        let h = rows.len() as f32 * row_h + 14.0;
        let frame = Rect::new(x, y, w, h);
        self.list
            .round_rect(frame, CONTROL_RADIUS + 1.0, theme.panel);
        let names = rows
            .iter()
            .map(|(n, _)| font::measure(n, SIZE, Weight::Regular))
            .fold(0.0, f32::max)
            .ceil();
        // A message stands at the panel's bottom; the facts it would cover are left out,
        // never shown in part (as in the Classic skin).
        let lines = message.map(|m| {
            let mut lines = wrap(m, w - 24.0 - 12.0, SIZE, Weight::Medium);
            lines.truncate(4);
            lines
        });
        let message_h = lines
            .as_ref()
            .map_or(0.0, |l| l.len() as f32 * LINE_H + 8.0);
        let limit = frame.bottom() - 7.0 - message_h;
        for (i, (name, value)) in rows.iter().enumerate() {
            let top = y + 7.0 + i as f32 * row_h;
            if top + row_h > limit + 0.01 {
                break;
            }
            let mid = top + row_h / 2.0;
            self.text(
                x + 12.0,
                mid,
                name,
                SIZE,
                Weight::Regular,
                theme.text_dim,
                frame,
                theme.panel,
            );
            let room = w - 24.0 - names - 12.0;
            let value = font::fit(value, room, SIZE, Weight::Regular);
            self.text(
                x + 12.0 + names + 12.0,
                mid,
                &value,
                SIZE,
                Weight::Regular,
                theme.text,
                frame,
                theme.panel,
            );
        }
        if let Some(lines) = lines {
            let top = frame.bottom() - 7.0 - lines.len() as f32 * LINE_H;
            self.list.round_rect(
                Rect::new(x + 12.0, top + 2.0, 3.0, lines.len() as f32 * LINE_H - 4.0),
                1.5,
                theme.accent,
            );
            for (i, line) in lines.iter().enumerate() {
                self.text(
                    x + 24.0,
                    top + LINE_H * (i as f32 + 0.5),
                    line,
                    SIZE,
                    Weight::Medium,
                    theme.text,
                    frame,
                    theme.panel,
                );
            }
        }
        h
    }

    fn credits(&mut self, lines: &[String], x: f32, y: f32, w: f32) -> f32 {
        let theme = self.theme;
        let row_h = self.m.fact_row;
        let h = lines.len() as f32 * row_h + 14.0;
        let frame = Rect::new(x, y, w, h);
        self.list
            .round_rect(frame, CONTROL_RADIUS + 1.0, theme.panel);
        for (i, line) in lines.iter().enumerate() {
            let (weight, ink) = if i == 0 {
                (Weight::Medium, theme.text)
            } else {
                (Weight::Regular, theme.text_dim)
            };
            let line = font::fit(line, w - 16.0, SIZE, weight);
            let tw = font::measure(&line, SIZE, weight);
            self.text(
                x + (w - tw) / 2.0,
                y + 7.0 + row_h * (i as f32 + 0.5),
                &line,
                SIZE,
                weight,
                ink,
                frame,
                theme.panel,
            );
        }
        h
    }

    /// The program's name line of an About text: the name large, an accent line under it,
    /// then what follows the colon.
    fn about_head(&mut self, x: f32, y: f32, w: f32, text: &str) -> f32 {
        let theme = self.theme;
        let (name, rest) = text.split_once(": ").unwrap_or((text, ""));
        let (size, weight) = (22.0, Weight::Medium);
        let name = font::fit(name, w, size, weight);
        let tw = font::measure(&name, size, weight);
        let within = Rect::new(x, y, w, 40.0);
        self.text(
            x + (w - tw) / 2.0,
            y + 16.0,
            &name,
            size,
            weight,
            theme.text,
            within,
            theme.raised,
        );
        self.list.round_rect(
            Rect::new(x + (w - 44.0) / 2.0, self.snap(y + 34.0), 44.0, 3.0),
            1.5,
            theme.accent,
        );
        let mut h = 48.0;
        if !rest.is_empty() {
            let mut line = rest.to_string();
            if let Some(first) = line.get(..1) {
                line = first.to_uppercase() + &line[1..];
            }
            h += self.paragraph(x, y + h, w, &line, Tone::Normal, false, true);
        }
        h
    }

    /// One column from `y` down; returns its height.
    fn column(&mut self, items: &[Item], kind: Kind, x: f32, y: f32, w: f32) -> f32 {
        let theme = self.theme;
        let label_w = items
            .iter()
            .filter_map(|i| match i {
                Item::Line { label, .. } | Item::Setting { label, .. } if !label.is_empty() => {
                    Some(font::measure(label, SIZE, Weight::Regular))
                }
                _ => None,
            })
            .fold(0.0, f32::max);
        // The first part of a row with more parts stands in the label column too.
        let label_w = items
            .iter()
            .filter_map(|i| match i {
                Item::Row(parts) if parts.len() > 1 => Self::natural(&parts[0]),
                _ => None,
            })
            .fold(label_w, f32::max);
        let control_x = if label_w > 0.0 {
            (label_w + LABEL_GAP).ceil()
        } else {
            0.0
        };
        let mut at = y;
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                // A heading belongs to what follows it.
                at += match &items[i - 1] {
                    Item::Heading(_) => 6.0,
                    Item::Text { .. } if matches!(item, Item::Text { .. }) => self.m.gap.min(8.0),
                    // Rows of radio buttons belong together.
                    Item::Row(_) if matches!(item, Item::Row(_)) => 4.0,
                    _ => self.m.gap,
                };
            }
            let top = self.snap(at);
            at = top
                + match item {
                    Item::Heading(text) => {
                        let within = Rect::new(x, top, w, 16.0);
                        let text = font::fit(text, w, SMALL, Weight::Medium);
                        self.text(
                            x,
                            top + 8.0,
                            &text,
                            SMALL,
                            Weight::Medium,
                            theme.text_dim,
                            within,
                            theme.raised,
                        );
                        16.0
                    }
                    Item::Text { text, .. } if kind == Kind::About && i == 0 => {
                        self.about_head(x, top, w, text)
                    }
                    Item::Text { text, tone, bullet } => {
                        let (tone, centred) = match kind {
                            Kind::About => (Tone::Dim, true),
                            Kind::Plain => (*tone, false),
                        };
                        self.paragraph(x, top, w, text, tone, *bullet, centred)
                    }
                    Item::Line { label, key, parts } => {
                        let mid = top + self.m.line / 2.0;
                        let within = Rect::new(x, top, control_x.max(1.0), self.m.line);
                        self.text(
                            x,
                            mid,
                            label,
                            SIZE,
                            Weight::Regular,
                            theme.text_dim,
                            within,
                            theme.raised,
                        );
                        self.underline(x, mid, label, *key, SIZE, Weight::Regular, theme.text_dim);
                        self.parts(parts, x + control_x, top, w - control_x, true);
                        self.m.line
                    }
                    Item::Setting {
                        label,
                        focused,
                        enabled,
                        parts,
                    } => {
                        let mid = top + self.m.line / 2.0;
                        let within = Rect::new(x, top, control_x.max(1.0), self.m.line);
                        let lw = font::measure(label, SIZE, Weight::Regular);
                        // The option the arrow keys change: its label on a quiet bar with
                        // the accent around it, like the field that has the keys.
                        let (ink, ground) = match (*enabled, *focused) {
                            (false, _) => (theme.disabled, theme.raised),
                            (true, true) => {
                                let bar = Rect::new(
                                    x - 7.0,
                                    self.snap(mid - 11.0),
                                    (lw + 14.0).ceil(),
                                    22.0,
                                );
                                self.list.round_rect(bar, 6.0, theme.hover);
                                self.list.ring(bar, 6.0, theme.accent, 1.5);
                                (theme.text, theme.hover)
                            }
                            (true, false) => (theme.text_dim, theme.raised),
                        };
                        self.text(x, mid, label, SIZE, Weight::Regular, ink, within, ground);
                        self.parts(parts, x + control_x, top, w - control_x, true);
                        self.m.line
                    }
                    Item::Picture {
                        width,
                        height,
                        picture,
                    } => self.picture(*width, *height, picture.as_ref(), x, top, w),
                    Item::Row(parts) => {
                        if let Some((first, rest)) = parts.split_first() {
                            self.parts(std::slice::from_ref(first), x, top, w, false);
                            if !rest.is_empty() {
                                self.parts(rest, x + control_x, top, w - control_x, true);
                            }
                        }
                        self.m.line
                    }
                    Item::Swatches(grid) => self.swatches(grid, x, top, w),
                    Item::Choices(rows) => self.choices(rows, x, top, w),
                    Item::List(view) => self.list_view(view, x, top, w),
                    Item::Facts { rows, message } => {
                        self.facts(rows, message.as_deref(), x, top, w)
                    }
                    Item::Credits(lines) => self.credits(lines, x, top, w),
                    Item::Rule => {
                        self.list.rect(Rect::new(x, top, w, 1.0), theme.hairline);
                        1.0
                    }
                };
        }
        at - y
    }
}

/// A dialog's content at a place: the drawing, the controls, the texts and the card's size.
struct Laid {
    list: List,
    spots: Vec<Spot>,
    texts: Vec<TextBox>,
    carets: Vec<(ControlId, Vec<(f32, i32)>)>,
    size: (f32, f32),
}

/// Lays `dialog` out with the card's top-left corner at (`x`, `y`).
fn lay_out(
    dialog: &Dialog,
    (x, y): (f32, f32),
    width: f32,
    m: Metrics,
    scale: f32,
    theme: &Theme,
    pointer: &Pointer,
) -> Laid {
    let mut d = Draw {
        list: List::new(),
        spots: Vec::new(),
        texts: Vec::new(),
        carets: Vec::new(),
        theme,
        pointer,
        m,
        scale,
    };
    let inner = width - 2.0 * PAD;
    // Title.
    let title = font::fit(&dialog.title, inner, TITLE_SIZE, Weight::Medium);
    let title_box = Rect::new(x + PAD, y, inner, m.title);
    d.text(
        x + PAD,
        y + m.title / 2.0 + 2.0,
        &title,
        TITLE_SIZE,
        Weight::Medium,
        theme.text,
        title_box,
        theme.raised,
    );
    // Columns: one, or two with the left one wider.
    let top = y + m.title;
    let widths: Vec<f32> = match dialog.columns.len() {
        0 | 1 => vec![inner],
        n => {
            let free = inner - COLUMN_GAP * (n as f32 - 1.0);
            let first = (free * 0.6).round();
            let rest = (free - first) / (n as f32 - 1.0);
            std::iter::once(first)
                .chain(std::iter::repeat_n(rest, n - 1))
                .collect()
        }
    };
    let mut cx = x + PAD;
    let mut body: f32 = 0.0;
    for (items, w) in dialog.columns.iter().zip(&widths) {
        body = body.max(d.column(items, dialog.kind, d.snap(cx), top, *w));
        cx += w + COLUMN_GAP;
    }
    // Buttons: right-aligned, in the Classic order.
    let by = d.snap(top + body + m.footer_gap);
    let sizes: Vec<f32> = dialog
        .buttons
        .iter()
        .map(|b| {
            (font::measure(&b.label, SIZE, Weight::Medium) + 36.0)
                .ceil()
                .max(BUTTON_MIN_W)
        })
        .collect();
    let total: f32 = sizes.iter().sum::<f32>() + BUTTON_GAP * sizes.len().saturating_sub(1) as f32;
    // Too many or too long for the card: they share its width.
    let shrink = (inner / total.max(1.0)).min(1.0);
    let mut bx = x + width - PAD - total * shrink;
    for (button, w) in dialog.buttons.iter().zip(&sizes) {
        let w = w * shrink;
        d.button(
            button,
            Rect::new(
                d.snap(bx),
                by,
                d.snap(w - BUTTON_GAP * (1.0 - shrink)),
                BUTTON_H,
            ),
        );
        bx += w + BUTTON_GAP * shrink;
    }
    let height = by + BUTTON_H + PAD - 2.0 - y;
    Laid {
        list: d.list,
        spots: d.spots,
        texts: d.texts,
        carets: d.carets,
        size: (width, height),
    }
}

/// Lays out and draws `dialog` as a card in the middle of `window` (points; `scale`
/// physical pixels per point). In a window too low for the roomy sizes the compact ones are
/// used. The scrim over the window is not part of the drawing (see `build::overlay`).
pub fn build(dialog: &Dialog, window: Rect, scale: f32, theme: &Theme, pointer: &Pointer) -> Built {
    let snap = |v: f32| (v * scale).round() / scale;
    let width = snap(card_width(dialog.size).min(window.w - 2.0 * MARGIN));
    let room = window.h - 2.0 * MARGIN;
    let measure = |m: Metrics| {
        lay_out(dialog, (0.0, 0.0), width, m, scale, theme, pointer)
            .size
            .1
    };
    let (m, compact) = if measure(ROOMY) <= room {
        (ROOMY, false)
    } else {
        (COMPACT, true)
    };
    let height = snap(measure(m));
    let x = snap(window.x + (window.w - width) / 2.0);
    let y = snap((window.y + (window.h - height) / 2.0).max(window.y + MARGIN.min(4.0)));
    let card = Rect::new(x, y, width, height);
    let mut list = List::new();
    // The shadow: many faint cards of falling size, lower than the card, so that it fades
    // without steps.
    let step = Color {
        a: (theme.shadow.a / SHADOW_STEPS as u8).max(1),
        ..theme.shadow
    };
    for i in (0..SHADOW_STEPS).rev() {
        let (grow, drop) = (1.0 + 3.0 * i as f32, 2.0 + 1.2 * i as f32);
        let r = card.inset(-grow);
        list.round_rect(Rect::new(r.x, r.y + drop, r.w, r.h), RADIUS + grow, step);
    }
    list.round_rect(card, RADIUS, theme.raised);
    list.ring(card, RADIUS, theme.hairline, 1.0);
    let drawn = lay_out(dialog, (x, y), width, m, scale, theme, pointer);
    list.append(drawn.list);
    let reach = card.inset(-SHADOW_REACH);
    let (x0, y0) = (reach.x.max(window.x), reach.y.max(window.y));
    let bounds = Rect::new(
        x0,
        y0,
        reach.right().min(window.right()) - x0,
        reach.bottom().min(window.bottom()) - y0,
    );
    Built {
        list,
        card,
        bounds,
        spots: drawn.spots,
        texts: drawn.texts,
        compact,
        carets: drawn.carets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::Op;
    use crate::raster::Renderer;
    use crate::theme::ThemeKind;
    use dpx_core::Target;
    use dpx_ui::dialog::{ListRow, ScrollBar};
    use dpx_ui::form_ctl;
    use dpx_ui::{
        AnimBrushRequester, AnimRequester, ArrangeRequester, ChoiceRequester, FillRequester,
        FillSettings, GridRequester, GridSettings, MoveRequester, NoteRequester, PageSizeRequester,
        PerspectiveRequester, PerspectiveSettings, PlayRangeRequester, Preset, RangeRequester,
        ReqEvent, ReqKey, ScreenFormatRequester, SpacingMode, SpacingRequester, SpacingSettings,
        StencilRequester, SymmetryRequester, SymmetrySettings, ToolRequester,
        TranslucencyRequester,
    };

    const WINDOWS: [(f32, f32); 3] = [(720.0, 520.0), (1280.0, 800.0), (1001.0, 640.0)];
    const SCALES: [f32; 3] = [1.0, 2.0, 1.5];

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn page_size() -> PageSizeRequester {
        let preset = |label, size| Preset { label, size };
        PageSizeRequester::new(
            (320, 256),
            [
                preset("Standard", (320, 256)),
                preset("Full Page", (320, 316)),
                preset("Overscan", (368, 283)),
            ],
        )
    }

    fn screen_format() -> ScreenFormatRequester {
        ScreenFormatRequester::new(Target::default(), (320, 256), 32)
    }

    fn press(r: &mut ScreenFormatRequester, id: ControlId) {
        let a = r.view().control(id).expect("control").area;
        r.handle(ReqEvent::Press(a.x + a.w / 2, a.y + a.h / 2));
        r.handle(ReqEvent::Release);
    }

    fn palette() -> dpx_core::Palette {
        dpx_core::Palette::dpaint_default_32()
    }

    fn tool(r: &ToolRequester) -> Dialog {
        r.view(&palette())
    }

    fn spacing(mode: SpacingMode) -> ToolRequester {
        ToolRequester::Spacing(SpacingRequester::new(SpacingSettings {
            mode,
            n: 8,
            airbrush_radius: 10,
        }))
    }

    fn fill() -> ToolRequester {
        ToolRequester::Fill(FillRequester::new(FillSettings {
            gradient: Some(dpx_core::GradientDirection::Horizontal),
            dither: true,
        }))
    }

    /// The requesters of slice 4b that are built on the shared form.
    fn forms(out: &mut Vec<(&'static str, Dialog)>) {
        let grid = ToolRequester::Grid(GridRequester::new(GridSettings { x: 16, y: 512 }));
        out.push(("grid", tool(&grid)));
        let symmetry = ToolRequester::Symmetry(SymmetryRequester::new(SymmetrySettings {
            mirror: true,
            order: 40,
        }));
        out.push(("symmetry", tool(&symmetry)));
        out.push(("spacing", tool(&spacing(SpacingMode::Continuous))));
        // A refused number: the message under the form.
        let mut refused = spacing(SpacingMode::Airbrush);
        for key in [ReqKey::Tab, ReqKey::Tab, ReqKey::Backspace, ReqKey::Enter] {
            refused.handle(ReqEvent::Key(key));
        }
        out.push(("spacing, refused", tool(&refused)));
        out.push(("fill", tool(&fill())));
        let translucency = ToolRequester::Translucency(TranslucencyRequester::new(100));
        out.push(("translucency", tool(&translucency)));
        // The Anim requesters, with the largest numbers their fields take.
        out.push(("frame count", AnimRequester::frame_count(999, 999).view()));
        out.push(("add frames", AnimRequester::add_frames(998).view()));
        let mut copy = AnimRequester::copy_frames(999, 999);
        out.push(("copy frames", copy.view()));
        copy.set_message("Out of memory for 999 frames");
        out.push(("copy frames, refused", copy.view()));
        out.push(("delete frames", AnimRequester::delete_frames(999, 1).view()));
        out.push(("go to frame", AnimRequester::goto(999, 999).view()));
        out.push(("rate", AnimRequester::rate(30).view()));
        // AnimBrush and Perspective: forms with their own buttons.
        let brush = |r: AnimBrushRequester| tool(&ToolRequester::AnimBrush(r));
        out.push((
            "pick up animbrush",
            brush(AnimBrushRequester::pick_up(2, 999)),
        ));
        let mut make = AnimBrushRequester::make(200, 134);
        for key in [ReqKey::Backspace, ReqKey::Backspace, ReqKey::Enter] {
            make.handle(ReqEvent::Key(key));
        }
        assert!(format!("{:?}", make).contains("must be 2 to 134"));
        out.push(("make animbrush, refused", brush(make)));
        let settings =
            AnimBrushRequester::settings(999, 999, 999, dpx_core::CelDirection::Backward);
        out.push(("animbrush settings", brush(settings)));
        let wide = PerspectiveSettings {
            grid: [4096, 4096, 4096],
            angle_step: 360,
            brush_axes: true,
            show_position: true,
        };
        let perspective = |brush| {
            tool(&ToolRequester::Perspective(PerspectiveRequester::new(
                wide, brush,
            )))
        };
        out.push(("perspective", perspective(Some((320, 200)))));
        out.push(("perspective, no brush", perspective(None)));
        // The palette requesters: Make Stencil, Arrange Palette and Range with 2, 32 and
        // 256 colours.
        let grey = |n: usize| {
            let colors = (0..n)
                .map(|i| dpx_core::Rgb8 {
                    r: (i * 255 / (n - 1)) as u8,
                    g: (i * 97 % 256) as u8,
                    b: 255 - (i * 255 / (n - 1)) as u8,
                })
                .collect();
            dpx_core::Palette::new(colors).expect("a palette")
        };
        let protected: Vec<bool> = (0..256).map(|i| i % 3 == 0).collect();
        for (name, n) in [("stencil", 32), ("stencil, 256", 256), ("stencil, 2", 2)] {
            let r = StencilRequester::new(n, Some(&protected));
            out.push((name, r.view(&grey(n))));
        }
        for (name, n) in [("arrange", 32), ("arrange, 256", 256), ("arrange, 2", 2)] {
            let mut r = ArrangeRequester::new(n);
            let (from, to) = (r.cell_rect(n - 1), r.cell_rect(0));
            r.handle(ReqEvent::Press(from.x + 1, from.y + 1));
            r.handle(ReqEvent::Drag(to.x + 1, to.y + 1));
            out.push((name, r.view(&grey(n))));
        }
        for (name, n) in [("range", 32), ("range, 256", 256), ("range, 2", 2)] {
            let pal = grey(n);
            let mut r = RangeRequester::new(&pal, &[], 0);
            if n == 2 {
                out.push(("range, empty", r.view(&pal)));
            }
            for cell in [0, (n - 1).min(40) as u8] {
                let c = r.cell_rect(cell);
                r.handle(ReqEvent::Press(c.x + 1, c.y + 1));
                r.handle(ReqEvent::Release);
            }
            assert!(r.current().defined(), "{name}");
            out.push((name, r.view(&pal)));
        }
        // Move, with the longest numbers, with one frame only, and refused.
        let far = dpx_core::anim_move::MovePath {
            dist: [-99_999, 99_999, -12_345],
            angle: [-99_999, 360, 0],
            dist_brush: true,
            cyclic: true,
            come_to: true,
            ease_out: 999,
            ease_in: 999,
            count: 999,
            ..Default::default()
        };
        out.push(("move", MoveRequester::new(&far, 999).view()));
        let mut still = MoveRequester::new(&Default::default(), 1);
        out.push(("move, one frame", still.view()));
        still.set_message("Dist X must be -99999 to 99999");
        out.push(("move, refused", still.view()));
        let mut range = PlayRangeRequester::new(999, Some((500, 999)));
        out.push(("play range", range.view()));
        for key in [ReqKey::Backspace, ReqKey::Backspace, ReqKey::Enter] {
            range.handle(ReqEvent::Key(key));
        }
        out.push(("play range, refused", range.view()));
    }

    /// The converted dialogs in the states that change what they show.
    fn dialogs() -> Vec<(&'static str, Dialog)> {
        let mut out = Vec::new();
        forms(&mut out);
        let notes = strings(&[
            "4 layers saved as one picture, as shown, 1 hidden layer left out (save as .dpxproj to keep them)",
            "PNG cannot store: display mode PAL:Low Res",
        ]);
        out.push((
            "note",
            NoteRequester::new("Picture exported", &notes).view(),
        ));
        let many: Vec<String> = (0..20).map(|i| format!("note number {i}")).collect();
        out.push((
            "many notes",
            NoteRequester::new("Picture loaded", &many).view(),
        ));
        let about = strings(&[
            "DeluxePainter 0.1.0: a pixel-art editor in the spirit of DeluxePaint IV AGA",
            "Written in Rust; source-available under the PolyForm Noncommercial License 1.0.0",
            "Not affiliated with Electronic Arts",
            "Page 1280x512, 256 colours, PAL:Super-High Res Laced",
        ]);
        out.push((
            "about",
            NoteRequester::about("About DeluxePainter", &about).view(),
        ));
        let quit = ChoiceRequester::new(
            "Quit",
            &strings(&["Quit without saving?"]),
            ["Yes", "No"],
            1,
            1,
        );
        out.push(("quit", quit.view()));
        let mut recover = ChoiceRequester::new(
            "Recover Picture",
            &strings(&[
                "Unsaved changes were found.",
                "Picture: \"Castle\"",
                "Autosaved: 2026-11-14 10:00 UTC",
            ]),
            ["Recover", "Discard"],
            0,
            1,
        );
        out.push(("recover", recover.view()));
        let (x, y) = recover.button_centre(0);
        recover.handle(ReqEvent::Press(x, y));
        out.push(("recover, held", recover.view()));
        // A question that shows a whole command, with a word longer than a line.
        let command = strings(&[
            "Run this command in a terminal to connect Claude Code:",
            "claude mcp add deluxepainter -- /Applications/DeluxePainter.app/Contents/MacOS/deluxepainter-with-a-very-long-name --mcp",
        ]);
        let tall = ChoiceRequester::with_max_lines(
            "Connect Claude Code",
            &command,
            ["Get Claude Code", "Close"],
            1,
            1,
            16,
        );
        out.push(("command", tall.view()));
        let mut page = page_size();
        out.push(("page size", page.view()));
        page.handle(ReqEvent::Key(ReqKey::Tab));
        page.set_message("Size 320x200 to 1024x1024");
        out.push(("page size, message", page.view()));
        let mut format = screen_format();
        out.push(("screen format", format.view()));
        press(&mut format, dpx_ui::requester::ctl::SCREEN);
        press(&mut format, dpx_ui::requester::ctl::SCREEN);
        press(&mut format, dpx_ui::requester::ctl::SCREEN_W);
        press(&mut format, dpx_ui::requester::ctl::PAGE);
        format.set_message(
            "A HAM picture needs 16 base colours; choose another palette size or keep the page",
        );
        out.push(("screen format, custom and refused", format.view()));
        let mut hires = screen_format();
        for _ in 0..4 {
            hires.handle(ReqEvent::Key(ReqKey::Up));
        }
        assert!(!hires.chipset_switchable(), "Super-High Res is AGA only");
        out.push(("screen format, chipset ghosted", hires.view()));
        files(&mut out);
        imports(&mut out);
        generators(&mut out);
        mcp(&mut out);
        out
    }

    fn mcp_view() -> dpx_ui::McpView {
        dpx_ui::McpView {
            on: true,
            status: "Ready on 127.0.0.1:8765, connected: claude-code via deluxepainter-bridge"
                .into(),
            read: true,
            edit: true,
            overwrite: false,
            folder: "~/Pictures/DeluxePainter".into(),
            activity: (1..=7)
                .map(|i| format!("{i}s draw_rectangle 10,{i} 200x100 colour {i} on layer 2"))
                .collect(),
            message: None,
            generation: "OpenAI, key set".into(),
            busy: false,
        }
    }

    /// Prefs ▸ AI & MCP (slice 4c) in the states that change what it shows.
    fn mcp(out: &mut Vec<(&'static str, Dialog)>) {
        let ready = dpx_ui::McpRequester::new(mcp_view(), (320, 256));
        out.push(("ai & mcp", ready.dialog()));
        let off = dpx_ui::McpView {
            on: false,
            status: "Off".into(),
            read: false,
            activity: Vec::new(),
            generation: "None set up".into(),
            ..mcp_view()
        };
        out.push((
            "ai & mcp, off",
            dpx_ui::McpRequester::new(off, (320, 200)).dialog(),
        ));
        let busy = dpx_ui::McpView {
            busy: true,
            message: Some("Asking Claude Code whether deluxe-painter is registered..".into()),
            overwrite: true,
            ..mcp_view()
        };
        out.push((
            "ai & mcp, connecting",
            dpx_ui::McpRequester::new(busy, (736, 566)).dialog(),
        ));
        let failed = dpx_ui::McpView {
            status: "Could not start: the token file in /dev/null/DeluxePainter cannot be \
                     written (not a directory); the server tries again when the settings \
                     change"
                .into(),
            message: Some("Claude Code not found: the command was copied.".into()),
            folder: "/Volumes/Work/Projects/A very long folder name/Pictures/DeluxePainter".into(),
            generation: "OpenAI, key kept for this session only (no keychain here)".into(),
            ..mcp_view()
        };
        out.push((
            "ai & mcp, an error",
            dpx_ui::McpRequester::new(failed, (320, 256)).dialog(),
        ));
    }

    fn generate_requester(prompt: &str, screen: (i32, i32)) -> dpx_ui::GenerateRequester {
        dpx_ui::GenerateRequester::new(
            dpx_ui::GenerateOptions::default(),
            prompt,
            32,
            screen,
            dpx_ui::ProviderInfo {
                ready: true,
                summary: "OpenAI Images: billed per picture".into(),
                // The app's longest note.
                note: "Your description goes over the internet to OpenAI. Every picture is \
                       billed to your OpenAI account, also a cancelled one. No free tier; \
                       the price depends on model, quality and size (see OpenAI's pricing \
                       page). Low quality is the cheapest. Model: gpt-image-2.6-flare."
                    .into(),
            },
        )
    }

    fn setup_requester(provider: dpx_ui::SetupProvider) -> dpx_ui::ImageGenSetupRequester {
        dpx_ui::ImageGenSetupRequester::new(
            provider,
            "gpt-image-2.6-flare",
            dpx_ui::SetupInfo {
                key_status: "API key saved in the system keychain".into(),
                has_key: true,
                removable: true,
                model_hint: "Default gpt-image-2.6-flare".into(),
                note: "OpenAI bills each picture to your API account, also a cancelled one; \
                       no free models. Default model checked 2026-11-12. The key goes to \
                       the system keychain, never to a file."
                    .into(),
            },
            (320, 256),
        )
    }

    /// Generate Picture and the Image Generator setup (slice 4c) in the states that change
    /// what they show.
    fn generators(out: &mut Vec<(&'static str, Dialog)>) {
        use dpx_ui::GenPhase;
        let ask = generate_requester(
            "a castle on a hill at sunset, seen from the lake",
            (320, 256),
        );
        out.push(("generate", ask.view()));
        let mut more = ask.clone();
        more.set_more(true);
        out.push(("generate, more", more.view()));
        let mut working = more.clone();
        working.set_phase(GenPhase::Working { seconds: 1234 });
        out.push(("generate, working", working.view()));
        for (name, more, screen) in [
            ("generate, result", false, (320, 256)),
            ("generate, more, result", true, (320, 256)),
            ("generate, result on the smallest screen", true, (320, 200)),
            ("generate, result on a large screen", false, (736, 566)),
        ] {
            let mut done = generate_requester("a castle", screen);
            done.set_more(more);
            done.set_phase(GenPhase::Result);
            let (w, h) = done.thumb_size();
            let fill = dpx_core::Rgba8::new(40, 160, 90, 255);
            done.set_thumbnail(Some(dpx_core::RgbaImage::new(w, h, fill)));
            done.set_message(Some(
                "Apply makes it the spare page (320x240, 32 colours).".into(),
            ));
            out.push((name, done.view()));
            if more {
                done.set_error(Some(
                    "The generator did not answer in time. The picture shown is the one \
                     from before."
                        .into(),
                ));
                out.push(("generate, a result and an error", done.view()));
            }
        }
        // No generator; a problem that keeps Generate ghosted; an empty description.
        let mut none = dpx_ui::GenerateRequester::new(
            dpx_ui::GenerateOptions::default(),
            "",
            32,
            (320, 256),
            dpx_ui::ProviderInfo {
                ready: false,
                summary: "No image generator is set up.".into(),
                note: "No image generator is set up. Set up.. chooses a generator and its \
                       API key."
                    .into(),
            },
        );
        out.push(("generate, no generator", none.view()));
        none.set_error(Some("No image generator is set up.".into()));
        none.set_more(true);
        out.push(("generate, more, an error", none.view()));
        let mut layer = more.clone();
        layer.set_options(dpx_ui::GenerateOptions {
            target: dpx_ui::GenTarget::NewLayer,
            ..dpx_ui::GenerateOptions::default()
        });
        layer.set_document(
            Target::new(dpx_core::Chipset::Aga, dpx_core::DisplayMode::PAL_LORES),
            (1024, 1024),
        );
        layer.set_doc_layers(dpx_core::layer::MAX_LAYERS);
        out.push(("generate, a new layer that has no room", layer.view()));
        // The setup.
        out.push((
            "setup, none",
            setup_requester(dpx_ui::SetupProvider::None).view(),
        ));
        let mut openai = setup_requester(dpx_ui::SetupProvider::OpenAi);
        out.push(("setup, openai", openai.view()));
        for k in dpx_ui::paste_keys(SECRET) {
            openai.handle(ReqEvent::Key(k));
        }
        out.push(("setup, a key typed", openai.view()));
        openai.set_test(Some(
            "The key works: OpenAI lists the model gpt-image-2.6-flare for this account.".into(),
        ));
        openai.handle(ReqEvent::Key(ReqKey::Tab));
        out.push(("setup, tested, the model has the keys", openai.view()));
    }

    /// A key as a test types it into the setup; it must show nowhere.
    const SECRET: &str = "sk-kit-Secret_0123456789abcdefghijklmnopqrstuvwxyz";

    fn import_requester(name: &str, screen: (i32, i32)) -> dpx_ui::ImportRequester {
        dpx_ui::ImportRequester::new(
            dpx_ui::ImportInfo {
                name: name.into(),
                size: (4032, 3024),
                colors: None,
                fitted: (320, 240),
                page: (320, 256),
            },
            dpx_core::import::ImportOptions::default(),
            256,
            32,
            screen,
            (1, 1),
        )
    }

    /// A thumbnail of the size the requester asks for: a red and a blue half.
    fn thumbnail(r: &dpx_ui::ImportRequester) -> dpx_core::RgbaImage {
        let (w, h) = r.thumb_size();
        let mut image = dpx_core::RgbaImage::new(w, h, dpx_core::Rgba8::new(200, 30, 30, 255));
        for y in 0..h {
            for x in w / 2..w {
                image.set(x, y, dpx_core::Rgba8::new(30, 30, 200, 255));
            }
        }
        image
    }

    /// Import Picture (slice 4c) in the states that change what it shows.
    fn imports(out: &mut Vec<(&'static str, Dialog)>) {
        let plain = import_requester("photo.jpg", (320, 256));
        out.push(("import, no thumbnail yet", plain.view()));
        let mut shown = plain.clone();
        shown.set_thumbnail(thumbnail(&shown));
        out.push(("import", shown.view()));
        // The focus on every option in turn; the current palette ghosts the count.
        let mut focused = shown.clone();
        for name in ["import, position", "import, palette", "import, colours"] {
            focused.handle(ReqEvent::Key(ReqKey::Tab));
            out.push((name, focused.view()));
        }
        focused.handle(ReqEvent::Key(ReqKey::Up));
        focused.handle(ReqEvent::Key(ReqKey::Tab));
        focused.handle(ReqEvent::Key(ReqKey::Down));
        out.push(("import, dither, the least colours", focused.view()));
        let mut current = shown.clone();
        current.handle(ReqEvent::Key(ReqKey::Tab));
        current.handle(ReqEvent::Key(ReqKey::Tab));
        current.handle(ReqEvent::Key(ReqKey::Down));
        out.push(("import, the current palette", current.view()));
        // A long name; a large screen (a larger thumbnail); a tall one.
        let long = "a-very-long-file-name-of-a-holiday-photograph-from-the-camera.jpeg";
        let mut wide = import_requester(long, (736, 566));
        wide.set_thumbnail(thumbnail(&wide));
        out.push(("import, a long name on a large screen", wide.view()));
        let mut small = import_requester("x.png", (320, 200));
        small.set_thumbnail(thumbnail(&small));
        out.push(("import, the smallest screen", small.view()));
    }

    /// A drawer as the file requester lists it: `files` files after two drawers.
    fn listing(files: usize) -> Vec<dpx_ui::Entry> {
        let place = |label: &str| dpx_ui::Entry::Place {
            kind: dpx_ui::PlaceKind::Dir,
            label: label.into(),
            drawer: format!("/work/{label}"),
        };
        let mut entries = vec![place("Brushes"), place("Pictures")];
        entries.extend((0..files).map(|i| dpx_ui::Entry::File(format!("castle-{i:02}.iff"))));
        entries.push(dpx_ui::Entry::File(format!(
            "a-{}.iff",
            "very-long-name-".repeat(12)
        )));
        entries
    }

    fn file_requester(kind: dpx_ui::FileKind, files: usize) -> dpx_ui::FileRequester {
        let mut r = dpx_ui::FileRequester::new(kind);
        r.set_listing("/work/pictures/castles", listing(files));
        r
    }

    /// The file requester (slice 4c) in the states that change what it shows.
    fn files(out: &mut Vec<(&'static str, Dialog)>) {
        use dpx_ui::FileKind as K;
        out.push(("load picture", file_requester(K::LoadPicture, 30).view()));
        let mut chosen = file_requester(K::LoadPicture, 30);
        chosen.set_file("castle-03.iff");
        chosen.set_message("castle-03.iff is not a picture this program reads");
        out.push(("load picture, a file and a message", chosen.view()));
        let mut short = file_requester(K::SavePicture, 2);
        short.set_file(&"a very long project name ".repeat(8));
        out.push(("save project, a short list", short.view()));
        let mut empty = dpx_ui::FileRequester::new(K::ExportPicture);
        empty.set_listing("", Vec::new());
        out.push(("export, nothing listed", empty.view()));
        for (name, kind) in [
            ("load animation", K::LoadAnim),
            ("save animation", K::SaveAnim),
            ("delete file", K::DeleteFile),
            ("load palette", K::LoadPalette),
            ("save brush", K::SaveBrush),
            ("load stencil", K::LoadStencil),
            ("save animbrush", K::SaveAnimBrush),
        ] {
            out.push((name, file_requester(kind, 9).view()));
        }
        let mut replace = file_requester(K::ExportPicture, 3);
        replace.set_file("castle-01.iff");
        replace.ask_overwrite();
        out.push(("export, replace?", replace.view()));
        let mut delete = file_requester(K::DeleteFile, 3);
        delete.set_file(&format!("a-{}.iff", "very-long-name-".repeat(12)));
        delete.ask_overwrite();
        out.push(("delete, a long name?", delete.view()));
    }

    fn built(dialog: &Dialog, window: (f32, f32), scale: f32, kind: ThemeKind) -> Built {
        build(
            dialog,
            Rect::new(0.0, 0.0, window.0, window.1),
            scale,
            &Theme::of(kind),
            &Pointer::default(),
        )
    }

    /// Every converted dialog, in both themes, in the smallest and larger windows and at
    /// device scales 1, 1.5 and 2: the card lies inside the window, every text inside the
    /// part it belongs to and inside the card, nothing is cut with an ellipsis, and every
    /// control inside the card.
    #[test]
    fn every_dialog_fits_its_card_and_every_text_its_box() {
        for (name, dialog) in dialogs() {
            for kind in ThemeKind::ALL {
                for window in WINDOWS {
                    for scale in SCALES {
                        let at = format!("{name}, {kind:?}, {window:?} at {scale}x");
                        let b = built(&dialog, window, scale, kind);
                        let win = Rect::new(0.0, 0.0, window.0, window.1);
                        assert!(win.encloses(&b.card), "{at}: card {:?}", b.card);
                        assert!(win.encloses(&b.bounds), "{at}");
                        assert!(b.card.h >= 100.0 && b.card.w >= 300.0, "{at}");
                        // On whole physical pixels.
                        for v in [b.card.x, b.card.y, b.card.w, b.card.h] {
                            assert!(((v * scale).round() - v * scale).abs() < 1e-3, "{at}");
                        }
                        let inner = Rect::new(
                            b.card.x + PAD - 0.5,
                            b.card.y,
                            b.card.w - 2.0 * PAD + 1.0,
                            b.card.h,
                        );
                        assert!(!b.texts.is_empty(), "{at}");
                        for t in &b.texts {
                            assert!(
                                t.within.inset(-0.5).encloses(&t.ink),
                                "{at}: '{}' {:?} leaves {:?}",
                                t.text,
                                t.ink,
                                t.within
                            );
                            assert!(inner.encloses(&t.ink), "{at}: '{}' leaves the card", t.text);
                        }
                        for s in &b.spots {
                            assert!(inner.encloses(&s.rect), "{at}: control {:?}", s.control);
                            assert!(s.rect.w >= 10.0 && s.rect.h >= 18.0, "{at}: {:?}", s.rect);
                        }
                    }
                }
            }
            // Nothing is cut that the dialog did not cut itself; only a list row's long
            // name and a long fact (a file name) may end in an ellipsis.
            let source = format!("{dialog:?}");
            let b = built(&dialog, WINDOWS[0], 1.0, ThemeKind::Dark);
            let rows: Vec<&str> = dialog
                .columns
                .iter()
                .flatten()
                .flat_map(|i| match i {
                    Item::List(list) => list.rows.iter().map(|r| r.label.as_str()).collect(),
                    Item::Facts { rows, .. } => rows.iter().map(|r| r.1.as_str()).collect(),
                    _ => Vec::new(),
                })
                .collect();
            for t in &b.texts {
                let own = t.text.matches('\u{2026}').count();
                let row = rows
                    .iter()
                    .any(|r| r.starts_with(t.text.trim_end_matches('\u{2026}')));
                assert!(
                    own == 0 || row || source.contains(&t.text) || name == "command",
                    "{name}: '{}' was cut",
                    t.text
                );
            }
        }
    }

    /// The ink of every text reaches 4.5:1 on its ground (ghosted ink 3:1), in both themes,
    /// in every state of the five dialogs, with the pointer over each control in turn.
    #[test]
    fn every_text_is_legible_on_its_ground() {
        for (name, dialog) in dialogs() {
            for kind in ThemeKind::ALL {
                let theme = Theme::of(kind);
                let rest = built(&dialog, WINDOWS[1], 1.0, kind);
                let mut pointers = vec![Pointer::default()];
                for s in &rest.spots {
                    let (x, y) = s.rect.center();
                    pointers.push(Pointer {
                        at: Some((x, y)),
                        down: false,
                        captured: None,
                    });
                }
                for pointer in pointers {
                    let window = Rect::new(0.0, 0.0, WINDOWS[1].0, WINDOWS[1].1);
                    let b = build(&dialog, window, 1.0, &theme, &pointer);
                    for t in &b.texts {
                        let ratio = t.color.contrast(t.ground);
                        let least = if t.color == theme.disabled { 3.0 } else { 4.5 };
                        assert!(
                            ratio >= least,
                            "{name}, {kind:?}: '{}' is {ratio:.2}:1 ({:?} on {:?})",
                            t.text,
                            t.color,
                            t.ground
                        );
                    }
                }
            }
        }
    }

    /// The fills the kit mixes itself keep the contrast of the tokens they are made from.
    #[test]
    fn mixed_fills_keep_text_legible() {
        for kind in ThemeKind::ALL {
            let t = Theme::of(kind);
            for (hover, pressed) in [(false, false), (true, false), (false, true), (true, true)] {
                let plain = t.text.contrast(button_fill(&t, hover, pressed));
                assert!(
                    plain >= 4.5,
                    "{kind:?}: plain {hover} {pressed}: {plain:.2}"
                );
                let accent = t.on_accent.contrast(default_fill(&t, hover, pressed));
                assert!(
                    accent >= 4.5,
                    "{kind:?}: default {hover} {pressed}: {accent:.2}"
                );
            }
            // The states differ visibly from each other.
            assert_ne!(button_fill(&t, false, false), button_fill(&t, true, false));
            assert_ne!(button_fill(&t, true, false), button_fill(&t, false, true));
            assert_ne!(
                default_fill(&t, false, false),
                default_fill(&t, true, false)
            );
            // Rims and marks that carry meaning stand out from the card.
            for (name, ink) in [("accent", t.accent), ("dim rim", t.text_dim)] {
                assert!(ink.contrast(t.raised) >= 3.0, "{kind:?}: {name}");
                assert!(ink.contrast(t.panel) >= 3.0, "{kind:?}: {name} on a field");
            }
        }
    }

    /// The middle and the corners of every control's place on the card map into the
    /// rectangle in which the Classic requester takes its press; a point dragged off the
    /// control maps outside it, on the same side.
    #[test]
    fn a_point_on_a_control_is_a_point_in_its_classic_rectangle() {
        for (name, dialog) in dialogs() {
            for window in WINDOWS {
                for scale in SCALES {
                    let b = built(&dialog, window, scale, ThemeKind::Dark);
                    let ids: Vec<ControlId> = dialog.controls().iter().map(|c| c.id).collect();
                    for s in &b.spots {
                        assert!(ids.contains(&s.control.id), "{name}");
                        let a = s.control.area;
                        let inside = |(x, y): (i32, i32)| a.contains(x, y);
                        let e = 0.01;
                        for (x, y) in [
                            s.rect.center(),
                            (s.rect.x, s.rect.y),
                            (s.rect.right() - e, s.rect.y),
                            (s.rect.x, s.rect.bottom() - e),
                            (s.rect.right() - e, s.rect.bottom() - e),
                        ] {
                            assert_eq!(b.spot_at(x, y).map(|t| t.control.id), Some(s.control.id));
                            let p = s.to_classic(x, y);
                            assert!(inside(p), "{name}: {:?} at ({x}, {y}) -> {p:?}", s.control);
                            let (taken, at) = b.press(x, y);
                            assert_eq!(taken, Some(s.control.id));
                            if b.carets.iter().any(|(id, _)| *id == s.control.id) {
                                // A text field sends the press at a caret stop.
                                assert!(inside(at), "{name}: {:?} -> {at:?}", s.control);
                            } else {
                                assert_eq!(at, p);
                            }
                        }
                        if s.slider.is_some() {
                            // A slider follows the pointer along its length only.
                            let left = s.to_classic(s.rect.x - 500.0, s.rect.y - 300.0);
                            let right = s.to_classic(s.rect.right() + 500.0, s.rect.y + 300.0);
                            assert!(inside(left) && inside(right) && left.0 < right.0, "{name}");
                            continue;
                        }
                        // The first and the last pixel of the area are reached.
                        assert_eq!(s.to_classic(s.rect.x, s.rect.y), (a.x, a.y), "{name}");
                        assert_eq!(
                            s.to_classic(s.rect.right() - e, s.rect.bottom() - e),
                            (a.x + a.w - 1, a.y + a.h - 1),
                            "{name}"
                        );
                        let (cx, cy) = s.rect.center();
                        let away = [
                            (s.rect.x - 30.0, cy, true, false),
                            (s.rect.right() + 30.0, cy, true, true),
                            (cx, s.rect.y - 30.0, false, false),
                            (cx, s.rect.bottom() + 30.0, false, true),
                        ];
                        for (x, y, horizontal, beyond) in away {
                            let p = b.drag(Some(s.control.id), x, y);
                            assert!(!inside(p), "{name}: dragged off {:?}", s.control);
                            let ok = match (horizontal, beyond) {
                                (true, false) => p.0 < a.x,
                                (true, true) => p.0 >= a.x + a.w,
                                (false, false) => p.1 < a.y,
                                (false, true) => p.1 >= a.y + a.h,
                            };
                            assert!(ok, "{name}: {p:?} is on the wrong side of {a:?}");
                        }
                    }
                    // Beside every control: nowhere, and a drag without a control stays there.
                    let (x, y) = (b.card.x + 3.0, b.card.y + 3.0);
                    assert_eq!(b.press(x, y), (None, NOWHERE));
                    assert_eq!(b.drag(None, 400.0, 300.0), NOWHERE);
                    assert_eq!(b.drag(Some(9999), 400.0, 300.0), NOWHERE);
                }
            }
        }
    }

    /// Every pixel of a slider's and a scroll bar's run can be reached from the card, so
    /// every value the Classic gadget has can be set.
    #[test]
    fn every_position_of_a_slider_and_a_scroll_bar_is_reached() {
        let dialog = screen_format().view();
        let b = built(&dialog, WINDOWS[0], 1.0, ThemeKind::Dark);
        let slider = b.spot(dpx_ui::requester::ctl::COLORS).expect("the slider");
        let mut xs: Vec<i32> = (0..=4000)
            .map(|i| {
                slider
                    .to_classic(slider.rect.x + slider.rect.w * i as f32 / 4000.0, 0.0)
                    .0
            })
            .collect();
        xs.dedup();
        let (a, knob) = (slider.control.area, slider.slider.expect("a slider").1);
        let wanted: Vec<i32> = (a.x + knob / 2..=a.x + knob / 2 + a.w - knob).collect();
        assert_eq!(xs, wanted);
        let bar = b
            .spot(dpx_ui::requester::ctl::SCROLL)
            .expect("the scroll bar");
        let mut ys: Vec<i32> = (0..4000)
            .map(|i| {
                bar.to_classic(
                    bar.rect.x + 1.0,
                    bar.rect.y + bar.rect.h * i as f32 / 4000.0,
                )
                .1
            })
            .collect();
        ys.dedup();
        let a = bar.control.area;
        assert_eq!(ys, (a.y..a.y + a.h).collect::<Vec<i32>>());
    }

    fn fills(list: &List, rect: Rect) -> Vec<Color> {
        list.ops()
            .iter()
            .filter_map(|op| match op {
                Op::RoundRect {
                    rect: r,
                    fill: Some(c),
                    ..
                } if *r == rect => Some(*c),
                _ => None,
            })
            .collect()
    }

    fn rings(list: &List, rect: Rect) -> Vec<(Color, f32)> {
        list.ops()
            .iter()
            .filter_map(|op| match op {
                Op::RoundRect {
                    rect: r,
                    fill: None,
                    stroke: Some(s),
                    ..
                } if *r == rect => Some((s.color, s.width)),
                _ => None,
            })
            .collect()
    }

    fn text_color(b: &Built, text: &str) -> Color {
        b.texts
            .iter()
            .find(|t| t.text == text)
            .unwrap_or_else(|| panic!("the text '{text}'"))
            .color
    }

    /// The card: a shadow under it, the raised ground with a hairline, the title in the
    /// medium cut at the top left; centred in the window.
    #[test]
    fn the_card_is_centred_with_shadow_ground_and_title() {
        for kind in ThemeKind::ALL {
            let theme = Theme::of(kind);
            let dialog = page_size().view();
            let b = built(&dialog, (1280.0, 800.0), 1.0, kind);
            assert_eq!(b.card.w, 400.0);
            assert_eq!(b.card.x, (1280.0 - 400.0) / 2.0);
            assert!((b.card.y - (800.0 - b.card.h) / 2.0).abs() <= 0.5);
            assert!(!b.compact);
            let ground = b
                .list
                .ops()
                .iter()
                .position(|op| {
                    matches!(op, Op::RoundRect { rect, radius, fill: Some(c), .. }
                        if *rect == b.card && *radius == RADIUS && *c == theme.raised)
                })
                .expect("the card's ground");
            assert_eq!(ground, SHADOW_STEPS, "the shadow lies under the card");
            for op in &b.list.ops()[..ground] {
                let Op::RoundRect {
                    rect,
                    fill: Some(c),
                    ..
                } = op
                else {
                    panic!("a shadow card: {op:?}");
                };
                assert!(c.a > 0 && c.a < 40 && rect.w > b.card.w && rect.y > b.card.y - 31.0);
                assert!(b.card.inset(-SHADOW_REACH).encloses(rect), "{rect:?}");
            }
            assert_eq!(rings(&b.list, b.card), [(theme.hairline, 1.0)]);
            let Some(Op::Text {
                x,
                text,
                size,
                weight,
                color,
                ..
            }) = b.list.ops().iter().find(|op| matches!(op, Op::Text { .. }))
            else {
                panic!("a title");
            };
            assert_eq!(
                (text.as_str(), *size, *weight, *color, *x),
                (
                    "Set Page Size",
                    TITLE_SIZE,
                    Weight::Medium,
                    theme.text,
                    b.card.x + PAD
                )
            );
            // The card's width gives way in a window narrower than it; a tall dialog uses
            // the compact sizes in a low window.
            let narrow = built(&dialog, (360.0, 520.0), 1.0, kind);
            assert_eq!(narrow.card.w, 360.0 - 2.0 * MARGIN);
            let big = screen_format().view();
            assert_eq!(built(&big, (1280.0, 800.0), 1.0, kind).card.w, 680.0);
            let low = built(&big, (720.0, 440.0), 1.0, kind);
            assert!(low.compact && low.card.h < built(&big, (720.0, 520.0), 1.0, kind).card.h);
        }
    }

    /// Buttons: right-aligned in the Classic order; the default one on the accent, the
    /// others raised with a hairline; under the pointer lighter, held darker or rimmed;
    /// ghosted flat and faint; the key's letter underlined.
    #[test]
    fn buttons_show_role_pointer_and_press() {
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = screen_format();
        let dialog = req.view();
        let b = build(&dialog, window, 1.0, &theme, &Pointer::default());
        let (use_, cancel) = (
            b.spot(dpx_ui::requester::ctl::USE).expect("Use").rect,
            b.spot(dpx_ui::requester::ctl::CANCEL).expect("Cancel").rect,
        );
        assert_eq!((use_.h, cancel.h), (BUTTON_H, BUTTON_H));
        assert!(use_.w >= BUTTON_MIN_W && cancel.w >= BUTTON_MIN_W);
        assert_eq!(cancel.right(), b.card.right() - PAD);
        assert_eq!(
            use_.right() + BUTTON_GAP,
            cancel.x,
            "Use, then Cancel, as in Classic"
        );
        assert!(b.card.bottom() - cancel.bottom() >= 16.0);
        assert_eq!(fills(&b.list, use_), [theme.accent]);
        assert!(rings(&b.list, use_).is_empty());
        assert_eq!(fills(&b.list, cancel), [theme.hover]);
        assert_eq!(rings(&b.list, cancel), [(theme.hairline, 1.0)]);
        assert_eq!(text_color(&b, "Use"), theme.on_accent);
        assert_eq!(text_color(&b, "Cancel"), theme.text);
        // The underline of U and C: a one-point bar just under each label.
        for rect in [use_, cancel] {
            let bars = b
                .list
                .ops()
                .iter()
                .filter(|op| {
                    matches!(op, Op::RoundRect { rect: r, radius, .. }
                        if *radius == 0.0 && r.h == 1.0 && rect.encloses(r) && r.w < 14.0)
                })
                .count();
            assert_eq!(bars, 1);
        }
        // Under the pointer.
        let over = |rect: Rect, down: bool, captured: Option<ControlId>| Pointer {
            at: Some(rect.center()),
            down,
            captured,
        };
        let h = build(&dialog, window, 1.0, &theme, &over(cancel, false, None));
        assert_eq!(fills(&h.list, cancel), [button_fill(&theme, true, false)]);
        assert_eq!(
            fills(&h.list, use_),
            [theme.accent],
            "only the one under the pointer"
        );
        let h = build(&dialog, window, 1.0, &theme, &over(use_, false, None));
        assert_eq!(fills(&h.list, use_), [default_fill(&theme, true, false)]);
        // A button held elsewhere does not light up another one.
        let other = Some(dpx_ui::requester::ctl::CANCEL);
        let h = build(&dialog, window, 1.0, &theme, &over(use_, true, other));
        assert_eq!(fills(&h.list, use_), [theme.accent]);
        // Held: the requester says so.
        let a = dialog.control(dpx_ui::requester::ctl::USE).unwrap().area;
        req.handle(ReqEvent::Press(a.x + 2, a.y + 2));
        let held = req.view();
        let captured = Some(dpx_ui::requester::ctl::USE);
        let p = build(&held, window, 1.0, &theme, &over(use_, true, captured));
        assert_eq!(fills(&p.list, use_), [theme.accent]);
        assert_eq!(
            rings(&p.list, use_),
            [(theme.on_accent.with_opacity(45), 2.0)]
        );
        req.handle(ReqEvent::Drag(0, 0));
        req.handle(ReqEvent::Release);
        let a = dialog.control(dpx_ui::requester::ctl::CANCEL).unwrap().area;
        req.handle(ReqEvent::Press(a.x + 2, a.y + 2));
        let captured = Some(dpx_ui::requester::ctl::CANCEL);
        let p = build(
            &req.view(),
            window,
            1.0,
            &theme,
            &over(cancel, true, captured),
        );
        assert_eq!(fills(&p.list, cancel), [button_fill(&theme, true, true)]);
        // Ghosted.
        let mut ghost = held.clone();
        ghost.buttons[0].enabled = false;
        ghost.buttons[0].pressed = false;
        let g = build(&ghost, window, 1.0, &theme, &over(use_, false, None));
        assert_eq!(fills(&g.list, use_), [theme.panel]);
        assert_eq!(text_color(&g, "Use"), theme.disabled);
        // A long label widens its button; both stay in the card.
        let wide = ChoiceRequester::new("T", &[], ["Get Claude Code", "Close"], 1, 1).view();
        let w = build(&wide, window, 1.0, &theme, &Pointer::default());
        let (first, second) = (w.spots[0].rect, w.spots[1].rect);
        assert!(first.w > second.w && second.w == BUTTON_MIN_W);
        assert!(first.x >= w.card.x + PAD && first.right() + BUTTON_GAP == second.x);
    }

    /// Fields: a quiet well with a hairline; the one with the keys has the accent around it
    /// and a caret after the characters before the cursor; a fixed value is faint and takes
    /// no pointer. A text longer than the field keeps the caret in view.
    #[test]
    fn fields_show_text_caret_and_focus() {
        use dpx_ui::page_size_requester::ctl;
        let theme = Theme::light();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = page_size();
        let draw =
            |req: &PageSizeRequester| build(&req.view(), window, 1.0, &theme, &Pointer::default());
        let caret = |b: &Built, field: Rect| -> Vec<Rect> {
            b.list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::RoundRect {
                        rect,
                        fill: Some(c),
                        ..
                    } if rect.w == 1.5 && *c == theme.text && field.encloses(rect) => Some(*rect),
                    _ => None,
                })
                .collect()
        };
        let b = draw(&req);
        let (width, height) = (
            b.spot(ctl::WIDTH).unwrap().rect,
            b.spot(ctl::HEIGHT).unwrap().rect,
        );
        assert_eq!(width.h, 28.0);
        assert_eq!(
            (width.x, width.w),
            (height.x, height.w),
            "the fields line up"
        );
        assert_eq!(fills(&b.list, width), [theme.panel]);
        assert_eq!(rings(&b.list, width), [(theme.accent, 1.5)]);
        assert_eq!(rings(&b.list, height), [(theme.hairline, 1.0)]);
        let at_end = caret(&b, width);
        assert_eq!(at_end.len(), 1);
        assert!(caret(&b, height).is_empty());
        let end = width.x + 10.0 + font::measure("320", SIZE, Weight::Regular);
        assert!((at_end[0].x - end).abs() <= 0.5, "{} vs {end}", at_end[0].x);
        // The cursor moved left by one: the caret stands before the last digit.
        req.handle(ReqEvent::Key(ReqKey::Left));
        let moved = caret(&draw(&req), width);
        let before = width.x + 10.0 + font::measure("32", SIZE, Weight::Regular);
        assert!((moved[0].x - before).abs() <= 0.5);
        // Tab: the other field has the keys.
        req.handle(ReqEvent::Key(ReqKey::Tab));
        let b = draw(&req);
        assert_eq!(rings(&b.list, height), [(theme.accent, 1.5)]);
        assert!(caret(&b, width).is_empty() && caret(&b, height).len() == 1);
        // A field of a fixed value.
        let format = screen_format().view();
        let b = build(&format, window, 1.0, &theme, &Pointer::default());
        assert!(
            b.spot(dpx_ui::requester::ctl::SCREEN_W).is_none(),
            "it takes no pointer"
        );
        assert_eq!(text_color(&b, "320"), theme.disabled);
        // A long text in a short field: cut at the left, the caret inside.
        let mut long = req.view();
        let Item::Line { parts, .. } = &mut long.columns[0][1] else {
            panic!("the width line");
        };
        let Part::Field(f) = &mut parts[0] else {
            panic!("a field");
        };
        f.text = "12345678901234567890".into();
        f.caret = Some(20);
        let b = build(&long, window, 1.0, &theme, &Pointer::default());
        let shown = b
            .texts
            .iter()
            .find(|t| t.text.ends_with("890"))
            .expect("the tail");
        assert!(
            shown.text.len() < 20 && width.encloses(&shown.ink),
            "{shown:?}"
        );
        let c = caret(&b, width);
        assert!(c.len() == 1 && c[0].right() <= width.right() - 6.0);
    }

    /// Cycle gadgets: two values side by side with the shown one on the accent (a press on
    /// the other one is the gadget's press, the shown one takes none); more values as a
    /// stepper with a chevron. Ghosted: faint, but the press still reaches the requester.
    #[test]
    fn cycle_gadgets_are_segments_or_steppers() {
        use dpx_ui::requester::ctl;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = screen_format();
        let b = build(&req.view(), window, 1.0, &theme, &Pointer::default());
        // Page Size: Screen | Keep Same, Screen shown.
        let other = b.spot(ctl::PAGE).expect("the other segment").rect;
        let n = b.spots.iter().filter(|s| s.control.id == ctl::PAGE).count();
        assert_eq!(n, 1, "the shown value takes no press");
        let shown = Rect::new(other.x - other.w, other.y, other.w, other.h);
        assert_eq!(fills(&b.list, shown), [theme.accent]);
        assert!(fills(&b.list, other).is_empty());
        assert_eq!(text_color(&b, "Screen"), theme.on_accent);
        assert_eq!(text_color(&b, "Keep Same"), theme.text_dim);
        assert_eq!(b.press(shown.x + 5.0, shown.y + 5.0), (None, NOWHERE));
        // A press on the other one switches; then the first one is the other.
        let (id, at) = b.press(other.x + 5.0, other.y + 5.0);
        assert_eq!(id, Some(ctl::PAGE));
        req.handle(ReqEvent::Press(at.0, at.1));
        req.handle(ReqEvent::Release);
        let b = build(&req.view(), window, 1.0, &theme, &Pointer::default());
        assert_eq!(b.spot(ctl::PAGE).map(|s| s.rect), Some(shown));
        assert_eq!(fills(&b.list, other), [theme.accent]);
        // Under the pointer the other segment is raised.
        let over = Pointer {
            at: Some(shown.center()),
            down: false,
            captured: None,
        };
        let h = build(&req.view(), window, 1.0, &theme, &over);
        assert_eq!(fills(&h.list, shown), [theme.hover]);
        // Screen Size and Preset: steppers with a chevron, the value at the left.
        for (id, value) in [
            (ctl::SCREEN, "Standard"),
            (ctl::PRESET, "PAL LoRes 320x256"),
        ] {
            let rect = b.spot(id).expect("a stepper").rect;
            assert_eq!(fills(&b.list, rect), [theme.hover]);
            assert_eq!(rings(&b.list, rect), [(theme.hairline, 1.0)]);
            let t = b.texts.iter().find(|t| t.text == value).expect("the value");
            assert!((t.ink.x - (rect.x + 12.0)).abs() < 0.01 && rect.encloses(&t.ink));
            let chevrons = b
                .list
                .ops()
                .iter()
                .filter(|op| {
                    matches!(op, Op::Path { path, stroke: Some(_), .. }
                        if path.bounds().is_some_and(|p| rect.encloses(&p) && p.x > rect.right() - 24.0))
                })
                .count();
            assert_eq!(chevrons, 1, "{value}");
        }
        // A stepper takes what its line has left: Preset the whole line, Screen Size what
        // the two number fields leave, so both lines end at the same place.
        let (preset, screen) = (
            b.spot(ctl::PRESET).unwrap().rect,
            b.spot(ctl::SCREEN).unwrap().rect,
        );
        assert!(preset.w > 250.0 && preset.x == screen.x && screen.w >= 100.0);
        let last = b
            .texts
            .iter()
            .find(|t| t.text == "256")
            .expect("the height");
        assert!(
            (last.within.right() - preset.right()).abs() <= 0.5,
            "{:?} {preset:?}",
            last.within
        );
        // A ghosted Chipset: faint, and the press on the other value still goes through.
        let mut hires = screen_format();
        for _ in 0..4 {
            hires.handle(ReqEvent::Key(ReqKey::Up));
        }
        let b = build(&hires.view(), window, 1.0, &theme, &Pointer::default());
        assert_eq!(text_color(&b, "OCS"), theme.disabled);
        // (The facts panel names the chipset too: the segment is the later text.)
        let shown = b.texts.iter().rfind(|t| t.text == "AGA").expect("AGA");
        assert_eq!(shown.color, theme.text_dim);
        let ocs = b.spot(ctl::CHIPSET).expect("OCS takes the press").rect;
        let (_, at) = b.press(ocs.x + 4.0, ocs.y + 4.0);
        hires.handle(ReqEvent::Press(at.0, at.1));
        let Item::Facts { message, .. } = &hires.view().columns[1][1] else {
            panic!("facts");
        };
        assert_eq!(message.as_deref(), Some("Super-High Res needs AGA"));
    }

    /// Check box, slider, radio rows and the list with its scroll bar.
    #[test]
    fn check_slider_rows_and_list_follow_the_state() {
        use dpx_ui::requester::ctl;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let draw = |d: &Dialog| build(d, window, 1.0, &theme, &Pointer::default());
        let mut req = screen_format();
        let b = draw(&req.view());
        // The check box: on the accent with a mark while on, an empty well when off.
        let check = b.spot(ctl::RETAIN).expect("Retain").rect;
        let boxed = |b: &Built| -> Vec<(Rect, Color)> {
            b.list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::RoundRect {
                        rect,
                        fill: Some(c),
                        ..
                    } if rect.w == CHECK && rect.h == CHECK && check.inset(-1.0).encloses(rect) => {
                        Some((*rect, *c))
                    }
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            boxed(&b).iter().map(|x| x.1).collect::<Vec<_>>(),
            [theme.accent]
        );
        assert_eq!(text_color(&b, "Retain Picture"), theme.text);
        req.handle(ReqEvent::Key(ReqKey::Char('r')));
        let off = draw(&req.view());
        assert_eq!(
            boxed(&off).iter().map(|x| x.1).collect::<Vec<_>>(),
            [theme.panel]
        );
        assert_eq!(rings(&off.list, boxed(&off)[0].0), [(theme.text_dim, 1.5)]);
        // The slider: its knob stands where the Classic knob stands, the value beside it.
        let knob_x = |b: &Built| -> f32 {
            let slider = b.spot(ctl::COLORS).expect("slider").rect;
            b.list
                .ops()
                .iter()
                .find_map(|op| match op {
                    Op::Path {
                        path,
                        fill: Some(c),
                        ..
                    } if *c == theme.accent => path
                        .bounds()
                        .filter(|p| {
                            slider.inset(-2.0).encloses(p) && (p.w - 2.0 * KNOB_R).abs() < 0.1
                        })
                        .map(|p| p.x + p.w / 2.0),
                    _ => None,
                })
                .expect("the knob")
        };
        let slider = b.spot(ctl::COLORS).unwrap();
        let first = knob_x(&b);
        assert!(b.texts.iter().any(|t| t.text == "32"));
        // 32 colours are step 5 of 8: four sevenths of the way.
        let run = slider.rect.w - 2.0 * KNOB_R;
        assert!((first - (slider.rect.x + KNOB_R + run * 4.0 / 7.0)).abs() < run / 98.0 + 0.01);
        // A press on the knob changes nothing; a press at the far end sets the most.
        let at = slider.to_classic(first, slider.rect.y + 3.0);
        req.handle(ReqEvent::Press(at.0, at.1));
        req.handle(ReqEvent::Release);
        assert_eq!(req.colors(), 32);
        let at = slider.to_classic(slider.rect.right() - 1.0, slider.rect.y + 3.0);
        req.handle(ReqEvent::Press(at.0, at.1));
        req.handle(ReqEvent::Release);
        assert_eq!(req.colors(), 256);
        let most = draw(&req.view());
        assert!((knob_x(&most) - (slider.rect.right() - KNOB_R)).abs() < 0.01);
        assert!(most.texts.iter().any(|t| t.text == "256"));
        // The list: five rows, the selected one on the accent; the scroll knob follows.
        let rows: Vec<Rect> = (0..5)
            .map(|i| b.spot(ctl::ROW + i).expect("a row").rect)
            .collect();
        for pair in rows.windows(2) {
            assert_eq!(pair[0].bottom(), pair[1].y);
            assert_eq!(pair[0].h, 26.0);
        }
        let lit: Vec<bool> = rows
            .iter()
            .map(|r| fills(&b.list, *r) == [theme.accent])
            .collect();
        assert_eq!(lit, [false, false, false, false, true]);
        assert_eq!(text_color(&b, "PAL:Low Res"), theme.on_accent);
        assert_eq!(text_color(&b, "PAL:High Res"), theme.text);
        let bar = b.spot(ctl::SCROLL).expect("the scroll bar").rect;
        assert!(bar.x >= rows[0].right() && bar.h > 100.0);
        let knob = |b: &Built| -> Rect {
            b.list
                .ops()
                .iter()
                .find_map(|op| match op {
                    Op::RoundRect {
                        rect,
                        fill: Some(c),
                        ..
                    } if *c == theme.disabled && bar.encloses(rect) => Some(*rect),
                    _ => None,
                })
                .expect("the scroll knob")
        };
        req.handle(ReqEvent::Scroll(99));
        let low = knob(&draw(&req.view()));
        req.handle(ReqEvent::Scroll(-99));
        let top = knob(&draw(&req.view()));
        assert!(
            top.y < low.y && (top.y - bar.y).abs() < 0.01,
            "{top:?} {low:?}"
        );
        assert!((low.bottom() - bar.bottom()).abs() < 1.0, "{low:?} {bar:?}");
        assert!(low.h >= 8.0 && low.h < bar.h / 2.0);
        // Radio rows (Page Size's presets): the chosen one has the accent ring and dot.
        let page = page_size();
        let b = draw(&page.view());
        let row = |i: u16| {
            b.spot(dpx_ui::page_size_requester::ctl::PRESET + i)
                .unwrap()
                .rect
        };
        assert_eq!(row(0).bottom(), row(1).y);
        assert_eq!(row(0).h, 30.0);
        let dots = |rect: Rect| -> Vec<Color> {
            b.list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Path {
                        path,
                        fill: Some(c),
                        ..
                    } if path
                        .bounds()
                        .is_some_and(|p| rect.encloses(&p) && p.w < 8.0) =>
                    {
                        Some(*c)
                    }
                    _ => None,
                })
                .collect()
        };
        assert_eq!(dots(row(0)), [theme.accent]);
        assert!(dots(row(1)).is_empty() && dots(row(2)).is_empty());
        let detail = b
            .texts
            .iter()
            .find(|t| t.text == "368 x 283")
            .expect("a size");
        assert_eq!(detail.color, theme.text_dim);
        assert!((detail.ink.right() - (row(2).right() - 12.0)).abs() < 0.01);
    }

    /// Text: wrapped to the card, notes with an accent bullet, a refused value with the
    /// accent bar, the facts panel with a message in place of the facts it covers, the
    /// About text centred under the program's name.
    #[test]
    fn text_wraps_and_messages_stand_out() {
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let draw = |d: &Dialog| build(d, window, 1.0, &theme, &Pointer::default());
        let lines = wrap("one two three four five six", 60.0, SIZE, Weight::Regular);
        assert!(lines.len() > 2 && lines.join(" ") == "one two three four five six");
        assert!(
            lines
                .iter()
                .all(|l| font::measure(l, SIZE, Weight::Regular) <= 60.0)
        );
        let long = wrap(
            "Supercalifragilisticexpialidocious",
            50.0,
            SIZE,
            Weight::Regular,
        );
        assert!(long.len() > 2 && long.concat() == "Supercalifragilisticexpialidocious");
        assert!(wrap("", 50.0, SIZE, Weight::Regular).is_empty());
        // A note: one bullet per note, the wrapped lines under each other.
        let notes = strings(&[
            "4 layers saved as one picture, as shown, 1 hidden layer left out (save as .dpxproj to keep them)",
            "second",
        ]);
        let b = draw(&NoteRequester::new("Picture exported", &notes).view());
        let bullets = b
            .list
            .ops()
            .iter()
            .filter(|op| {
                matches!(op, Op::Path { fill: Some(c), path, .. }
                if *c == theme.accent && path.bounds().is_some_and(|p| p.w < 5.0))
            })
            .count();
        assert_eq!(bullets, 2);
        let body: Vec<&TextBox> = b.texts.iter().skip(1).filter(|t| t.text != "Ok").collect();
        assert!(body.len() >= 3, "the long note wraps: {}", body.len());
        assert!(body.iter().all(|t| t.ink.x == b.card.x + PAD + 14.0));
        assert_eq!(body[1].ink.y - body[0].ink.y, LINE_H);
        // Page Size refuses a size: the message with the accent bar, in the medium cut.
        let mut page = page_size();
        page.set_message("Size 320x200 to 1024x1024");
        let b = draw(&page.view());
        let message = b
            .texts
            .iter()
            .find(|t| t.text.starts_with("Size"))
            .expect("message");
        assert_eq!(message.color, theme.text);
        let bars = fills(
            &b.list,
            Rect::new(b.card.x + PAD, message.within.y + 2.0, 3.0, LINE_H - 4.0),
        );
        assert_eq!(bars, [theme.accent]);
        // Screen Format: the message covers the last facts, which are left out whole.
        let mut format = screen_format();
        let all = draw(&format.view());
        for name in [
            "Chipset",
            "Max Colors",
            "Size",
            "Max Size",
            "Pixel Shape",
            "Aspect",
        ] {
            assert!(
                all.texts.iter().filter(|t| t.text == name).count() >= 1,
                "{name}"
            );
        }
        format.set_message("OCS shows at most 32 colours in this mode");
        let refused = draw(&format.view());
        let has = |b: &Built, text: &str| b.texts.iter().any(|t| t.text == text);
        assert!(has(&refused, "Max Colors") && !has(&refused, "Aspect"));
        assert!(
            refused
                .texts
                .iter()
                .any(|t| t.text.starts_with("OCS shows"))
        );
        assert_eq!(refused.card, all.card, "a message does not move the card");
        // About: the name large and centred, the rest centred and subdued.
        let about = strings(&["DeluxePainter 0.1.0: a pixel-art editor", "Not affiliated"]);
        let b = draw(&NoteRequester::about("About DeluxePainter", &about).view());
        let name = b
            .texts
            .iter()
            .find(|t| t.text == "DeluxePainter 0.1.0")
            .expect("the name");
        let centre = b.card.x + b.card.w / 2.0;
        assert!((name.ink.x + name.ink.w / 2.0 - centre).abs() < 0.01);
        assert!(name.ink.h > 14.0, "set large");
        let rest = b
            .texts
            .iter()
            .find(|t| t.text == "A pixel-art editor")
            .expect("the rest");
        assert!((rest.ink.x + rest.ink.w / 2.0 - centre).abs() < 0.01);
        assert_eq!(text_color(&b, "Not affiliated"), theme.text_dim);
    }

    /// A list without a scroll bar and a dialog without buttons or columns do not break
    /// the layout (the kit is used by requesters that are not converted yet).
    #[test]
    fn odd_descriptions_are_laid_out() {
        let row = |i: u16| ListRow {
            control: Control::new(i, dpx_core::Rect::new(0, i as i32 * 8, 100, 8)),
            label: format!(
                "A very long file name number {i} that does not fit the row it is shown in.iff"
            ),
            detail: "12345".into(),
            selected: i == 1,
        };
        let dialog = Dialog {
            title: "A title that is much longer than any card is wide, to be cut with an ellipsis"
                .into(),
            kind: Kind::Plain,
            size: Size::Medium,
            columns: vec![vec![
                Item::List(ListView {
                    rows: (0..3).map(row).collect(),
                    capacity: 6,
                    scroll: None,
                }),
                Item::Rule,
                Item::Credits(strings(&["One", "Two"])),
            ]],
            buttons: Vec::new(),
        };
        let b = built(&dialog, (720.0, 520.0), 1.0, ThemeKind::Light);
        assert_eq!(b.card.w, 480.0);
        assert_eq!(b.spots.len(), 3);
        assert!(b.texts[0].text.ends_with('\u{2026}'));
        for t in &b.texts {
            assert!(t.within.inset(-0.5).encloses(&t.ink), "{t:?}");
        }
        let cut: Vec<&str> = b.texts.iter().map(|t| t.text.as_str()).collect();
        assert!(
            cut.iter()
                .any(|t| t.starts_with("A very long") && t.ends_with('\u{2026}')),
            "{cut:?}"
        );
        let none = Dialog {
            title: String::new(),
            kind: Kind::Plain,
            size: Size::Small,
            columns: Vec::new(),
            buttons: Vec::new(),
        };
        let b = built(&none, (720.0, 520.0), 2.0, ThemeKind::Dark);
        assert!(b.spots.is_empty() && b.card.h > 0.0);
        let bar = ScrollBar {
            control: Control::new(9, dpx_core::Rect::new(0, 0, 10, 0)),
            knob: dpx_core::Rect::new(0, 0, 10, 0),
        };
        let degenerate = Dialog {
            columns: vec![vec![Item::List(ListView {
                rows: Vec::new(),
                capacity: 2,
                scroll: Some(bar),
            })]],
            ..none
        };
        let b = built(&degenerate, (720.0, 520.0), 1.0, ThemeKind::Dark);
        assert!(b.list.len() > 3);
    }

    /// The card reaches the pixels: its ground, the accent of the default button, a field's
    /// well, and ink inside every text's box, dark at 1x and light at 2x.
    #[test]
    fn the_card_reaches_the_pixels() {
        for (kind, scale) in [(ThemeKind::Dark, 1.0f32), (ThemeKind::Light, 2.0)] {
            let theme = Theme::of(kind);
            let (w, h) = (720.0, 520.0);
            let b = built(&page_size().view(), (w, h), scale, kind);
            let (pw, ph) = ((w * scale) as u32, (h * scale) as u32);
            let rgba = Renderer::new().render(&b.list, &[], pw, ph, scale, theme.canvas);
            let at = |x: f32, y: f32| {
                let i = (((y * scale) as u32 * pw + (x * scale) as u32) * 4) as usize;
                Color::rgb(rgba[i], rgba[i + 1], rgba[i + 2])
            };
            assert_eq!(at(b.card.x + 8.0, b.card.y + 30.0), theme.raised);
            assert_eq!(at(3.0, 3.0), theme.canvas);
            // Just outside the card: the shadow darkens the canvas.
            let beside = at(b.card.x + b.card.w / 2.0, b.card.bottom() + 3.0);
            assert!(beside != theme.canvas && beside.luminance() < theme.canvas.luminance());
            let ok = b.spot(dpx_ui::page_size_requester::ctl::OK).unwrap().rect;
            assert_eq!(at(ok.x + 5.0, ok.y + ok.h / 2.0), theme.accent);
            let field = b
                .spot(dpx_ui::page_size_requester::ctl::HEIGHT)
                .unwrap()
                .rect;
            assert_eq!(
                at(field.right() - 6.0, field.y + field.h / 2.0),
                theme.panel
            );
            for t in &b.texts {
                let mut inked = 0;
                let (x0, x1) = (
                    (t.ink.x * scale) as u32,
                    (t.ink.right() * scale).ceil() as u32,
                );
                let (y0, y1) = (
                    (t.ink.y * scale) as u32,
                    (t.ink.bottom() * scale).ceil() as u32,
                );
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = ((y * pw + x) * 4) as usize;
                        let c = Color::rgb(rgba[i], rgba[i + 1], rgba[i + 2]);
                        if c.contrast(t.ground) >= 3.0 {
                            inked += 1;
                        }
                    }
                }
                assert!(inked >= 4, "{kind:?}: no ink for '{}'", t.text);
            }
        }
    }
    /// In the window: the card lies over the picture and the chrome, both veiled; the
    /// picture keeps the place it has without a requester (it hosts none); the overlay
    /// covers the picture and the card with its shadow.
    #[test]
    fn the_window_shows_the_card_over_the_veiled_picture() {
        use crate::build::{self, Model};
        use crate::frame::{Frame, render_window};
        use crate::raster::Image;
        let palette = dpx_core::Palette::dpaint_default_32();
        let ui = dpx_ui::UiState::default();
        let page: Vec<u8> = [200u8, 150, 100, 255].repeat(320 * 256);
        let dialog = page_size().view();
        for (kind, scale, size) in [
            (ThemeKind::Dark, 1.0, (1280.0, 800.0)),
            (ThemeKind::Light, 2.0, (720.0, 520.0)),
        ] {
            let theme = Theme::of(kind);
            let frame = |dialog: Option<&'static Dialog>, plain_picture: bool| Frame {
                size,
                scale,
                theme,
                model: Model {
                    dimmed: dialog.is_some() || !plain_picture,
                    dialog,
                    plain_picture,
                    ..Model::new(&ui, &palette)
                },
                picture: Image {
                    width: 320,
                    height: 256,
                    rgba: &page,
                },
                per_ui_pixel: 1,
            };
            // (A leaked copy: the frame borrows the description for as long as `ui`.)
            let shown = frame(Some(Box::leak(Box::new(dialog.clone()))), true);
            let mut idle = frame(None, true);
            idle.model.dimmed = false;
            let l = shown.layout();
            assert_eq!(l, idle.layout(), "the picture keeps its place");
            let (bounds, list) = build::overlay(&l, &theme, &shown.model).expect("the card");
            assert_eq!(
                list.ops()[0],
                Op::RoundRect {
                    rect: l.picture,
                    radius: 0.0,
                    fill: Some(theme.scrim),
                    stroke: None
                },
                "the veil over the picture comes first"
            );
            let card = built(&dialog, size, scale, kind);
            assert!(bounds.encloses(&card.bounds) && bounds.encloses(&l.picture));
            assert!(l.window.encloses(&bounds));
            assert_eq!(list.len(), card.list.len() + 1);
            let (w, _, rgba) = render_window(&shown, &mut Renderer::new());
            let at = |x: f32, y: f32| {
                let i = (((y * scale) as u32 * w + (x * scale) as u32) * 4) as usize;
                Color::rgb(rgba[i], rgba[i + 1], rgba[i + 2])
            };
            assert_eq!(at(card.card.x + 8.0, card.card.y + 30.0), theme.raised);
            let a = u32::from(theme.scrim.a);
            let veiled = |c: Color| {
                let ch = |under: u8, over: u8| {
                    ((u32::from(over) * a + u32::from(under) * (255 - a) + 127) / 255) as u8
                };
                Color::rgb(
                    ch(c.r, theme.scrim.r),
                    ch(c.g, theme.scrim.g),
                    ch(c.b, theme.scrim.b),
                )
            };
            let near = |a: Color, b: Color| {
                a.r.abs_diff(b.r) <= 1 && a.g.abs_diff(b.g) <= 1 && a.b.abs_diff(b.b) <= 1
            };
            let corner = at(l.picture.x + 2.0, l.picture.y + 2.0);
            assert!(
                near(corner, veiled(Color::rgb(200, 150, 100))),
                "{kind:?}: {corner:?}"
            );
            let bar = l.bar.expect("bar");
            assert!(near(
                at(bar.right() - 3.0, bar.y + 3.0),
                veiled(theme.panel)
            ));
        }
    }

    /// A form (slice 4b): the first radio button or check box of a row stands where the
    /// labels stand, what follows it starts at one place in every line; the chosen radio
    /// button has the accent and a dot; a ghosted one is faint and takes no pointer; a
    /// refused number is shown under the form.
    #[test]
    fn form_rows_line_up_and_show_their_state() {
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let draw = |d: &Dialog| build(d, window, 1.0, &theme, &Pointer::default());
        let b = draw(&tool(&spacing(SpacingMode::EveryN)));
        let left = b.card.x + PAD;
        let radios: Vec<Rect> = (0..4)
            .map(|i| b.spot(form_ctl::RADIO + i).expect("a radio button").rect)
            .collect();
        assert!(radios.iter().all(|r| r.x == left), "{radios:?}");
        for pair in radios.windows(2) {
            assert!(
                pair[1].y - pair[0].bottom() <= 8.0,
                "the rows belong together"
            );
        }
        let fields: Vec<Rect> = (0..4)
            .map(|i| b.spot(form_ctl::FIELD + i).expect("a field").rect)
            .collect();
        assert!(
            fields
                .iter()
                .all(|f| f.x == fields[0].x && f.w == fields[0].w)
        );
        assert!(
            fields[0].x >= radios[2].right() + 4.0,
            "{fields:?} {radios:?}"
        );
        assert!(
            fields[3].y > fields[2].bottom() + 10.0,
            "the rule lies between"
        );
        // The dots: only "Every Nth dot" has one, in the accent.
        let dot = |rect: Rect| -> Vec<Color> {
            b.list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Path {
                        path,
                        fill: Some(c),
                        ..
                    } if path
                        .bounds()
                        .is_some_and(|p| rect.encloses(&p) && (p.w - 7.0).abs() < 0.1) =>
                    {
                        Some(*c)
                    }
                    _ => None,
                })
                .collect()
        };
        let lit: Vec<bool> = radios.iter().map(|r| dot(*r) == [theme.accent]).collect();
        assert_eq!(lit, [false, false, true, false]);
        assert_eq!(text_color(&b, "pixels"), theme.text_dim);
        assert_eq!(text_color(&b, "Airbrush size"), theme.text_dim);
        // Fill Type: two columns of radio buttons, the second ghosted; the check box under
        // the first column.
        let b = draw(&tool(&fill()));
        let solid = b.spot(form_ctl::RADIO).expect("Solid").rect;
        let check = b.spot(form_ctl::CHECK).expect("Dither").rect;
        assert_eq!((solid.x, check.x), (left, left));
        for ghost in 5..10 {
            assert!(
                b.spot(form_ctl::RADIO + ghost).is_none(),
                "ghosted: no pointer"
            );
        }
        assert_eq!(text_color(&b, "Brush"), theme.disabled);
        assert_eq!(text_color(&b, "Solid"), theme.text);
        let brush = b.texts.iter().find(|t| t.text == "Brush").expect("Brush");
        let ridge = b.texts.iter().find(|t| t.text == "Ridge").expect("Ridge");
        assert_eq!(brush.ink.x, ridge.ink.x, "the second column lines up");
        assert!(brush.ink.x > solid.right());
        assert!(b.texts.iter().any(
            |t| t.text == "Range: FG colour range, else FG to BG" && t.color == theme.text_dim
        ));
        // The refused number.
        let mut refused = spacing(SpacingMode::Airbrush);
        let before = draw(&tool(&refused));
        for key in [ReqKey::Tab, ReqKey::Tab, ReqKey::Backspace, ReqKey::Enter] {
            refused.handle(ReqEvent::Key(key));
        }
        let after = draw(&tool(&refused));
        let message = after
            .texts
            .iter()
            .find(|t| t.text == "Airbrush must be 1 to 999")
            .expect("the message");
        assert_eq!(message.color, theme.text);
        assert!(after.card.h > before.card.h);
        let ok = after.spot(form_ctl::OK).expect("Ok").rect;
        assert!(message.ink.bottom() < ok.y);
    }

    fn part_line(parts: Vec<Part>) -> Dialog {
        Dialog {
            title: "Parts".into(),
            kind: Kind::Plain,
            size: Size::Medium,
            columns: vec![vec![Item::Line {
                label: "Label".into(),
                key: None,
                parts,
            }]],
            buttons: Vec::new(),
        }
    }

    /// Buttons and icon buttons inside a line: quiet with a hairline, latched on the accent,
    /// under the pointer lighter, ghosted flat without a pointer; side by side they stand
    /// closer together than other parts. Every pictogram draws something inside its button.
    #[test]
    fn buttons_in_a_line_are_quiet_latched_or_ghosted() {
        use dpx_ui::dialog::Glyph;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let area = |i: i32| dpx_core::Rect::new(10 * i, 0, 9, 9);
        let push = |id: ControlId, label: &str, on: bool, enabled: bool| {
            Part::Button(Push {
                control: Control::new(id, area(i32::from(id))),
                label: label.into(),
                on,
                pressed: false,
                enabled,
            })
        };
        let glyphs = [
            Glyph::Forward,
            Glyph::Backward,
            Glyph::PingPong,
            Glyph::GoFrom,
            Glyph::ComeTo,
            Glyph::InPlace,
            Glyph::StepForward,
            Glyph::StepBackward,
        ];
        let mut parts = vec![
            push(0, "Move", true, true),
            push(1, "Ex", false, true),
            push(2, "Clear", false, false),
        ];
        for (i, glyph) in glyphs.into_iter().enumerate() {
            parts.push(Part::Icon(IconButton {
                control: Control::new(10 + i as ControlId, area(10 + i as i32)),
                glyph,
                on: i == 1,
                pressed: false,
                enabled: i != 2,
            }));
        }
        let dialog = part_line(parts);
        let b = build(&dialog, window, 1.0, &theme, &Pointer::default());
        let (on, off) = (b.spot(0).unwrap().rect, b.spot(1).unwrap().rect);
        assert_eq!(fills(&b.list, on), [theme.accent]);
        assert!(rings(&b.list, on).is_empty());
        assert_eq!(fills(&b.list, off), [theme.hover]);
        assert_eq!(rings(&b.list, off), [(theme.hairline, 1.0)]);
        assert_eq!(off.x - on.right(), GROUP_GAP);
        assert_eq!(
            (text_color(&b, "Move"), text_color(&b, "Ex")),
            (theme.on_accent, theme.text)
        );
        assert!(off.w >= PUSH_MIN_W && on.w > off.w && on.h == 28.0);
        assert!(b.spot(2).is_none(), "ghosted: no pointer");
        assert_eq!(text_color(&b, "Clear"), theme.disabled);
        let over = Pointer {
            at: Some(off.center()),
            down: false,
            captured: None,
        };
        let h = build(&dialog, window, 1.0, &theme, &over);
        assert_eq!(fills(&h.list, off), [button_fill(&theme, true, false)]);
        // The icon buttons: equal squares in a row, the second latched, the third ghosted.
        let icons: Vec<Option<Rect>> = (0..8).map(|i| b.spot(10 + i).map(|s| s.rect)).collect();
        assert!(icons[2].is_none());
        let first = icons[0].expect("an icon button");
        assert_eq!((first.w, first.h), (ICON_W, 28.0));
        assert_eq!(icons[1].unwrap().x - first.right(), GROUP_GAP);
        assert_eq!(fills(&b.list, icons[1].unwrap()), [theme.accent]);
        for (i, rect) in icons.iter().enumerate() {
            let Some(rect) = rect else { continue };
            let ink = if i == 1 { theme.on_accent } else { theme.text };
            let drawn = b
                .list
                .ops()
                .iter()
                .filter(|op| match op {
                    Op::Path {
                        path, fill, stroke, ..
                    } => {
                        path.bounds().is_some_and(|p| rect.inset(5.0).encloses(&p))
                            && (*fill == Some(ink) || stroke.is_some_and(|s| s.color == ink))
                    }
                    _ => false,
                })
                .count();
            assert!(drawn >= 1, "glyph {i} is drawn inside its button");
        }
    }

    /// The palette as swatches: a grid of equal squares, centred, or a strip as wide as the
    /// column; one control whose cells lie over the Classic cells, so a point on a swatch
    /// is a point on the same swatch there, also in a short last row; the marks are drawn.
    #[test]
    fn swatches_lie_over_the_classic_cells() {
        use dpx_ui::dialog::{Swatch, Swatches};
        let pitch = 8;
        let grid = |colours: usize, cols: usize, strip: bool| {
            let rows = colours.div_ceil(cols) as i32;
            let cells = (0..colours)
                .map(|i| Swatch {
                    rgb: [i as u8, 255 - i as u8, 128],
                    mark: match i {
                        1 => Mark::Chosen,
                        2 => Mark::Picked,
                        3 => Mark::Target,
                        _ => Mark::None,
                    },
                })
                .collect();
            Dialog {
                title: "Swatches".into(),
                kind: Kind::Plain,
                size: Size::Medium,
                columns: vec![vec![Item::Swatches(Swatches {
                    control: Control::new(
                        7,
                        dpx_core::Rect::new(56, 17, cols as i32 * pitch, rows * pitch),
                    ),
                    cols,
                    cells,
                    strip,
                })]],
                buttons: Vec::new(),
            }
        };
        for (colours, cols, strip) in [
            (32, 16, false),
            (256, 16, false),
            (20, 16, false),
            (2, 2, true),
            (32, 16, true),
            (256, 32, true),
        ] {
            let dialog = grid(colours, cols, strip);
            for window in WINDOWS {
                for scale in SCALES {
                    let at = format!("{colours} colours, strip {strip}, {window:?} at {scale}x");
                    let b = built(&dialog, window, scale, ThemeKind::Dark);
                    let win = Rect::new(0.0, 0.0, window.0, window.1);
                    assert!(win.encloses(&b.card), "{at}: {:?}", b.card);
                    let spot = b.spot(7).expect("the grid");
                    let rows = colours.div_ceil(cols);
                    let (cw, ch) = (spot.rect.w / cols as f32, spot.rect.h / rows as f32);
                    if strip {
                        assert!((spot.rect.w - (b.card.w - 2.0 * PAD)).abs() < 0.01, "{at}");
                    } else {
                        assert!((cw - ch).abs() < 0.01 && cw >= 18.0, "{at}: {cw} x {ch}");
                        let centre = spot.rect.x + spot.rect.w / 2.0;
                        assert!((centre - (b.card.x + b.card.w / 2.0)).abs() <= 1.0, "{at}");
                    }
                    // The middle and the corners of every swatch are that swatch in Classic.
                    for i in 0..colours {
                        let (col, row) = ((i % cols) as f32, (i / cols) as f32);
                        for (fx, fy) in [(0.5, 0.5), (0.04, 0.04), (0.96, 0.96)] {
                            let (x, y) =
                                (spot.rect.x + (col + fx) * cw, spot.rect.y + (row + fy) * ch);
                            let (id, p) = b.press(x, y);
                            assert_eq!(id, Some(7), "{at}");
                            let cell = ((p.1 - 17) / pitch) as usize * cols
                                + ((p.0 - 56) / pitch) as usize;
                            assert_eq!(cell, i, "{at}: ({x}, {y}) -> {p:?}");
                        }
                    }
                }
            }
            // Every swatch is filled with its colour; the three marks draw over theirs.
            let b = built(&dialog, WINDOWS[1], 1.0, ThemeKind::Light);
            let colour = |i: usize| Color::rgb(i as u8, 255 - i as u8, 128);
            let filled = |c: Color| {
                b.list
                    .ops()
                    .iter()
                    .filter(|op| matches!(op, Op::RoundRect { fill: Some(f), .. } if *f == c))
                    .count()
            };
            for i in 0..colours {
                assert_eq!(filled(colour(i)), 1, "swatch {i} of {colours}");
            }
            let marks = b
                .list
                .ops()
                .iter()
                .filter(|op| matches!(op, Op::Path { fill: Some(f), .. } if *f == MARK_DARK))
                .count();
            assert_eq!(marks, if colours > 2 { 2 } else { 1 }, "chosen and picked");
        }
    }

    /// A ghosted slider shows its groove and its value faint and takes no pointer.
    #[test]
    fn a_ghosted_slider_takes_no_pointer() {
        let theme = Theme::light();
        let slider = |enabled| {
            part_line(vec![Part::Slider(Slider {
                control: Control::new(3, dpx_core::Rect::new(51, 70, 174, 9)),
                knob: dpx_core::Rect::new(60, 71, 10, 7),
                value: "16".into(),
                enabled,
            })])
        };
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let on = build(&slider(true), window, 1.0, &theme, &Pointer::default());
        let off = build(&slider(false), window, 1.0, &theme, &Pointer::default());
        assert!(on.spot(3).is_some() && off.spot(3).is_none());
        assert_eq!(text_color(&on, "16"), theme.text);
        assert_eq!(text_color(&off, "16"), theme.disabled);
        assert!(off.list.len() < on.list.len(), "no knob");
    }
    /// The file requester's fields (slice 4c): a text field for a long text takes the
    /// line's width, all three stand under each other, and a press sets the caret under
    /// the pointer: it is sent at the Classic x of the caret position nearest to it.
    #[test]
    fn a_text_field_takes_the_line_and_sets_the_caret_under_the_pointer() {
        use dpx_ui::file_requester::ctl;
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = file_requester(dpx_ui::FileKind::SavePicture, 4);
        req.set_file("castle.dpxproj");
        let b = build(
            &req.view(),
            window,
            1.0,
            &Theme::dark(),
            &Pointer::default(),
        );
        let (drawer, pattern, file) = (
            b.spot(ctl::DRAWER).unwrap().rect,
            b.spot(ctl::PATTERN).unwrap().rect,
            b.spot(ctl::FILE).unwrap().rect,
        );
        assert_eq!((drawer.x, drawer.w), (file.x, file.w));
        assert_eq!((pattern.x, pattern.w), (file.x, file.w));
        assert_eq!(
            file.right(),
            b.card.right() - PAD,
            "to the card's inner edge"
        );
        // Every caret position of the shown text: a press just right of it lands on it.
        let text = "castle.dpxproj";
        let mid = file.y + file.h / 2.0;
        for caret in 0..=text.len() {
            let x = file.x + 10.0 + font::measure(&text[..caret], SIZE, Weight::Regular);
            let (taken, at) = b.press(x + 0.4, mid);
            assert_eq!(taken, Some(ctl::FILE));
            let mut r = req.clone();
            r.handle(ReqEvent::Press(at.0, at.1), 0);
            for _ in caret..text.len() {
                r.handle(ReqEvent::Key(ReqKey::Delete), 0);
            }
            assert_eq!(r.file(), &text[..caret], "caret {caret}");
        }
        // Far right of the text: the end. Left of it: the start.
        for (x, kept) in [(file.right() - 1.0, text), (file.x + 1.0, "")] {
            let (_, at) = b.press(x, mid);
            let mut r = req.clone();
            r.handle(ReqEvent::Press(at.0, at.1), 0);
            for _ in 0..text.len() {
                r.handle(ReqEvent::Key(ReqKey::Delete), 0);
            }
            assert_eq!(r.file(), kept);
        }
        // The position is inside the field's Classic rectangle.
        let area = req.view().control(ctl::FILE).unwrap().area;
        let (_, at) = b.press(file.x + 40.0, mid);
        assert!(at.0 >= area.x && at.0 < area.x + area.w);
        assert!(at.1 >= area.y && at.1 < area.y + area.h);
    }

    /// The file list (slice 4c): drawers say what they are, the chosen file is marked, a
    /// long name ends in an ellipsis inside its row, and the bar's knob follows the list.
    #[test]
    fn the_file_list_shows_places_the_chosen_file_and_long_names() {
        use dpx_ui::file_requester::ctl;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = file_requester(dpx_ui::FileKind::LoadPicture, 30);
        req.set_file("castle-02.iff");
        let b = build(&req.view(), window, 1.0, &theme, &Pointer::default());
        let texts: Vec<&str> = b.texts.iter().map(|t| t.text.as_str()).collect();
        assert!(texts.contains(&"Brushes") && texts.contains(&"Drawer"));
        let long = b
            .texts
            .iter()
            .find(|t| t.text.starts_with("a-very-long-name-"))
            .expect("the long name");
        assert!(long.text.ends_with('…'), "{}", long.text);
        assert!(long.within.encloses(&long.ink));
        let row = b.spot(ctl::ROW + 5).expect("the chosen row").rect;
        assert!(fills(&b.list, row).contains(&theme.accent));
        assert_eq!(text_color(&b, "castle-02.iff"), theme.on_accent);
        // The scroll bar maps into the Classic track, top to top and bottom to bottom.
        let bar = b.spot(ctl::SCROLL).expect("the bar");
        let area = bar.control.area;
        assert_eq!(bar.to_classic(bar.rect.x + 1.0, bar.rect.y).1, area.y);
        assert_eq!(
            bar.to_classic(bar.rect.x + 1.0, bar.rect.bottom() - 0.01).1,
            area.y + area.h - 1
        );
    }
    /// Import Picture (slice 4c): the option rows stand under each other, the focused
    /// option's label lies on a bar with the accent around it, a ghosted option is faint and
    /// its buttons take no pointer, and the thumbnail is a picture of the list in a frame:
    /// whole picture pixels where the column allows, never wider than the column.
    #[test]
    fn option_rows_show_the_focus_and_the_thumbnail_is_in_the_list() {
        use dpx_ui::import_requester::ctl;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = import_requester("photo.jpg", (320, 256));
        req.set_thumbnail(thumbnail(&req));
        let draw = |r: &dpx_ui::ImportRequester, scale: f32| {
            build(&r.view(), window, scale, &theme, &Pointer::default())
        };
        let b = draw(&req, 1.0);
        // The first radio button of every option starts at the same x.
        let xs: Vec<f32> = [ctl::POSITION, ctl::PALETTE, ctl::FEWER, ctl::DITHER]
            .iter()
            .map(|&id| b.spot(id).expect("a control").rect.x)
            .collect();
        assert!(xs.iter().all(|&x| x == xs[0]), "{xs:?}");
        // All three dither choices are on the card, left to right, without overlap.
        let dither: Vec<Rect> = (0..3)
            .map(|i| b.spot(ctl::DITHER + i).unwrap().rect)
            .collect();
        assert!(dither[0].right() <= dither[1].x && dither[1].right() <= dither[2].x);
        // No focus: no accent ring but the default button's fill; with Tab the label's bar.
        let rings = |b: &Built| {
            b.list
                .ops()
                .iter()
                .filter(|op| {
                    matches!(op, Op::RoundRect { stroke: Some(s), rect, .. }
                        if s.color == theme.accent && rect.h == 22.0)
                })
                .count()
        };
        assert_eq!(rings(&b), 0);
        assert_eq!(text_color(&b, "Position"), theme.text_dim);
        req.handle(ReqEvent::Key(ReqKey::Tab));
        let b = draw(&req, 1.0);
        assert_eq!(rings(&b), 1);
        assert_eq!(text_color(&b, "Position"), theme.text);
        assert_eq!(text_color(&b, "Palette"), theme.text_dim);
        // The current palette: the count's label and number are faint, its buttons gone.
        req.handle(ReqEvent::Key(ReqKey::Tab));
        req.handle(ReqEvent::Key(ReqKey::Down));
        let b = draw(&req, 1.0);
        assert_eq!(text_color(&b, "Colours"), theme.disabled);
        assert_eq!(text_color(&b, "32"), theme.text_dim);
        assert!(b.spot(ctl::FEWER).is_none() && b.spot(ctl::MORE).is_none());
        // The thumbnail: one op that carries the pixels, in the right column, in a frame.
        let (tw, th) = req.thumb_size();
        for scale in [1.0, 1.5, 2.0] {
            let b = draw(&req, scale);
            let pictures: Vec<(Rect, u32, u32)> = b
                .list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Pixels { rect, pixels } => Some((*rect, pixels.width, pixels.height)),
                    _ => None,
                })
                .collect();
            assert_eq!(pictures.len(), 1);
            let (rect, w, h) = pictures[0];
            assert_eq!((w, h), (tw, th));
            assert!(b.card.encloses(&rect));
            assert!(rect.x > dither[2].right(), "right of the options");
            // A whole number of points per picture pixel, on whole physical pixels.
            let k = rect.w / tw as f32;
            assert!(k >= 1.0 && (k - k.round()).abs() < 0.02, "{k}");
            assert!(((rect.h / th as f32) - k).abs() < 0.02);
            for v in [rect.x, rect.y, rect.w, rect.h] {
                assert!(
                    ((v * scale).round() - v * scale).abs() < 1e-3,
                    "{v} at {scale}"
                );
            }
        }
        // Rasterised: the left half red, the right half blue, the frame around it.
        let b = draw(&req, 1.0);
        let rect = b
            .list
            .ops()
            .iter()
            .find_map(|op| match op {
                Op::Pixels { rect, .. } => Some(*rect),
                _ => None,
            })
            .unwrap();
        let rgba = Renderer::new().render(&b.list, &[], 1280, 800, 1.0, theme.canvas);
        let at = |x: f32, y: f32| {
            let i = ((y as usize) * 1280 + x as usize) * 4;
            [rgba[i], rgba[i + 1], rgba[i + 2]]
        };
        let (cx, cy) = rect.center();
        assert_eq!(at(rect.x + 2.0, cy), [200, 30, 30]);
        assert_eq!(at(rect.right() - 2.0, cy), [30, 30, 200]);
        assert_eq!(at(rect.x + 2.0, rect.y + 1.0), [200, 30, 30]);
        assert_eq!(
            at(cx, rect.bottom() + 2.0),
            [theme.panel.r, theme.panel.g, theme.panel.b]
        );
        // Without a thumbnail: the frame with an empty well of the same size.
        let empty = draw(&import_requester("photo.jpg", (320, 256)), 1.0);
        assert!(
            !empty
                .list
                .ops()
                .iter()
                .any(|op| matches!(op, Op::Pixels { .. }))
        );
        assert_eq!(empty.card, b.card);
        // A picture much larger than the column is fitted into it.
        let big = Dialog {
            columns: vec![vec![Item::Picture {
                width: 2000,
                height: 500,
                picture: Some(Thumbnail::new(2000, 500, vec![9; 2000 * 500 * 4])),
            }]],
            ..req.view()
        };
        let b = build(&big, window, 1.0, &theme, &Pointer::default());
        let rect = b
            .list
            .ops()
            .iter()
            .find_map(|op| match op {
                Op::Pixels { rect, .. } => Some(*rect),
                _ => None,
            })
            .unwrap();
        assert!(b.card.encloses(&rect) && rect.w > 400.0);
    }
    /// The Image Generator setup (slice 4c): the key field shows one star per character and
    /// the caret; the key is in no text, no drawing and no dump of the card.
    #[test]
    fn the_setup_shows_stars_and_never_the_key() {
        use dpx_ui::imagegen_setup::ctl;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = setup_requester(dpx_ui::SetupProvider::OpenAi);
        for k in dpx_ui::paste_keys("sk-short-Key1") {
            req.handle(ReqEvent::Key(k));
        }
        let b = build(&req.view(), window, 1.0, &theme, &Pointer::default());
        let stars = "*".repeat("sk-short-Key1".len());
        assert_eq!(text_color(&b, &stars), theme.text, "the stars are shown");
        // The field with the keys has the accent around it and a caret after the stars.
        let key = b.spot(ctl::KEY).expect("the key field").rect;
        assert!(rings(&b.list, key).contains(&(theme.accent, 1.5)));
        let model = b.spot(ctl::MODEL).expect("the model field").rect;
        assert_eq!((key.x, key.w), (model.x, model.w));
        // A long key: still stars only, in the field, with nothing of the key anywhere.
        for k in dpx_ui::paste_keys(SECRET) {
            req.handle(ReqEvent::Key(k));
        }
        for (name, dialog) in dialogs().into_iter().chain([("typed twice", req.view())]) {
            if !name.starts_with("setup") && name != "typed twice" {
                continue;
            }
            for scale in SCALES {
                let b = build(&dialog, window, scale, &theme, &Pointer::default());
                let dump = format!("{:?} {:?} {:?} {dialog:?}", b.list, b.texts, b.spots);
                for part in ["sk-", "Secret", "Key1", "0123456789", "abcdefgh", "short"] {
                    assert!(!dump.contains(part), "{name}: '{part}' is in the dump");
                }
                for t in &b.texts {
                    assert!(t.within.inset(-0.5).encloses(&t.ink), "{name}: {}", t.text);
                }
            }
        }
        // None: the fields are faint and empty and take no pointer; their buttons too.
        let none = setup_requester(dpx_ui::SetupProvider::None);
        let b = build(&none.view(), window, 1.0, &theme, &Pointer::default());
        assert!(b.spot(ctl::KEY).is_none() && b.spot(ctl::MODEL).is_none());
        assert!(b.spot(ctl::TEST).is_none() && b.spot(ctl::GET_KEY).is_none());
        assert_eq!(text_color(&b, "API key"), theme.disabled);
        assert_eq!(text_color(&b, "Test"), theme.disabled);
        // The model id is still named by the hint under the ghosted field, nowhere else.
        let named = b
            .texts
            .iter()
            .filter(|t| t.text.contains("gpt-image-2.6-flare"))
            .count();
        assert_eq!(named, 1);
    }

    /// Generate Picture (slice 4c): the arrows of an option stand side by side before its
    /// value, a ghosted arrow takes no pointer, and "More.." puts four options into the
    /// right column over the note.
    #[test]
    fn generate_rows_have_arrows_and_more_fills_the_right_column() {
        use dpx_ui::GenRow;
        use dpx_ui::generate_requester::ctl;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let mut req = generate_requester("a castle", (320, 256));
        let b = build(&req.view(), window, 1.0, &theme, &Pointer::default());
        let prompt = b.spot(ctl::PROMPT).expect("the description").rect;
        let next = b.spot(ctl::next(GenRow::Resolution)).unwrap().rect;
        let prev = b.spot(ctl::prev(GenRow::Resolution)).unwrap().rect;
        assert_eq!(prev.right() + GROUP_GAP, next.x);
        assert!(prompt.bottom() < prev.y);
        assert_eq!(
            prompt.x,
            b.card.x + PAD,
            "the description starts at the card's edge"
        );
        // OCS is the first chipset: its "previous" arrow is ghosted.
        assert!(b.spot(ctl::prev(GenRow::Chipset)).is_none());
        assert!(b.spot(ctl::next(GenRow::Chipset)).is_some());
        assert!(
            b.spot(ctl::next(GenRow::Target)).is_none(),
            "not shown without More.."
        );
        req.set_more(true);
        let m = build(&req.view(), window, 1.0, &theme, &Pointer::default());
        let (dither, target) = (
            m.spot(ctl::next(GenRow::Dither)).unwrap().rect,
            m.spot(ctl::next(GenRow::Target)).unwrap().rect,
        );
        assert!(dither.x < prompt.right() && target.x > prompt.right());
        // The default button is Generate; with a result it is Apply.
        let default =
            |b: &Built, id| fills(&b.list, b.spot(id).unwrap().rect).contains(&theme.accent);
        assert!(default(&m, ctl::GENERATE));
        req.set_phase(dpx_ui::GenPhase::Result);
        let (w, h) = req.thumb_size();
        let fill = dpx_core::Rgba8::new(1, 2, 3, 255);
        req.set_thumbnail(Some(dpx_core::RgbaImage::new(w, h, fill)));
        let r = build(&req.view(), window, 1.0, &theme, &Pointer::default());
        assert!(default(&r, ctl::APPLY) && !default(&r, ctl::GENERATE));
        assert_eq!(text_color(&r, "Again"), theme.text);
        assert!(
            r.list
                .ops()
                .iter()
                .any(|op| matches!(op, Op::Pixels { .. }))
        );
    }
    /// Prefs ▸ AI & MCP (slice 4c): two columns, the permissions as check boxes under each
    /// other (a ghosted one is faint), the connect buttons in two lines that start at the
    /// same x, the last calls one line each in a quiet panel, cut with an ellipsis.
    #[test]
    fn ai_and_mcp_shows_status_permissions_and_the_last_calls() {
        use dpx_ui::mcp_requester::ctl;
        let theme = Theme::dark();
        let window = Rect::new(0.0, 0.0, 1280.0, 800.0);
        let req = dpx_ui::McpRequester::new(mcp_view(), (320, 256));
        let b = build(&req.dialog(), window, 1.0, &theme, &Pointer::default());
        let rect = |id| b.spot(id).unwrap_or_else(|| panic!("control {id}")).rect;
        assert_eq!(rect(ctl::CONNECT_DESKTOP).x, rect(ctl::DISCONNECT).x);
        assert!(rect(ctl::CONNECT_DESKTOP).bottom() < rect(ctl::DISCONNECT).y);
        assert_eq!(
            rect(ctl::CONNECT_DESKTOP).right() + GROUP_GAP,
            rect(ctl::CONNECT_CODE).x
        );
        let (read, edit, over) = (rect(ctl::READ), rect(ctl::EDIT), rect(ctl::OVERWRITE));
        assert!(read.x == edit.x && edit.x == over.x);
        assert!(read.bottom() <= edit.y && edit.bottom() <= over.y);
        assert!(read.x > rect(ctl::CONNECT_CODE).right(), "the right column");
        assert!(rect(ctl::SET_UP).y > over.bottom());
        // On is chosen; the status is whole.
        assert!(
            b.texts
                .iter()
                .any(|t| t.text.starts_with("Status: Ready on 127.0.0.1:8765"))
        );
        // Five calls, each in the panel, the newest last.
        let calls: Vec<&TextBox> = b
            .texts
            .iter()
            .filter(|t| t.text.contains("draw_rectangle"))
            .collect();
        assert_eq!(calls.len(), 5);
        assert!(calls[4].text.starts_with("7s ") && calls[4].ink.y > calls[0].ink.y);
        for call in &calls {
            assert_eq!(call.ground, theme.panel);
            assert!(call.within.encloses(&call.ink));
        }
        // Without reading, the other permissions are faint.
        let none = dpx_ui::McpView {
            read: false,
            ..mcp_view()
        };
        let b = build(
            &dpx_ui::McpRequester::new(none, (320, 256)).dialog(),
            window,
            1.0,
            &theme,
            &Pointer::default(),
        );
        assert_eq!(text_color(&b, "Read document info"), theme.text);
        assert_eq!(text_color(&b, "Edit pictures and palettes"), theme.disabled);
        assert_eq!(
            text_color(&b, "Overwrite files without asking"),
            theme.disabled
        );
    }
}
