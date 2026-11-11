# Features — condensed

The essence of all Phase 0 research (master plan, DPaint IV AGA manual — see `research/manual-ui-notes.md`, DPaint IV manual, tutorial video, screenshots, format and dependency research), reduced to what DeluxePainter must do. Note: the AGA manual's reference chapters are identical to the 4.0 manual; all AGA additions come from its 13-page addendum. Detail lives in the linked documents; this page is the scope contract.

Priorities: **P0** = first usable release core · **P1** = classic DPaint toolset · **P2** = Amiga special modes & animation · **P3** = later.

## 1. Screen & UI

| P | Feature | Notes |
|---|---|---|
| P0 | Pixel-perfect retro UI at integer scale 1×–5× / Auto, never blurred (Retina, 4K, Windows 125/150 %) | ADR-3 |
| P0 | Title bar = menu bar: idle it shows program name, paint mode, frame `n/total`, status flags (A/a antialias, B background, P process, S/s stencil, T translucency) and coordinates; menus Picture, Brush, Mode, Anim, Color, Effect, Prefs drop down | DPaint IV AGA menu set; historic single-key shortcuts kept where conflict-free, Cmd/Ctrl for file/edit |
| P0 | Two-column toolbox at the right: 10 built-in brushes, then Dotted/Continuous freehand, Line/Curve, Fill/Airbrush, Rect/Circle, Ellipse/Polygon, Brush selector/Text, Grid/Symmetry, Magnify/Zoom, Undo/CLR. Shape icons split diagonally: upper-left = outline, lower-right = filled | left-click selects; right-click opens the tool's options (Spacing, Fill Type, Font, Gridding, Symmetry, size drag, Clear) |
| P0 | Colour indicator + palette under the toolbox, FG = left mouse, BG = right mouse, AGA page selector for 256 colours; `[ ]` / `{ }` step FG/BG | |
| P0 | Canvas zoom (1–16×), pan, pixel grid; coordinates with lower-left (DPaint default) or upper-left origin | |
| P1 | F10 hide toolbox + bar, F9 bar only, Shift-F10 all panels; `S` show page | |
| P1 | Tooltips: English name and shortcut of the toolbox and Control Panel button under the resting pointer (a DeluxePainter addition, owner request) | done (ADR-41): after 0.5 s, the next button at once, hidden under menus, requesters and playback; split buttons name the half under the pointer |
| P1 | Keyboard navigation in requesters (AGA addition) | |
| P0 | Undo / redo (multi-level) | DPaint had one level; we keep the button, add history |
| P1 | Requesters (modal, keyboard: Enter/Esc/Tab): Screen Format, Page Size, Palette, Fill Type, Spacing, Grid, Symmetry, Font, File | Square corners, 1-px bevels |
| P1 | Magnify window (split view: normal + zoomed) | DPaint's signature zoom; done (ADR-14), 2×–20× |
| P1 | Spare page (swap, copy, merge front/back) | |
| P1 | Hide/show toolbox and menu bar for full-screen painting | |
| P0 | Window: snaps to the picture's screen in whole and half pixel-size steps (Prefs ▸ Pixel Size Largest/1x–8x, Cmd/Ctrl +/−/0), full screen centred with Ctrl+Cmd+F / F11 | done (ADR-13); layout C (screen larger than the picture's screen), grey work area, movable requesters and status line dropped |
| P1 | Settings file: pixel size, Prefs toggles, last drawer, grid/symmetry/spacing/fill, translucency and Light Table choices are remembered in `settings.json` in the config directory (`--no-settings` skips it) | done (ADR-33); full-screen state and window position not saved |
| P1 | Layout preference "Classic screen": fixed 640×512 (PAL) / 640×400 (NTSC) virtual screen, letterboxed | decided, design option A |

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
| P0 | Freehand (dotted / continuous / filled), single pixel | built-in brush sizes/shapes; done (ADR-14) |
| P0 | Straight line | done (ADR-14); Shift = horizontal/vertical |
| P1 | Curve | done (ADR-14): drag, release, bend, click |
| P0 | Rectangle, circle, ellipse — outline / filled | done (ADR-14); Shift = square, Be Square for circles; ellipse rotation open |
| P1 | Polygon — outline / filled | done (ADR-14) |
| P0 | Flood fill (solid) | done (ADR-14), 4-connected |
| P1 | Fill types: solid, brush, wrap, pattern, perspective(P3), 8 gradient types (linear, shape, circle, contour, highlight …) with dither, random + range | Fill Type requester; also used by filled shapes. Done for the Fill tool (ADR-20): solid and four gradients (Vertical, Horizontal, Vertical Line, Horizontal Line) through the colour range of FG, optional dither; the rest ghosted, filled shapes still solid |
| P1 | Airbrush, spacing (N total / every Nth / airbrush / continuous) | Spacing requester; airbrush done (ADR-14), Spacing requester and airbrush radius done (ADR-20) |
| P0 | Pipette (pick colour) | |
| P1 | Brush pick-up (rectangle, lasso), last brush, custom brush stamping | |
| P1 | Brush transforms: flip H/V, rotate 90°/any, stretch/halve/double, outline/trim, handle | flip, rotate 90, halve/double, outline/trim, handle centre/corner, BG/FG colour items, load/save brush files (ILBM) done (ADR-19); stretch, any angle, shear, bend, place handle (drag gestures), spare brush and Remap done (ADR-30); Brush ▸ Spare ▸ Metamorph.. (Alt+`m`) done (ADR-36) |
| P1 | Text tool with bitmap fonts | original fonts only; UI font done (ADR-14), Choose Font open |
| P1 | Grid snap, symmetry (point/mirror, N-fold) | Grid and cyclic symmetry done (ADR-14); Gridding and Symmetry requesters (X/Y spacing, cyclic or mirror, order 2–40) done (ADR-20); tile open |
| P1 | Stencil (protect colours) | Effect ▸ Stencil Make.. / Remake / Reverse / On-Off / Free done (ADR-20); Show, Load, Save done (ADR-31, own JSON file); Stencil ▸ Paint (Ctrl+`s`) and Background ▸ Lock FG done as an area stencil that tools, Fill and CLR paint around (ADR-39); painted stencils are not saved, and fills and CLR still ignore the colour stencil |
| P1 | Paint modes (F1–F8 + menu): Matte, Color, Replace, Smear, Shade, Blend, Cycle, Smooth, Mix, HBrite | implemented in `paint::plot` (ADR-19, Mix is a guess); the tools call it once custom brushes are wired |
| P2 | Effects: Translucency, Process, AntiAlias (off/low/high, off by default), fixed Background | Effect menu; Translucency, Process, AntiAlias (custom brushes) done (ADR-31) |
| P0 | Antialiasing **off** by default for all tools | |
| P3 | Perspective, bend/shear, metamorph | bend/shear done (ADR-30); Effect ▸ Perspective done: Do, FillScreen, Reset, Center, Settings.., keypad keys on the main keys (ADR-37); Metamorph done (ADR-36); the Perspective fill type stays ghosted |

