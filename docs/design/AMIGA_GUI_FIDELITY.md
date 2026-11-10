# Amiga GUI fidelity rules (binding)

Owner directive (2026-11-09): **the GUI is the Amiga DeluxePaint GUI — reproduce it, do not reinterpret it.** These rules override the earlier sketch mockups in `mockups/`, which were too loose (HiRes-sized widgets, placeholder font, grey work area, status line, modern list rows).

## 1. The screen is an Amiga screen

- The UI is rendered in the **pixel grid of the document's Amiga screen mode**. On a LoRes screen (320 px wide) the title bar, font, toolbox and palette are drawn in LoRes pixels and therefore look twice as chunky as on a HiRes screen (640 px). This is exactly what the reference screenshots show.
- Display aspect follows the mode: LoRes non-interlaced ≈ square pixels; HiRes non-interlaced = pixels twice as tall as wide (each line shown twice); LoRes interlaced = pixels twice as wide; HiRes interlaced ≈ square. The final image is scaled to the window by an integer factor with nearest-neighbour filtering.
- The **page fills the screen**. Toolbox, palette and title bar sit on top of the picture. No grey work area, no status line, no extra panels.
- Layout decision "C" means only: the virtual screen may be **larger** than a stock Amiga mode so it fills a big window (like a bigger screen mode; the page scrolls as in DPaint). Everything drawn on it is identical. Decision "A" = stock Amiga screen sizes (e.g. 320×256, 640×256, 640×512), letterboxed.

## 2. UI colours come from the picture palette

DeluxePaint draws its interface with pens taken from the **current screen palette**. With few colours (e.g. 16) the menu bar, toolbox and requesters therefore appear in picture colours (reference: DPaint II with a 16-colour Tutankhamun picture → gold bars and buttons, black text/icons).

Implementation:

