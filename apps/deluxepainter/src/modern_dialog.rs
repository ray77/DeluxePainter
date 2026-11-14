//! Requesters in the Modern look (ADR-47, slice 4a): which open requester the Modern skin
//! draws itself as a card, and how the pointer over that card reaches the requester.
//!
//! A converted requester keeps its state and its `handle` function. It describes itself as
//! a `dpx_ui::dialog::Dialog`; `dpx_ui_modern::dialog` lays that out, draws it and maps a
//! pointer position over a control to a position inside the rectangle in which the Classic
//! requester takes that control's press. The events then go the Classic way
//! (`modal_events`, the `*_event` functions), so both skins answer alike.
//!
//! Only the requester that has the pointer and the keys (the first one `pass()` asks) is
//! shown as a card. Every requester that is not converted yet is shown in the Classic look
//! inside the picture quad, as before.

use dpx_core::Palette;
use dpx_ui::dialog::Dialog;
use dpx_ui::{
    AnimControlRequester, AnimRequester, ChoiceRequester, FileRequester, GenerateRequester,
    ImportRequester, McpRequester, NoteRequester, PageSizeRequester, RangeRequester,
    ScreenFormatRequester, ToolRequester, UiState,
};
use dpx_ui_modern::Theme;
use dpx_ui_modern::dialog as kit;
use eframe::egui;

use crate::PainterApp;

/// The requesters the Modern skin draws itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Card {
    /// A question with two buttons (Yes/No, Recover/Discard, Replace/Keep ..).
    Choice,
    /// Picture ▸ Screen Format.
    ScreenFormat,
    /// A note, and Picture ▸ About.
    Note,
    /// Picture ▸ Page Size.
    PageSize,
    /// A toolbox settings requester: Gridding, Symmetry, Spacing, Fill Type, Translucency,
    /// Make Stencil, Arrange Palette, the AnimBrush requesters, Perspective.
    Tool,
    /// Color ▸ Ranges..
    Range,
    /// An Anim menu requester (Set Frame Count, Add, Copy, Delete Frames, Go to Frame, Rate).
    Anim,
    /// Anim ▸ Control ▸ Set Range.. and Anim ▸ Move..
    AnimControl,
    /// The file requester: Load, Save, Export, Delete, palettes, brushes, stencils, anims.
    File,
    /// Import Picture (after Picture ▸ Load.. of a PNG or JPEG).
    Import,
    /// Picture ▸ Generate..
    Generate,
    /// The Image Generator setup (Set up.. in Generate, and Prefs ▸ AI & MCP..).
    Setup,
    /// Prefs ▸ AI & MCP..
    Mcp,
}

/// The requester that has the pointer and the keys, if the Modern skin draws it: the
/// requesters in the order `PainterApp::pass` asks them, up to the first that is open.
/// `screen_format`: the Screen Format requester is open; `playing`: an animation plays;
/// `preview`: the Move preview runs.
pub(crate) fn card(
    ui: &UiState,
    screen_format: bool,
    playing: bool,
    preview: bool,
) -> Option<Card> {
    if preview {
        return None;
    }
    if ui.anim_control.is_some() {
        return Some(Card::AnimControl);
    }
    if ui.choice_requester.is_some() {
        return Some(Card::Choice);
    }
    if ui.import_requester.is_some() {
        return Some(Card::Import);
    }
    if ui.imagegen_setup.is_some() {
        return Some(Card::Setup);
    }
    if ui.generate_requester.is_some() {
        return Some(Card::Generate);
    }
    if ui.mcp_requester.is_some() {
        return Some(Card::Mcp);
    }
    if screen_format {
        return Some(Card::ScreenFormat);
    }
    if ui.tool_requester.is_some() {
        return Some(Card::Tool);
    }
    if playing {
        return None;
    }
    if ui.anim_requester.is_some() {
        return Some(Card::Anim);
    }
    if ui.palette_requester.is_some() {
        return None;
    }
    if ui.range_requester.is_some() {
        return Some(Card::Range);
    }
    if ui.note_requester.is_some() {
        return Some(Card::Note);
    }
    if ui.file_requester.is_some() {
        return Some(Card::File);
    }
    ui.page_size_requester.is_some().then_some(Card::PageSize)
}

/// What `card` shows; `palette` gives the swatches of a palette requester their colours.
pub(crate) fn view(
    card: Card,
    ui: &UiState,
    screen_format: Option<&ScreenFormatRequester>,
    palette: &Palette,
) -> Option<Dialog> {
    match card {
        Card::Tool => ui.tool_requester.as_ref().map(|r| r.view(palette)),
        Card::Range => ui.range_requester.as_ref().map(|r| r.view(palette)),
        Card::Anim => ui.anim_requester.as_ref().map(AnimRequester::view),
        Card::AnimControl => ui.anim_control.as_ref().map(AnimControlRequester::view),
        Card::Choice => ui.choice_requester.as_ref().map(|r| r.view()),
        Card::ScreenFormat => screen_format.map(ScreenFormatRequester::view),
        Card::Note => ui.note_requester.as_ref().map(|r| r.view()),
        Card::PageSize => ui.page_size_requester.as_ref().map(|r| r.view()),
        Card::File => ui.file_requester.as_ref().map(FileRequester::view),
        Card::Import => ui.import_requester.as_ref().map(ImportRequester::view),
        Card::Generate => ui.generate_requester.as_ref().map(GenerateRequester::view),
        Card::Setup => ui.imagegen_setup.as_ref().map(|r| r.view()),
        Card::Mcp => ui.mcp_requester.as_ref().map(McpRequester::dialog),
    }
}

