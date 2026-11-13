# ADR-47: Modern UI skin

- Status: accepted (2026-11-13). Written from three design proposals and three code maps at HEAD f4d9512. Slice 0 (groundwork, no visible change), slice 1a (the renderer crate `dpx-ui-modern`, the bundled font and headless snapshots of bar, dock and picture) slice 1b (the live window: bar, dock and picture with pointer, keys, zoom, HiDPI and the region cache) slice 1c (the Modern pull-down menus and tooltips, the switch by Prefs ▸ Modern UI, F12 and Cmd/Ctrl+Shift+U, Prefs ▸ Theme) and slice 1d (the layer column, the scrim over the chrome under a requester, Screen Format on the whole window) are implemented: slice 1 of the owner's decision 3 is complete. Slices 2 and later are the plan, not the program.
- Amends: ADR-3 (whole UI software-rendered into a low-res framebuffer), ADR-13 (the screen fills the window), ADR-33 (settings file: two new keys), and the first non-negotiable of `CLAUDE.md`.

## Context

- Today the whole visible UI is the pixel UI of ADR-3: `dpx-ui` draws title bar, toolbox, palette, menus, about 30 requesters, layer panel and tooltips into one indexed image the size of the Amiga screen; `apps/deluxepainter/src/main.rs` uploads it as one NEAREST texture. Picture and chrome are separated only at the `Hit` level and inside `compose_screen` (`tools.rs:1868`).
- The owner asks (binding, translated): make the program modern as well; no pixels in the UI, only in the picture, which is then zoomed; switchable; a dark and a light design; it should still remind of Deluxe Paint and have the same hotkeys, but come across as very modern.
- Hard project rules that stay: Rust only; one `Command` engine and no pixel logic outside `dpx-core`/`dpx-raster`; licences MIT/Apache-2.0/BSD/Zlib/0BSD only, fonts included (OFL and the Ubuntu Font Licence are not on the list); no original DeluxePaint artwork or fonts; every milestone has tests and real headless screenshots (the owner declines remote control of the app); CI runs fmt, clippy `-D warnings` and tests on macOS, Windows and Linux runners without a guaranteed GPU.
- Facts from the code maps that shape the decision:
  - All hotkeys are read from `ctx.input` events in app code and are renderer-independent, as long as no focusable egui widget exists.
  - `Hit` (`screen.rs:261`) + `handle_press`, `MenuState` + `hovered_action` + `run_action`, `layer_panel_press` and `tooltip_hover` are semantic seams; a second renderer that produces the same `Hit` values and `MenuState` fields reuses everything behind them.
  - Window → pixel mapping exists at four sites in `main.rs`, all driven by `(origin, ppp, scale, view)`; every requester goes through `modal_events_with`.
  - Every preview (rubber bands, hover brush, text cursor, stencil dim, light table, perspective marks) is already inside the page image, so a picture-only texture gets them for free.
  - `--snapshot` has no egui, no `PainterApp` and no GPU; ui-loop tests run `PainterApp::ui` on a bare `egui::Context` and drop the textures. egui/epaint 0.37 contain no CPU mesh rasteriser; `egui_kittest` renders through wgpu.

## The owner's decisions (binding)

1. The app starts **Classic** on first start. The choice Classic/Modern is remembered and can be switched at any time by menu and by key.
2. Menus in the modern UI work as in DPaint: a bar at the top, the pull-downs appear while the **right** mouse button is held on it, in the new look.
3. The first slice is the **main window only** (title/menu bar, toolbox, palette, layer panel, picture with zoom, dark/light, the switch). The requesters keep the classic pixel look for now and become modern step by step.
4. The colour scheme follows the system (dark/light) on first start; Prefs can fix Dark or Light.

## Decided by the lead (easy to change)

The owner was not asked again about these visible points; each is a small, local choice that can be changed later without touching the structure.

