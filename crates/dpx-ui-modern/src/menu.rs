//! The Modern skin's pull-down menus (ADR-47). They work as in DeluxePaint: while the right
//! mouse button is held on the bar, the menu under the pointer drops down as a card, a
//! submenu opens beside its item, and letting go over an item picks it. They also work as
//! a usual menu bar: a left click on a title opens its menu and it stays open
//! ([`open_sticky`]); the pointer rules are the same, the app decides what a click does.
//!
//! Only the geometry and the look are new. What is open and what is under the pointer is
//! written into the same [`MenuState`] the Classic skin uses, so the action under the
//! pointer, the check marks, the ghosting and everything a picked item does are shared
//! (`MenuState::hovered_action`, `Action::checked`, `Action::enabled`).

use dpx_ui::menu::{Action, Entry, Item, Key, MENUS, MenuContext, MenuState, menu_enabled};

use crate::font;
use crate::icons;
use crate::layout::MainLayout;
use crate::paint::{Color, List, Rect, Weight};
use crate::theme::Theme;

/// Height of one item.
pub const ROW_H: f32 = 24.0;
/// Free space above the first and below the last item of a card.
pub const PAD: f32 = 5.0;
/// Corner radius of a card.
pub const RADIUS: f32 = 8.0;
/// Corner radius of the highlight under the pointer.
const ROW_RADIUS: f32 = 5.0;
/// The highlight's distance from the card's left and right edge.
const INSET: f32 = 4.0;
/// Where an item's text starts, from the card's left edge, in a card without marks.
const LABEL_X: f32 = 12.0;
/// Extra room at the left in a card whose items carry check marks or radio dots.
const MARK_W: f32 = 16.0;
/// Least space between the widest label and the widest shortcut.
const KEY_GAP: f32 = 28.0;
/// Free space right of the shortcuts.
const RIGHT: f32 = 12.0;
/// Width of the submenu chevron's column.
const CHEVRON_W: f32 = 6.0;
/// Side of the key cap that stands for the Amiga key, and the space after it.
const CAP: f32 = 15.0;
const CAP_GAP: f32 = 4.0;
/// No card is narrower than this.
const MIN_W: f32 = 132.0;
/// The space between the bar and a menu's card.
const DROP: f32 = 2.0;
/// A submenu's card overlaps its parent by this much.
const SUB_OVERLAP: f32 = 3.0;
/// The space a card keeps to the window's edges.
const EDGE: f32 = 4.0;
/// How far a card's shadow reaches beyond it (for the overlay's bounds).
pub const SHADOW_REACH: f32 = 12.0;

const SIZE: f32 = font::SIZE_BAR;

/// A menu or submenu as a card: where it is and how its rows are laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub rect: Rect,
    /// The number of items.
    pub rows: usize,
    /// Its items carry check marks or radio dots: the labels start further right.
    pub marks: bool,
}

/// Whether an item shows a mark when it is on.
fn marked(item: &Item) -> bool {
    item.action().is_some_and(|a| {
        matches!(
            a,
            Action::Mode(_)
                | Action::Pref(_)
                | Action::PixelSize(_)
                | Action::UiStyle
                | Action::Theme(_)
        ) || matches!(
            a,
            Action::Command("anim.method_compressed" | "anim.method_expanded")
        )
    })
}

/// Whether an item's mark is a radio dot (one of a group) rather than a check mark.
fn radio(action: Action) -> bool {
    !matches!(action, Action::Pref(_) | Action::UiStyle)
}

/// The width of what an item shows at its right: shortcut or submenu chevron.
fn hint_width(item: &Item) -> f32 {
    match (item.entry, item.key) {
        (Entry::Submenu(_), _) => CHEVRON_W,
        (_, Key::Amiga(c)) => {
            CAP + CAP_GAP + font::measure(c.encode_utf8(&mut [0; 4]), SIZE, Weight::Regular)
        }
        (_, Key::Plain(k)) if on_chip(k) => chip_width(k),
        (_, Key::Plain(k)) => font::measure(k, SIZE, Weight::Regular),
        (_, Key::None) => 0.0,
    }
}

/// Height of the chip a punctuation key stands on, and the least width.
const CHIP: f32 = 18.0;

/// A key that is one punctuation mark (the backtick, the tilde, a comma ..) is shown on a
/// chip in the Medium weight: alone and in the subdued colour such a glyph is a speck that
/// does not read as a key.
fn on_chip(key: &str) -> bool {
    let mut chars = key.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_punctuation())
}

