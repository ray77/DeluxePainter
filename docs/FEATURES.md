# Features — condensed

The essence of all Phase 0 research (master plan, DPaint IV AGA manual, DPaint IV manual, tutorial video, screenshots, format and dependency research), reduced to what DeluxePainter must do. Detail lives in the linked documents; this page is the scope contract.

Priorities: **P0** = first usable release core · **P1** = classic DPaint toolset · **P2** = Amiga special modes & animation · **P3** = later.

## 1. Screen & UI

| P | Feature | Notes |
|---|---|---|
| P0 | Pixel-perfect retro UI at integer scale 1×–5× / Auto, never blurred (Retina, 4K, Windows 125/150 %) | ADR-3 |
| P0 | Menu bar with drop-down menus: Picture, Brush, Mode, Anim, Color, Effect, Prefs | DPaint IV AGA menu set; modern Cmd/Ctrl shortcuts |
| P0 | Two-column toolbox at the right; left-click = select tool, right-click = tool options requester | |
| P0 | Palette panel with FG/BG colour (left/right mouse), AGA page selector for 256 colours | |
| P0 | Canvas zoom (1–16×), pan, pixel grid, coordinates display | |
| P0 | Undo / redo (multi-level) | DPaint had one level; we keep the button, add history |
| P1 | Requesters (modal, keyboard: Enter/Esc/Tab): Screen Format, Page Size, Palette, Fill Type, Spacing, Grid, Symmetry, Font, File | Square corners, 1-px bevels |
| P1 | Magnify window (split view: normal + zoomed) | DPaint's signature zoom |
| P1 | Spare page (swap, copy, merge front/back) | |
| P1 | Hide/show toolbox and menu bar for full-screen painting | |
| P1 | Layout: extended work area (default) or classic fixed screen | owner decision pending |

## 2. Documents & hardware targets

| P | Feature | Notes |
|---|---|---|
| P0 | New document dialog = AGA-style "Choose Display Mode" requester: mode list, page size, palette size | `HARDWARE_MODES.md` |
| P0 | OCS PAL/NTSC LoRes/HiRes (+ interlace), 2–32 colours, RGB4 palette | |
| P1 | AGA LoRes/HiRes/SuperHiRes (+ interlace), up to 256 colours, RGB8 palette | |
| P2 | EHB (64 = 32 + half-brite), HAM6, HAM8 | ADR-4 |
| P1 | Page larger than screen (scrolling), aspect-correct preview toggle | |
| P1 | Change hardware target with validation report (never silent data loss) | |

## 3. Painting tools

| P | Tool | Options |
|---|---|---|
| P0 | Freehand (dotted / continuous), single pixel | built-in brush sizes/shapes |
| P0 | Straight line | |
| P1 | Curve | |
| P0 | Rectangle, circle, ellipse — outline / filled | Shift constrains |
| P1 | Polygon — outline / filled | |
| P0 | Flood fill (solid) | |
| P1 | Fill types: brush, wrap, perspective(P3), gradients (linear, shape, circle, contour, highlight) with dither + range | Fill Type requester |
| P1 | Airbrush, spacing (N total / every Nth / airbrush / continuous) | Spacing requester |
| P0 | Pipette (pick colour) | |
| P1 | Brush pick-up (rectangle, lasso), last brush, custom brush stamping | |
| P1 | Brush transforms: flip H/V, rotate 90°/any, stretch/halve/double, outline/trim, handle | |
| P1 | Text tool with bitmap fonts | original fonts only |
| P1 | Grid snap, symmetry (point/mirror, N-fold) | |
| P1 | Stencil (protect colours) | |
| P1 | Paint modes: Matte, Color, Replace, Smear, Shade, Blend, Cycle, Smooth, Tint, HBrite | |
| P0 | Antialiasing **off** by default for all tools | |
| P3 | Perspective, bend/shear, metamorph | |

## 4. Colour

| P | Feature | Notes |
|---|---|---|
| P0 | Palette editor: RGB/HSV sliders, spread, copy, exchange | Palette requester docked at bottom like DPaint |
| P1 | Remap picture/brush to palette, use brush palette, default palette, load/save palette | |
| P2 | Colour cycling: up to N ranges, rate, direction, live preview (display-only) | CRNG/CCRT |

## 5. Layers & projects (modern additions)

| P | Feature | Notes |
|---|---|---|
| P0 | `.dpxproj` save/open — lossless, atomic writes | ADR-1, `DPXPROJ_FORMAT.md` (Phase 4) |
| P1 | Layers: add/delete/rename/reorder, visibility, lock, opacity, Normal blend | |
| P1 | Layer groups | |
| P1 | Autosave & crash recovery | |
| P3 | Masks, more blend modes, adjustment layers | |

## 6. Files

| P | Format | Import | Export |
|---|---|---|---|
| P0 | IFF ILBM (ByteRun1, mask, CMAP, CAMG) | ✓ | ✓ with ExportAnalyzer report |
| P0 | PNG | ✓ (as layer or quantised) | ✓ native resolution |
| P0 | JPEG | ✓ | ✗ (never) |
| P2 | ILBM EHB / HAM6 / HAM8, CRNG/CCRT | ✓ | ✓ |
| P2 | IFF ANIM (op 5 first) | ✓ | ✓ |
| P2 | IFF brushes / AnimBrushes | ✓ | ✓ |
| P3 | PCHG, SHAM, CTBL, DEEP, PBM | after documented need | — |

## 7. Animation

| P | Feature |
|---|---|
| P2 | Frames: add/copy/delete/set count, frame rate, go to, play / play once / ping-pong |
| P2 | Light table (onion skin: previous / two back / next / spare) |
| P2 | AnimBrush pick-up and settings |
| P3 | Move requester (path/rotation with easing), metamorph |

## 8. MCP (automation)

| P | Feature |
|---|---|
| P0 | Spike: `app_status`, `new_document`, `get_document_state` (Phase 1) |
| P1 | Drawing, palette, layer, frame, undo/redo, transaction tools; preview PNG; revision checks |
| P1 | Import/export + `validate_amiga_export` |
| P2 | Live bridge to the running GUI with "changed by MCP" indicator |

## 9. Explicitly out of scope

Emulation of the original program; GIF/BMP/TIFF/PSD export; JPEG export; web/Electron UI; copying original EA artwork, fonts, icons or manual text; claims of full DPaint feature parity before it is verified.
