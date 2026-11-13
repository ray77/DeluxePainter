# ADR-47: Modern UI skin

- Status: accepted (2026-11-13). Written from three design proposals and three code maps at HEAD f4d9512. Slice 0 (groundwork, no visible change) is implemented; the Modern skin itself is not built yet, so everything from "Frame in Modern mode" on describes the plan, not the program.
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
  - `theme`, `font`, `icons`, `layout` (`MainLayout`), `hit` (returns `dpx_ui::Hit`), `menu` (own geometry, writes `MenuState.menu/item/sub_item`), `build` (`title_bar`, `dock`, `layers`, `overlay` → `List`), `frame` (`render_window(..) -> RGBA8` for snapshots).
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
- Layer column: always docked in Modern. Rows (eye, name, active row), inline rename (the existing `LayerPanel` rename state and `layer_rename_keys`; names stay printable ASCII so both skins agree), six icon buttons, wheel scrolling.
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
| `on_accent` | #1A1206 | #FFFFFF | ink on the accent |
| `scrim` | #000000 at 45 % | #14161C at 30 % | chrome under a modal |
| `shadow` | #000000 at 40 % | #000000 at 18 % | menu cards, picture edge |

`ThemeChoice::System` resolves through `ctx.system_theme()` (winit feeds it; no new dependency), every frame, with Dark when the OS reports nothing. A unit test checks `text` on `panel` and `on_accent` on `accent` for a contrast of at least 4.5:1.

### Fonts and icons

- **Font:** one bundled family in `assets/fonts/` with its licence text beside it, loaded with `include_bytes!`; no system fonts, so metrics are the same on every OS and in tests. The family is **Go** (Regular and Medium; BSD-3-Clause, Bigelow & Holmes), lead decision 7. The vendoring step (slice 1a) must read the licence string in each font file's name table before committing. Roboto 2 (Apache-2.0) was the draft's first proposal and is not used; Roboto from Google Fonts or `roboto-3-classic` is OFL and must never be used.
- Sizes: 13.5 pt bar and menus, 12 pt panels and tooltips.
- egui's bundled fonts stay unused, as today. Dropping eframe's `default_fonts` feature is a separate clean-up outside this ADR's slices (unverified on Linux and Windows).
- **Icons:** own vector artwork as path data on a 24-unit grid in `dpx-ui-modern/src/icons.rs`, 1.75-unit round-capped strokes, filled variants for the filled halves; designed from each tool's meaning, never traced from the 9×9 bitmaps, the original program or the manuals. 18 tools, eye, six layer buttons, scroll chevrons, check mark, radio dot, submenu chevron, a key-cap glyph for Amiga-key shortcuts. Built-in brushes are discs and squares sized from the core brush dimensions.

### Runtime switch and settings

- State on `PainterApp`: `ui_style: UiStyle { Classic, Modern }` (default Classic) and `theme: ThemeChoice { System, Dark, Light }` (default System). Both types live in `dpx_ui::skin` because the menu actions and the menu context need them.
- `Config` (`config.rs`): top-level keys `ui_style` (`"classic"`, `"modern"`) and `theme` (`"system"`, `"dark"`, `"light"`) in the struct, `default`, `from_json`, `to_json`, `capture`, `apply` and the `changed()` fixture. Old files lack the keys and read as Classic/System, and so does a name this version does not know: no version bump, no format break. Tests and snapshots keep `config: None`.
- Menu: Prefs gets "Modern UI" (check item) and "Theme ▸ System / Dark / Light" (radio items, the `Action::PixelSize` pattern) before Pixel Size, which stays last. Theme is ghosted in Classic. This makes the classic Prefs pull-down two rows taller: the only visible classic change, and it follows from decision 1.
- Key: F12, handled next to F9/F10, guarded by `modal_open()`, typing and `menu.active`; second binding Cmd/Ctrl+Shift+U because F12 can belong to the OS.
- The switch flips the field, marks dirty, drops the textures and closes an open menu. Classic → Modern leaves the window as it is; Modern → Classic re-arms `snap_window` so the window returns to screen × pixel size. Document, tool, scroll, Magnify and menu state are shared.
- Menus: the right-button protocol at `main.rs:1987-2013` and `:2048` is kept; in Modern the two geometry calls (`menu.open`, `menu.pointer`) are replaced by `dpx_ui_modern::menu::pointer(layout, &mut MenuState, pos)`. `hovered_action`, `checked`, `enabled`, `menu_enabled`, `run_action`, left-click multi-select and release-to-select are reused. Cards flip and clamp at the window edges.
- Hotkeys: no change. Modern adds no focusable widget, so `Event::Text` reaches `tool_keys` as today. Keys that have a menu label but no handler today stay as they are.