fn chip_width(key: &str) -> f32 {
    (font::measure(key, SIZE, Weight::Medium) + 10.0).max(CHIP)
}

impl Card {
    /// The card of `items` with its top-left corner wanted at (`x`, `y`): moved into the
    /// window where it would leave it, its edges on whole physical pixels.
    fn new(items: &[Item], x: f32, y: f32, layout: &MainLayout) -> Card {
        let marks = items.iter().any(marked);
        let label = items
            .iter()
            .map(|it| font::measure(it.label, SIZE, Weight::Regular))
            .fold(0.0, f32::max);
        let hint = items.iter().map(hint_width).fold(0.0, f32::max);
        let hint_room = if hint > 0.0 { KEY_GAP + hint } else { 0.0 };
        let w = (LABEL_X + if marks { MARK_W } else { 0.0 } + label + hint_room + RIGHT)
            .ceil()
            .max(MIN_W);
        let h = 2.0 * PAD + items.len() as f32 * ROW_H;
        let window = layout.window;
        // Below the bar when there is room, never outside the window.
        let top = layout.bar.map_or(EDGE, |b| b.bottom() + DROP);
        let x = x.min(window.right() - EDGE - w).max(EDGE);
        let y = y.min(window.bottom() - EDGE - h).max(top);
        let snap = |v: f32| (v * layout.scale).round() / layout.scale;
        let (x0, y0) = (snap(x), snap(y));
        Card {
            rect: Rect::new(x0, y0, snap(x + w) - x0, snap(y0 + h) - y0),
            rows: items.len(),
            marks,
        }
    }

    /// The rectangle of item `i`: the highlight under the pointer.
    pub fn row(&self, i: usize) -> Rect {
        Rect::new(
            self.rect.x + INSET,
            self.rect.y + PAD + i as f32 * ROW_H,
            self.rect.w - 2.0 * INSET,
            ROW_H,
        )
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y)
    }

    /// The item under (`x`, `y`), if any. A row answers over the card's whole width.
    pub fn item_at(&self, x: f32, y: f32) -> Option<usize> {
        let dy = y - (self.rect.y + PAD);
        (self.contains(x, y) && dy >= 0.0)
            .then(|| (dy / ROW_H) as usize)
            .filter(|&i| i < self.rows)
    }

    /// Where the labels start.
    fn label_x(&self) -> f32 {
        self.rect.x + LABEL_X + if self.marks { MARK_W } else { 0.0 }
    }
}

/// The card of top-level menu `menu`, below its title. `None` while the bar is hidden.
pub fn menu_card(layout: &MainLayout, menu: usize) -> Option<Card> {
    let bar = layout.bar?;
    let title = layout.menu_titles.get(menu)?;
    Some(Card::new(
        MENUS.get(menu)?.items,
        title.x,
        bar.bottom() + DROP,
        layout,
    ))
}

/// The card of the submenu `items` of item `item` in `parent`: at the parent's right with
/// its first row level with the item, or at its left when it would leave the window.
pub fn submenu_card(layout: &MainLayout, parent: &Card, item: usize, items: &[Item]) -> Card {
    let y = parent.row(item).y - PAD;
    let right = Card::new(items, parent.rect.right() - SUB_OVERLAP, y, layout);
    if parent.rect.right() - SUB_OVERLAP + right.rect.w <= layout.window.right() - EDGE {
        right
    } else {
        Card::new(items, parent.rect.x + SUB_OVERLAP - right.rect.w, y, layout)
    }
}

/// The right button went down on the bar at the window point (`x`, `y`): the menus are
/// held open, the one under the pointer dropped down.
pub fn open(layout: &MainLayout, state: &mut MenuState, x: f32, y: f32) {
    *state = MenuState {
        active: true,
        ..MenuState::default()
    };
    pointer(layout, state, x, y);
}

/// The left button went down on a title of the bar at the window point (`x`, `y`): its
/// menu drops down and stays open after the button is let go (`MenuState::sticky`).
/// Returns whether a menu opened: nothing happens beside the titles or with the bar hidden.
pub fn open_sticky(layout: &MainLayout, state: &mut MenuState, x: f32, y: f32) -> bool {
    if crate::hit::menu_title(layout, x, y).is_none() {
        return false;
    }
    open(layout, state, x, y);
    state.sticky = true;
    true
}

/// Whether the window point (`x`, `y`) belongs to the open menus: a title of the bar or a
/// card that is shown. A left click anywhere else closes menus that stay open.
pub fn covers(layout: &MainLayout, ctx: &MenuContext, state: &MenuState, x: f32, y: f32) -> bool {
    crate::hit::menu_title(layout, x, y).is_some()
        || cards(layout, ctx, state)
            .iter()
            .any(|(card, ..)| card.contains(x, y))
}

