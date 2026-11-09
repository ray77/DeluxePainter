# Video UI analysis — "Deluxe Paint IV: Tutorial (English)"

| Field | Value |
|---|---|
| URL | https://www.youtube.com/watch?v=F3csY10bVZI |
| Uploader | AmigaMuseum (uploaded 2020-02-04) |
| Original | VHS tutorial by Saddleback Graphics, © 1991 Prism Multimedia (per end credits) |
| Duration | 58:17 |
| Program shown | **DeluxePaint IV (v4.0, OCS/ECS)** — *not* the 4.5 AGA version. Screen Format requester offers Lo-Res/Med-Res/Interlace/Hi-Res, 2–64 colours + HAM, which is the pre-AGA requester. |
| Accessed | 2026-11-09 |

## Method and limits

- The YouTube transcript (auto-generated English captions) could not be loaded without a signed-in session, so **no spoken narration was analysed**.
- Frames were sampled from the video element every 20 s (18 s in the last part) and arranged into contact sheets; ~170 frames were inspected at thumbnail resolution. **Timestamps are ±10 s.** Small text in requesters was partially legible only; field names marked `?` are uncertain.
- Everything here must be cross-checked with the DPaint IV AGA manual (`docs/research/manual-ui-notes.md`). Where the video (v4.0) and the AGA manual (v4.5) differ, the AGA manual wins for the design basis.

## Chapter map