**Custom brushes (ADR-18), done:** Brush Selector rectangle pick-up (`b`; left = copy, right = cut, Shift = square, BG or AutoTransp corner colour transparent, handle centred, Matte afterwards), `B` and right-click on the Brush Selector bring the last brush back, a built-in brush click drops it. Every brush tool (freehand, line, curve, shape outlines, airbrush, symmetry copies) stamps it, and it follows the pointer as a preview. Matte, Color and Replace paint as in DeluxePaint; the Mode menu's mode, the colour range holding FG and the stencil reach every stroke and shape command, filled shapes included. Open: lasso/polygon pick-up, AnimBrush, the other modes' pixel effects (separate work), flood fill through the stencil.

## 4. Colour

| P | Feature | Notes |
|---|---|---|
| P0 | Colour mixer: RGB/HSV sliders, spread, copy, exchange, undo; AGA "Scale" RGB4→RGB8 | docked as a strip at the bottom like DPaint |
| P1 | Arrange palette, remap picture/brush, use brush palette, default palette, load/save palette | Arrange and picture Remap done (ADR-31); Color Set Load/Save = palette files; brush Remap done (ADR-30); Use Brush Palette done, Recompute only on HAM screens, so ghosted for now (ADR-39) |
| P2 | Colour cycling: 8 ranges × up to 32 colours, rate 0–63, direction, live preview (display-only) | Range requester (bottom strip); CRNG/CCRT; Tab cycles on screen, also under requesters/menus and during playback (ADR-35) |

Range requester (Color ▸ Ranges.., ADR-22): done as a centred modal requester (range 1–8, span by two clicks or a drag, rate 0–63, direction, active, clear) with two inactive default ranges on new pictures; the requester is not yet a bottom strip, has no colour beads/free RGB values, Random/Dither or Undo/Revert, and the ranges are not yet used by gradient fills, Shade or Cycle modes.

## 5. Layers & projects (modern additions)

| P | Feature | Notes |
|---|---|---|
| P0 | `.dpxproj` save/open — lossless, atomic writes | Picture ▸ Save.. writes the project, Load.. opens it, Export.. writes IFF ILBM / PNG (ADR-23, ADR-27, [PROJECT_FORMAT.md](PROJECT_FORMAT.md)); animation frames saved as format version 2 (ADR-28); one layer, no undo history or spare page yet |
| P1 | Layers: add/delete/rename/reorder, visibility, lock, opacity, Normal blend | |
| P1 | Layer groups | |
| P1 | Autosave & crash recovery | Recovery project after 30 s idle and on quit; "Recover Picture" on start; Quit asks when unsaved (ADR-34) |
| P3 | Masks, more blend modes, adjustment layers | |

## 6. Files