/// Tracks the pointer at the window point (`x`, `y`) while the menus are held; returns
/// whether the visible state changed. The rules are the Classic skin's
/// (`MenuState::pointer`): on the bar the title under the pointer opens its menu, in a card
/// the row under the pointer is the item, a submenu's card comes first, and outside
/// everything no item is chosen while the menu stays open. One difference: back on the
/// submenu's own parent row the sub item is no longer chosen.
pub fn pointer(layout: &MainLayout, state: &mut MenuState, x: f32, y: f32) -> bool {
    let before = *state;
    if !state.active {
        return false;
    }
    let Some(bar) = layout.bar else {
        return false;
    };
    if y < bar.bottom() {
        if let Some(i) = crate::hit::menu_title(layout, x, y) {
            state.menu = Some(i);
        }
        state.item = None;
        state.sub_item = None;
        return *state != before;
    }
    let Some((m, card)) = state.menu.and_then(|m| Some((m, menu_card(layout, m)?))) else {
        return false;
    };
    if let Some(it) = state.item
        && let Entry::Submenu(items) = MENUS[m].items[it].entry
    {
        let sub = submenu_card(layout, &card, it, items);
        if sub.contains(x, y) {
            state.sub_item = sub.item_at(x, y);
            return *state != before;
        }
    }
    if card.contains(x, y) {
        // Back on the menu's own card nothing of a submenu is chosen any more, also on the
        // row the submenu belongs to (the Classic skin keeps the sub item there).
        state.item = card.item_at(x, y);
        state.sub_item = None;
    } else {
        state.item = None;
        state.sub_item = None;
    }
    *state != before
}

/// The cards that `state` shows, the menu's first: nothing while no menu is open, and no
/// submenu card for a ghosted item (as in the Classic skin).
pub fn cards(
    layout: &MainLayout,
    ctx: &MenuContext,
    state: &MenuState,
) -> Vec<(Card, &'static [Item], Option<usize>)> {
    let mut out = Vec::new();
    if !state.active {
        return out;
    }
    let Some((m, card)) = state.menu.and_then(|m| Some((m, menu_card(layout, m)?))) else {
        return out;
    };
    let items = MENUS[m].items;
    out.push((card, items, state.item));
    if let Some(it) = state.item
        && let Entry::Submenu(sub_items) = items[it].entry
        && items[it].enabled(ctx)
    {
        out.push((
            submenu_card(layout, &card, it, sub_items),
            sub_items,
            state.sub_item,
        ));
    }
    out
}

/// Draws the open menu and submenu as cards over the window. Returns the rectangle the
/// drawing covers (cards and their shadows), `None` when nothing is open.
pub fn draw(
    list: &mut List,
    layout: &MainLayout,
    theme: &Theme,
    ctx: &MenuContext,
    state: &MenuState,
) -> Option<Rect> {
    let mut bounds: Option<Rect> = None;
    for (card, items, lit) in cards(layout, ctx, state) {
        draw_card(list, theme, ctx, &card, items, lit);
        let r = card.rect.inset(-SHADOW_REACH);
        bounds = Some(bounds.map_or(r, |b| union(&b, &r)));
    }
    bounds
}

/// The smallest rectangle around both.
pub(crate) fn union(a: &Rect, b: &Rect) -> Rect {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Rect::new(
        x,
        y,
        a.right().max(b.right()) - x,
        a.bottom().max(b.bottom()) - y,
    )
}

/// A card's ground: a soft shadow, the raised surface and a hairline around it.
pub(crate) fn card_ground(list: &mut List, theme: &Theme, rect: Rect, radius: f32) {
    // Five rings of falling size, each a fifth of the shadow's strength: soft at the rim.
    let step = Color {
        a: theme.shadow.a / 5,
        ..theme.shadow
    };
    for (grow, drop) in [(8.0, 4.0), (6.0, 3.0), (4.0, 2.0), (2.0, 1.5), (1.0, 1.0)] {
        let r = rect.inset(-grow);
        list.round_rect(Rect::new(r.x, r.y + drop, r.w, r.h), radius + grow, step);
    }
    list.round_rect(rect, radius, theme.raised);
    list.ring(rect, radius, theme.hairline, 1.0);
}