| Pen | Ideal colour | Used for |
|---|---|---|
| `face` | light grey (≈ #AAAAAA) | title/menu bar, toolbox buttons, requester background |
| `dark` | black | text, icons, outlines, bevel shadow |
| `shine` | white | bevel highlight |
| `select` | Amiga blue (≈ #0055AA) | highlighted menu item / selected toggle |
| `ghost` (optional) | grey 60 % from `dark` to `face` (≈ #666666) | text of disabled items; only if the palette has a close grey, else checkerboard fallback |

- Each pen = palette entry nearest to its ideal colour (perceptual distance, deterministic tie-break by lowest index). Recomputed on every palette change.
- If two pens collapse onto the same entry, fall back so that text stays readable (`dark` must differ from `face`; then `shine` may equal `face`).
- Exact DPaint selection rules (e.g. preferred register indices) are **to be verified** against clean emulator captures; until then the nearest-colour rule above applies.

## 3. Elements to reproduce

| Element | Reference | Notes |
|---|---|---|
| Title bar | Wikipedia LoRes shot, `inapp_gfx_03`, `Inapp_gfx_02` | `face` background, text **"DeluxePainter"** + two spaces + paint mode name, depth gadget at the right, `dark` line beneath. Becomes the menu strip while the right mouse button is held. The original shows its own product name here; DeluxePainter always shows its own name (owner directive 2026-11-09: never "DeluxePaint" in the GUI — trademark). |
| Menus | `inapp_gfx_03` | Intuition style: `face` box with `dark` border, highlighted item filled in `select` with `shine` text, Right-Amiga shortcuts as key glyph + letter, plain-key shortcuts in `select`, `▸` for submenus, check marks in Mode/Prefs, ghosted items for unavailable entries (solid `ghost` pen, see §4). Geometry in §4. |
| Toolbox | all screenshots, manual §1.4 | Right edge, 2 columns, built-in brush block on top, 10 tool rows, then colour indicator bar, then palette. Buttons: `face` fill, `shine` top/left edge, `dark` gaps/shadow, `dark` icons; selected tool inverted. LoRes width 24 px (to calibrate). |
| Palette | screenshots | Columns of colour cells below the indicator; column count depends on number of colours; AGA page selector `◂ a ▸` for >64 colours. |
| Requesters | `Inapp_gfx_04/05`, `Choose_Setup` | `face` panel, 2-px `shine`/`dark` bevels, centred title, bevelled rectangular buttons, numeric fields, selected toggles in `select`. Square corners only. |
| Font | all | 8×8 bold Topaz-like bitmap font. Topaz itself is part of the Amiga ROM and not redistributable, so DeluxePainter ships an **original** font drawn to the same metrics and style. |
| Icons | all | Redrawn pixel by pixel at the original sizes and in the original style. |
| Mouse pointer | `Inapp_gfx_01/04/05` | Amiga-style arrow pointer in `select`/`shine`-like colours; crosshair while painting. |

## 4. Measured LoRes geometry (implemented in `crates/dpx-ui/src/screen.rs`)

Measured on the 2× captures `inapp_gfx_03/04/05` (320×256 PAL LoRes) by averaging 2×2 blocks.

| Element | Geometry (screen pixels) |
|---|---|
| Title bar | rows 0–10: row 0 `shine`, rows 1–9 `face`, row 10 `dark`; text origin (3, 1) |
| Depth gadget | 15×9 at (`w`−16, 1), two overlapping rectangles in `dark` |
| Toolbox | x `w`−24 … `w`−1, `dark` background |
| Brush block | (`x0`, 11), 23×20, `shine` top/left edge; 10 brushes: round 1/3/5/7, square 5/4/3/2, two scatter brushes (3 and 5 pixels; the top dot of the 5-pixel one sits at toolbox x 18, y 24) |
| Tool buttons | 9 rows from y 32, pitch 12, button 11×11 (`shine` top/left edge, `face` fill), 1-px `dark` gaps; icon 9×9 at (+1, +1) |
| Split icons | outline shape + anti-diagonal outside the shape + fill where x+y ≥ 8; split freehand shows only the diagonal's corner ticks |
| Selected tool | only the chosen variant, inverted (`face` on `dark`); filled circle = 7×7 disc |
| Colour indicator | (`x0`+5, 142), 11×5, `shine` top/left edge, FG colour inside |
| Palette | from y 152; 32 colours = 4 columns × 8 rows, cells 6×13; 16 colours = 2 × 8 |
| FG cell | 1-px `face` frame, offset 1 px up/left |

Font: original 8×8 font, set **proportionally** like the reference screen font (owner decision 2026-11-09): each glyph is trimmed to its inked columns plus a 1-px gap, space advances 7 px, digits are tabular (common width) so coordinates do not jitter. Glyphs with a centre stem are drawn symmetric in the 8-px cell (stem in columns 3–4).

### Menus (implemented in `crates/dpx-ui/src/menu.rs`)

Measured on `inapp_gfx_03` (Picture menu open, "Load.." highlighted).

| Element | Geometry (screen pixels) |
|---|---|
| Menu strip | rows 0–10 all `face` (no shine/dark line, no depth gadget) |
| Title slots | first slot at x 3; title text at slot + 2; highlighted title = slot filled in `select` on rows 0–8, text `shine` |
| Menu box | left = slot − 3, top = 9; `dark` border 2 px left/right, 1 px top/bottom; `face` inside |
| Items | first item top = box top + 4, pitch 10; text at (box x + 6, item top + 1); 3 rows padding + border below the last item |
| Item highlight | x box + 4 … box right − 4, rows item top … item top + 9, `select`; text `shine` |
| Shortcut column | letter at `box_x + box_w − 12`; Right-Amiga glyph 14×8 at letter − 17; plain keys (e.g. `S`, `Q`) in `select`; submenu triangle 4×7 at letter + 3 |
| Box width | widest item: text + 8 + shortcut column, or text + 10 without shortcut (reference: "Screen Format.." + 10) |

Not visible in the references, therefore chosen and **to calibrate**: check-mark indent (13 px, Intuition LOWCHECKWIDTH), submenu position (right of the parent with 4 px overlap, first sub item level with its parent item; flipped to the left at the screen edge), which shortcuts are displayed inside submenus.

Known deviations:
- Picture ▸ Screen Format opens only from the menu, not at program start (owner decision 2026-11-10). It is drawn on its own HiRes screen like the AGA original; layout follows `Choose_Setup.png` but is not pixel-calibrated. Screen Size offers only Standard; the information panel shows our own fields (chipset, colours, size, pixel shape, aspect) and the credits panel our own text (ADR-7).
- Picture ▸ Load also reads true-colour PNG and JPEG (ADR-9), which DeluxePaint IV did not: big pictures are scaled to the screen, colours reduced with Floyd–Steinberg, transparent pixels shown as pink colour 0.
- Picture ▸ Load/Save use our reconstruction of the DeluxePaint file requester (ADR-8), measured from the scanned manual figure (not pixel-calibrated). Disks/Assigns list modern volumes and well-known folders; hidden files are not listed; `*` works as `#?`; names may be longer than AmigaDOS' 30 characters; "# of Frames" is ghosted until animation exists; the overwrite question and its Yes/No box are our own wording and layout. In palettes without a blue, the `select` pen used for `<DIR>` entries may be a grey.
- In HiRes, laced and SuperHiRes modes the LoRes interface is drawn with enlarged pixels (2 or 4 screen pixels per UI pixel) instead of DeluxePaint's HiRes artwork (ADR-7).
- Disabled items are drawn in a solid `ghost` pen instead of Intuition's 1-in-4 dot pattern (0x2222/0x8888), which tore the 2-px strokes of our font (owner decision 2026-11-10). The ghost pen is the palette entry closest to a grey 60 % of the way from `dark` to `face`; if the palette has no such grey, the item is thinned with a 50 % `face` checkerboard instead. No reference shows how DeluxePaint itself ghosts items.
- Our font is about 1 px per glyph wider than the original screen font, so menus are wider than in the reference (Picture box 123 instead of 112 px) and title slots use text + 6 instead of text + 8 so that all seven titles fit into 320 px. Owner decision (2026-11-10): keep the current font; the wider menus are accepted.
- The Right-Amiga key glyph is an original drawing (an "A" cut out of a key cap), not a copy of the system imagery.
- Color ▸ BG -> FG and BG <-> FG are undoable in DeluxePainter (not in DeluxePaint).
- Undo keeps 64 steps instead of one; `Shift+U` and a right click on UNDO redo (ADR-6).
- AGA addendum items (Brush ▸ Free, clipboard Cut/Paste) are not in the menus yet: their menu position is not documented.

## 5. Calibration

The local screenshots are lossy (JPEG artefacts, non-integer scaling). For pixel-exact geometry the owner can supply **lossless captures from an emulator** (WinUAE/FS-UAE screenshot PNG, native resolution) of: idle screen LoRes and HiRes, an open menu, the palette requester, Fill Type and Spacing requesters, Screen Format. Until then, geometry is measured from the existing references and marked "to calibrate".
