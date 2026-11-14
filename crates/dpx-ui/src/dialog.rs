//! What a requester shows, as data (ADR-47, slice 4a): the model the Modern skin draws its
//! dialogs from.
//!
//! A requester stays the single owner of its state and behaviour: it takes [`ReqEvent`]s in
//! its own (Classic) coordinates and answers as before. For the Modern skin it additionally
//! describes itself as a [`Dialog`]: a title, columns of items (text, form lines, option
//! rows, a list) and the buttons at the bottom. Every part that takes the pointer carries a
//! [`Control`]: an id and the rectangle in which the Classic requester takes that part's
//! press. The Modern skin lays the parts out in points, draws them in its own look and turns
//! a pointer position over one of them into the matching position inside the control's
//! Classic rectangle. The requester then gets the same events as in the Classic skin, so no
//! dialog logic exists twice.
//!
//! Nothing here draws, and nothing here is used by the Classic skin.
//!
//! [`ReqEvent`]: crate::requester::ReqEvent

use dpx_core::Rect;

/// Names a control inside one dialog (unique there; each requester defines its own ids).
pub type ControlId = u16;

/// A part that takes the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    pub id: ControlId,
    /// Where the Classic requester takes this control's press, in the coordinates of its
    /// `handle` function. Never empty.
    pub area: Rect,
}

impl Control {
    pub const fn new(id: ControlId, area: Rect) -> Self {
        Self { id, area }
    }
}

/// How text is weighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    /// Secondary: explanations, read-only values.
    Dim,
    /// A message that needs attention (a refused value).
    Alert,
}

/// What a button means to the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Return presses it.
    Default,
    /// Esc presses it.
    Cancel,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub control: Control,
    pub label: String,
    pub role: Role,
    /// Held down with the pointer over it.
    pub pressed: bool,
    pub enabled: bool,
    /// The letter of the label that is its key (underlined), if it has one.
    pub key: Option<char>,
}

impl Button {
    pub fn new(control: Control, label: &str, role: Role, pressed: bool) -> Self {
        Self {
            control,
            label: label.to_string(),
            role,
            pressed,
            enabled: true,
            key: None,
        }
    }

    pub fn key(mut self, key: char) -> Self {
        self.key = Some(key);
        self
    }
}

/// A one-line text or number field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub control: Control,
    pub text: String,
    /// The field has the keys: the caret stands before this character (the text's length
    /// for the end).
    pub caret: Option<usize>,
    /// `false`: the value is fixed by another choice and only shown.
    pub enabled: bool,
    /// Digits only (the field is as wide as `max_chars` digits).
    pub numeric: bool,
    /// Most characters the field takes; its width follows.
    pub max_chars: usize,
}

/// A gadget whose press moves on to the next of a few values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cycle {
    pub control: Control,
    /// The shown value alone, or both values of a gadget that has exactly two: the Modern
    /// skin then shows them side by side and a press on the other one switches.
    pub options: Vec<String>,
    /// Index of the shown value in `options`.
    pub selected: usize,
    /// `false`: ghosted (a press may still be answered with a message).
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub control: Control,
    pub label: String,
    pub on: bool,
    pub enabled: bool,
    pub key: Option<char>,
}

/// A slider that takes the pointer along its length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slider {
    /// The whole track.
    pub control: Control,
    /// The knob inside the track, in the same coordinates.
    pub knob: Rect,
    /// The value in words, shown beside the slider.
    pub value: String,
    /// `false`: ghosted, takes no pointer (nothing to set yet).
    pub enabled: bool,
}

/// One of a few choices that stand in a form line (a radio button).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Radio {
    pub control: Control,
    pub label: String,
    pub on: bool,
    pub enabled: bool,
}

/// A button inside a form line. `on`: it stays down (one of a few modes, or a switch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Push {
    pub control: Control,
    pub label: String,
    pub on: bool,
    /// Held down with the pointer over it.
    pub pressed: bool,
    pub enabled: bool,
}

/// What an icon button shows (the Modern skin draws its own pictograms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// Play forward: a triangle pointing right.
    Forward,
    /// Play backward: a triangle pointing left.
    Backward,
    /// There and back: two triangles pointing apart.
    PingPong,
    /// Move away from here: an arrow leaving a dot.
    GoFrom,
    /// Arrive here: an arrow reaching a dot.
    ComeTo,
    /// Stay on the frame: two upright bars.
    InPlace,
    /// Step on frame by frame: two chevrons pointing right.
    StepForward,
    /// Step back frame by frame: two chevrons pointing left.
    StepBackward,
}

/// A button that shows a pictogram, inside a form line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconButton {
    pub control: Control,
    pub glyph: Glyph,
    /// It is the chosen one of its group.
    pub on: bool,
    pub pressed: bool,
    pub enabled: bool,
}

/// How a swatch is marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    None,
    /// Chosen (a protected colour, a colour of the range).
    Chosen,
    /// Picked up: the swatch a drag started on.
    Picked,
    /// Where a dragged swatch would land.
    Target,
}

/// One colour of a swatch grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swatch {
    pub rgb: [u8; 3],
    pub mark: Mark,
}

/// The palette as a grid of colours that takes the pointer as one control: the Classic
/// requester has the same number of columns and rows in `control.area`, all cells alike, so a
/// point on a swatch is a point on the same swatch there (also during a drag over several).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Swatches {
    /// The whole grid: `cols` columns and as many full rows as `cells` need.
    pub control: Control,
    pub cols: usize,
    /// Row by row; the last row may be short.
    pub cells: Vec<Swatch>,
    /// `true`: a palette strip (the cells touch and share the column's width); `false`:
    /// separate square swatches.
    pub strip: bool,
}