fn draw_card(
    list: &mut List,
    theme: &Theme,
    ctx: &MenuContext,
    card: &Card,
    items: &[Item],
    highlight: Option<usize>,
) {
    card_ground(list, theme, card.rect, RADIUS);
    let hint_right = card.rect.right() - RIGHT;
    for (i, item) in items.iter().enumerate() {
        let row = card.row(i);
        let mid = row.y + row.h / 2.0;
        let enabled = item.enabled(ctx);
        let lit = enabled && highlight == Some(i);
        if lit {
            list.round_rect(row, ROW_RADIUS, theme.accent);
        }
        let (ink, hint_ink) = match (enabled, lit) {
            (false, _) => (theme.disabled, theme.disabled),
            (true, true) => (theme.on_accent, theme.on_accent),
            (true, false) => (theme.text, theme.text_dim),
        };
        if let Some(action) = item.action()
            && action.checked(ctx) == Some(true)
        {
            let cx = card.rect.x + LABEL_X + 4.0;
            if radio(action) {
                list.fill(icons::radio_dot(cx, mid), ink);
            } else {
                list.stroke(icons::check_mark(cx, mid), ink, icons::MARK_STROKE);
            }
        }
        let base = font::baseline_centered(mid, SIZE, Weight::Regular);
        list.text(card.label_x(), base, item.label, SIZE, Weight::Regular, ink);
        match (item.entry, item.key) {
            (Entry::Submenu(_), _) => {
                list.stroke(
                    icons::chevron(hint_right - CHEVRON_W / 2.0, mid),
                    hint_ink,
                    icons::MARK_STROKE,
                );
            }
            (_, Key::Amiga(c)) => {
                let letter = c.to_string();
                let w = font::measure(&letter, SIZE, Weight::Regular);
                let cap = Rect::new(hint_right - w - CAP_GAP - CAP, mid - CAP / 2.0, CAP, CAP);
                list.ring(cap, 4.0, hint_ink, 1.0);
                list.stroke(icons::key_cap_letter(cap), hint_ink, 1.2);
                list.text(
                    hint_right - w,
                    base,
                    letter,
                    SIZE,
                    Weight::Regular,
                    hint_ink,
                );
            }
            (_, Key::Plain(k)) if on_chip(k) => {
                let w = chip_width(k);
                let chip = Rect::new(hint_right - w, mid - CHIP / 2.0, w, CHIP);
                if !lit {
                    list.round_rect(chip, 4.0, theme.panel);
                }
                list.ring(chip, 4.0, hint_ink, 1.0);
                let text_w = font::measure(k, SIZE, Weight::Medium);
                list.text(
                    chip.x + (chip.w - text_w) / 2.0,
                    font::baseline_centered(mid, SIZE, Weight::Medium),
                    k,
                    SIZE,
                    Weight::Medium,
                    ink,
                );
            }
            (_, Key::Plain(k)) => {
                let w = font::measure(k, SIZE, Weight::Regular);
                list.text(hint_right - w, base, k, SIZE, Weight::Regular, hint_ink);
            }
            (_, Key::None) => {}
        }
    }
}

/// How a menu title in the bar is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitleLook {
    /// No menu is held: subdued.
    Idle,
    /// The menus are held: readable.
    Ready,
    /// This title's menu is open.
    Open,
    /// The menus are held and nothing in this menu can be chosen.
    Ghosted,
}