| Time | Topic | Observed UI |
|---|---|---|
| 0:00–3:10 | Intro | Presenter, title cards |
| 3:30 | Screen format | **Choose Screen Format** requester: Format list (Lo-Res 320×200, Med-Res 640×200, Interlace 320×400, Hi-Res 640×400), Colors (2/4/8/16/32/64, HAM), Overscan (Off/Std/Max), NTSC/PAL toggle, Program Loading (Save/Load All), Ok |
| 3:50–5:30 | Display modes | Explainer cards: resolution × colour matrix, 32 base colours + Halfbrite (EHB), HAM 4096 colours, Overscan maximum |
| 5:50–7:30 | Main screen | Empty page, menu bar (`DeluxePaint` + current mode name, e.g. "Color"/"Matte"), toolbox at right, palette below toolbox. Effect menu open: Stencil, LightTable, Background, AntiAlias(?), Process(?), Translucency, Perspective |
| 8:30–9:10 | Freehand | Dotted freehand (spaced) vs continuous freehand; on-screen captions name the tool ("Reduce"…) |
| 9:30–12:10 | Fill types | Solid, Brush, Wrap, Perspective; gradient fills Linear / Shape / Circle / Contour / Highlite. **Fill Type requester**: Solid/Brush/Wrap/Perspective/Pattern buttons, From brush, gradient type buttons, Range selector, Dither slider, Random, Cancel/Ok |
| 12:50–13:10 | Spacing | Spacing types explained: N Total, Every Nth dot, Airbrush, Continuous (matches local screenshot `Inapp_gfx_05.png`) |
| 13:30–14:10 | Shapes | Curve, airbrush, ellipse demos; captions name each toolbox button |
| 14:30–14:50 | Brushes | Brush pick-up (rectangle and lasso), "Last brush" |
| 15:10–15:30 | Text | Font requester: font list, size list, Bold/Italic/Underline, Show, Cancel/Ok |
| 16:30–18:30 | View aids | Grid on/off, Symmetry (kaleidoscope pattern), Magnify/zoom, CLR (clear), crosshair coordinates |
| 18:50 | Menus chapter | |
| 19:10–19:30 | Picture menu | Load, Save, Delete(?), Print, Flip, Spare ▸, Page Size, Show Page, Screen Format, About, Quit |
| 20:10–21:30 | File requesters | **Load Picture / Save Picture / Delete File**: file list with scrollbar, Drawer, Pattern(?), File fields, Disks / Assigns / Parent buttons, action button + Cancel |
| 21:50–22:10 | Spare page | Spare ▸ Swap, Copy to Spare, Merge in Front, Merge in Back, Delete this Page |
| 23:10 | Page size | Screen-format style requester with Page Size: Screen / Overscan(?) |
| 23:30–25:10 | Brush menu | Load, Save, Restore, Size ▸ (Stretch, Halve, Double, Double Horiz, Double Vert), Flip ▸ (Horiz, Vert), Edge ▸ (Outline, Trim), Rotate ▸ (90°, Any angle, Shear), Bend, Handle ▸ (Center, Corner, Place); Brush→Spare |
| 25:30–26:50 | Mode menu (paint modes) | Matte, Color, Replc, Smear, Shade, Blend, Cycle, Smooth, Mix, HBrite (names confirmed by the manual). Demonstrated on a photo-like image |
| 27:10–34:30 | Color menu | Ranges…, Cycle, Palette, Color Set ▸, BG→FG, BG↔FG, Remap, Brush ▸ (Remap, Change Transparency). Palette requester is a **horizontal strip at the bottom of the screen** (palette grid, RGB/HSV sliders, spread/copy/exchange buttons, range editing). Color Set ▸ Mixer On/Off, Arrange, Use Brush Palette, Restore Palette, Default Palette, Load, Save |
| 34:50–38:50 | Effect menu | Stencil ▸ (Show, Make, Remake, Lock FG, Paint On/Off, Reverse, Free, Load, Save) with **Make Stencil** requester (colour grid, Clear, Invert, Make, Cancel); LightTable ▸ (On/Off, Two Back, Previous, Next, Spare); Background ▸; AntiAlias (None/Low/High); Translucency (Tint/Hue/Value?); Perspective ▸ (Do, FillScreen, Reset, Center, Settings…) |
| 39:10–39:50 | Perspective settings | Requester: X/Y/Z angle fields, Grid, From Brush, Angle Step, Type (Screen/Brush), Display (Angle/Pos), Cancel/Ok |
| 40:10–42:10 | Prefs menu | Coords, Fast FB, MultiCycle, Be Square, Workbench, ExclBrush, AutoTransp, No Icons, AutoGrid(?), further items illegible |
| 43:20–47:40 | Animation | Load Anim requester (Load / Append); Anim menu: AnimBrush ▸, Load, Save, Move…, Frames ▸ (Set #, Add Frames, Copy Frames, Delete Frames), Control ▸ (Set Rate, Previous, Next, Go To, Play, Play once, Ping-pong), Method ▸; requesters "Set # of frames", "Set frames per second", "Go to Frame"; frame-control bar at the bottom of the screen |
| 48:20–49:40 | AnimBrush | AnimBrush ▸ Pick up, Load, Save, Settings…; requesters "Pick up AnimBrush: Number of cels", "AnimBrush Settings" (cels, duration, current, direction) |
| 50:00–53:20 | Move requester | Dist X/Y/Z, Angle X/Y/Z, Clear, Go Back, Brush/Cyclic, Ease-Out, Ease-In, Count, Direction, Move/Record, Preview, Trails, Draw, Cancel, Load, Save, Exit; Load Move file requester |
| 53:40–54:52 | Metamorph | Metamorphosis between two brushes (egg → chicken), "Make AnimBrush: Number of cels" |
| 55:10–56:04 | LightTable | Onion-skin overlay of previous frame while drawing |
| 56:22 | Anim method | Method ▸ Compressed / (second option illegible) |
| 56:40 | Clear | Clear requester: Current Frame, Range, All Frames |
| 57:16–58:17 | Outro | Credits |

## Interaction patterns (cross-check against manual)

| Action | Start state | Window / popup | Settings | Confirm / cancel | Result |
|---|---|---|---|---|---|
| Choose screen format | Program start / Picture ▸ Screen Format | Modal requester, centred | Resolution, colours, overscan, PAL/NTSC | Ok / (Cancel) | New screen + page; picture retained or cleared |
| Pick a tool | Any | none | — | — | Toolbox button highlighted; menu bar shows tool name briefly |
| Tool options | Right-click a toolbox button (to verify in manual) | Modal requester (Fill Type, Spacing, Font, Grid, Symmetry, Magnify) | Tool-specific | Ok / Cancel | Option stored, tool active |
| Open a menu | Hold right mouse button on menu bar | Drop-down, Amiga style (menus visible only while RMB held) | — | Release RMB on item | Command executes; `▸` submenus open to the right |
| Load / save picture | Picture ▸ Load / Save | File requester | Drawer, file, disks/assigns | Load|Save / Cancel | File read/written |
| Edit palette | Color ▸ Palette (or `p`) | Bottom strip requester (not a centred window) | Per-colour RGB/HSV, spread, copy, ranges | Ok / Cancel / Undo | Palette changed |
| Stencil | Effect ▸ Stencil ▸ Make | Modal colour grid | Which colours are protected | Make / Cancel | Painting cannot alter protected colours |
| Animate brush along path | Anim ▸ Move | Large modal requester | Distance, angle, easing, count, direction | Preview / Draw / Exit | Frames rendered |

## Visual notes for the pixel UI

- Menu bar: one-line, light grey with black text; screen title "DeluxePaint" plus current paint mode name at left; menu titles appear only while RMB is held (in the video the bar shows `DeluxePaint  Color` when idle).
- Toolbox: two columns of square icon buttons at the right edge; top block is the built-in brush picker (dots of different sizes); below: freehand dotted/continuous, line/curve, fill/airbrush, rectangle/circle, ellipse/polygon, brush pickup/text, grid/symmetry, magnify/zoom, undo/CLR. The FG/BG indicator and palette sit below.
- Requesters: grey panel, 1-pixel black/white bevels, title strip centred, rectangular buttons with bevel; selected toggle buttons are drawn inverted/blue. **All corners are square** — the rounded corners visible in some local screenshots come from the screenshot source, not from DPaint.
- Palette requester docks as a full-width bar at the bottom of the screen instead of a floating window.