1. The classic Prefs menu gets "Modern UI" and "Theme ▸ System / Dark / Light" before Pixel Size (two rows more), so the switch is reachable by menu in both skins.
2. The switch key is F12, with Cmd/Ctrl+Shift+U as a second binding.
3. The accent is a warm orange: #FF8A1F in the dark theme, #D66214 in the light one.
4. The menu titles are always visible in the modern bar, in a subdued colour; the pull-downs still appear only while the right button is held.
5. In Modern, Cmd/Ctrl +, − and 0 and Prefs ▸ Pixel Size zoom the picture instead of resizing the window.
6. Slice 1 shows one Amiga screen of the picture centred in the window (larger pictures scroll as today), and the Animation Control Panel stays in pixel look inside the picture quad until slice 2.
7. The font is Go (Regular and Medium, BSD-3-Clause). Roboto is not used and nothing is downloaded for the skin.
8. (Slice 1a.) The ink on the accent is dark (#1A1206) in both themes. The draft had white for the light theme, but white on #D66214 reaches only 3.7:1, below the 4.5:1 this ADR demands; the accent itself (decision 3) is kept.
9. (Slice 1a.) Details of the dock: the built-in brushes sit in a 2 × 5 well (round 1, 3, 5, 7 over square 2, 3, 4, 5, the two scatter brushes in the last column); the Clear button shows the letters CLR drawn in our own strokes, as a nod to the DeluxePaint button; every palette swatch has a fine rim so that colours close to the panel's keep their shape; in a low window the palette cells shrink first (to 7 pt), then the tool buttons' height (to 22 pt).

10. (Slice 1b.) In Modern, Cmd/Ctrl + and − count from what is shown and always change it: each press picks the nearest Pixel Size that makes the picture at least one physical pixel per pixel larger or smaller (at device scale 1 a whole point, at scale 2 half a point). Cmd/Ctrl 0 stores the smallest Pixel Size that fills the viewport. Prefs ▸ Pixel Size stores its value as it is; the layout shows at most what fits. One Pixel Size shows a picture as large as in Classic, to the whole physical pixel below (a HiRes or interlaced screen has two picture pixels per interface pixel).
11. (Slice 1b.) `m` and `n` use the last page pixel the pointer was over inside the picture. Classic also follows the pointer under its toolbox; the Modern dock has no page under it.

12. (Slice 1c.) Prefs ▸ Modern UI shows its key F12 in the menu (in both skins); the second binding is not shown. Prefs ▸ Theme is ghosted in Classic and its submenu does not open there.
13. (Slice 1c.) The Modern menus follow the Classic pointer rules with one difference: when the pointer returns from a submenu to the menu's own card, no sub item is chosen any more, also on the row the submenu belongs to. (Classic keeps the sub item on that row, so letting go there still picks it; Classic is not changed by this ADR.)
14. (Slice 1c.) A switch to Modern in a window smaller than 720 × 520 points asks the window to grow to that size; a larger window keeps its size. A tooltip and an open menu do not survive a switch.
15. (Slice 1c.) Look of the overlay: menu cards with 24 pt rows, 8 pt radius and a soft shadow; the open title on the accent, the other titles in the text colour while the menus are held; a check mark for on/off items, a dot for the chosen item of a group; shortcuts right-aligned in the subdued colour; the Amiga key as a small key cap with an "A" of our own strokes. A tooltip is a card left of the dock, level with its button, with the key on a chip.

16. (Slice 1d.) The layer column is 200 pt wide, left of the dock, with 26 pt rows and one row of six icon buttons at the bottom (plus, two sheets, bin, arrow up, arrow down, arrow onto a line); the scroll arrows sit in its title row and show only while rows are scrolled away. The active layer is a raised card with the accent at its left edge; a hidden layer has a closed eye and a subdued name. The column is shown with the dock: F10 hides both.
17. (Slice 1d.) Under a requester, a note or Show Page the scrim covers bar, dock, layer column and the canvas around the picture; the picture, which holds the requester, is not veiled. Playback and the Move preview are the picture running, not a question: the chrome stays lit then (it takes no input either way).
18. (Slice 1d.) The Screen Format requester has a screen of its own, as in Classic. In Modern it gets the whole window without bar, dock and layer column, so its 640 × 512 points fit the smallest window.
19. (Slice 1d.) The tooltip of a layer column button stands left of the column, level with the button. A `--ui modern` snapshot takes `--layers-panel` and the requester flags that draw on the paint screen (`--palette-requester`, `--page-size-requester`, `--file-requester`, ..); `--requester` and `--show-page` stay refused there, because they are snapshot branches of their own (the live window hosts both, and its tests write images of them).

## Options considered

| | A: own CPU renderer (tiny-skia + ab_glyph), per-region layers | B: egui `Painter` chrome, own CPU mesh rasteriser for snapshots | C: display list + tiny-skia + ttf-parser, later a `Canvas` trait through the requesters |
|---|---|---|---|
| Headless deterministic snapshots and ui-loop tests without GPU (must have) | 9: window and snapshot run the same renderer | 6: snapshot rasteriser is a second implementation that only approximates wgpu; 165 ms per 1800×1240 frame | 9 |
| Classic untouched | 8 | 7: changes eframe's feature set for the classic build too | 6: the `Canvas` trait rewrites every classic requester's drawing |
| Reuse of state, input, hotkeys | 9 | 9 | 9 |
| Crisp HiDPI, reads as modern | 8 | 9 (GPU shapes, egui text) | 7 (unhinted path-filled text, 13 ms per 2× frame) |
| Licence-clean fonts and crates | 8 | 9 | 8 |
| Effort slice 1 | 6 | 7 | 6 |
| Effort requesters later | 6 | 6 | 7 (if requesters share widget helpers; not read) |
| Risk on three OSes | 8 | 5 (eframe without default features unverified on Linux/Windows; Wayland decorations) | 8 |
| Total | **62** | 58 | 60 |

Licence claims re-checked for this ADR (not taken from the proposals):
- Local registry `Cargo.toml` files: tiny-skia 0.11.4 and tiny-skia-path 0.11.4 BSD-3-Clause; ab_glyph 0.2.32, ab_glyph_rasterizer 0.1.10 and owned_ttf_parser 0.25.1 Apache-2.0; ttf-parser 0.25.1 MIT OR Apache-2.0; arrayref 0.3.9 BSD-2-Clause; strict-num 0.1.1 MIT; bytemuck 1.25.2 Zlib OR Apache-2.0 OR MIT; arrayvec 0.7.8 MIT OR Apache-2.0. tiny-skia 0.11.4, ab_glyph 0.2.32 and ttf-parser 0.25.1 are already in the repository's `Cargo.lock`.
- GitHub's licence API for `googlefonts/roboto-2` reports Apache-2.0 (the repository's `LICENSE` file is the Apache 2.0 text).
- The README next to the Go fonts in `golang/image` (`font/gofont/ttfs/README`) is a BSD 3-clause text, copyright Bigelow & Holmes.
- crates.io lists `egui_kittest` 0.37.0 as MIT OR Apache-2.0 (not used).

## Decision

**Approach A wins: Modern is a second, own, CPU-rendered skin. egui stays what ADR-3 made it: window, input and texture presenter.** Grafted from the others: C's display list (as the thing that is built, cached and tested), B's and C's reuse of `PixelSize` as the picture zoom, B's hosting of the classic screen in the picture quad for requesters, B's verified Go fonts as the fallback font, C's contrast test.

Why A and not B: the project's working rule is that the screenshot the owner sees is what the program shows. With B the window is drawn by wgpu and the screenshot by a second rasteriser we would have to write and keep faithful; with A both are the same bytes apart from the picture quad (a nearest-neighbour blit at an integer factor, identical by construction). A also needs no change to eframe's features, so nothing in the classic build moves.

### Structure

