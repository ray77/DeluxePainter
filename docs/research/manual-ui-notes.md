# DeluxePaint IV AGA (v4.5 basis) - UI notes derived from the manuals

Purpose: reference for the "DeluxePainter" re-implementation. Written from the OCR text and, where OCR was
useless (key tables, toolbox and menu figures), from page images of the same PDFs. Behaviour is described in
the author's own words; only labels (menu items, buttons, keys) are quoted.

## Page-reference convention

* `(AGA p.N)` = PDF page N of `Deluxe-Paint-4-AGA-Manual-ENG.pdf` (OCR file `aga/pN.txt`).
  Printed manual page = PDF page - 19 (e.g. Reference "Tools" starts at printed p.185 = AGA p.204).
  The AGA booklet ("Read Me First" addendum) occupies AGA p.4-16 (printed i-xiii).
* `(DP4 p.N)` = PDF page N of `DeluxePaintIV.pdf` (non-AGA). Printed page = PDF page - 6.
* **Key finding:** the body of the AGA manual is the DeluxePaint IV manual unchanged. A word-level diff of the
  whole Reference chapter against the DP4 file differs only by OCR noise (and by how well figure text was OCR'd).
  All AGA-specific behaviour lives in the Read Me addendum (AGA p.4-16). Wherever this document says
  "manual body", the text applies to both versions; AGA-only deltas are collected in sections 7 and 11.
* Key legend: **Alt** = Alt key; **Amiga** = Amiga key (file shortcuts are Right-Amiga); shortcut letters are
  case sensitive (capital = Shift+letter) (AGA p.23). One boxed key glyph, OCR'd as "i" / "is", is the plain
  **I** key (seen in the page image, AGA p.210): held while clicking certain tools it changes their behaviour.

---

## 1. Screen layout

### 1.1 Overall (AGA p.43, p.36, p.204)
* A single full-screen custom screen with:
  * **Title Bar** along the top: program name, current paint mode, status flags. Pointing at it and holding the
    right mouse button turns it into the **Menu Bar** (standard Amiga pull-down behaviour: hold right button,
    drag to a menu name, drag down to an item, release to select; submenus open to the right, marked by a
    triangle) (AGA p.44, p.227).
  * **Page / painting area**: everything left of the Toolbox. The page may be larger than the screen.
  * **Toolbox** on the **right edge**: built-in brushes, tool icons, UNDO / CLR, the Color Indicator and
    underneath it the **Palette** column (AGA p.204, p.36-37).
* Toolbox and Title Bar sit above the page; the user can paint underneath them (AGA p.68).
* Menu names left to right: **Picture, Brush, Mode, Anim, Color, Effect, Prefs** (figure, AGA p.44). Same seven in AGA.

### 1.2 Showing / hiding panels (AGA p.68, p.204, p.294)
| Key | Effect |
|---|---|
| F10 | Hide / show Toolbox and Title/Menu Bar together |
| F9 | Hide / show the Menu (Title) Bar alone; after F10 hid everything, F9 brings back the bar without the Toolbox |
| Shift-F10 | Hide all panels (Appendix B) |
| S | Show Page: hides Toolbox/Title Bar to show the whole page (AGA p.236) |
| Delete | Cursor arrow on/off (Appendix B) |
* The pointer must not be in the Title Bar area when pressing F9/F10 (key ignored).
* With the bar hidden, menus remain reachable: put the pointer at the top of the screen (not the extreme
  upper-left corner) and press the right button (AGA p.68).
* Tools stay reachable via keyboard equivalents (section 6).
* F9/F10 force a screen refresh, which cancels the chance to UNDO the previous action (AGA p.225).

### 1.3 Title Bar contents (AGA p.285-286, p.43, p.256)
* Left: program name followed by the **current paint mode** (Matte, Color, Replc, Smear, Blend, Cycle, Smooth ...).
  With animation frames the left corner shows **current frame / total frames** (e.g. "1/20", AGA p.84).
  Ctrl-a temporarily shows free Fast/Chip memory there (AGA p.286).
* Middle: **Color Fill box** (preview of the current gradient / pattern / perspective fill; absent for Solid) and
  one-letter flags: `A` antialias High, `a` antialias Low, `B` background fixed, `P` Process on, `S` stencil
  active, `s` stencil active with Show/Paint in use, `T` Translucency on.
* Right: **coordinates** when Prefs>Coords is on. Default origin (0,0) = lower-left of the screen; holding either
  mouse button makes the press point a temporary origin and shows displacement; OriginUL puts the origin top-left;
  units are pixels (AGA p.282-285). In Perspective mode the right corner shows X/Y/Z rotation (or positions; `\`
  toggles); Coords overrides it (AGA p.280, p.286).
* While dragging an ellipse the bar shows its rotation angle (AGA p.217).

### 1.4 Toolbox layout (AGA p.204 image; text p.47-56, p.205-226)
Two icon columns. Rows top to bottom, left column / right column:

| Row | Left | Right |
|---|---|---|
| 0 | **Built-in brushes**: block of 10 small brush icons (4 round, 4 square, 2 multi-pixel scatter brushes; the 3- and 5-pixel brushes are named in the text) | |
| 1 | Dotted Freehand | Continuous Freehand (upper-left half) / Filled Freehand Shape (lower-right half), one icon split diagonally |
| 2 | Straight Line | Curve |
| 3 | Fill | Airbrush |
| 4 | Unfilled / Filled Rectangle | Unfilled / Filled Circle |
| 5 | Unfilled / Filled Ellipse | Unfilled / Filled Polygon |
| 6 | Brush Selector (double-click switches it to polygon mode, icon changes) | Text |
| 7 | Grid | Symmetry |
| 8 | Magnify | Zoom |
| 9 | UNDO | CLR |
| 10 | **Color Indicator** (inner small rectangle = FG, larger outer rectangle = BG) | |
| 11+ | **Palette** swatches | |
* The Guided Tour says "two columns of five" for the tool icons (AGA p.48), but Figure 8.1 shows nine tool rows
  below the brush block; the figure is taken as authoritative.
* The selected tool/brush is highlighted. Grid, Symmetry and Magnify are toggles (click again to switch off) (AGA p.74).
* For the shape pairs (Freehand, Rectangle, Circle, Ellipse, Polygon): clicking the **upper-left half** of the icon
  selects the outline variant, the **lower-right half** the filled variant (AGA p.48, p.51-53, p.206, p.215).

### 1.5 Left- vs right-click on toolbox icons (AGA p.74-78, p.205-226)
Left-click selects. Right-click modifies the tool:

| Icon | Right-click |
|---|---|
| Built-in brush | SIZE cursor: move to page, hold either button and drag diagonally to resize (1x1 to 100x100 px). `+`/`-` also resize on the fly |
| Airbrush | Same SIZE drag, sets the spray area (shown as a solid circle) |
| Straight Line, Curve, unfilled Rectangle / Circle / Ellipse / Polygon | **Spacing** requester |
| Fill, Filled Freehand, filled Rectangle / Circle / Ellipse / Polygon | **Fill Type** requester |
| Brush Selector | Restore last custom brush / AnimBrush (toggle) |
| Text | **Choose Font** requester |
| Grid | **Gridding** requester (in Perspective mode: **Perspective** requester) |
| Symmetry | **Symmetry** requester |
| Zoom | Shrinks magnification (left-click enlarges) |
| CLR | **Clear** requester (AGA addendum; also Shift-K) |
| Color Indicator | Opens the Color **Mixer** |
| Palette swatch | Sets background colour (left-click = foreground) |
| Dotted/Continuous Freehand, Magnify, UNDO | none documented |
Painting rule: **left button paints FG colour, right button BG colour** (BG painting doubles as erasing);
palette: left-click picks FG, right-click picks BG (AGA p.24, p.46).

### 1.6 Palette area and FG/BG indicator (AGA p.44-46, p.206-207, p.225-226, p.240)
* Click colour = FG; right-click = BG. The FG swatch gets a grey border. Defaults: BG black (colour 0), FG grey (AGA p.37).
* Changing BG does not repaint the page; the old BG stays as a "wash" until CLR (AGA p.45).
* Click on the Color Indicator or press `,` = eyedropper: click on screen picks FG, right-click picks BG.
* `[` / `]` step FG through the palette, Shift versions `{` / `}` step BG (AGA p.226, Appendix B).
* HAM: a selector under the palette shows which of 16 sets (A-P, 16 colours each) of the 256-colour set is shown;
  arrows move, Shift-click jumps to first/last; set A = base palette, the rest are HAM colours.
* Colour count depends on the screen format (section 7). Pixels store palette indices, hence the Remap command.

---

## 2. Menus

Operation: right button on the Title Bar. Shortcut keys are ignored when the pointer is in the Title Bar; most also work while
a mouse button is held (AGA p.82). Key **a** ("Again") repeats the last menu command (AGA p.227). Item lists come from menu
figures (images) and text.

### 2.1 Picture (AGA p.227-243)
Figure order: Load.., Save.., Delete.., Print.., Flip >, Spare >, Page Size.., Show Page, Screen Format.., About.., Quit.

| Item | Shortcut | Behaviour |
|---|---|---|
| Load.. | Right-Amiga-l | Load Picture requester; palette, stencil and perspective data load too; replaces the picture (the current frame of an animation). "# of Frames" loads a numbered series as animation frames (all same format; first file's palette wins) (AGA p.228-230) |
| Save.. | Right-Amiga-s | Save Picture requester; names up to 29 chars; overwrite asks confirmation; with several frames, from/to boxes save numbered separate pictures (AGA p.230-231) |
| Delete.. | Right-Amiga-d | Delete File requester (any file type) (AGA p.232) |
| Print.. | Right-Amiga-p | Print Picture requester (3.17) |
| Flip > Horiz / Vert | - | Flip picture (or chosen frames) about X / Y axis (AGA p.234) |
| Spare > Swap | j | Show the spare page; both pages share one palette, BG colours may differ (AGA p.69, p.234) |
| Spare > Copy to Spare | Ctrl-j | Copy current page to spare (visible part if larger) (AGA p.235) |
| Spare > Merge in Front | - | Merge spare over current; spare's BG pixels transparent |
| Spare > Merge in Back | - | Merge spare behind current; current page's BG pixels transparent |
| Spare > Delete This Page | - | Frees that page (asks first) and switches to the other |
| Page Size.. | - | Set Page Size requester (3.3) |
| Show Page | S | Whole page view (reduced if bigger than screen); drag the rectangle to choose the view; arrows nudge it, Alt+arrows by screen size; any key returns (AGA p.236) |
| Screen Format.. | - | Choose Screen Format / Display Mode requester (3.1, 3.2) |
| About.. | ? or Ctrl-a | Version, authors, copyright, Chip/Fast memory, anim and AnimBrush size (AGA p.241, p.289) |
| Quit | Q | Exit, asks to save if modified (AGA p.38, p.242) |
When animating, the spare page is labelled "Scratch" and may be larger than the frames (AGA p.68).

### 2.2 Brush (AGA p.242-246; menu image p.242)
Figure order: Load.., Save.., Restore, Spare >, Size >, Flip >, Edge >, Rotate >, Bend >, Handle >.
The AGA addendum adds **Free** and clipboard Cut/Paste; menu position and shortcuts are not given (AGA p.7-8).

| Item | Shortcut | Behaviour |
|---|---|---|
| Load.. / Save.. | - | Brush files carry palette + cycling data; loading leaves the picture palette untouched (use Color>Palette>Use Brush Palette) |
| Restore | B | Return to last custom brush after using an AnimBrush, built-in brush or modification; some manipulations are not reversible by it (repeat the operation instead) |
| Spare > Brush->Spare | Alt-n | Copy current brush (current cel of an AnimBrush) to the hidden spare brush |
| Spare > Brush<->Spare | Alt-b | Swap current and spare brush; AGA: pressing Alt-b before Metamorph reverses morph direction (AGA p.11-12) |
| Spare > Metamorph.. | Alt-m | "Make AnimBrush" requester asks for number of cels; morphs spare -> current. Index lists it under Spare (AGA p.243, p.308). DP4 morph limit about 250 px wide; AGA/4.1 limited only by memory (AGA p.10) |
| Size > Stretch | Z | Drag to stretch freely; Shift keeps aspect; snaps back if memory is short |
| Size > Halve | h | 50 % in both dimensions |
| Size > Double | H | 200 % in both |
| Size > Double Horiz | X | double width (Lo-Res -> Med-Res proportion fix) |
| Size > Double Vert | Y | double height |
| Flip > Horiz / Vert | x / y | Flip about X / Y axis (all cels of an AnimBrush) |
| Edge > Outline | o | Add 1-pixel border in FG colour, repeatable (HAM may give another colour) |
| Edge > Trim | O | Remove 1-pixel border, repeatable |
| Rotate > 90 Degrees | z | Clockwise 90 degrees |
| Rotate > Any Angle | - | Drag outline about its bottom-left corner; a second use restarts from the original orientation |
| Rotate > Shear | - | Top anchored, drag bottom sideways |
| Bend > Horiz / Vert | - | Drag to bend; cursor position along the other axis sets the bend line |
| Handle > Center | Alt-s | Handle at centre (default) |
| Handle > Corner | Alt-x, Alt-y | Handle at a corner (Alt-x toggles lower corners, Alt-y upper); pick-up drag direction decides the corner |
| Handle > Place | Alt-z | Drag to set any offset |
AGA addendum: Alt-x alone cycles through five handle positions and replaces Alt-y/Alt-s (AGA p.10), but Appendix B still prints Alt-s/x/y/z - contradiction (section 12).

### 2.3 Mode (AGA p.247-249; image p.247)
Figure: Matte F1, Color F2 (ticked = default), Replc F3, Smear F4, Shade F5, Blend F6, Cycle F7, Smooth F8, Mix, HBrite. Mix and HBrite have no F-key;
HBrite is greyed unless the screen is 64-colour EHB. Details in section 5.

### 2.4 Anim (AGA p.249-259; image p.249)
Figure: AnimBrush >, Load.., Save.., Move.. (M), Frames >, Control >, Method >.

| Item | Shortcut | Behaviour |
|---|---|---|
| AnimBrush > Load.. / Save.. | - | AnimBrush files (sample extension `.anbr`) |
| AnimBrush > Pick up.. | Alt while using Brush Selector | Requester asks number of cels (taken from the current frame onward); then drag a box. Single-frame AnimBrush is not possible |
| AnimBrush > Settings.. | - | Number of cels (info), **Duration** (frames per full cel cycle), **Current** cel, **Direction** icons (forward / backward / ping-pong) |
| AnimBrush > Use | right-click Brush Selector, B | Make the last AnimBrush current again |
| AnimBrush > Free | - | Free its memory, not recoverable |
| Load.. | - | Load Anim; replaces current animation; append variant adds to the end |
| Save.. | - | Save Anim; from/to frame boxes save a section as an animation |
| Move.. | M | Move requester (3.15) |
| Frames > Set #.. | - | Set Frame Count (grow/shrink; step to frame N and change count to add/remove after N) |
| Frames > Add Frames.. | Alt-= adds one; panel "+" | Adds frames after current, copying current content |
| Frames > Copy Frames.. | - | Copy current frame to a range/all; "Insert Before Frame" box |
| Frames > Delete Frames.. | panel "-" | Delete current / range / all |
| Control > Panel On/Off | Alt-a | Animation Control Panel |
| Control > Set Rate.. | - | Frames per second, default 30, range 1-60 (above 30 unpredictable) |
| Control > Set Range.. | - | Play range From/To (From < To); used by Play once and Ping-pong, not by Play |
| Control > Previous / Next | 1 / 2 | Step frame, wraps |
| Control > Goto.. | 3 (Shift-3 = last go-to) | Go to Frame requester; Ctrl-click on the Go-to panel icon also opens it |
| Control > Play | 4 | Loop forward until space bar; `r` reverses |
| Control > Play once | 5 | Frame 1 to last |
| Control > Ping-pong | 6 | Back and forth until stopped |
| Method > Compressed / Expanded | - | RAM memory model only, not the disk format |
Other animation keys: Shift-1/Shift-2 first/last frame, Shift-4/Shift-5 reverse play, 7/8 AnimBrush cel, Shift-7/8 first/last cel, Left/Right arrows slow/speed up (AGA p.296).

### 2.5 Color (AGA p.259-271; image p.259)
Figure: Ranges.., Cycle (TAB), Palette >, Color Set >, BG -> FG, BG <-> FG, Remap, Recompute, Brush >.

| Item | Shortcut | Behaviour |
|---|---|---|
| Ranges.. | Ctrl-r | Range requester replaces the palette strip at the bottom of the screen |
| Cycle | Tab | Colour cycling on/off (only colours in ranges move) |
| Palette > Mixer On/Off | p | Show/hide Color Mixer (hiding keeps the mixing area; OK clears it) |
| Palette > Arrange | P | Arrange Palette requester |
| Palette > Use Brush Palette | - | Adopt brush palette incl. cycle info; reduces to picture's colour count if needed |
| Palette > Restore Palette | - | Back to the previous palette |
| Palette > Default Palette | - | Startup palette and default ranges |
| Palette > Load.. / Save.. | - | Palette file; only base-register colours are saved |
| Color Set > Load.. / Save.. | - | 256-colour set file; Load opens a second requester (3.4) |
| BG -> FG | - | Recolour all BG pixels to FG, on screen only, not undoable |
| BG <-> FG | - | Swap the two colours in the picture, not undoable |
| Remap | - | Re-point pixels to the current palette positions after rearranging (EX), do it straight away; not undoable |
| Recompute | - | HAM only: recompute picture data against the palette (reduces fringing for imported HAM) |
| Brush > BG -> FG / BG <-> FG | - | Same, but inside the current brush only |
| Brush > Remap | - | Fit brush colours to the closest palette colours (all AnimBrush cels) |
| Brush > Change Transparency | - | Transparent colour of the brush becomes the current BG |

### 2.6 Effect (AGA p.271-282; image p.271)
Figure: Stencil >, LightTable >, Background >, AntiAlias >, Process >, Translucency >, Perspective >.

| Item | Shortcut | Behaviour |
|---|---|---|
| Stencil > Show | - | Dim picture, masked areas lighter; flag "S" becomes "s"; painting disabled meanwhile |
| Stencil > Make.. | ~ | Make Stencil requester (3.10) |
| Stencil > Remake | - | Re-lock colours including newly painted ones |
| Stencil > Paint | Ctrl-s | Paint an area-based stencil with the current brush/tool; choose again to leave; discarded if a colour stencil is made later |
| Stencil > Reverse | - | Same as Invert in the Make requester |
| Stencil > On/Off | ` (grave) | Toggle, keeps the stencil |
| Stencil > Free | - | Delete stencil, release memory |
| Stencil > Load.. / Save.. | - | Stencil layer file; area based, screen size only, loads at original position |
| LightTable > On/Off | l | Onion skin: show neighbouring frames behind current (max four images) |
| LightTable > Merge | - | Merge visible images into the current frame and switch off; not undoable |
| LightTable > Dim | Alt-l | Dim non-current frames (default on; not in HAM) |
| LightTable > Two Back / Previous / Next / Spare | Ctrl-1 / Ctrl-2 / Ctrl-3 / Ctrl-4 | Toggle which images show (Spare = scratch page) |
| Background > Fix | - | Freeze picture; paint on top; CLR and right-button erase only new paint; no colour pick from fixed BG; costs memory like a spare page |
| Background > Free | - | Merge and release |
| Background > Lock FG | - | Stencil defined by areas painted since Fix, regardless of colour |
| AntiAlias > None / Low / High | Alt-/ cycles | Smooth jaggies; None default; needs intermediate palette shades; High is slow for big areas |
| Process > On/Off | Alt-p | Enables process painting (flag P) |
| Process > Tint (default) / Hue / Value | - | Tint: tint underlying colours toward FG; Hue: substitute hue only; Value: apply brightness only (text mentions a "None" too) |
| Translucency > On/Off | Alt-t | Blend FG over image by a percentage (flag T) |
| Translucency > Settings.. | Ctrl-t | Slider requester, default 50 % |
| Perspective > Do | keypad Enter | Enter perspective mode (brush shown as 4-cell matrix); click a tool to leave |
| Perspective > FillScreen | keypad - | Tile the screen with the rotated brush |
| Perspective > Reset | keypad 0 | Reset rotation and requester settings (centre kept) |
| Perspective > Center | keypad . | Place the centre/horizon with a large crosshair |
| Perspective > Settings.. | right-click Grid in perspective | Perspective requester (3.11) |
Translucency does not work with Text, 1-pixel Airbrush, and the 3- and 5-pixel built-in brushes (AGA p.278).

### 2.7 Prefs (AGA p.282-285; image p.282)
Check-mark toggles; several can be toggled with the menu held open. Items: Coords (Shift-\), Fast FB, MultiCycle, Be Square, Workbench,
ExclBrush, AutoTransp, No Icons, AutoGrid, OriginUL, FastAdjust.

| Item | Behaviour |
|---|---|
| Coords | Coordinate readout in Title Bar |
| Fast FB | Lines/unfilled shapes use a 1-px feedback while dragging, final result with real brush |
| MultiCycle | Cycle mode with a multicolour brush cycles each colour separately (needs ranges); off = whole brush treated as FG colour |
| Be Square | Corrects non-square pixels for circles/squares, built-in brushes, symmetry (not grid, not perspective). **AGA: on by default** and covers more tools (AGA p.10) |
| Workbench | Opens/closes the Workbench screen (default off) |
| ExclBrush | With Grid on, pick-up leaves out the right and bottom pixel border (for seamless pattern fills) |
| AutoTransp | If all brush corners share a colour it becomes transparent; else BG colour |
| No Icons | Do not write `.info` files |
| AutoGrid | Perspective grid follows the brush size automatically (default on) |
| OriginUL | Coordinate origin upper-left |
| FastAdjust | HAM only (default on there): skip fringe correction while moving a brush |
Presets: Workbench Tool Types (item name typed exactly). AGA: shell options `RUN DPAINT C FF B N FA` with codes C Coords, FF Fast FB, M MultiCycle,
B Be Square, W Workbench, X ExclBrush, A AutoTransp, N No Icons, G AutoGrid, U OriginUL, FA FastAdjust, T ToolLimit=minx,miny,maxx,maxy, I Inherit; `LOAD="file"` (AGA p.8).

---

## 3. Requesters / dialogs

General rules (AGA p.6, p.25-26): action buttons trigger commands, OK accepts changes, Cancel discards them; edit boxes accept typing
(Backspace left, Del right, arrows move); lists scroll with arrows or the scroll box; highlighted list item is the selection.
**AGA only:** press the underlined letter of a button, Tab moves between text gadgets, Enter accepts, Right-Amiga-X clears the active text gadget (AGA p.9-10).

### 3.1 Choose Screen Format (4.0 style, also AGA start-up fallback description) (AGA p.237-241, p.36)
Title "Choose Screen Format".
* **Format** list: Lo-Res 320x200 (default), Med-Res 640x200, Interlace 320x400, Hi-Res 640x400.
* **Colors** buttons: 2, 4, 8, 16, 32 (default for Lo-Res), 64, HAM; only counts valid for the format are enabled.
* **Overscan**: Off (default) / Std / Max; new dimensions are shown next to the formats.
* **Monitor**: NTSC / PAL.
* **Page Size**: Screen / Keep Same.
* Cancel, Ok. Start-up variant adds **Program Loading**: Swap / Load All.
* Ctrl+cursor keys adjust screen position (AGA p.240).
* Changing format discards brush, spare page, last requester, font directory; too many colours for memory silently yields fewer.
* Page Size: Screen = page takes new screen size; Keep Same = old page kept (AGA p.241).

### 3.2 Choose Display Mode (AGA addendum, AGA p.6-7)
Controls named in the text (screenshot too small to read layout):
* **Choose Display Mode** scrolling list of modes (depends on chipset, OS version, monitors present in Devs/Monitors, "Mode Names" setting).
* **Display Mode** text gadget: type a mode name or pattern (`#?`, `?`, new `^` = anchor at start, `~` = not; examples `^NTSC`, `~NTSC#?HALFBRITE`).
* **Display Mode Information** panel.
* **Screen Size** cycle gadget (includes CUSTOM = between min and max of the mode; limits follow Preferences overscan).
* **Page Size** cycle gadget: Screen (= screen dimensions) / Keep Same (page grows if too small for the screen).
* Palette size choices 2-256 (default HAM = HAM8 with 64 colours; choose 16 for HAM6).
* OK / Cancel. Appears at program start and via Picture>Screen Format. Text/standard overscan dimensions come from system Preferences.

### 3.3 Set Page Size (AGA p.235-236)
Title "Set Page Size": **Width**, **Height** edit boxes; presets **Standard** (screen size), **Full Page** (printable page proportion), **Overscan** (full video area); Cancel, Ok.
Clicking a preset fills the boxes (NTSC Lo-Res figure, read at 400 dpi: 320x200 / 320x340 / 368x241; Standard drawn selected). Fields read "Type in size:" / "Width:" / "Height:", presets under "Or select one:". Minimum = screen resolution; recognised maximum 1008 x 1024 (needs fewer colours / memory); p.50 says "1008 by 1008". Show Page (p.236): every n-th pixel when reduced; arrows move the rectangle one unit (page size / screen size), Alt+arrows by a screen; any key returns without repositioning; p.51: Space returns.

### 3.4 Palette: Color Mixer and Arrange (AGA p.226, p.244-248, p.267, p.89-96)
Title "Color Mixer"; opened by right-click on Color Indicator, Palette>Mixer or `p`; replaces the palette strip; painting remains possible.
* **Color Set grid** 32 slots per row, 8 rows (0-7) = 256 slots; scroll arrows 1 row (Shift = to first/last); palette colours in row 0 with white top/left edges, others light blue; empty slots checkered (paint white).
* Sample-colour box, **RGB / HSV** toggle button, three sliders (RGB 0-15; HSV hue 0-360 degrees, saturation and value 0-100 %); drag box or click either side.
* **Mixing Area**: paint with the current brush to blend colours, PICK a result into a slot.
* Buttons: **COPY**, **EX**, **SPREAD** (HSV-based) - each shows "TO" on the cursor for the second click; **PICK** (eyedropper; left = FG, right = BG; also `,`); **DELETE** (one or a range); **UNDO** (last change only); **REVERT** (back to state when opened); **OK**.
* **AGA: Scale** converts RGB4 (0-15) values to RGB8 (0-255) (AGA p.11).
* EHB: second-row halfbrite colours cannot be edited (AGA p.263). Non-HAM: a colour outside the palette paints as the nearest palette colour.
* **Arrange Palette** (`P`): same buttons; 4 rows (128 colours) visible with row number in the corner; arrow 1 row, Shift+arrow 4 rows; replaces Mixer if open.
* **Load Color Set** second requester: pick colours (click toggles; AIl / None / Base), then Add (first empty slots) or Overwrite (from colour 0), Cancel.
* Advice: strip extra Color Set colours before distributing pictures (other programs mishandle them).

### 3.5 Range requester (AGA p.259-262, p.73)
Strip at the bottom. **RANGE** slider (1-8), colour row, sample square, range bar with bead cursor, **SHOW**, **REVERSE** (Alt-r), **CLEAR**, **RATE** slider, **RANDOM** toggle, **DITHER** slider, **UNDO**, **REVERT**, **OK**. See section 8.

### 3.6 Fill Type (AGA p.210-215)
Opened by right-click on Fill / filled shapes, or `F`. Title "Fill Type". Controls (figure p.210): **Solid** (default), **Brush**, **Wrap**, **Perspective**, **Pattern** with **From Brush** button and pattern preview; **HBrite** only on EHB screens;
**Range** number box + direction buttons (horizontal, vertical, horizontal-line) + gradient shapes **Line, Shap, Cir, Con, Hi**; **Random** toggle with **Dither** slider; Cancel, Ok.
Meanings: Brush = one stretched copy of the custom brush; Wrap = brush bent to the shape's horizontal/vertical extent; Perspective = brush pattern per current perspective; Pattern = tiled pattern captured by From Brush; Range gradients use the range selected;
Line = uniform linear ignoring shape; Shap = linear following shape; Cir = radial; Con = radial following contours; Hi = contour variant for highlights. Line/radial types show a direction line from shape centre to cursor, click confirms. Title Bar Color Fill box previews it. (Eight gradient fill types in total, AGA p.104.)

### 3.7 Spacing (AGA p.74-75, p.207-208)
Title "Spacing" (right-click line/curve/unfilled shapes, or `V`). Four exclusive options: **N Total** (edit box: exact number of stamps), **Every Nth dot** (edit box: pixel interval), **Airbrush** (edit box: sprays per pixel along the path), **Continuous** (default). OK / Cancel.

### 3.8 Gridding (AGA p.77-78, p.222)
Title "Gridding": **X-spacing**, **Y-spacing** edit boxes (pixels), **Adjust** (drag a matrix on the page, upper-left fixed, lower-right dragged; click repositions), Cancel, Ok.
Tools that obey the grid: Dotted Freehand, Straight Line, Rectangle, Circle, Ellipse, Brush Selector, Text. `g` toggles; `G` also uses the brush handle as a grid point.

### 3.9 Symmetry (AGA p.78, p.204-205)
Title "Symmetry". **Point** / **Tile** buttons.
* Point: **Cyclic / Mirror** toggle, **Order** edit box (number of points), **Place** (click new centre).
* Tile: **Width**, **Height** edit boxes (pixels).
* Cancel, Ok. Not usable with Text and Brush Selector. `/` toggles the tool. Tile mode overwrites the picture under each tile (use Background>Fix first).

### 3.10 Make Stencil (AGA p.271-273, p.10)
Floating requester (draggable by title). Select colours to lock in the requester palette, the picture or the main palette (**AGA: left selects, right deselects**). Buttons **MAKE**, **Clear**, **Invert**.
In HAM also **Show**, **Fine Tune** and **Tolerance** (0-48 edit box; click a colour to lock all similar ones; 0 = identical only, 48 = everything).
Stencil is regenerated per frame automatically (colour based).

### 3.11 Perspective (AGA p.279-281)
Title "Perspective": **Grid** X / Y / Z edit boxes (default brush size, Z = Y), **From Brush**, **Angle Step** (default 90 degrees), **Type** Screen (Euler, default) / Brush, **Display** Angle / Pos, Cancel, Ok. Keypad controls in section 6.

### 3.12 Brush size / rotate / stretch (AGA p.74, p.243-245)
Not requesters: built-in brush size via SIZE cursor drag or `+`/`-`; Stretch, Rotate>Any Angle, Shear and Bend are drag operations on the page.

### 3.13 Text / Choose Font (AGA p.76-77, p.201-203)
Title "Choose Font": scrolling **font list**, **Drawer** edit box (default `FONTS:`; type path + Return), **Size** edit box with up/down arrows (list limited to existing sizes; AGA: type any size to get a scaled screen font), **Bold / Italic / Underline** toggles, **Show** (load + sample), Cancel, **Ok**.
Colour fonts (<= 16 colours, require the ColorText helper) ask whether to use the font's palette. Karafonts are not shipped with AGA (AGA p.4).
Text tool: click to place, type in FG colour, auto-wraps at page edge (no word wrap), Backspace edits until you click elsewhere, Return new line, Esc or any tool ends text mode.

### 3.14 File requester: Load / Save / Delete (also brush, anim, AnimBrush, palette, color set, stencil) (AGA p.209-212, p.38)
File list with prefixes DEV / VOL / DIR (assigns shown as ASN; directories/volumes in blue), scroll arrows/box, **Drawer** edit box, **File** edit box (max 29 chars), **Pattern** edit box (`?`, `#?`, `#<pattern>`), buttons **Disks**, **Assigns**, **Parent**, **Load** / **Save** / **Delete**, **Cancel**.
Load adds **# of Frames**; Save shows frame range boxes when several frames exist. Double-click loads/saves/deletes. Disk swaps need Disks to refresh. Overwrite asks confirmation. Folders cannot be created.

### 3.15 Move (AGA p.252-255)
Title "Move". X / Y / Z columns; **Dist** row (pixels) and **Angle** row (degrees), each with a **Brush** toggle (tick = brush axes, default; no tick = screen axes, Euler order X,Y,Z; rotation about the brush handle);
**Clear**, **Go Back**, **Cyclic** toggle; **Ease-Out**, **Ease-In** (frame counts), **Count**; **Direction**: Move (Go From / Come To) and Record (Forward default / In Place / Backward; only with >1 frame);
**Preview** (wire frame; space interrupts), **Trails** (text only, >1 frame), **Fill**, **Draw**, **Cancel**, **Load**, **Save** (move definitions), **Exit** (keep settings). Keyboard "click": Amiga + Alt (AGA p.182).

### 3.16 Other animation requesters (AGA p.150, p.250-258)
**Set Frame Count** (Count, Cancel, Ok); **Add Frames**; **Copy Frames** (range to, Insert Before Frame); **Delete Frames**; **Set frames per second** (1-60, default 30); **Set Play Range** (From, To, Range button, Ok);
**Go to Frame** (frame number, Cancel, Ok); **Clear** (CLR with several frames: Current frame / Range / All Frames - AGA: left-click CLR clears at once, right-click or Shift-K opens it); **Pick up AnimBrush** (cel count); **Make AnimBrush** (cel count for Metamorph); **AnimBrush Settings** (2.4).

### 3.17 Print Picture (AGA p.232-234)
**Aspect** Normal (default) / Sideways; **Image** positive / negative; **Shade** Color / Gray / B&W; **Placement** left / centred; **% / Dots** size mode with **% Wide** (default 100) and **% High** (default 0 = keep screen aspect) or dot sizes; **Line Feeds**; **Copies**; **Form Feed**; **Cancel**, **Exit** (save settings only), **Print** (a Stop button appears while printing).

### 3.18 About (AGA p.241)
Version, authors, copyright, available memory (Chip, Fast, max block), anim and AnimBrush sizes.

### 3.19 Translucency (AGA p.277)
Slider (click either side or drag) + Ok; default 50 % = share of the original image that remains.

---

## 4. Tools (AGA p.204-226)

General: left button FG, right button BG. **Shift** constrains (horizontal/vertical for freehand, line, first leg of curve, airbrush; equal width/height for rectangle and brush pick-up). **Ctrl** while drawing line/curve/shape tools leaves "traces". Holding **I** while clicking a filled tool makes it fill then outline with the brush per the Spacing settings. Keys work while the mouse button is down (AGA p.82).

| Tool | Keys | Behaviour / sub-modes |
|---|---|---|
| Built-in brushes (10) | `-` smaller, `=` larger (and `_`, `+` 2 steps) | 4 round, 4 square, 2 scatter; resize 1x1 to 100x100; `.` = 1-px brush |
| Dotted Freehand | s | Stamps; gaps grow with mouse speed |
| Continuous Freehand | d | Unbroken line; Shift constrains |
| Filled Freehand | D | Releasing early closes the shape with a straight line; fill per Fill Type; I-click = filled + outline |
| Straight Line | v (V Spacing) | Drag; Shift horizontal/vertical; Ctrl traces |
| Curve | q | Drag endpoints, release, move to bend, click; Shift on first leg; Ctrl traces |
| Fill | f (F Fill Type) | Click inside enclosed area (spout pixel); Space aborts; I-click fills out to BG colour; Alt-click = animfill; left FG, right BG |
| Airbrush | - | Sprays with current brush, builds up while held; Shift constrains |
| Rectangle | r / R | Corner drag; Shift equal sides; Ctrl traces; filled uses Fill Type |
| Circle | c / C | Centre outward; Ctrl traces |
| Ellipse | e / E | Drag, move to shape, press and circle to rotate, release |
| Polygon | - | Click corners; click origin or Space closes filled polygon (Space on unfilled only lets go of the rubber band); Ctrl traces |
| Brush Selector | b new, B restore | Left-drag copies, right-drag cuts (BG fill left behind); double-click = polygon pick (Space closes); Shift = square; Alt = AnimBrush; BG (or common corner colour with AutoTransp) becomes transparent |
| Text | t, T font | see 3.13 |
| Grid | g, G | Restrict listed tools to grid points |
| Symmetry | / | All tools but Text/Brush Selector; freehand/airbrush draw copies live, others after release |
| Magnify | m | Right part of screen magnified; any tool works on both halves; arrows scroll, `n` recentres; click again to end; up to 400 % |
| Zoom | `>` / `<` | Enlarge / shrink magnification (also sets the selection rectangle size) |
| UNDO | u | Undo last action (including UNDO) provided no mouse click or refresh intervened |
| CLR | K, Shift-K | Clear to BG; with several frames use the Clear requester |
| Color Indicator | `,` | Eyedropper |
Fix Background makes right-button painting a true eraser (AGA p.83).

---

## 5. Paint modes (AGA p.247-249, p.60-62, p.81)

| Mode | Key | Pixel effect |
|---|---|---|
| Matte | F1 | Brush keeps its own colours; pixels equal to its recorded BG colour are transparent. Selected automatically after loading/picking a custom brush |
| Color | F2 | Brush silhouette filled with FG colour (transparent areas respected). Default mode |
| Replc | F3 | As Matte but nothing transparent - whole brush rectangle replaces pixels |
| Smear | F4 | Drags the colours under the brush; no new colours; brush colour ignored |
| Shade | F5 | Affects only pixels whose colour is in the current range: left button moves to the next colour of the range, right button to the previous |
| Blend | F6 | Like Smear but averages and picks the nearest palette colour (new shades appear) |
| Cycle | F7 | Cycles through the current range while drawing; if FG is in no range paints plain FG. MultiCycle: each brush colour cycles separately |
| Smooth | F8 | Softens edges with intermediate palette shades between adjacent colours |
| Mix | - | Brush colour mixes interactively with image colours, best in HAM (availability condition garbled, see section 12) |
| HBrite | - | EHB only: left button darkens first-32 colours to their half-bright twin, right button restores half-bright pixels |
Right button paints BG in Color mode; the right button only has a different meaning in Shade and HBrite (AGA p.247).
Modifiers (Effect menu): **Process** (Tint / Hue / Value), **Translucency**, **AntiAlias**, **Stencil**, **Background Fix**. Non-HAM modes always snap results to the nearest palette colour.

---

## 6. Keyboard equivalents (Appendix B, AGA p.292-296; read from page images)

Entries are exactly as printed; `i-c` etc. keep the printed glyph (see section 12).

### Brush commands (AGA p.292)
| Key | Action |
|---|---|
| F1 | Matte |
| F2 | Color |
| F3 | Replc |
| F4 | Smear |
| F5 | Shade |
| F6 | Blend |
| F7 | Cycle |
| F8 | Smooth |
| `-` | Brush smaller |
| `_` | Custom brush smaller by 2 steps |
| `=` | Brush larger |
| `+` | Custom brush larger by 2 steps |
| Alt-n | Copy custom brush to spare brush |
| Alt-b | Swap current and spare custom brush |
| Alt-m | Metamorphosis from spare to current brush |
| Z | Stretch |
| h | Halve |
| H | Double |
| X | Double horizontal |
| Y | Double vertical |
| x | Flip horizontal |
| y | Flip vertical |
| z | 90 degree rotate |
| o | Edge>Outline |
| O | Edge>Trim |
| Alt-s | Center brush handle |
| Alt-x | Flip brush handle horizontal |
| Alt-y | Flip brush handle vertical |
| Alt-z | Place brush handle |

### Toolbox commands (AGA p.293)
| Key | Action |
|---|---|
| b | Brush Selector |
| B | Restore last custom brush / AnimBrush |
| c | Unfilled Circle |
| C | Filled Circle |
| i-c | Filled and Outlined Circle |
| d | Continuous Freehand |
| D | Filled Freehand |
| i-d | Filled and Outlined Freehand Shape |
| e | Unfilled Ellipse |
| E | Filled Ellipse |
| i-e | Filled and Outlined Ellipse |
| f | Fill |
| F | Fill requester |
| g | Grid on/off |
| G | Grid on/off using brush handle as a grid point |
| K | CLR (clear screen) |
| m | Magnify on/off |
| p | Palette Mixer requester |
| P | Arrange Palette requester |
| q | Curve |
| r | Unfilled Rectangle |
| R | Filled Rectangle |
| i-r | Filled and Outlined Rectangle |
| s | Dotted Freehand |
| t | Text |
| T | Choose Font requester |
| u | Undo |
| v | Straight Line |
| V | Line Spacing requester |
| `>` | Increase magnification |
| `<` | Decrease magnification |
| `,` | PICK color cursor |
| `.` | One-pixel brush |
| `[` `]` | Change foreground colour |
| `{` `}` | Change background colour |
| `/` | Toggle Symmetry on/off |

### Special keys (AGA p.294)
| Key | Action |
|---|---|
| ? | About / memory info box |
| Delete | Cursor arrow on/off |
| F9 | Menu bar on/off |
| F10 | Toolbox and menu bar on/off |
| Shift-F10 | Hide all panels |
| Cursor keys | Scroll page (not in text mode) |
| Ctrl-Cursor keys | Adjust screen position (also moves Mixer / Arrange / Range panels when pointer is over them) |
| n | Centre area under cursor |
| Shift | Constrain cursor |
| Ctrl | Leave traces with line/shape tools |
| Ctrl-a | Show available memory in the menu bar |
| Tab | Color cycle on/off |
| S | Show page |
| Ctrl-s | Toggle Paint Stencil mode |
| ~ (tilde) | Make Stencil |
| ` (grave) | Stencil on/off |
| a | Again: repeat last menu command |
| `\|` (Shift-`\`) | Coords on/off |
| Spacebar | Cancel operation in progress |
| Esc | Stop operation in progress |
| j | Spare page |
| Ctrl-j | Copy to Spare |
| Alt-/ | Step through Antialias settings |
| Help | Select Freehand tool and single-pixel brush |
| Right Amiga-l | Load requester |
| Right Amiga-s | Save requester |
| Right Amiga-d | Delete requester |
Also stated in the body text: Right-Amiga-p Print (AGA p.232), Q Quit (AGA p.242), Shift-K Clear requester (AGA p.10), Alt-[ / Alt-] range stepping.

### Colour controls (AGA p.295)
| Key | Action |
|---|---|
| Ctrl-r | Range requester |
| Alt-r | Reverse direction of range |
| Alt-] / Alt-[ | Step forward / backward through ranges |
| Alt-t | Translucency on/off |
| Ctrl-t | Translucency Setting requester |
| Alt-p | Process on/off (Tint default) |
| p | Color Mixer requester |
| P | Arrange Palette requester |

### Perspective commands (AGA p.295)
| Key | Action |
|---|---|
| Enter (keypad) | Enter / exit perspective mode |
| Keypad 7 / 8 | Rotate about X (Shift: by Angle Step) |
| Keypad 9 | Reset X axis to 0 degrees |
| Keypad 4 / 5 | Rotate about Y (Shift: by Angle Step) |
| Keypad 6 | Reset Y axis |
| Keypad 1 / 2 | Rotate about Z (Shift: by Angle Step) |
| Keypad 3 | Reset Z axis |
| Keypad 0 | Reset all axes |
| Shift Keypad 0 | Reset all axes, positions and settings |
| Shift Keypad 9 / 6 / 3 | Fix X / Y / Z axis |
| Keypad - | Fill screen with current brush at current perspective |
| Keypad . | Reset (place) centre |
| `;` and `'` | Move brush along its fixed axis (perpendicular to its plane) |
| Shift `;` `'` | Same, larger increment |
| `<` / `>` | Modify observer distance |
| Ctrl | Temporarily fix Y axis (mouse moves brush on X and Z) |
| `\` | Toggle Angle / Position display |

### Animation keys (AGA p.296)
| Key | Action |
|---|---|
| 1 / 2 | Previous / next frame |
| 3 | Go To Frame requester |
| 4 | Play continuously until Space or mouse button |
| 5 | Play once |
| 6 | Ping-pong |
| 7 / 8 | Previous / next AnimBrush cel |
| Shift-1 / Shift-2 | First / last frame |
| Shift-3 | Last frame you went to |
| Shift-4 / Shift-5 | Play continuously / once in reverse |
| Shift-7 / Shift-8 | First / last AnimBrush cel |
| Space bar | Stop playing |
| r | Reverse while playing |
| Left / Right Arrow | Slower / faster playback |
| Alt (held) | Animpainting while painting |
| M | Move requester |
| Alt-a | Anim Control Panel on/off |
| l | LightTable on/off |
| Alt-l | LightTable Dim on/off |
| Ctrl-1 (main keyboard) | LightTable 2 back on/off |
| Ctrl-2 / Ctrl-3 / Ctrl-4 | LightTable Previous / Next / Spare page |
| Alt-= | Add one frame |
Total about 150 printed entries. Player utility keys (AGA p.297): Tab cycling, Left/Right arrow rate, r reverse, Esc/Space stop, 1, 2, 4, 5, 6.

---

## 7. Screen modes / AGA specifics

### 7.1 Base (4.0) formats (AGA p.73, p.237-240; identical in DP4)
| Format | Resolution | Max colours | Notes |
|---|---|---|---|
| Lo-Res | 320x200 | 32 (64 EHB; HAM 4096) | default |
| Med-Res | 640x200 | 16 | tall narrow pixels; use Double Horiz/Vert |
| Interlace | 320x400 | 32 (64 EHB; HAM) | flicker |
| Hi-Res | 640x400 | 16 | interlaced, Lo-Res pixel shape |
Fewer colours allowed (powers of two). **EHB**: 64 colours, first 32 editable, second 32 half-intensity copies. **HAM (4.0)**: 16 registers, 4096 colours, each pixel modifies one RGB component of its left neighbour ("ramping", temporary "fringing" while a brush moves; Color Set sets A-P). Overscan Off / Std / Max, NTSC / PAL; page sizes 320x200 up to 1008 x 1024.

### 7.2 AGA additions (AGA p.4-12)
* AGA chipset: 8 bits per colour gun, 2-256 register modes, 320x200 up to 1280x400 plus overscan (AGA p.5).
* **HAM8**: 64 register palette, over 262,000 colours; default HAM in AGA; 16-colour palette selects HAM6. HAM in all resolutions (4.0: only 320x200 / 320x400) (AGA p.5-6).
* 256-colour Color Set (RGB8, more than 16 million reachable); **Scale** button RGB4 -> RGB8.
* New **Choose Display Mode** requester (3.2); modes depend on Monitors drawer; best flexibility with OS 3.0, multiscan monitor, overscan preference at maximum.
* Reads 24-bit IFF (converted to current mode, never written), reads PBM; 4.0 already loads 256-colour ILBM (into 64 colours or HAM) (AGA p.29). Writes 8-bit images and animations.
* 256-colour AnimBrushes/animations with size limit (width/8) x (height-1) <= 65,535 (AGA p.11).
* Clipboard brush cut/paste, Brush>Free, AppIcon / AppMenu, IFF parse library, shell options.
* Be Square default on and widened; requester keyboard navigation; CLR left/right behaviour; stencil left-select/right-deselect.
* HAM ramps from black so black is excluded from cycling in HAM.
* "Rub through" technique (spare page + LightTable + Translucency).
* Max/video overscan needs the Workbench overscan preference raised (AGA p.4).
* "Retain picture" is not mentioned anywhere in the manual (searched); "Keep Same" page size is the nearest concept.

---

## 8. Colour cycling and ranges (AGA p.259-262, p.73, p.85-88)

* Up to **8 ranges** per picture, each up to **32 colours** (AGA p.259, p.73).
* Placing colours: pick from colour row (becomes bead cursor), click on the bar; click an existing bead then another position to move; click off the bar to remove; CLEAR empties the range; leftmost = start colour; position order sets cycle and gradient direction; REVERSE (Alt-r) flips it. Intermediate shades are interpolated.
* Beads with a dot underneath are palette registers; beads without are free RGB values (pick with `,` from the gradient). Free colours allow cycling a single register through many colours.
* Three cycling styles: conventional palette cycling; cycling with non-palette colours; one register through a range (AGA p.28, p.105-106).
* **RATE**: slider, abstract scale 0-63 with non-linear steps; the shown number only helps match ranges; cycling is visible while dragging even if cycling is off. No fps unit.
* Tab toggles cycling; a colour in several ranges selects the first; colours not in a range never cycle. Alt-[ / Alt-] step the current range.
* **RANDOM** + **DITHER**: random (instead of patterned) mixing at colour borders, strength by slider; dither only active when Random is on.
* Copy a range: show source, drag slider to target number, click Undo with no clicks between.
* Non-HAM: gradients use only palette colours (nearest match; leave no empty notches between beads). HAM shows full-colour gradients (dither visible only in Fill Type preview).
* A strip showing the true gradient above the requester existed in 4.0 and was removed in AGA (AGA p.12).
* Ranges drive color cycling, gradient fills (Fill Type range number), Shade and Cycle modes. Brush files store their cycle info.

---

## 9. Animation (AGA p.84-85, p.127-180 chapters, p.249-259, p.297-300)

* **Frames**: Anim>Frames (Set #, Add, Copy, Delete), panel +/-, Alt-=. Counter "n/m" in Title Bar. Spare page = Scratch.
* **Control Panel** (Alt-a): frame scroll bar with First / Last Frame end buttons plus 15 buttons (figure AGA p.257): play once backwards, loop backwards, previous frame, next frame, play once forward, loop forward, ping-pong, add frame, delete frame, Go To (Ctrl-click opens requester), LightTable, "2" two back, "P" previous, "N" next, "S" spare page. Button order inferred from figure.
* **Playback**: default 30 fps (1-60); Set Range limits Play once and Ping-pong; Space stops, `r` reverses, arrows change speed.
* **Animpainting**: hold Alt when pressing the mouse button; frames advance per stamp; N Total spacing gives exact stamp counts; Alt-click Fill = animfill.
* **AnimBrush**: Alt + Brush Selector or Anim>AnimBrush>Pick up; Settings (Duration, Current, Direction); keys 7/8; Use, Free, Load, Save; Metamorph builds one from spare+current brushes.
* **LightTable** (l): previous/next/two-back/spare images behind current; Dim; Merge. Key-frame workflow.
* **Move requester** (M): 3D path over Count frames; Go From/Come To; Forward/In Place/Backward; Cyclic; Ease-In/Out; Preview; Trails; Fill; Load/Save of moves.
* **Method**: Compressed (differences only: more frames, slower, fragmentation prone) vs Expanded (full bitmaps, smooth flipping; e.g. 20 frames of 320x200x32 colours need about 800 KB) (AGA p.258-259, p.290).
* **Saving**: Anim>Save with from/to saves an animation; Picture>Save with frame range writes a numbered picture series. The manual **does not name** disk compression methods (ANIM opcodes etc.); only that compressed-model disk size depends on frame-to-frame change (AGA p.290).
* Frame series loaded via the # of Frames box take the first picture's palette; load one at a time to append without discarding.
* **Player** utility (Appendix C): single menu "Animate" (Load, Play, About, Quit), CLI form `player path [seconds | N loops]`, `@script` files; AGA version plays 256-colour and HAM8; non-commercial redistribution only.

---

## 10. File formats and saving options mentioned

| Item | Details | Page |
|---|---|---|
| Pictures | IFF; saved with palette, Color Set, ranges, stencil, perspective info; sample suffixes `.LoRes`, `.Ham`; 256-colour ILBM loadable | AGA p.29, 209-211, 230-231 |
| Frame series | Save with frame range -> one picture per frame, 3-digit number appended | AGA p.231 |
| 24-bit IFF | read only | AGA p.7 |
| PBM | read only (PC DeluxePaint) | AGA p.7 |
| Brushes | saved with palette + cycling; clipboard cut/paste in AGA | AGA p.223, 242, 7 |
| AnimBrush | separate load/save; sample `.anbr` | AGA p.84, 249 |
| Animation | ANIM (sample `.ANIM`); load replaces or appends | AGA p.229, 251-252 |
| Palette | base-register colours only | AGA p.268 |
| Color Set | 256 colours, selective load Add/Overwrite | AGA p.268-269 |
| Stencil | separate layer, area-based, screen size | AGA p.273 |
| Move | Load/Save of move definitions (format not described) | AGA p.27, 255 |
| Fonts | Amiga bitmap fonts, scalable (AGA), ColorFonts <= 16 colours | AGA p.76, 202, 222 |
| Icons | `.info` files unless Prefs>No Icons | AGA p.265 |
| Print | via Preferences printer driver | AGA p.232 |
File name limit 29 chars (AGA p.231).

---

## 11. Differences 4.0 vs 4.5 AGA

The manual bodies are identical; deltas come from the AGA addendum.

| Area | 4.0 | AGA |
|---|---|---|
| Chipset / colours | OCS/ECS: 32 colours, EHB 64, HAM6 4096 | AGA: 2-256 colours, 8-bit RGB, HAM8 (>262k) |
| HAM | HAM6 at 320x200 / 320x400 | HAM8 default (64 palette) or HAM6 (16), all resolutions |
| Colour set | 256 slots RGB4 | 256 slots RGB8, Scale button |
| Screen format | Fixed Format/Colors/Overscan/NTSC-PAL requester | Choose Display Mode list, mode text gadget with patterns (`^`, `~`), Screen Size / Page Size cycles, info panel |
| Overscan | Off / Std / Max buttons | Dimensions from Preferences; Workbench overscan must be raised |
| File input | IFF, 256-colour ILBM | + 24-bit IFF read, PBM read, 8-bit images/anims written |
| Brush | morph limit about 250 px | memory-limited morphs, Alt-b reverses morph order; clipboard cut/paste; Brush>Free |
| Be Square | off by default | on by default, applies to built-ins, circle, rectangle |
| Brush handle keys | Alt-s/x/y/z | Alt-x cycles five positions (addendum; Appendix B not updated) |
| Requesters | mouse only | keyboard navigation, Right-Amiga-X clears field |
| CLR | opens options with several frames | left-click clears, right-click/Shift-K opens Clear requester |
| Stencil colours | click toggles | left select / right deselect |
| Range requester | gradient preview strip above | removed |
| Animation | up to 64 colours / HAM6 | 256-colour AnimBrushes/anims (size formula) |
| Fonts | bitmap + ColorFonts, Karafonts supplied | + scalable fonts; Karafonts not supplied |
| System | - | AppIcon/AppMenu, IFF parse library, shell options, `LOAD=` |
| Player | Art2 disk, 4.0 modes | program disk, 256-colour/HAM8 |
| Misc | - | ToolLimit tool type, fixed stencil with animations, rub-through tip |
The About figure OCR in the DP4 file reads "Version 4.6"; neither manual states a version number reliably (AGA p.241).
Introduced in 4.0 vs DeluxePaint III (What's New, AGA p.26-29): HAM, Control Panel, Anim frame commands + move save/load, LightTable, Metamorphosis, Color Mixer, palette/Color Set files, Ranges, gradient fills, antialias, Process and Translucency, extra cycling styles, Stencil>Paint, overscan painting, FastAdjust, Mix mode, better file requesters.

---

## 12. Unclear / OCR-damaged

* Toolbox figure: arrangement of the 10 built-in brush icons and pixel sizes are not legible (AGA p.204).
* The boxed key glyph read as "i" (AGA p.210, 215-218): treated as the I key. Appendix B prints `i-c`, `i-d`, `i-e`, `i-r`; whether that glyph is I, Amiga or Alt cannot be confirmed.
* Appendix B `_` / `+` ("by 2 steps") versus body text where `-`/`+` resize: shifted forms as printed.
* Brush handle: Appendix B (Alt-s/x/y/z) vs addendum (Alt-x cycles five positions) unreconciled.
* Mix mode availability sentence is truncated across a page break and seems to repeat the HBrite condition (AGA p.248-249).
* Process submenu: Tint/Hue/Value/On-Off confirmed; "None" implied by text only.
* Page Size preset numbers: NTSC Lo-Res values legible at 400 dpi (320x340 Full Page, 368x241 Overscan); PAL values not stated.
* Defaults not legible: Symmetry Order and Tile size, Move requester Count/Ease values, Spacing numbers. Stated: Translucency 50 %, Angle Step 90 degrees, Set Rate 30 fps, Print % Wide 100 / % High 0.
* AGA Choose Display Mode requester: only a tiny screenshot (AGA p.6); control list inferred from text.
* Move requester "Trails" is described but not visible in the figure; ease-in/out wording in the OCR may be swapped.
* Anim Control Panel button order inferred from figure labels.
* Sub-order of Stencil / Palette / Color Set Load/Save items taken from text order, not menu images.
* Clipboard (Cut/Paste/Copy) and Brush>Free menu positions/shortcuts not documented.
* Disk formats: no ILBM chunk list, no ANIM compression opcode names, no Move/Stencil/Color Set file layout.
* OCR garbage: AGA p.239-240 and p.302-306 figures (HAM and bit-plane diagrams), AGA p.230 title lines; figure text in AGA OCR is much poorer than in the DP4 OCR.