| P | Format | Import | Export |
|---|---|---|---|
| P0 | IFF ILBM (ByteRun1, mask, CMAP, CAMG) | ✓ | ✓ with ExportAnalyzer report |
| P0 | PNG | ✓ (indexed as is; true colour quantised ✓ ADR-9; Picture ▸ Load.. asks first: fit into the page, centre / top left, colour count or current palette, dither none / Floyd–Steinberg / ordered, true-colour thumbnail ✓ ADR-42; as layer later) | ✓ native resolution |
| P0 | JPEG | ✓ (quantised, EXIF orientation; ADR-9; Import requester as for PNG, ADR-42) | ✗ (never) |
| P2 | ILBM EHB / HAM6 / HAM8, CRNG/CCRT | ✓ (EHB opens as 64 free colours, HAM6/HAM8 decoded and reduced to the screen's palette, ADR-26; save writes plain indexed ILBM) | ✓ (CRNG/CCRT only; EHB/HAM export in Phase 5) |
| P2 | IFF ANIM (op 5 first) | ✓ | ✓ |
| P2 | IFF brushes / AnimBrushes | ✓ | ✓ |
| P2 | 24-bit IFF ILBM, PBM | ✓ (DPaint IV AGA reads these) | — |
| P1 | Brush via system clipboard (copy/paste image) | ✓ | ✓ |
| P3 | PCHG, SHAM, CTBL, DEEP | after documented need | — |

## 7. Animation

| P | Feature | Notes |
|---|---|---|
| P2 | Frames: add/copy/delete/set count, frame rate, go to, play / play once / ping-pong | done (ADR-28); Set Range, loop and play once backward, Shift+`1`–`5` done (ADR-38) |
| P2 | Light table (onion skin: previous / two back / next / spare) | done (ADR-28) |
| P2 | Animation Control Panel, Method | done (ADR-38); Method changes nothing here (it only describes DeluxePaint's memory use) |
| P2 | AnimBrush pick-up and settings | done (ADR-36) |
| P3 | Move requester (path/rotation with easing), metamorph | Move done (ADR-38; its Fill, Load and Save stay ghosted); Metamorph done (ADR-36) |

**Frames and Light Table (ADR-28), done:** a document holds 1–999 frames (at most 128 MiB of frame pixels); every page command paints the current frame and undo steps back to the frame they changed. Anim ▸ Frames ▸ Set #.. / Add Frames.. / Copy Frames.. / Delete Frames.., Control ▸ Set Rate.. / Previous (`1`) / Next (`2`) / Goto.. (`3`) / Play (`4`) / Play once (`5`) / Ping-pong (`6`), Alt+`=` adds a frame; playback only redraws (Esc, Space or a click stops, Left/Right change the rate). The title bar shows `n/total` with several frames. Effect ▸ LightTable (`l`; Alt+`l` Dim; Ctrl+`1`–`4` Two Back / Previous / Next / Spare; Merge, undoable) shows the other frames through the BG colour, dimmed by nearest palette colours, on screen only. **IFF ANIM files (ADR-32), done:** Anim ▸ Load.. (and Picture ▸ Load..) opens a FORM ANIM as a multi-frame document, Anim ▸ Save.. (or Export.. with a `.anim` name) writes IFF ANIM (op 5, closing loop frames); `info` and `convert` handle `.anim`, MCP `open_picture` / `save_picture` too. Export to a picture format still writes the current frame with a note. Open: animpainting (Alt held while drawing), per-frame palettes, the multi-frame Clear requester, menu check marks for the LightTable items, `r` (reverse) and the arrow keys during playback as listed in DeluxePaint's key table.

**AnimBrush and Metamorph (ADR-36), done:** Anim ▸ AnimBrush ▸ Pick up.. (or Alt held with the Brush Selector) takes one cel per frame from the current frame on; every stamp paints the next cel (Settings..: Duration, Current cel, forward / backward / ping-pong). Use, Free, Load.. and Save.. (IFF ANIM with transparent colour and handle), `B` and a right-click on the Brush Selector bring it back; `7` / `8` step through the cels, Shift+`7` / Shift+`8` jump to the first / last. Brush transforms change every cel. Brush ▸ Spare ▸ Metamorph.. (Alt+`m`) morphs the custom brush into the spare brush as a new AnimBrush. One command (`anim_brush_paint`) for GUI, CLI and MCP.

**Anim control and Move (ADR-38), done:** Control ▸ Set Range.. limits Play once, Ping-pong and Play once backward to a range of frames (not Play, as in the reference; owner question in ADR-38); Control ▸ Panel On/Off (Alt+`a`) shows the control bar with a frame slider and ten buttons at the bottom of the screen; Method is a Compressed / Expanded pair. Anim ▸ Move.. (`M`) moves and turns the brush in 3D over the frames (distance and angle X/Y/Z, ease in/out, Cyclic, Go From / Come To, Record forward / in place / backward, Preview, Trails, Draw); the whole move is one undo step (`move_brush`).

## 8. MCP (automation)

| P | Feature |
|---|---|
| P0 | Spike: `app_status`, `new_document`, `get_document_state` (Phase 1) |
| P1 | Drawing, palette, layer, frame, undo/redo, transaction tools; preview PNG; revision checks |
| P1 | Import/export + `validate_amiga_export` |
| P2 | Live bridge to the running GUI with "changed by MCP" indicator |

## 9. Explicitly out of scope

Emulation of the original program; GIF/BMP/TIFF/PSD export; JPEG export; web/Electron UI; copying original EA artwork, fonts, icons or manual text; claims of full DPaint feature parity before it is verified.
