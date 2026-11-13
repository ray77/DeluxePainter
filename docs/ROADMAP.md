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

1. ~~**UI layout**~~ — decided: C (extended work area) by default, A (classic fixed screen) as preference. See [design/README.md](design/README.md). Revised 2026-11-10 after trying C: the window snaps to the picture's screen in whole and half pixel-size steps (ADR-13).
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

- Notes after Load/Save (ADR-12): whatever was not kept, reduced, scaled or cannot be stored is listed in a "Picture loaded" / "Picture saved" requester with Ok (also after `--open`), not only on stderr; notes about DPPS, DPPV and TINY stay on stderr only (ADR-35). Verified by tests and a snapshot.

- Window scaling (ADR-13, owner request): the window snaps to the picture's screen in whole and half pixel-size steps (1x–8x), so the picture fills it without bars; Prefs ▸ Pixel Size and Cmd/Ctrl +/−/0; full screen (Ctrl+Cmd+F / F11) centres the screen at the largest step. Verified by tests; dragging, keys and full screen not tried by hand.

- File diagnostics (ADR-17): broken and foreign files give a short first line for the file requester plus details (chunk, offset, values) on stderr and in `info`; foreign formats are named (PBM, ACBM, ANIM, LIST, GIF, PowerPacker …); HAM with odd plane counts is refused (HAM/EHB themselves open since ADR-26). Harmless quirks (BODY ending early → colour 0, odd CMAP sizes, missing pad bytes, wrong FORM sizes, trailing data, short CRNG/CAMG, lasso) load with a note. Golden fixtures built byte by byte from the spec (1–8 planes, odd widths, ByteRun1, mask, CRNG, 4-bit CMAP, truncated and broken variants) with exact pixels and texts. Verified by tests and CLI output; the app's Load handler still has to show only the first line in the file requester (main.rs, open).

Done since (night of 2026-11-10/11, parallel agent waves; verified by unit/app tests, headless snapshots and the CI gate on macOS, Windows and Linux — **not yet tried by hand in the window**):
- Phase 3 tools (ADR-14, 18-20, 30, 31): line, curve, rectangle, circle, ellipse, polygon (outline/filled), fill with gradients, airbrush, text, grid, symmetry (cyclic/mirror), Magnify; custom brushes (Brush Selector, cut, Matte/Color/Replace) with every tool; paint modes Smear, Shade, Blend, Cycle, Smooth, Mix, HBrite; Brush menu incl. interactive Stretch/Rotate/Shear/Bend; requesters for Grid, Symmetry, Spacing, Fill Type, Stencil, Ranges, Arrange; spare page, colour cycling, palette files, fixed background, translucency, antialias, process.
- Phase 4 start: `.dpxproj` project format (ADR-23); Picture ▸ Save writes it, Export.. writes ILBM/PNG/ANIM (ADR-27).
- Phase 5 groundwork: EHB and HAM6/HAM8 codecs (ADR-25); EHB/HAM pictures open (ADR-26); HAM export not yet.
- Phase 6 start: animation frames, Anim menu, playback, Light Table (ADR-28); IFF ANIM op-5 read/write (ADR-29) wired into Load / Anim ▸ Save / Export (ADR-32).
- Phase 7 start: MCP server with painting, viewing and file tools in approved directories (ADR-21, 24).
- Autosave and crash recovery (ADR-34); settings file (ADR-33).
- Wave 7 (2026-11-11): every menu item but Picture ▸ Print now works. AnimBrush and Metamorph (ADR-36), Effect ▸ Perspective (ADR-37), Set Range, Control Panel, Method and Move (ADR-38), Use Brush Palette, Stencil ▸ Paint, Background ▸ Lock FG and Recompute for HAM screens (ADR-39). Also: colour cycling keeps running under requesters, menus and playback (ADR-35); faster debug builds and one frame less display latency (ADR-40); tooltips for the toolbox and Control Panel buttons (ADR-41); the Import Picture requester for PNG and JPEG on Picture ▸ Load.. (fit into the page, position, colour count or current palette, dither, true-colour thumbnail; owner decisions, ADR-42).
- CI on macOS, Windows and Linux (GitHub Actions).
- An independent code review found 20 issues (2 high: a hostile .dpxproj could exhaust memory; MCP opened HAM/EHB wrongly); fixes are in progress.

- Wave 8 (2026-11-12/13, the owner's spec "DeluxePainter – MCP-Anbindung und KI-Bildgenerierung"; see "Wave 8 status" below): live MCP in the running app with the stdio bridge, the `.mcpb` extension and Prefs ▸ AI & MCP.. (ADR-43, supersedes ADR-5's socket plan); layers with the layer panel and project format version 3 (ADR-44); AI image generation with Picture ▸ Generate.. and OpenAI Images (ADR-45); the chipset choice, screen presets and hardware checks on export (ADR-46).

Still open: hand testing in the window; layer opacity, lock and groups (not wanted for now, owner decision); EHB/HAM export (Phase 5); animpainting (Alt while drawing), per-frame palettes, Move ▸ Fill / Load / Save, the Perspective, Brush and Wrap fill types. Picture ▸ Print stays out of scope (ADR-39).

Noted for Phase 3: choose the colour used for transparent pixels on import (owner idea, ADR-9) together with palette editing.

## Wave 8 status (2026-11-13)

