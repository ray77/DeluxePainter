# Roadmap

Vertical slices: every phase ends with a runnable build, automated tests and real screenshots or CLI output. A feature counts as done only when it is usable, not when it exists as a stub. Feature priorities (P0–P3) are defined in [FEATURES.md](FEATURES.md).

| Phase | Goal | Key deliverables | Done when |
|---|---|---|---|
| **0 — Research & design** ✅ | Know exactly what to build | Reference catalog, manual/video/format/dependency research, hardware & IFF truth tables, architecture + ADRs, layout mockups | Owner picks a layout option; open toolchain question resolved |
| **1 — First paint** 🚧 | A window you can draw in | Cargo workspace, `dpx-core` (Document, Palette, Command engine, undo/redo), RetroUi framebuffer + integer scaling, bitmap font, menu bar, toolbox shell, palette, freehand/pixel tool, zoom, status line; MCP spike (`app_status`, `new_document`, `get_document_state`) | User paints pixels in a 320×256 OCS document, zooms, undoes/redoes; pixels stay sharp at every scale; CI green on macOS |
| **2 — Files** | Amiga files in and out | ILBM read/write (BMHD, CMAP incl. 4-bit fix, CAMG, BODY, ByteRun1, mask, CRNG/CCRT), PNG import/export, JPEG import, file requester, error diagnostics, golden tests | Self-made and real third-party ILBMs survive a pixel-exact roundtrip |
| **3 — DPaint tools** | Productive classic painting | Lines, curves, rect/circle/ellipse/polygon (filled/outline), flood fill + Fill Type requester (gradients, dither), airbrush, spacing, built-in brushes, brush pick-up + transforms, text, grid, symmetry, magnify, spare page, stencil, paint modes, palette requester, keyboard shortcuts | Small pixel art can be made productively with mouse + keyboard, matching the reference workflows |
| **4 — Projects & layers** | Photoshop-like editability | `.dpxproj` (ZIP, atomic save, autosave/recovery), layer tree + groups, opacity, compositing, ExportAnalyzer + export preview | Multi-layer project saves/reloads bit-identical; ILBM export never destroys editability |
| **5 — OCS/AGA special modes** | Full hardware fidelity | EHB, HAM6, HAM8 encode/decode, AGA 8-plane modes, colour cycling editor + preview, hardware validator, unsupported modes greyed out | Exports display correctly in an independent decoder/emulator |
| **6 — Animation** | DPaint-style animation | Frames, timeline/anim control, light table (onion skin), ANIM op-5 import/export, AnimBrush, Move requester | Small animation saved, exported as ANIM, re-imported and played back by an independent player |
| **7 — Full MCP** | Agents can do everything | All MCP tools/resources, GUI IPC bridge, revisions, transactions, security limits, acceptance scenario test | MCP acceptance scenario runs reproducibly in CI |
| **8 — Release** | Shippable on 3 platforms | CI for macOS/Windows/Linux, app icon/bundles, signing (if owner sets it up), crash recovery, performance checks, third-party notices | Test builds start on all three platforms; no known data-loss bugs |

## Open owner decisions

1. ~~**UI layout**~~ — decided: C (extended work area) by default, A (classic fixed screen) as preference. See [design/README.md](design/README.md).
2. ~~**Rust toolchain**~~ — decided: rustup, Rust 1.99.1 pinned in `rust-toolchain.toml`; eframe/egui 0.37.

## Phase 2 status (started 2026-11-10, before Phase 1 was finished, on owner request)