/// The look of the title of menu `menu`.
pub fn title_look(ctx: &MenuContext, state: &MenuState, menu: usize) -> TitleLook {
    if !state.active {
        TitleLook::Idle
    } else if state.menu == Some(menu) {
        TitleLook::Open
    } else if menu_enabled(menu, ctx) {
        TitleLook::Ready
    } else {
        TitleLook::Ghosted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Chrome;
    use crate::paint::Op;
    use dpx_ui::{PaintMode, PixelSize, Pref, ThemeChoice, UiState, UiStyle};

    const WINDOWS: [((f32, f32), f32); 5] = [
        ((1280.0, 800.0), 1.0),
        ((1280.0, 800.0), 2.0),
        ((720.0, 520.0), 1.0),
        ((1001.0, 640.0), 1.5),
        ((900.0, 600.0), 1.25),
    ];

    fn layout(window: (f32, f32), scale: f32) -> MainLayout {
        MainLayout::new(
            window,
            scale,
            Chrome::ALL,
            32,
            (320, 256),
            PixelSize::default(),
        )
    }

    fn modern_ctx() -> MenuContext {
        UiState {
            ui_style: UiStyle::Modern,
            ..UiState::default()
        }
        .menu_context()
    }

    fn title_point(l: &MainLayout, menu: usize) -> (f32, f32) {
        let r = l.menu_titles[menu];
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    fn row_point(card: &Card, i: usize) -> (f32, f32) {
        let r = card.row(i);
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    /// Every item of every menu and submenu is reached by the pointer and sets the same
    /// `MenuState` fields the Classic skin would, in every window.
    #[test]
    fn every_item_is_reachable_and_sets_the_shared_menu_state() {
        for (window, scale) in WINDOWS {
            let l = layout(window, scale);
            for (m, menu) in MENUS.iter().enumerate() {
                let mut state = MenuState::default();
                let (tx, ty) = title_point(&l, m);
                open(&l, &mut state, tx, ty);
                assert_eq!(
                    state,
                    MenuState {
                        active: true,
                        menu: Some(m),
                        item: None,
                        sub_item: None,
                        sticky: false,
                    }
                );
                let card = menu_card(&l, m).expect("bar shown");
                for (i, item) in menu.items.iter().enumerate() {
                    let (x, y) = row_point(&card, i);
                    pointer(&l, &mut state, x, y);
                    assert_eq!(
                        (state.menu, state.item, state.sub_item),
                        (Some(m), Some(i), None),
                        "{} > {} in {window:?} at {scale}",
                        menu.title,
                        item.label
                    );
                    let Entry::Submenu(sub_items) = item.entry else {
                        continue;
                    };
                    let sub = submenu_card(&l, &card, i, sub_items);
                    for (s, sub_item) in sub_items.iter().enumerate() {
                        let (x, y) = row_point(&sub, s);
                        // The part of the row that the parent does not lie under.
                        let x = if sub.rect.x >= card.rect.x {
                            x.max(card.rect.right() + 1.0)
                        } else {
                            x.min(card.rect.x - 1.0)
                        };
                        pointer(&l, &mut state, x, y);
                        assert_eq!(
                            (state.menu, state.item, state.sub_item),
                            (Some(m), Some(i), Some(s)),
                            "{} > {} > {}",
                            menu.title,
                            item.label,
                            sub_item.label
                        );
                    }
                    // Back on the parent's row: the submenu's choice is gone.
                    let (x, y) = row_point(&card, i);
                    let x = if sub.rect.x >= card.rect.x {
                        x.min(sub.rect.x - 1.0)
                    } else {
                        x.max(sub.rect.right() + 1.0)
                    };
                    pointer(&l, &mut state, x, y);
                    assert_eq!((state.item, state.sub_item), (Some(i), None));
                }
            }
        }
    }

    /// Cards lie inside the window, below the bar, on whole physical pixels; a submenu
    /// touches its parent and never hides its parent item's label.
    #[test]
    fn cards_stay_in_the_window_and_flip_at_its_edge() {
        for (window, scale) in WINDOWS {
            let l = layout(window, scale);
            let bar = l.bar.expect("bar");
            let inside = |c: &Card, what: &str| {
                let r = c.rect;
                assert!(r.x >= 0.0 && r.right() <= l.window.right(), "{what}: {r:?}");
                assert!(
                    r.y >= bar.bottom() && r.bottom() <= l.window.bottom(),
                    "{what}: {r:?} in {window:?}"
                );
                for v in [r.x, r.y, r.right(), r.bottom()] {
                    assert!(((v * scale) - (v * scale).round()).abs() < 1e-3, "{what}");
                }
            };
            for (m, menu) in MENUS.iter().enumerate() {
                let card = menu_card(&l, m).expect("card");
                inside(&card, menu.title);
                for (i, item) in menu.items.iter().enumerate() {
                    let Entry::Submenu(items) = item.entry else {
                        continue;
                    };
                    let sub = submenu_card(&l, &card, i, items);
                    let what = format!("{} > {}", menu.title, item.label);
                    inside(&sub, &what);
                    let left = sub.rect.right() <= card.rect.x + SUB_OVERLAP + 0.01;
                    let label_end =
                        card.label_x() + font::measure(item.label, SIZE, Weight::Regular);
                    assert!(left || sub.rect.x >= label_end, "{what} covers its parent");
                    assert!(
                        left || sub.rect.x <= card.rect.right(),
                        "{what} touches its parent"
                    );
                }
            }
        }
        // A window so narrow that the last menu's submenus do not fit at the right.
        let l = layout((720.0, 520.0), 1.0);
        let prefs = MENUS.len() - 1;
        let card = menu_card(&l, prefs).expect("card");
        let last = MENUS[prefs].items.len() - 1;
        let Entry::Submenu(items) = MENUS[prefs].items[last].entry else {
            panic!("Pixel Size is a submenu");
        };
        let narrow = MainLayout::new(
            (card.rect.right() + 40.0, 520.0),
            1.0,
            Chrome::ALL,
            32,
            (320, 256),
            PixelSize::default(),
        );
        let card = menu_card(&narrow, prefs).expect("card");
        let sub = submenu_card(&narrow, &card, last, items);
        assert!(
            sub.rect.right() <= card.rect.x + SUB_OVERLAP + 0.01,
            "flipped"
        );
        assert!(sub.rect.bottom() <= narrow.window.bottom());
    }

    #[test]
    fn the_pointer_rules_are_the_classic_ones() {
        let l = layout((1280.0, 800.0), 1.0);
        let mut state = MenuState::default();
        // Not held: the pointer does nothing.
        assert!(!pointer(&l, &mut state, 20.0, 15.0));
        assert_eq!(state, MenuState::default());
        // Held on the bar right of the titles: no menu yet.
        open(&l, &mut state, 900.0, 15.0);
        assert_eq!((state.active, state.menu), (true, None));
        // Along the bar: each title opens its menu; the bar between titles keeps it.
        let (x, y) = title_point(&l, 2);
        assert!(pointer(&l, &mut state, x, y));
        assert_eq!(state.menu, Some(2));
        pointer(&l, &mut state, 900.0, 15.0);
        assert_eq!(state.menu, Some(2), "the open menu stays");
        // Into the card: an item; outside all cards: none, the menu stays open.
        let card = menu_card(&l, 2).expect("card");
        let (x, y) = row_point(&card, 3);
        pointer(&l, &mut state, x, y);
        assert_eq!(state.item, Some(3));
        assert_eq!(
            state.hovered_action(&modern_ctx()),
            Some(Action::Mode(PaintMode::Smear))
        );
        assert!(pointer(&l, &mut state, 700.0, 500.0));
        assert_eq!((state.menu, state.item), (Some(2), None));
        assert!(!pointer(&l, &mut state, 701.0, 500.0), "nothing changed");
        // Back on the bar over another title: that menu, no item.
        let (x, y) = title_point(&l, 0);
        pointer(&l, &mut state, x, y);
        assert_eq!(
            (state.menu, state.item, state.sub_item),
            (Some(0), None, None)
        );
        // Without the bar (F9) there are no menus.
        let hidden = MainLayout::new(
            (1280.0, 800.0),
            1.0,
            Chrome {
                bar: false,
                dock: true,
                layers: false,
            },
            32,
            (320, 256),
            PixelSize::default(),
        );
        let mut none = MenuState::default();
        open(&hidden, &mut none, 20.0, 15.0);
        assert_eq!(none.menu, None);
        assert!(menu_card(&hidden, 0).is_none());
    }

    fn texts(list: &List) -> Vec<(String, Color)> {
        list.ops()
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, color, .. } => Some((text.clone(), *color)),
                _ => None,
            })
            .collect()
    }

    fn held(l: &MainLayout, menu: usize, item: Option<usize>, sub: Option<usize>) -> MenuState {
        let mut state = MenuState::default();
        let (x, y) = title_point(l, menu);
        open(l, &mut state, x, y);
        state.item = item;
        state.sub_item = sub;
        state
    }

    /// Ghosting, the highlight and the marks follow `enabled` and `checked` of the shared
    /// menu model.
    #[test]
    fn the_cards_show_ghosting_marks_and_the_item_under_the_pointer() {
        let (l, theme) = (layout((1280.0, 800.0), 1.0), Theme::dark());
        let ctx = modern_ctx();
        // Mode with Smear under the pointer: one accent row; HBrite ghosted (no EHB).
        let state = held(&l, 2, Some(3), None);
        let mut list = List::new();
        let bounds = draw(&mut list, &l, &theme, &ctx, &state).expect("a card");
        let card = menu_card(&l, 2).expect("card");
        assert!(bounds.encloses(&card.rect));
        let texts_ = texts(&list);
        let colour = |label: &str| {
            texts_
                .iter()
                .find(|(t, _)| t == label)
                .unwrap_or_else(|| panic!("{label} is drawn"))
                .1
        };
        assert_eq!(colour("Smear"), theme.on_accent);
        assert_eq!(colour("F4"), theme.on_accent);
        assert_eq!(colour("Matte"), theme.text);
        assert_eq!(colour("F1"), theme.text_dim);
        assert_eq!(colour("HBrite"), theme.disabled);
        let accents: Vec<Rect> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::RoundRect {
                    rect,
                    fill: Some(c),
                    ..
                } if *c == theme.accent => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(accents, [card.row(3)]);
        // One radio dot: the current mode (Color), in the text colour.
        let marks = |list: &List| {
            list.ops()
                .iter()
                .filter(|op| matches!(op, Op::Path { .. }))
                .count()
        };
        assert_eq!(marks(&list), 1);
        // A ghosted item under the pointer gets no highlight.
        let state = held(&l, 2, Some(9), None);
        let mut list = List::new();
        draw(&mut list, &l, &theme, &ctx, &state);
        assert!(list.ops().iter().all(|op| !matches!(
            op,
            Op::RoundRect { fill: Some(c), .. } if *c == theme.accent
        )));

        // Prefs: a check mark per preference that is on, and one for Modern UI in Modern.
        let prefs = MENUS.len() - 1;
        let mut ui = UiState {
            ui_style: UiStyle::Modern,
            ..UiState::default()
        };
        let on = Pref::ALL.iter().filter(|p| ui.prefs.get(**p)).count();
        let count = |ui: &UiState| {
            let state = held(&l, prefs, None, None);
            let mut list = List::new();
            draw(&mut list, &l, &theme, &ui.menu_context(), &state);
            // Every submenu item has a chevron; the rest are marks.
            let chevrons = MENUS[prefs]
                .items
                .iter()
                .filter(|i| i.action().is_none())
                .count();
            marks(&list) - chevrons
        };
        assert_eq!(count(&ui), on + 1);
        ui.ui_style = UiStyle::Classic;
        assert_eq!(count(&ui), on);
        ui.prefs.set(Pref::Coords, true);
        assert_eq!(count(&ui), on + 1);
    }

    /// A key that is a single punctuation mark stands on a chip, in the text colour: alone
    /// and subdued, the backtick of Stencil ▸ On/Off was a speck.
    #[test]
    fn a_punctuation_key_stands_on_a_chip() {
        assert!(on_chip("`") && on_chip("~") && on_chip(","));
        assert!(!on_chip("p") && !on_chip("F12") && !on_chip("TAB") && !on_chip(""));
        for theme in [Theme::dark(), Theme::light()] {
            let l = layout((1280.0, 800.0), 1.0);
            let ctx = modern_ctx();
            // Effect ▸ Stencil: Make.. (~) and On/Off (`).
            let effect = MENUS
                .iter()
                .position(|m| m.title == "Effect")
                .expect("Effect");
            let stencil = MENUS[effect]
                .items
                .iter()
                .position(|it| it.label == "Stencil")
                .expect("Stencil");
            let state = held(&l, effect, Some(stencil), None);
            let mut list = List::new();
            draw(&mut list, &l, &theme, &ctx, &state).expect("cards");
            for key in ["~", "`"] {
                let (at, size, weight, colour) = list
                    .ops()
                    .iter()
                    .find_map(|op| match op {
                        Op::Text {
                            text,
                            x,
                            baseline,
                            size,
                            weight,
                            color,
                        } if text == key => Some(((*x, *baseline), *size, *weight, *color)),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{key} is drawn"));
                assert_eq!((weight, size), (Weight::Medium, SIZE), "{key}");
                // Enabled or ghosted, the key has the label's ink, never the dim one.
                assert!(colour == theme.text || colour == theme.disabled, "{key}");
                // A ring of at least 18 x 18 points around it, filled with the panel.
                let chip = list
                    .ops()
                    .iter()
                    .find_map(|op| match op {
                        Op::RoundRect {
                            rect,
                            fill: None,
                            stroke: Some(_),
                            ..
                        } if rect.contains(at.0, at.1 - 3.0) && rect.h == CHIP => Some(*rect),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{key} has a chip"));
                assert!(chip.w >= CHIP, "{chip:?}");
                let text_w = font::measure(key, SIZE, Weight::Medium);
                assert!(
                    (at.0 - (chip.x + (chip.w - text_w) / 2.0)).abs() < 0.01,
                    "{key} is centred"
                );
            }
            // A letter key stays plain text in the subdued colour.
            let state = held(&l, 0, None, None);
            let mut list = List::new();
            draw(&mut list, &l, &theme, &ctx, &state).expect("a card");
            assert!(texts(&list).contains(&("Q".to_string(), theme.text_dim)));
        }
    }

    #[test]
    fn a_submenu_opens_beside_its_item_unless_the_item_is_ghosted() {
        let (l, theme) = (layout((1280.0, 800.0), 1.0), Theme::light());
        let prefs = MENUS.len() - 1;
        let theme_item = MENUS[prefs].items.len() - 2;
        assert_eq!(MENUS[prefs].items[theme_item].label, "Theme");
        let state = held(&l, prefs, Some(theme_item), Some(2));
        // Modern: the Theme submenu with the chosen theme marked, Light under the pointer.
        let ui = UiState {
            ui_style: UiStyle::Modern,
            theme: ThemeChoice::Dark,
            ..UiState::default()
        };
        let ctx = ui.menu_context();
        assert_eq!(cards(&l, &ctx, &state).len(), 2);
        assert_eq!(
            state.hovered_action(&ctx),
            Some(Action::Theme(ThemeChoice::Light))
        );
        let mut list = List::new();
        draw(&mut list, &l, &theme, &ctx, &state);
        let texts = texts(&list);
        assert!(texts.contains(&("Light".to_string(), theme.on_accent)));
        assert!(texts.contains(&("System".to_string(), theme.text)));
        // Classic has no theme: the item is ghosted and its submenu does not open.
        let classic = UiState::default().menu_context();
        assert_eq!(cards(&l, &classic, &state).len(), 1);
        assert_eq!(state.hovered_action(&classic), None);
        // Amiga-key shortcuts show a key cap and the letter.
        let state = held(&l, 0, None, None);
        let mut list = List::new();
        draw(&mut list, &l, &theme, &ctx, &state);
        let caps = list
            .ops()
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    Op::RoundRect { rect, fill: None, stroke: Some(_), .. } if rect.w == CAP
                )
            })
            .count();
        let amiga = MENUS[0]
            .items
            .iter()
            .filter(|i| matches!(i.key, Key::Amiga(_)))
            .count();
        assert_eq!(caps, amiga);
        assert!(amiga > 0);
    }

    /// A left click opens a menu only on a title, and what belongs to the open menus is
    /// the titles and the cards that are shown.
    #[test]
    fn a_menu_that_stays_open_starts_on_a_title_and_covers_titles_and_cards() {
        let ctx = modern_ctx();
        for (window, scale) in WINDOWS {
            let l = layout(window, scale);
            let bar = l.bar.expect("bar shown");
            let mut state = MenuState::default();
            // Beside the titles and below the bar nothing opens.
            assert!(!open_sticky(&l, &mut state, bar.right() - 2.0, bar.y + 2.0));
            assert!(!open_sticky(&l, &mut state, 20.0, bar.bottom() + 30.0));
            assert_eq!(state, MenuState::default());
            for m in 0..MENUS.len() {
                let (tx, ty) = title_point(&l, m);
                assert!(open_sticky(&l, &mut state, tx, ty));
                assert!(state.active && state.sticky);
                assert_eq!((state.menu, state.item), (Some(m), None));
                let card = menu_card(&l, m).expect("bar shown");
                let row = card.row(0);
                let (ix, iy) = (row.x + row.w / 2.0, row.y + row.h / 2.0);
                // The pointer rules are those of the held menus; the flag stays.
                assert!(pointer(&l, &mut state, ix, iy));
                assert_eq!((state.item, state.sticky), (Some(0), true));
                assert!(covers(&l, &ctx, &state, tx, ty), "its title");
                assert!(covers(&l, &ctx, &state, ix, iy), "its card");
                assert!(!covers(&l, &ctx, &state, bar.right() - 2.0, bar.y + 2.0));
                let below = card.rect.bottom() + 6.0;
                if below < l.window.bottom() {
                    assert!(!covers(&l, &ctx, &state, ix, below), "below the card");
                }
            }
            // The right button's `open` and `close` both drop the flag.
            let (tx, ty) = title_point(&l, 2);
            open(&l, &mut state, tx, ty);
            assert!(state.active && !state.sticky);
            assert!(open_sticky(&l, &mut state, tx, ty));
            state.close();
            assert_eq!(state, MenuState::default());
        }
    }

    #[test]
    fn titles_are_subdued_until_the_menus_are_held() {
        let ctx = modern_ctx();
        let idle = MenuState::default();
        assert_eq!(title_look(&ctx, &idle, 0), TitleLook::Idle);
        let l = layout((1280.0, 800.0), 1.0);
        let state = held(&l, 3, None, None);
        assert_eq!(title_look(&ctx, &state, 3), TitleLook::Open);
        assert_eq!(title_look(&ctx, &state, 0), TitleLook::Ready);
    }
}