impl Swatches {
    /// Rows of the grid (at least one).
    pub fn rows(&self) -> usize {
        self.cells.len().div_ceil(self.cols.max(1)).max(1)
    }
}

/// One part of a form line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Field(Field),
    Cycle(Cycle),
    Check(Check),
    Slider(Slider),
    Text(String, Tone),
    Radio(Radio),
    Button(Push),
    Icon(IconButton),
}

/// One of a few rows of which one is chosen (radio rows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub control: Control,
    pub label: String,
    /// Shown after the label, subdued (a size, a key).
    pub detail: String,
    pub selected: bool,
    pub pressed: bool,
    pub enabled: bool,
}

/// One visible row of a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRow {
    pub control: Control,
    pub label: String,
    /// Right-aligned in the row.
    pub detail: String,
    pub selected: bool,
}

/// The scroll bar of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollBar {
    /// The whole track.
    pub control: Control,
    /// The knob inside the track, in the same coordinates.
    pub knob: Rect,
}

/// A list of rows; only the visible ones are described.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListView {
    pub rows: Vec<ListRow>,
    /// How many rows the list shows at most (its height).
    pub capacity: usize,
    pub scroll: Option<ScrollBar>,
}

/// One item of a column, from top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A small heading over a group.
    Heading(String),
    /// A paragraph, wrapped to the column; `bullet` marks one entry of several.
    Text {
        text: String,
        tone: Tone,
        bullet: bool,
    },
    /// A form line: its label at the left, its parts after it.
    Line {
        label: String,
        key: Option<char>,
        parts: Vec<Part>,
    },
    /// A form line without a label: its first part (a radio button or a check box) stands
    /// where the labels of the other lines stand, the rest follows as in a `Line`.
    Row(Vec<Part>),
    /// Rows of which one is chosen.
    Choices(Vec<Choice>),
    List(ListView),
    /// The palette as swatches.
    Swatches(Swatches),
    /// Names and values in a quiet panel (read-only facts). A message, if any, is shown in
    /// the panel in place of the facts it would cover in the Classic skin.
    Facts {
        rows: Vec<(String, String)>,
        message: Option<String>,
    },
    /// Centred lines in a quiet panel.
    Credits(Vec<String>),
    /// A fine line between groups.
    Rule,
}

/// How much room a dialog asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// A message or a question.
    Small,
    /// A form.
    Medium,
    /// Two columns.
    Large,
}

/// The look of the whole dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    /// The program's own "About" text: its first paragraph names the program.
    About,
}

/// A requester as the Modern skin shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dialog {
    pub title: String,
    pub kind: Kind,
    pub size: Size,
    /// One column, or two side by side.
    pub columns: Vec<Vec<Item>>,
    /// The buttons at the bottom, in the Classic order from left to right.
    pub buttons: Vec<Button>,
}

impl Dialog {
    /// Every control of the dialog, in the order of its description.
    pub fn controls(&self) -> Vec<Control> {
        let mut out = Vec::new();
        for item in self.columns.iter().flatten() {
            match item {
                Item::Line { parts, .. } | Item::Row(parts) => {
                    for part in parts {
                        match part {
                            Part::Field(f) => out.push(f.control),
                            Part::Cycle(c) => out.push(c.control),
                            Part::Check(c) => out.push(c.control),
                            Part::Slider(s) => out.push(s.control),
                            Part::Radio(r) => out.push(r.control),
                            Part::Button(b) => out.push(b.control),
                            Part::Icon(b) => out.push(b.control),
                            Part::Text(..) => {}
                        }
                    }
                }
                Item::Choices(rows) => out.extend(rows.iter().map(|r| r.control)),
                Item::List(list) => {
                    out.extend(list.rows.iter().map(|r| r.control));
                    out.extend(list.scroll.map(|s| s.control));
                }
                Item::Swatches(grid) => out.push(grid.control),
                Item::Heading(_)
                | Item::Text { .. }
                | Item::Facts { .. }
                | Item::Credits(_)
                | Item::Rule => {}
            }
        }
        out.extend(self.buttons.iter().map(|b| b.control));
        out
    }

    /// The control with this id.
    pub fn control(&self, id: ControlId) -> Option<Control> {
        self.controls().into_iter().find(|c| c.id == id)
    }
}

/// The start of `original` that holds its first `shown` characters which are not white
/// space, and whether something was cut. Requesters wrap their text into lines for the
/// Classic skin; this gives the same amount of text back as running text.
pub(crate) fn shown_prefix(original: &str, shown: usize) -> (String, bool) {
    let mut seen = 0;
    let mut out = String::new();
    let mut gap = false;
    for word in original.split_whitespace() {
        for c in word.chars() {
            if seen == shown {
                return (out, true);
            }
            if gap {
                out.push(' ');
                gap = false;
            }
            out.push(c);
            seen += 1;
        }
        gap = true;
    }
    (out, false)
}

/// The number of characters of `lines` that are not white space.
pub(crate) fn ink_count<'a>(lines: impl Iterator<Item = &'a str>) -> usize {
    lines
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_keeps_the_words_apart_and_says_when_it_cut() {
        let text = "one  two three";
        assert_eq!(shown_prefix(text, 99), ("one two three".into(), false));
        assert_eq!(shown_prefix(text, 11), ("one two three".into(), false));
        assert_eq!(shown_prefix(text, 6), ("one two".into(), true));
        assert_eq!(shown_prefix(text, 4), ("one t".into(), true));
        assert_eq!(shown_prefix(text, 0), (String::new(), true));
        assert_eq!(shown_prefix("", 0), (String::new(), false));
        assert_eq!(ink_count(["a b", " cd "].into_iter()), 4);
    }
}