/// A requester of the shared interface state that is still shown in the Classic look is
/// open (the picture quad hosts it).
pub(crate) fn classic_look_open(ui: &UiState) -> bool {
    ui.palette_requester.is_some()
}

/// `ui` without the requesters the Modern skin draws itself: what the picture quad is
/// composed from while a card is shown.
pub(crate) fn without_cards(ui: &mut UiState) {
    Cards::take(ui);
}

/// The converted requesters, taken out of the interface state for a while.
pub(crate) struct Cards {
    note: Option<NoteRequester>,
    choice: Option<ChoiceRequester>,
    page_size: Option<PageSizeRequester>,
    tool: Option<ToolRequester>,
    anim: Option<AnimRequester>,
    range: Option<RangeRequester>,
    anim_control: Option<AnimControlRequester>,
    file: Option<FileRequester>,
    import: Option<ImportRequester>,
    generate: Option<GenerateRequester>,
    setup: Option<dpx_ui::ImageGenSetupRequester>,
    mcp: Option<McpRequester>,
}

impl Cards {
    /// Takes every requester the Modern skin draws itself out of `ui`.
    pub(crate) fn take(ui: &mut UiState) -> Self {
        Self {
            note: ui.note_requester.take(),
            choice: ui.choice_requester.take(),
            page_size: ui.page_size_requester.take(),
            tool: ui.tool_requester.take(),
            anim: ui.anim_requester.take(),
            range: ui.range_requester.take(),
            anim_control: ui.anim_control.take(),
            file: ui.file_requester.take(),
            import: ui.import_requester.take(),
            generate: ui.generate_requester.take(),
            setup: ui.imagegen_setup.take(),
            mcp: ui.mcp_requester.take(),
        }
    }

    /// Puts them back.
    pub(crate) fn put_back(self, ui: &mut UiState) {
        ui.note_requester = self.note;
        ui.choice_requester = self.choice;
        ui.page_size_requester = self.page_size;
        ui.tool_requester = self.tool;
        ui.anim_requester = self.anim;
        ui.range_requester = self.range;
        ui.anim_control = self.anim_control;
        ui.file_requester = self.file;
        ui.import_requester = self.import;
        ui.generate_requester = self.generate;
        ui.imagegen_setup = self.setup;
        ui.mcp_requester = self.mcp;
    }
}

impl PainterApp {
    /// The requester the Modern skin shows as a card now; `None` in the Classic skin, with
    /// no requester open, and while the one on top is not converted yet.
    pub(crate) fn modern_card(&self) -> Option<Card> {
        if !self.is_modern() {
            return None;
        }
        card(
            &self.ui,
            self.requester.is_some(),
            self.anim.playing.is_some(),
            self.anim_control.preview.is_some(),
        )
    }

    /// The description of the card that is shown.
    pub(crate) fn modern_dialog(&self) -> Option<Dialog> {
        view(
            self.modern_card()?,
            &self.ui,
            self.requester.as_ref(),
            self.palette(),
        )
    }

    /// The Screen Format requester while it has a screen of its own: in the Classic skin.
    /// In the Modern skin it is a card over the main window.
    pub(crate) fn own_screen(&self) -> Option<&ScreenFormatRequester> {
        self.requester.as_ref().filter(|_| !self.is_modern())
    }

    /// A card is all that is open: the picture's image holds no requester.
    pub(crate) fn modern_plain_picture(&self) -> bool {
        self.modern_card().is_some() && !classic_look_open(&self.ui) && self.show_page.is_none()
    }

    /// While a card is shown: the pointer of this pass in the coordinates of the requester's
    /// `handle` function (`Some(None)`: the pointer is outside the window). `None` when no
    /// card is shown and the Classic mapping applies.
    ///
    /// A press is taken by the control under it and stays with that control until the
    /// button is up, as in the Classic requesters; over no control the position is
    /// [`kit::NOWHERE`]. Also keeps what the card's drawing needs to know about the pointer.
    pub(crate) fn modern_dialog_pos(&mut self, ctx: &egui::Context) -> Option<Option<(i32, i32)>> {
        let dialog = self.modern_dialog()?;
        let layout = self.modern_layout()?;
        let (hover, press, down) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
            )
        });
        let area = ctx.content_rect();
        let mut pointer = self.modern.pointer;
        pointer.at = hover.map(|p| (p.x - area.min.x, p.y - area.min.y));
        pointer.down = down;
        // Where the controls are does not depend on the theme.
        let built = kit::build(
            &dialog,
            layout.window,
            layout.scale,
            &Theme::dark(),
            &pointer,
        );
        let pos = pointer.at.map(|(x, y)| {
            if press {
                let (taken, at) = built.press(x, y);
                pointer.captured = taken;
                at
            } else if down {
                built.drag(pointer.captured, x, y)
            } else {
                kit::NOWHERE
            }
        });
        if !down {
            pointer.captured = None;
        }
        self.modern.pointer = pointer;
        Some(pos)
    }

    /// Once per pass: with no card shown nothing of a card's pointer is kept.
    pub(crate) fn modern_dialog_pass(&mut self) {
        if self.modern_card().is_none() {
            self.modern.pointer = kit::Pointer::default();
        }
    }
}