Done (verified by tests and on real DeluxePaint-era ILBM files, see IFF_SUPPORT_MATRIX.md §8):
- `dpx-iff`: IFF container, ByteRun1 (written by a Sonnet subagent, reviewed), ILBM read/write incl. CMAP 4-bit fix, CAMG, CRNG/CCRT, GRAB, mask plane, unknown chunks kept.
- `dpx-formats`: indexed PNG import/export (palette kept exactly).
- CLI: `deluxepainter info <file>`, `deluxepainter convert <in> <out>` (reports whatever the target format cannot store), `--open <file>` for the GUI and snapshots.
- Display modes (ADR-7): PAL/NTSC × LoRes/HiRes/SuperHiRes × laced, chipset limits, `SetFormat` command with whole-document undo; Picture ▸ Screen Format requester on its own HiRes screen (menu only, owner decision); screens shown with the right pixel shape; `--open` picks the picture's mode.
- Picture ▸ Load/Save (ADR-8): DeluxePaint-style file requester on the paint screen (list, scroll bar, Drawer/Pattern/File, Disks/Assigns/Parent, AmigaDOS patterns, double-click, overwrite confirmation); ILBM saves write the document's CAMG, aspect and page size and keep CRNG/GRAB/transparent colour from the loaded file; `.png` names save PNG. Covered by unit tests and headless snapshots; not yet clicked through by hand in the window.
- True-colour import (ADR-9): RGB/RGBA/greyscale PNG and JPEG (EXIF orientation) load through Picture ▸ Load, `--open` and the CLI. Pictures that fit a screen format keep their size; bigger ones are scaled to the current screen. New palette with the mode's colour count (exact when the picture has that few, 12-bit on OCS), median cut + k-means, Floyd–Steinberg; transparent pixels become pink colour 0, saved as the ILBM transparent colour. Verified by tests, real ILBMs round-tripped through RGB PNG, and release timings (see ADR-9).

- Pages larger than the screen (ADR-10): pages are at least screen size and at most 1008×1024 (SuperHiRes up to its overscan width); Picture ▸ Page Size requester with Standard / Full Page / Overscan presets (undoable `SetPageSize`, new area in the BG colour); arrow keys, Alt+arrows, `n` and the mouse wheel scroll; Picture ▸ Show Page (Shift+S) shows the reduced page with a draggable rectangle. Verified by tests and snapshots; real key/wheel input not tried by hand.

- Overscan and custom screens (ADR-11): Screen Format ▸ Screen Size cycles Standard / Overscan / Custom (editable width/height, up to 368/736/1472 × 241/283, laced 482/566); the information panel shows Size and Max Size; pictures open on the screen from their ILBM page fields or their own size; ILBM saves write the screen size. Verified by tests and snapshots.

Next in Phase 2:
1. Error diagnostics for broken files; more golden fixtures.
2. Visible notes (e.g. "DPPS chunk not kept", "reduced to 256 colours") instead of stderr only.

Noted for Phase 3: choose the colour used for transparent pixels on import (owner idea, ADR-9) together with palette editing.

## Phase 1 status

Done (verified by tests and headless snapshots):
- Cargo workspace (`dpx-core`, `dpx-ui`, `deluxepainter`), toolchain pin, clippy/fmt clean.
- Indexed screen renderer in Amiga LoRes pixels; UI pens derived from the picture palette.
- Title bar ("DeluxePainter" + paint mode), depth gadget, toolbox (brush block, 18 tools, split icons, selected-variant display), colour indicator, palette with FG frame — matched against reference screenshots.
- Original 8×8 bitmap font, set proportionally with tabular digits.
- Pull-down menus: full DPaint IV AGA menu set (Picture, Brush, Mode, Anim, Color, Effect, Prefs) with submenus, shortcuts, check marks and ghosted items; geometry matched against `inapp_gfx_03`. Working actions: Mode (also F1–F8), Prefs toggles (Coords readout), Picture ▸ Flip, Color ▸ BG -> FG / BG <-> FG, Palette ▸ Default Palette, Quit. Other items are shown but do nothing yet.
- Command engine in `dpx-core` (ADR-6): validated, serialisable commands (stroke, clear, flip, replace/swap colours, set palette), 64-step undo/redo, revision checks and dry runs. Freehand strokes are previewed and committed as one undo step; all menu commands go through it.
- Window with integer nearest-neighbour scaling (starts; painting and menu handling are covered by unit tests of the logic, not yet by an automated UI test).

Next:
1. Zoom/Magnify and pixel grid.
2. Requester framework (Screen Format first) and HiRes geometry.
3. MCP spike (`app_status`, `new_document`, `get_document_state`) on top of the engine, and CI on macOS/Windows/Linux.