- **New crate `crates/dpx-ui-modern`** (depends on `dpx-core`, `dpx-ui` for model types only, `tiny-skia`, `ab_glyph`; no egui):
  - `paint`: a display list `List` of `Op::{RoundRect, Line, Path, Text, Image, Clip}` in points (f32). The list is what layout code produces, what the per-region cache hashes, and what most tests assert on (it is bit-identical on every OS).
  - `raster`: `render(&List, scale) -> RGBA8` with tiny-skia; text through `ab_glyph` outlines and a glyph cache keyed by glyph id, pixel size and quarter-pixel bin. The same `font::measure` serves layout and raster.
  - `theme`, `font`, `icons`, `layout` (`MainLayout`), `hit` (returns `Spot`: `Ui(dpx_ui::Hit)` for the chrome, `Picture { x, y }` for a pixel of the picture quad's image, which the app maps to the page as in Classic, or `Canvas`), `menu` (own geometry, writes `MenuState.menu/item/sub_item`), `build` (`title_bar`, `dock`, `layers`, `overlay` → `List`), `frame` (`render_window(..) -> RGBA8` for snapshots).
- **`dpx-ui`**: classic drawing, geometry and hit testing are not touched. Additions only: the model types `UiStyle` and `ThemeChoice` with their settings-file names (`skin.rs`), `pub` on `Item::action`/`Item::enabled` (`menu.rs`), `Action::UiStyle` and `Action::Theme(ThemeChoice)`, two `MenuContext` fields, two Prefs entries.
- **App**: new `apps/deluxepainter/src/modern.rs` (frame branch, picture and region textures, modal hosting, snapshot branch); small branches in `main.rs` (`ui()`, `layout()`, `window_keys`, `dispatch_action`, the `--snapshot` flags), `config.rs` (two fields), `tools.rs` (`magnify_layout` gets a covered-width parameter: `toolbox_cover(view)` in Classic, 0 in Modern).
- **Dependencies added as direct ones**: `tiny-skia = { version = "0.11.4", default-features = false, features = ["std", "simd"] }` (no png 0.17), `ab_glyph = "0.2.32"`. Both versions are the ones already in `Cargo.lock`; tiny-skia becomes part of the macOS and Windows builds (today it is Linux-only, transitive).

### Frame in Modern mode

1. The per-pass syncs and the whole key block of `ui()` run unchanged. `snap_window` is skipped; the window is freely resizable (minimum about 720×520 pt).
2. `MainLayout::new(window_pt, ppp, &ui, palette_len, layer_rows)` gives the chrome rectangles and the picture viewport. All region edges are rounded to whole physical pixels.
3. `layout()` gets a Modern arm that returns the same `(View, scale)` tuple; `fit` comes from the viewport instead of the window, the scale is rounded down to a whole number of physical pixels per screen pixel (times `fx`/`fy`), and `origin` is centred in the viewport. So the **picture quad is the classic screen rectangle**, and the four mapping sites, `screen_to_page`, scroll, `centre_on`, Symmetry's centre, Show Page and every `*_input` function work unchanged.
4. Picture texture: `compose_screen()` is called with the chrome flags (`title_visible`, `toolbox_visible`, overlay layer panel) switched off for the call and restored after it, then `to_rgba(.., 1, 1)` and the existing NEAREST upload. No pixel code is moved or copied. The toolbox toggle marks are skipped because there is no toolbox in that image.
5. Pointer: `modern::hit` checks bar, layer column, dock, then the picture quad (inside it the existing `to_screen` + `ui_pos` + `screen_to_page`). It returns `Hit`; `handle_press`, `handle_drag`, `handle_hover`, `hover_brush`, `release`, `tooltip_hover`, `layer_hover`, `layer_rename_press`, `layer_panel_press`, `layer_wheel` are reused. The drag guard uses the quad.
6. Chrome: each region (bar background, status text, dock, layer column, overlay for menu and tooltip) builds its `List`; a region is rasterised at physical resolution and uploaded only when its list's hash, the theme or the scale changed, and painted 1:1 (no filtering). The pre-paint hooks (`tooltip_guard`, `cycle_tick`, `follow_document`) run as in Classic.
7. While `modal_open()`: the same quad shows the classic screen with the requester (step 4 without hiding anything the requester needs), the chrome is drawn under a scrim and takes no input, and the unchanged `*_input` functions get the quad's `(origin, ppp, scale, view)`. Screen Format, Show Page, playback and the Color Mixer use their existing render sources in the same quad; Import and Generate thumbnails keep their frame width.

### How slice 1b built the frame

- **One pass, two surroundings.** `PainterApp::ui` is a wrapper around the old body (`pass`). In Modern, `modern_begin_pass` moves the two classic chrome flags (`title_visible`, `toolbox_visible`) out of the shared `UiState` into `modern::State` and sets them to off; `modern_end_pass` puts them back. So for the whole pass the classic code composes, lays out and hit-tests a screen without title bar and toolbox (this replaces "switched off for the call" of step 4, because the Animation Control Panel, the layer panel and Magnify must be hit where they are drawn). Between passes the shared flags say what is shown, as in Classic; F9 and F10 change them through `chrome_flags()`.
- **`layout()`** returns `(view, zoom)` from `MainLayout` in Modern; `view()` no longer goes through `layout()`. `ui()` takes the quad's origin from the layout; `to_screen`, `screen_to_page` and every `*_input` function are unchanged. `toolbox_cover` is 0 in Modern, `layer_strip_px` too.
- **Pointer.** `dpx_ui_modern::hit` runs first. `Spot::Ui(hit)` goes to `handle_press` as it is; `Spot::Picture` runs the classic `hit_test` on the chrome-less screen; `Spot::Canvas` is nothing. A right press on the Modern bar does nothing yet (slice 1c).
- **Regions.** Viewport (canvas colour and the picture's shadow), dock and bar each have a display list, built every pass. A region is rasterised (`Renderer::render_region`, which takes the region's origin in physical pixels) and uploaded only when the fingerprint of its list, its pixel rectangle, the device scale, the background or the tile size changed. A region larger than the GPU's largest texture is cut into tiles. The picture is the existing NEAREST texture, painted over the viewport region and clipped to it.
- **Window.** No `snap_window` in Modern (the snap state is forgotten, so Classic snaps again after a switch back); the minimum size becomes 720 × 520 points while Modern is shown and 320 × 200 again in Classic.
- (Since slice 1d the scrim, the layer column, Screen Format on the whole window and the tooltips of dock and column outside the quad are built; see the result of slice 1d below.) **Not yet in slice 1b, although the frame steps above list them:** the scrim over the chrome under a requester (the chrome already takes no input then, and nothing in it is raised), the Modern menus and tooltips (a tool tooltip's state is kept as in Classic but nothing draws it), and the layer column. Until slice 1d the layer panel lies over the picture in the Classic look, as it does in a Classic window. Screen Format and Show Page use the same quad; in a small window their screen may be larger than the viewport and is clipped (slice 1d).

### Main-window layout (points; multiplied by the OS scale)

```
+--------------------------------------------------------------------------------+
| Picture Brush Mode Anim Color Effect Prefs | DeluxePainter  Color  [AB]  x,y   |  bar 30 pt
+------------------------------------------------------+-------------+-----------+
|                                                      | Layers    ^v| o o o o o |  brushes 2x5
|                canvas colour                         |-------------| o o o o o |
|                                                      | (o) Layer 3 |-----------|
|        +----------------------------------+          | (o) Layer 2 | [..] [~~] |  18 tools,
|        |                                  |          | ( ) Layer 1 | [/ ] [( ]|  9 rows x 2,
|        |   picture: one Amiga screen,     |          |             | [bk] [sp] |  TOOL_ROWS order,
|        |   nearest neighbour, integer     |          |             | [[]] [()] |  36 pt buttons,
|        |   physical pixels per pixel      |          |             | [<>] [/\] |  split tools
|        |   (x fx, fy); Magnify split,     |          |             | [::] [ A] |  divided on the
|        |   classic requesters and the     |          |             | [##] [-|-]|  diagonal
|        |   anim panel (slice 1) live      |          |             | [Q ] [+-] |
|        |   inside this quad               |          |             | [<-] [CLR]|
|        +----------------------------------+          |             |-----------|
|                                                      |             | [FG][bg]  |  indicator
|                                                      |-------------| # # # #   |  palette, up to
|                                                      | + c x ^ v m | # # # #   |  64, column-major
+------------------------------------------------------+-------------+-----------+
   picture viewport (all that is left)                   200 pt        92 pt dock
                                                         (Prefs Layers on)
```

- Bar: menu titles left, status (app title or frame counter "n/m", paint mode, effect flags) after them, coordinates or the perspective readout and "Layer: name" (panel off) right. Content logic is that of `draw_title_bar`.
- Dock: same top-to-bottom order as the DPaint toolbox. Selected tool or half filled with the accent colour; Grid, Symmetry and Magnify carry an accent ring while on; no brush selected while a custom brush is the brush. Palette reads `PainterApp::palette()` so cycling animates; FG has an accent ring, BG a thin ring (the BG marker is new).
- Layer column: shown with the dock while Prefs ▸ Layers is on, always beside the picture in Modern. Rows (eye, name, active row), inline rename (the existing `LayerPanel` rename state and `layer_rename_keys`; names stay printable ASCII so both skins agree), six icon buttons, wheel scrolling.
- F9/F10 keep their meaning (bar; bar + dock + layer column).
- Picture zoom: the existing `PixelSize` value, capped by what fits the viewport. Cmd/Ctrl +, − and 0 and the Pixel Size submenu therefore zoom the picture in Modern and never resize the window. DPaint Magnify (`m`, `<`, `>`) stays the split view inside the quad.
- Slice 1 limitation: the visible picture is one Amiga screen; a larger window shows canvas colour around it. The free viewport is slice 3.

### Theme tokens

Fixed RGBA values, never picture-palette pens. The picture never changes with the theme. Flat surfaces, 5–8 pt radii, hairline separators, no bevels.

| Token | Dark | Light | Used for |
|---|---|---|---|
| `canvas` | #16171A | #D6D9DF | picture viewport |
| `panel` | #202227 | #F0F1F4 | bar, dock, layer column |
| `raised` | #30333A | #FFFFFF | buttons, menu cards, tooltips |
| `hairline` | #3A3D45 | #C9CDD5 | separators, card borders |
| `text` | #E4E6EB | #1C1E24 | labels |
| `text_dim` | #9A9FAA | #5E636E | idle menu titles, shortcuts |
| `disabled` | #5C606A | #A4A8B1 | ghosted items and buttons |
| `accent` | #FF8A1F | #D66214 | selection, menu hover, toggle rings |
| `on_accent` | #1A1206 | #1A1206 | ink on the accent (lead decision 8) |
| `hover` | #3D414A | #E3E6EC | a button under the pointer |
| `scrim` | #000000 at 45 % | #14161C at 30 % | chrome under a modal |
| `shadow` | #000000 at 40 % | #000000 at 18 % | menu cards, picture edge |

`ThemeChoice::System` resolves through `ctx.system_theme()` (winit feeds it; no new dependency), every frame, with Dark when the OS reports nothing. A unit test checks `text` on `panel` and `on_accent` on `accent` for a contrast of at least 4.5:1.

### Fonts and icons

- **Font:** one bundled family in `assets/fonts/` with its licence text beside it, loaded with `include_bytes!`; no system fonts, so metrics are the same on every OS and in tests. The family is **Go** (Regular and Medium; BSD-3-Clause, Bigelow & Holmes), lead decision 7. The vendoring step (slice 1a) read the licence record in each font file's name table (name ID 13): a BSD-3-Clause style text, identical in both files, shipped verbatim as `assets/fonts/LICENSE-Go.txt`; a unit test compares the two. Roboto 2 (Apache-2.0) was the draft's first proposal and is not used; Roboto from Google Fonts or `roboto-3-classic` is OFL and must never be used.
- Sizes: 13.5 pt bar and menus, 12 pt panels and tooltips.
- egui's bundled fonts stay unused, as today. Dropping eframe's `default_fonts` feature is a separate clean-up outside this ADR's slices (unverified on Linux and Windows).
- **Icons:** own vector artwork as path data on a 24-unit grid in `dpx-ui-modern/src/icons.rs`, 1.75-unit round-capped strokes, filled variants for the filled halves; designed from each tool's meaning, never traced from the 9×9 bitmaps, the original program or the manuals. 18 tools (slice 1a); eye, six layer buttons, scroll chevrons, check mark, radio dot, submenu chevron and a key-cap glyph for Amiga-key shortcuts follow with the slices that draw them. Built-in brushes are discs, squares and dots sized from the brushes' real footprints. A split tool shows its outline with the lower-right half filled; when it is selected, only the chosen half of the button takes the accent.

### Runtime switch and settings

- State: `ui_style: UiStyle { Classic, Modern }` (default Classic) and `theme: ThemeChoice { System, Dark, Light }` (default System). Both types live in `dpx_ui::skin`. Since slice 1c the two values are fields of the shared `UiState` (beside `pixel_size`), not of `PainterApp`: the classic menu drawing reads its marks and ghosting from `UiState::menu_context()`, and one place for them avoids a copy that could go stale. Nothing else in `dpx-ui` reads them.
- `Config` (`config.rs`): top-level keys `ui_style` (`"classic"`, `"modern"`) and `theme` (`"system"`, `"dark"`, `"light"`) in the struct, `default`, `from_json`, `to_json`, `capture`, `apply` and the `changed()` fixture. Old files lack the keys and read as Classic/System, and so does a name this version does not know: no version bump, no format break. Tests and snapshots keep `config: None`.
- Menu: Prefs gets "Modern UI" (check item) and "Theme ▸ System / Dark / Light" (radio items, the `Action::PixelSize` pattern) before Pixel Size, which stays last. Theme is ghosted in Classic. This makes the classic Prefs pull-down two rows taller: the only visible classic change, and it follows from decision 1.
- Key: F12, handled next to F9/F10, guarded by `modal_open()`, typing and `menu.active`; second binding Cmd/Ctrl+Shift+U because F12 can belong to the OS.
- The switch is asked for during a pass (`toggle_skin`) and happens between two passes (`modern_apply_switch`, after the chrome flags are back in the shared state): it flips the field, marks dirty, closes an open menu and drops the tooltip. Classic → Modern leaves the window as it is unless it is smaller than the Modern minimum (lead decision 14); Modern → Classic lets `snap_window` return the window to screen × pixel size. Document, tool, scroll and Magnify are shared.
- Menus: the right-button protocol in `pass()` is kept; in Modern the two geometry calls (`menu.open`, `menu.pointer`) are replaced by `dpx_ui_modern::menu::{open, pointer}(layout, &mut MenuState, x, y)`. `hovered_action`, `checked`, `enabled`, `menu_enabled`, `run_action`, left-click multi-select and release-to-select are reused. Cards flip and clamp at the window edges.
- Hotkeys: no change. Modern adds no focusable widget, so `Event::Text` reaches `tool_keys` as today. Keys that have a menu label but no handler today stay as they are.

### Testing and snapshots

- **Snapshot flags** on the existing `--snapshot` branch: `--ui modern`, `--theme dark|light` (default dark; never the machine's setting), `--size WxH` in points (default 1280x800), `--device-scale 1|2`. Since slice 1c `--menu M[:I[:S]]` draws the Modern cards, and `--pointer X,Y --tooltip` the tooltip of the dock button under a rested pointer; Classic's `--hover` (UI pixels) is refused with `--ui modern`. The paragraph below is the state of slice 1a. The modern branch builds `Document` and `UiState` from the existing flags, calls `dpx_ui_modern::frame::render_window` and the existing `write_png`. As of slice 1a it honours the document flags, `--tool`, `--fg` and `--pointer X,Y` (a resting pointer in window points, which shows the hover state; the classic `--hover` is in UI pixels and stays Classic's); `--menu`, tooltips and `--layers-panel` are not drawn by the Modern skin before slices 1c and 1d, and `--show-page` and `--requester` (full-screen views) are refused with `--ui modern`. The Modern flags without `--ui modern` are an error, not ignored. Without `--ui modern` nothing changes, so every classic screenshot regenerates identically. As in Classic, snapshots show no stroke preview and no Magnify split.
- **Crate tests (no GPU, no egui):** list-level assertions (bit-identical everywhere): layout rectangles lie in the window and do not overlap at the minimum size; the centre of every tool, half, brush and palette rectangle hits the expected `Hit`; every `MENUS` item is reachable and sets the right `MenuState`; contrast of the theme tokens. Raster assertions as probes, not image hashes (tiny-skia's SIMD paths may differ by a level between architectures; not verified): palette swatch centres equal the palette colour, the selected tool's centre is the accent, the bar has the panel colour, text has ink inside its rectangle, the picture at factor N has exact N-pixel blocks.
- **ui-loop tests:** `ui_pass_with(app, ctx, t, events, TestWindow { size, ppp, system_theme, max_texture_side })` beside the untouched `ui_pass` (it returns the window commands of the pass); `modern_point(app, Place)` beside `ui_point`; `pick_modern` beside `pick`. Parity tests run the same key script and the same clicks in both skins and compare document and `UiState` (every tool button left and right, palette left and right, a menu pick by item id); the tooltip key-equals-click test runs in both skins; switch by F12 and by menu with a config round trip in a temp directory; system theme injected Dark and Light, fixed themes ignore it; a classic requester opened in Modern takes a click.
- **Classic guard:** before any other change, `apps/deluxepainter/src/classic_pin_tests.rs` pins `compose_screen()` and `snapshot_rgba()` by hash for four fixed scenes (start, a painted page with the Mode menu held open, two layers with the layer panel and Magnify, a requester); it also runs after a Classic → Modern → Classic round trip through real `ui()` passes. The scenes avoid the Prefs menu, which decision 1 of the lead makes two rows taller.
- **CI:** no workflow change (no GPU, no display server, no new system package).

## Slice plan

Slice 1 of the owner's decision 3 is slices 1a–1d here. Each slice ends with the gate (`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`), and from 1a on with real `--snapshot` PNGs.

**Slice 0 – groundwork, no visible change (done).**
- Files: `docs/adr/0047-modern-ui-skin.md`, update notes in `docs/adr/0003-*.md` and `docs/adr/0033-*.md`, `CLAUDE.md` (the wording below; the owner asked for the skin); `apps/deluxepainter/src/classic_pin_tests.rs` (the pin test); `apps/deluxepainter/src/config.rs`; `apps/deluxepainter/src/main.rs` (two fields; only the settings file reads them yet); `apps/deluxepainter/src/tools.rs` (`magnify_layout` parameter and `toolbox_cover` only); `crates/dpx-ui/src/skin.rs` (the two model types) and `crates/dpx-ui/src/menu.rs` (the two `pub` accessors only).
- Acceptance: the pin test is committed first and passes unchanged after the other commits; config round-trip tests cover both keys, a file without them gives Classic/System; no screenshot changes.

**Slice 1a – renderer crate and first screenshots (done).**
- Files: `Cargo.toml` (workspace member, two dependencies), `crates/dpx-ui-modern/**`, `assets/fonts/**` (font files + licence text), `docs/research/dependencies.md` (rows for tiny-skia, ab_glyph, the font); `apps/deluxepainter/src/modern.rs` (snapshot function only) and the `--snapshot` flag parsing in `main.rs`.
- Delegable in parallel with fixed file scope: `icons.rs` (18 tools + panel glyphs), `theme.rs`, `layout.rs` + `hit.rs`, `raster.rs` + `font.rs`.
- Acceptance: `--snapshot --ui modern` writes bar, dock with own icons, palette and the nearest-scaled picture, dark and light, at 1x and 2x, in the Go font; crate tests as listed; `cargo tree` shows no new licence outside the list.
- Result: `docs/design/screenshots/modern-{dark,light}-{1x,2x}.png` and `modern-dark-compact.png` (720×520). Not in this slice: the `menu` module, the layer column, the `overlay` list (menu cards, tooltips), the scrim under a requester and the per-region cache; the crate tests on menu reachability come with slice 1c.

**Slice 1b – live window (done).**
- Files: `apps/deluxepainter/src/modern.rs`, `main.rs` (`ui()` branch, `layout()` arm, `window_keys`, test helpers), `tools.rs` (callers of `magnify_layout`), `layers.rs` only if `layer_strip_px` needs a Modern guard.
- Acceptance: with `ui_style` set in a test, the modern frame runs under `ui_pass_with`; no `snap_window` in Modern; parity tests for all tool buttons, brushes, palette cells and the key script pass; Cmd/Ctrl +/−/0 change the picture factor and send no `InnerSize` command; the classic pin test passes.

- Result: `apps/deluxepainter/src/modern.rs` (state, pass wrapper, layout, zoom, region cache, painting) with small branches in `main.rs`, `tools.rs` and `layers.rs`; in the crate `MainLayout::for_picture`, `layout::{Picture, wanted_zoom, pixel_size_stepped, pixel_size_for}` and `Renderer::render_region`. Tests in `modern.rs` run a script in a Classic and a Modern window and compare the whole `UiState`, the tool settings, the page, the palette, undo and scroll after every step: every tool button with both mouse buttons (both halves of split tools, requesters closed with Esc), all ten brushes, palette cells with both buttons, at device scales 1, 1.5 and 2; and a key script with strokes, a right-button rectangle, Magnify, F9/F10, undo/redo, the spare page, the Color Mixer and scrolling. Further tests: the zoom keys and no `InnerSize` command, no snapping, the minimum size, the four corners of page pixels at four device scales, the region cache counters, the injected system theme, and probes of an image made from the live window's own lists and screen. `docs/design/screenshots/modern-live-{magnify,requester,anim-panel}.png` are such images (the window's textures cannot be read back in a test); `modern-dark-{1x,2x}.png` regenerate byte-identically.
- For slice 1c: a switch of the skin in the middle of a pass must take effect with the next pass (the chrome flags are held aside until the pass ends).

**Slice 1c – menus, tooltips, switch, theme (done).**
- Files: `crates/dpx-ui-modern/src/{menu,build}.rs`; `crates/dpx-ui/src/menu.rs` (`Action` variants, `MenuContext` fields, the two Prefs entries) and `screen.rs` (`menu_context` only); `main.rs` (`dispatch_action`, F12 and the second binding, the two geometry calls of the menu branch); `apps/deluxepainter/src/tooltip.rs` only for a modern anchor rectangle.
- Acceptance: `pick_modern` runs the same action as `pick` for an item of every menu, including a submenu, a check item and left-click multi-select; ghosting and marks match `checked`/`enabled`; F12 and the menu item switch both ways and persist; the theme follows an injected system theme and a fixed choice; the regenerated classic Prefs-menu screenshot is the only classic screenshot that differs; snapshots dark with the Mode menu held open and light with a tooltip.

- Result:
  - `dpx-ui`: `Action::UiStyle` and `Action::Theme(ThemeChoice)`, `MenuContext.ui_style`/`.theme`, `UiState.ui_style`/`.theme`, the Prefs entries "Modern UI" (key label F12) and "Theme ▸ System / Dark / Light" before Pixel Size. The classic Prefs box is two rows taller (168 pixels); it still fits a 320 × 200 screen.
  - `dpx-ui-modern`: `menu` (`Card`, `menu_card`, `submenu_card`, `open`, `pointer`, `cards`, `draw`, `title_look`), `build::overlay` (menu cards or tooltip, with the rectangle they cover), `build::tooltip_rect`, the marks in `icons` (check mark, radio dot, chevron, key-cap letter), `Renderer::render_overlay` (transparent where nothing is drawn, premultiplied alpha).
  - App: the menu branch of `pass()` calls the Modern geometry in a Modern pass; the overlay is one more image over the regions and the picture, redone only when its list changes; `toggle_skin`/`modern_apply_switch`; F12 and Cmd/Ctrl+Shift+U next to F9/F10 (the chord is not Shift+U's redo); `--tooltip` for snapshots.
  - Tests: every item of every menu and submenu is reached by the pointer and sets the shared `MenuState` (crate, five windows; app, in the live window at device scale 2); cards stay in the window and flip; ghosting, marks and highlight follow `enabled`/`checked`; a script of menu picks (plain, submenu, radio, check, ghosted, three requesters, left-click multi-select, letting go away, a hotkey under the menu) leaves a Classic and a Modern window in the same state after every step, at device scales 1 and 1.5; F12 and the chord switch both ways, not under a requester or while typing; Prefs ▸ Modern UI switches in both skins and the choice and the theme are read back from a settings file in a temp directory; the theme follows the injected system theme unless fixed; the tooltip appears after the delay beside the dock and goes with a press; overlay and region counters; image probes of the held menu, dark at 1x and light at 2x.
  - Screenshots: `modern-menu-dark.png` (`--ui modern --theme dark --commands wave8-layer-panel.json --menu 2:3 --tool rectangle:filled --fg 5`), `modern-menu-light.png` (`--theme light ... --menu 6:14:2`: Prefs ▸ Theme), `modern-tooltip-light.png` (`--theme light ... --pointer 1213,150 --tooltip`), `modern-menu-compact-2x.png` (`--size 720x520 --device-scale 2 --menu 6:15:5`: Prefs ▸ Pixel Size in the smallest window). Regenerated classic Prefs screenshots: `wave8-layer-prefs-menu.png` (`--commands wave8-layer-panel.json --menu 6:11`), `phase2-menu-pixel-size.png` (`--scale 3 --menu 6:15:6`; Pixel Size is item 15 now) and `wave8-prefs-menu-ntsc.png` (`--preset ntsc_lores --commands wave8-layer-panel.json --menu 6:15:0`), all with `--no-settings`.
  - Not built here: the tooltip key-equals-click test in both skins (the Modern tooltip shows the shared `Tooltip`, whose text that test already covers in Classic), and a tooltip anchor in `apps/deluxepainter/src/tooltip.rs` (the rectangle comes from `build::tooltip_rect`).

**Slice 1d – layer column, modal hosting, documentation (done).**
- Files: `crates/dpx-ui-modern/src/build.rs` (layers), `crates/dpx-ui/src/layer_panel.rs` (a `pub` accessor for the rename cursor only), `apps/deluxepainter/src/modern.rs`, `main.rs` (modal branch), `docs/FEATURES.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `docs/design/screenshots/modern-*.png`.
- Acceptance: layer tests (select, eye, buttons, wheel, double-click rename with caret) pass in both skins; one requester of each kind (a tool-settings requester, the file requester, Screen Format, Show Page, Color Mixer, Import) opens in Modern, takes a click through the unchanged mapping and closes; chrome takes no input while a modal is open; the anim panel (Alt-a) works inside the quad; final set of screenshots: dark and light at 1x and 2x, menu open, layer column, classic requester over the modern window.

- Result:
  - `dpx-ui`: `LayerPanel::rename_cursor` (the one accessor).
  - `dpx-ui-modern`: `Chrome.layers`, `LayerColumn` and `MainLayout.layers` (`layout`); `hit_in(layout, ui, x, y)` and `layer_hit` beside `hit`, which knows the column only as background (`hit`); `build::layers`, `Model.dimmed` and the scrim at the end of every region's list, `build::canvas(.., dimmed)`, the tooltip rectangle of a column button (`build`); eye, the six button icons and the scroll arrows (`icons`).
  - App: a fourth region for the column; `modern_chrome` (column with the dock while Prefs ▸ Layers is on; no chrome at all under Screen Format); in Modern the shared `LayerPanel` is marked docked and gets the column's row count (`layers::sync_rows`, `modern_layer_rows`), so the classic screen neither draws nor hits it and `layer_panel_press`, `layer_wheel`, `layer_rename_press` and `layer_rename_keys` run unchanged; `compose_for_skin` leaves the tooltips of dock and column out of the quad; a pass whose keys or clicks changed the layout asks for the next pass at once.
  - Fixed from slice 1b on the way: after a key or a menu pick that moves the picture (F9, F10, a zoom step, Prefs ▸ Layers) the picture was painted at its old place until the next input event. `modern_paint` now asks for a repaint when the quad it was given is not the layout's.
  - Tests (10 in the crate, 9 in the app): the column's rectangles at four device scales and three window sizes; its hits (rows, eye, name, field, buttons, arrows) at five; its list (order, active mark, hidden layer, ghosted and held buttons, hover, scroll arrows, the rename field with a caret that follows the cursor, a long name cut); the scrim as the last op of every region and under the picture on the canvas; raster probes of column and scrim, dark at 1x and light at 2x. In the app, scripts that leave a Classic and a Modern window in the same state after every step: the whole layer panel (key, New, painting on a layer, eye, select, right button, all six buttons incl. ghosted ones, rename with Return, Esc, a click in the field and a click elsewhere, undo and redo) at device scales 1, 1.5 and 2; one requester of each kind with clicks on the first and last pixels of its buttons (tool settings: Cancel and Ok and a pixel beside them; file: typing, Cancel; Screen Format: the Chipset gadget, Cancel; Show Page on a 640 × 512 page: a click moves the view; Color Mixer: a colour cell and a slider; Import; Generate; AI & MCP) at the same three scales; the Control Panel's buttons inside the quad. Modern only: the column beside the picture, F9/F10, back to Classic; wheel and arrows over 30 layers; the column's own region and its tooltip (the images with and without it differ only in the Modern tooltip's rectangle); under three requesters in both themes every region is veiled and clicks with both buttons on tool, brush, palette, row, eye, column button and menu title change nothing; Screen Format's screen pixel for pixel in the smallest window; Show Page veiled; playback not veiled.
  - Screenshots: `modern-{dark,light}-{1x,2x}.png` (regenerated: `--ui modern --commands wave8-layer-panel.json --tool rectangle:filled --fg 5` with `--theme` and `--device-scale`), `modern-layers-dark.png`, `modern-layers-light-2x.png` and `modern-layers-compact.png` (the same with `--layers-panel`; compact: `--size 720x520`), `modern-requester-dark.png` (`--layers-panel --palette-requester`), `modern-requester-light.png` (`--layers-panel --page-size-requester`), `modern-anim-panel-dark.png` (`--frames 4 --anim-panel`), all with `--no-settings`; from the live-window tests (`DPX_MODERN_SHOTS`): `modern-live-requester.png` (now veiled), `modern-live-screen-format.png`, `modern-live-show-page.png`, `modern-live-layers-rename.png`, `modern-live-layers-tooltip.png`. The menu screenshots of slice 1c stand (`modern-menu-dark.png` regenerates byte-identically).
  - Not built: a tooltip key-equals-click test in both skins (as in 1c); `--requester` and `--show-page` in a Modern snapshot.

**Slice 2 – Animation Control Panel** as a modern bottom strip (`dpx-ui-modern`, `anim_control.rs` hit mapping), playback view. Acceptance: panel parity tests; no pixel-look element is left in the main window.

**Slice 3 – free picture viewport.** Visible size parametrised in `max_scroll`, `clamp_scroll`, `centre_on`, `magnify_layout`, `page_scroll_input`, Show Page; optional vector pixel grid from factor 8, drawn by the chrome, never into the picture. Acceptance: classic pin tests unchanged; a 640×512 page fills a large window; scroll and Magnify tests in both skins.

**Slice 4 and later – requesters,** one family per slice, each shippable because an unconverted requester falls back to the classic host: a widget kit in `dpx-ui-modern` (panel, button, toggle, radio, slider, text field, list, swatch grid: list builder + hit function), then tool settings (Spacing, Fill, Grid, Symmetry), choice and note, file and Page Size, Color Mixer and Range, the anim requesters, Import/Generate/AI & MCP, Screen Format and Show Page last. A requester keeps its state struct and its `ReqEvent` input; it gains a second layout in points returning the same hit values. Classic drawing is not rewritten onto a shared trait (that would put classic byte-identity at risk for no visible gain). Acceptance per family: dark and light snapshots, parity tests against the classic requester, classic tests unchanged.

## What `CLAUDE.md` says since slice 0

The owner asked for the Modern skin, so the non-negotiables were changed with this ADR. The first one now reads:

> - **Rust only** for product code. Desktop host: `eframe`/`egui`, used only for the window, input and presenting textures. The visible UI is our own software-rendered UI in two switchable skins that share one state, one input path and the same hotkeys (ADR-47):
>   - **Classic skin** (default on first start): the pixel UI (bitmap font, bevels, icons) drawn by `dpx-ui` into the low-res framebuffer and shown with nearest-neighbour scaling at integer factors. Its output must not change when the Modern skin is worked on.
>   - **Modern skin**: a resolution-independent vector UI with a dark and a light theme, drawn by `dpx-ui-modern` on the CPU. Only the picture is pixels (nearest neighbour, integer factors). Until a requester is converted it is shown in the Classic look.
>   - No standard egui widgets and no egui text in either skin.

The second:

> - **Design basis: DeluxePaint IV AGA (v4.5).** Don't mix in DPaint III/V behaviour without documenting it. Both skins keep its structure, its right-button menus and its hotkeys. All Classic window/menu/requester corners are square; the Modern skin has its own look (ADR-47) and never copies original artwork.

Added to the licence line:

> Bundled fonts fall under the same list (no OFL, no Ubuntu Font Licence).

## Consequences

- Two renderers for the main window, one state: every new main-window feature needs a drawing in both skins and a parity test. Requesters stay single until converted.
- Two new direct dependencies and one bundled font; tiny-skia now builds on macOS and Windows too. tiny-skia contains `unsafe` internally (the workspace's `forbid(unsafe_code)` covers our crates only).
- The classic Prefs menu is two rows taller. Nothing else in Classic changes.
- In Modern the window no longer snaps to the screen size and Pixel Size means picture zoom.
- Mixed looks until later slices: classic requesters over the modern window, and in slice 1 the anim panel in pixel look inside the picture.
- Chrome is rasterised on the CPU. A prototype outside the repository measured about 1.5 ms for the full chrome at 2560×1600 (release build, one Apple-silicon Mac); the region cache keeps it off the painting path, a window resize re-renders all regions each frame.
- Text is unhinted and blended without gamma correction; at 1x it is slightly softer than native text.

## Not verified

- Only slice 0 was built and run in the repository (macOS; the gate of `CLAUDE.md`). Its pinned hashes were made on one Apple-silicon Mac; the classic renderer is integer code, but the values were not yet seen on the Windows and Linux runners. Line numbers in this ADR are from the code maps at HEAD f4d9512. The three prototypes are standalone projects with placeholder icons and stand-in fonts; of their PNGs none was inspected for this ADR.
- Where the Go font files came from: they were taken from the prototype's working copy, not downloaded again, and not compared with an upstream release. Their licence was read from the files themselves (slice 1a).
- Slice 1a was built and run on one Apple-silicon Mac only. Of the committed classic screenshots, `phase1-lores-default.png` and `phase1-lores-gold16.png` were regenerated and are byte-identical; the others were not regenerated (their exact flags are not recorded), so classic identity rests on the pin test.
- Slice 1b was built and run on one Apple-silicon Mac only, and only through tests: the Modern window was never opened on a screen. What the GPU shows (region textures painted one to one, the picture texture clipped to the viewport, tiles) is not covered by any test, because ui-loop tests drop the textures; the test image is made from the same lists and the same screen by the CPU renderer. Device scales other than 1, 1.25, 1.5 and 2, a real change of the device scale, and `MinInnerSize` on each OS are not verified.
- A region rasterised alone equals its part of the whole window except for a few anti-aliased pixels of curves (tiny-skia; measured: at most 16 pixels of the dock, by up to 42 levels). Tile seams of regions larger than the GPU limit were not looked at.
- In Modern: display modes other than PAL LoRes, pages larger than the screen, Screen Format, Show Page, playback and every requester except Gridding and the Color Mixer were not exercised; the layer panel over the picture was not exercised either.
- `modern-light-{1x,2x}.png` of slice 1a were not regenerated (their pointer flag is not recorded).
- Slice 1c was built and run on one Apple-silicon Mac only, through tests and `--snapshot`. The Modern menus and tooltips were never used on a screen: how the overlay texture (premultiplied alpha, NEAREST, one to one) looks on the GPU, how holding the right button feels, and whether the OS lets F12 or Cmd/Ctrl+Shift+U through are not verified. Cmd/Ctrl+Shift+U was tested with injected key events without a text event; what each OS sends for that chord was not seen.
- Classic identity in slice 1c: the four pin scenes pass unchanged, and `phase1-lores-default.png`, `wave8-layer-panel.png`, `wave8-layer-panel-tooltip.png` and `wave8-layer-label.png` regenerate byte-identically. The other classic screenshots were not regenerated. `phase3-menus.png` (`--menu 4:2`) does not regenerate identically; the difference lies in the Brush and Anim titles and the Color menu, which this slice did not touch (the file predates later menu work), but that was not traced to a commit. `modern-dark-{1x,2x}.png` did not regenerate identically with plain `--ui modern` flags (their original flags are not recorded); they were left as committed.
- In the Modern menus, only PAL LoRes with the 32-colour palette was run in the live window; menus over a HiRes or interlaced picture, with the bar hidden (F9: no menus, as in Classic), and menu items that open requesters other than the Color Mixer, Page Size and Set Rate were not exercised in Modern.
- Slice 1d was built and run on one Apple-silicon Mac only, through tests and `--snapshot`; nothing of it was used on a screen. Of the requesters, eight kinds were clicked in Modern (tool settings, file, Screen Format, Show Page, Color Mixer, Import, Generate, AI & MCP); the others (Page Size, Range, notes, questions, the Anim and AnimBrush requesters, Set Range and Move, the Image Generator setup, Perspective, Stencil, Arrange) go through the same `modal_events` mapping but were not clicked there. Requesters and the layer column were run on a PAL LoRes picture only.
- A screen larger than the viewport is centred and cut at both edges (slice 3 frees the viewport): a HiRes or interlaced picture beside the layer column needs a window about 940 points wide, and a requester on such a screen in a smaller window is partly outside it. Not exercised.
- The repaint request after a layout change in the middle of a pass was checked as "the pass asks for another one at once", not on a screen.
- Classic identity in slice 1d: the four pin scenes pass unchanged; no classic screenshot was regenerated in this slice. No classic drawing or hit code was changed (one accessor was added to `layer_panel.rs`).
- Rendering times of the production renderer (the 1.5 ms above is the prototype's).
- Whether tiny-skia's output is bit-identical on the three CI platforms; timings on Windows, Linux, debug builds and 4K/5K windows.
- Whether any requester positions itself relative to the classic toolbox or title bar in a way that the chrome-less compose shifts: the eight kinds of slice 1d are centred on the screen and were hit where they are drawn; the others were not checked.
- `RawInput.system_theme` at runtime on each OS; F12 availability on each OS.
- How uniformly the requesters draw through shared widget helpers (decides how mechanical slice 4 is).