Done (verified by unit, ui-loop and integration tests, real snapshot renders and debug runs on macOS, Apple Silicon; details and the exact lists in the ADRs):
- **Live MCP (ADR-43):** the running app serves MCP Streamable HTTP on `127.0.0.1` (port 8765, fallbacks) while Prefs ▸ AI & MCP.. has MCP control on (off by default) or with `--mcp`; bearer token, Host and Origin checks, separate read/edit permissions, approved folders, the "Replace File?" question, Revoke access; the stdio bridge `dpx-mcp-bridge` (also `deluxepainter mcp-bridge`) that the Claude apps start; connect buttons for Claude Code (shows and runs `claude mcp add --scope user deluxe-painter -- <app> mcp-bridge` after a click) and Claude Desktop (opens the `.mcpb` extension); `cargo run -p xtask -- mcpb`, with a CI job (`mcpb`) for a macOS (Apple Silicon) and a Windows (x86_64) bundle. Headless `deluxepainter mcp` (stdio) and `deluxepainter mcp --http` stay.
- **Layers (ADR-44):** the layer model shared by every frame, colour 0 transparent above the bottom layer, shown/hidden only, up to 32 layers, project format version 3, exports of the composite with a note; the layer panel (Prefs ▸ Layers, `L`); MCP layer tools.
- **AI image generation (ADR-45):** the `dpx-imagegen` crate (provider trait, OpenAI Images, keychain, jobs with at most 2 requests in flight), `dpx_core::import::convert`, Picture ▸ Generate.. with targets Spare page (default), Picture, Brush and New layer, the Image Generator setup (also from Prefs ▸ AI & MCP.. ▸ Set up..), the MCP tools `generate_image`, `get_generation_status`, `apply_generated_image`, `cancel_generation`.
- **Modes and hardware rules (ADR-46):** Chipset and Preset in Screen Format (one preset table for Screen Format, Generate and MCP, with "Pixel art 320x240 (custom)"), OCS colours 12-bit as a document rule, IFF exports checked and refused with the broken rule, honest notes for custom screens and the lost chipset; older OCS projects with 24-bit colours open as stored with a note.
- [MCP_API.md](MCP_API.md) and the README guide "Connect to Claude in two minutes".

The owner's acceptance tests (spec §10), as far as they were checked:

| # | Test | State |
|---|---|---|
| 1 | Start, MCP on, connect Claude Code, create a 320×240 OCS 32-colour picture by a spoken request | **not verified with real Claude Code**; the connect flow ran against a fake `claude`, the 320×240 OCS picture was made over HTTP and through the bridge in a debug run of the GUI |
| 2 | Claude draws a rectangle; it shows at once and can be undone | verified in ui-loop tests (the change lands in the same pass, `U` and MCP `undo` take it back); not watched by a person in a visible window |
| 3 | Claude reads the palette and changes a colour; the editor follows | verified by the `dpx-mcp` tool tests and in the window's snapshot `wave8-mcp-gui-calls.png` (palette entries set through the GUI workspace with `--mcp-calls`); not with a real client |
| 4 | Claude Desktop connected through the `.mcpb` without editing JSON; the app shows the status | **not verified**: Claude Desktop never installed the extension (unsigned); the bundle's bridge answered `initialize`/`tools/list` and forwarded calls to a debug run of the GUI |
| 5 | A configured generator makes a picture of exactly 320×240 with at most 32 OCS colours after preview and confirmation | verified with the debug test pattern and gated test providers through the requester and the MCP tools; **no real OpenAI picture** (no key) |
| 6 | Without a generator: a clear message, nothing faked, no crash | verified (requester and MCP tests) |
| 7 | Saved as project and as IFF ILBM, read back | verified (ADR-46 round trip; an OCS IFF file reopens as AGA, the project keeps OCS) |
| 8 | OCS HiRes with too many colours is refused with a reason | verified (`create_document`, Screen Format, every IFF export) |
| 9 | Turn on, connect, disconnect and reconnect, also after a restart of the app | partly: the token survives restarts and the bridge reconnects to a restarted server (tests); not with real clients |
| 10 | Windows, macOS and Linux stay usable without Claude or a provider | macOS only; Windows and Linux not verified for wave 8 |

Not verified in wave 8 (see the ADRs' "Not verified" lists): real Claude Code, Claude Desktop and MCP Inspector; a real OpenAI request and the real system keychain; Windows and Linux (builds, tests, file permissions, the Claude paths, `.mcpb` opening); the stateless MCP `2026-07-28` protocol era; everything by hand in a visible window (the owner declines remote control). Open owner decisions: whether an MCP `generate_image` should also ask in the app before it sends a billed request (ADR-45, "Integration (wave 8)"); signing and notarizing the bridge, an icon for the extension and release packaging (ADR-43); whether the permissive but unlisted licences the full licence scan found (ISC, BSL-1.0, OFL-1.1, Ubuntu-font-1.0, Unicode-3.0; all through `eframe` or `url`, none new in wave 8) are acceptable ([research/dependencies.md](research/dependencies.md) §7.3).

Open follow-ups found while documenting wave 8 (small code changes, not done in the docs step):
- `create_document`'s tool description still says a chipset change replaces the picture; the code makes it one undoable step (ADR-46).
- `apply_generated_image` does not report how many entries of the current palette it rounded for an OCS target (`TODO(wave8-integration)` in `apps/deluxepainter/src/generate.rs`); the Generate requester's Apply does.
- `docs/design/screenshots/wave8-mcp-gui-calls.png` (640×512) predates the snapshot following the calls' display mode and renders 640×480 now (ADR-44, "Integration (wave 8)").
- A few doc comments still name ADR-24's `save_picture` tool (`dpx_core::hardware::validate`, a `dpx-mcp` test comment).

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
3. ~~MCP spike and CI~~ Written (ADR-21): `deluxepainter mcp` (rmcp, stdio, headless engine) with `app_status`, `new_document`, `get_document_state`, tested end to end by spawning the binary on macOS. `.github/workflows/ci.yml` runs the gate on macOS/Windows/Linux but has not run yet; Windows/Linux and real MCP clients are unverified.
