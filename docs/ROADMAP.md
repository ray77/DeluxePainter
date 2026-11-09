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

## Phase 1 status

Done (verified by tests and headless snapshots):
- Cargo workspace (`dpx-core`, `dpx-ui`, `deluxepainter`), toolchain pin, clippy/fmt clean.
- Indexed screen renderer in Amiga LoRes pixels; UI pens derived from the picture palette.
- Title bar, depth gadget, toolbox (brush block, 18 tools, split icons, selected-variant display), colour indicator, palette with FG frame — matched against reference screenshots.
- Original 8×8 bitmap font.
- Window with integer nearest-neighbour scaling (starts; manual painting not yet verified by an automated UI test).

Next:
1. Pull-down menus (title bar + right mouse) with the DPaint IV AGA menu set.
2. Command engine with multi-level undo in `dpx-core`; move painting there.
3. Zoom/Magnify and pixel grid.
4. Requester framework (Screen Format first) and HiRes geometry.
5. MCP spike (`app_status`, `new_document`, `get_document_state`) and CI on macOS/Windows/Linux.