### Testing and snapshots

- **Snapshot flags** on the existing `--snapshot` branch: `--ui modern`, `--theme dark|light` (default dark; never the machine's setting), `--size WxH` in points (default 1280x800), `--device-scale 1|2`. The modern branch builds `Document` and `UiState` from the existing flags (`--tool`, `--fg`, `--menu M:I:S`, `--hover`, `--layers-panel`), calls `dpx_ui_modern::frame::render_window` and the existing `write_png`. Without `--ui modern` nothing changes, so every classic screenshot regenerates identically. As in Classic, snapshots show no stroke preview and no Magnify split.
- **Crate tests (no GPU, no egui):** list-level assertions (bit-identical everywhere): layout rectangles lie in the window and do not overlap at the minimum size; the centre of every tool, half, brush and palette rectangle hits the expected `Hit`; every `MENUS` item is reachable and sets the right `MenuState`; contrast of the theme tokens. Raster assertions as probes, not image hashes (tiny-skia's SIMD paths may differ by a level between architectures; not verified): palette swatch centres equal the palette colour, the selected tool's centre is the accent, the bar has the panel colour, text has ink inside its rectangle, the picture at factor N has exact N-pixel blocks.
- **ui-loop tests:** `ui_pass_with(app, ctx, t, events, size, ppp, system_theme)` beside the untouched `ui_pass`; `modern_point(app, Region)` beside `ui_point`; `pick_modern` beside `pick`. Parity tests run the same key script and the same clicks in both skins and compare document and `UiState` (every tool button left and right, palette left and right, a menu pick by item id); the tooltip key-equals-click test runs in both skins; switch by F12 and by menu with a config round trip in a temp directory; system theme injected Dark and Light, fixed themes ignore it; a classic requester opened in Modern takes a click.
- **Classic guard:** before any other change, `apps/deluxepainter/src/classic_pin_tests.rs` pins `compose_screen()` and `snapshot_rgba()` by hash for four fixed scenes (start, a painted page with the Mode menu held open, two layers with the layer panel and Magnify, a requester); it also runs after a Classic → Modern → Classic round trip through real `ui()` passes. The scenes avoid the Prefs menu, which decision 1 of the lead makes two rows taller.
- **CI:** no workflow change (no GPU, no display server, no new system package).

## Slice plan

Slice 1 of the owner's decision 3 is slices 1a–1d here. Each slice ends with the gate (`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`), and from 1a on with real `--snapshot` PNGs.

**Slice 0 – groundwork, no visible change (done).**
- Files: `docs/adr/0047-modern-ui-skin.md`, update notes in `docs/adr/0003-*.md` and `docs/adr/0033-*.md`, `CLAUDE.md` (the wording below; the owner asked for the skin); `apps/deluxepainter/src/classic_pin_tests.rs` (the pin test); `apps/deluxepainter/src/config.rs`; `apps/deluxepainter/src/main.rs` (two fields; only the settings file reads them yet); `apps/deluxepainter/src/tools.rs` (`magnify_layout` parameter and `toolbox_cover` only); `crates/dpx-ui/src/skin.rs` (the two model types) and `crates/dpx-ui/src/menu.rs` (the two `pub` accessors only).
- Acceptance: the pin test is committed first and passes unchanged after the other commits; config round-trip tests cover both keys, a file without them gives Classic/System; no screenshot changes.

**Slice 1a – renderer crate and first screenshots.**
- Files: `Cargo.toml` (workspace member, two dependencies), `crates/dpx-ui-modern/**`, `assets/fonts/**` (font files + licence text), `docs/research/dependencies.md` (rows for tiny-skia, ab_glyph, the font); `apps/deluxepainter/src/modern.rs` (snapshot function only) and the `--snapshot` flag parsing in `main.rs`.
- Delegable in parallel with fixed file scope: `icons.rs` (18 tools + panel glyphs), `theme.rs`, `layout.rs` + `hit.rs`, `raster.rs` + `font.rs`.
- Acceptance: `--snapshot --ui modern` writes bar, dock with own icons, palette and the nearest-scaled picture, dark and light, at 1x and 2x, in the Go font; crate tests as listed; `cargo tree` shows no new licence outside the list.

**Slice 1b – live window.**
- Files: `apps/deluxepainter/src/modern.rs`, `main.rs` (`ui()` branch, `layout()` arm, `window_keys`, test helpers), `tools.rs` (callers of `magnify_layout`), `layers.rs` only if `layer_strip_px` needs a Modern guard.
- Acceptance: with `ui_style` set in a test, the modern frame runs under `ui_pass_with`; no `snap_window` in Modern; parity tests for all tool buttons, brushes, palette cells and the key script pass; Cmd/Ctrl +/−/0 change the picture factor and send no `InnerSize` command; the classic pin test passes.

**Slice 1c – menus, tooltips, switch, theme.**
- Files: `crates/dpx-ui-modern/src/{menu,build}.rs`; `crates/dpx-ui/src/menu.rs` (`Action` variants, `MenuContext` fields, the two Prefs entries) and `screen.rs` (`menu_context` only); `main.rs` (`dispatch_action`, F12 and the second binding, the two geometry calls of the menu branch); `apps/deluxepainter/src/tooltip.rs` only for a modern anchor rectangle.
- Acceptance: `pick_modern` runs the same action as `pick` for an item of every menu, including a submenu, a check item and left-click multi-select; ghosting and marks match `checked`/`enabled`; F12 and the menu item switch both ways and persist; the theme follows an injected system theme and a fixed choice; the regenerated classic Prefs-menu screenshot is the only classic screenshot that differs; snapshots dark with the Mode menu held open and light with a tooltip.

**Slice 1d – layer column, modal hosting, documentation.**
- Files: `crates/dpx-ui-modern/src/build.rs` (layers), `crates/dpx-ui/src/layer_panel.rs` (a `pub` accessor for the rename cursor only), `apps/deluxepainter/src/modern.rs`, `main.rs` (modal branch), `docs/FEATURES.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `docs/design/screenshots/modern-*.png`.
- Acceptance: layer tests (select, eye, buttons, wheel, double-click rename with caret) pass in both skins; one requester of each kind (a tool-settings requester, the file requester, Screen Format, Show Page, Color Mixer, Import) opens in Modern, takes a click through the unchanged mapping and closes; chrome takes no input while a modal is open; the anim panel (Alt-a) works inside the quad; final set of screenshots: dark and light at 1x and 2x, menu open, layer column, classic requester over the modern window.

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
- The licence string inside the Go TTF files (only the README beside them was read); slice 1a reads the name tables before vendoring.
- Whether tiny-skia's output is bit-identical on the three CI platforms; timings on Windows, Linux, debug builds and 4K/5K windows.
- Whether any requester positions itself relative to the classic toolbox or title bar in a way that the chrome-less compose shifts (Import has `under_import_requester`); to be checked per requester in slice 1d.
- `RawInput.system_theme` at runtime on each OS; F12 availability on each OS.
- How uniformly the requesters draw through shared widget helpers (decides how mechanical slice 4 is).
