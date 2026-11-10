# ADR-14: The drawing tools

- Status: accepted (Phase 3, 2026-11-10)
- Context: The toolbox showed every DeluxePaint IV tool, but only Dotted and Continuous Freehand painted. Owner directive: speed and visible results, DeluxePaint IV AGA behaviour where the manual describes it, no questions; deviations are listed here. Tool behaviour comes from `docs/research/manual-ui-notes.md` §4 and §6 (AGA manual p.204–226, keyboard table p.293).
- Decision:
  - **Commands (dpx-core):**
    - `shape` = `{shape, brush, color, filled, symmetry?}`. Shape kinds: `line {from,to}`, `curve {from,to,through}`, `rect {from,to}`, `ellipse {center,radius:[rx,ry]}` (equal radii = circle), `polygon {points,closed}`, `dots {points}`.
    - `flood_fill` = `{x, y, color}`.
    - Outline: the brush is stamped on every pixel of the outline path. The path's points are joined with Bresenham lines; `closed` also joins the last point to the first.
    - Filled: a solid shape in the colour without the brush. The fill is even-odd, sampled at pixel centres, and also sets every edge pixel, so a 1-pixel outline always lies inside the filled shape.
    - `dots` stamps the brush once per point (airbrush, symmetric dotted freehand, text). `brush` may be omitted for filled shapes.
    - Limits: coordinates and radii within ±16384, at most 100 000 points, symmetry order 1–64, pixel aspect 1–4.
    - Undo footprint: the bounding rect of all copies, plus the brush for outlines. For `flood_fill` it is the bounding rect of the filled region, found by filling a copy first.
    - `Command::paint(&mut IndexedImage)` draws `stroke`, `shape` and `flood_fill` exactly as the engine applies them; `Command::paint_bounds` gives the clipped rect.
    - Freehand without symmetry still commits the existing `stroke` command.
  - **Rasterizers (`raster.rs`):**
    - Ellipse: the midpoint algorithm, returned as an ordered, 8-connected ring. A zero radius gives a line.
    - Curve: a quadratic curve that passes through the bend point halfway (control point = 2·bend − (from+to)/2). It is sampled at twice the path length, and staircase corners are thinned so the line stays one pixel wide. The bend point is never thinned.
    - Polygon fill as above. Flood fill: 4-connected scanline fill.
    - `magnified` enlarges part of the page for the Magnify view.
  - **Preview = result:** the gesture in progress keeps a copy of the page with its candidate command painted by `Command::paint`.
    - Freehand and airbrush paint each new part on top. Painting in parts gives the same pixels as the whole command.
    - Other tools repaint the copy from the document inside the previous candidate's rect, then paint the new candidate.
    - Filled freehand and filled polygons show a 1-pixel outline while being drawn and fill when closed, as in DeluxePaint. A polygon's rubber band is dropped on commit.
    - Every finished shape is one command and one undo step.
  - **Tools** (left button = FG, right = BG, the selected built-in brush for outlines):

    | Tool | Behaviour |
    |---|---|
    | Line | Drag. Shift = horizontal or vertical. |
    | Curve | Drag the ends, release, move to bend (the curve passes through the pointer), click to fix. A click without a drag draws nothing. Shift constrains the first leg. |
    | Rectangle | Corner drag. Shift = square. |
    | Circle | Drag from the centre outward. |
    | Ellipse | Drag from the centre; the radii are the distances along x and y. Rotation is not implemented. |
    | Polygon | Each click adds a corner; press-drag-release adds the release point too. Clicking within 2 pixels of the first corner closes the polygon. Space/Return: filled polygons close; unfilled ones keep the open outline ("lets go of the rubber band", manual). Esc cancels. |
    | Filled Freehand | Lower-right half of the freehand button, or `D`. Releasing closes the outline with a straight line and fills it. |
    | Fill | Click: 4-connected flood fill of the clicked colour. |
    | Airbrush | While held, 4 brush stamps per frame at random points in a disc of radius 10. The random numbers come from an xorshift generator in the app; the command stores the sprayed points, so undo and replay are deterministic. |
    | Text | Click places the text, typing draws the UI bitmap font in FG. Return starts a new line and Backspace deletes. Esc, a click elsewhere or another tool commits the pixels as one `dots` command. Shortcut letters are typed, not run, while text is active. No auto-wrap at the page edge and no Choose Font yet. |

  - **Keys (manual keyboard table):** `s` Dotted Freehand, `d`/`D` Continuous/Filled Freehand, `v` Line, `q` Curve, `f` Fill, `r`/`R`, `c`/`C`, `e`/`E` outline/filled Rectangle, Circle, Ellipse, `t` Text, `g` (and `G`) Grid, `/` Symmetry, `m` Magnify, `>`/`<` Zoom, `K` CLR. Polygon and Airbrush have no key. `G` does not use the brush handle yet.
  - **Grid:** 16×16 page pixels from the page origin; points snap to the nearest grid point. As in the manual, only Dotted Freehand, Line, Rectangle, Circle, Ellipse and Text obey it. No Gridding requester.
  - **Symmetry:** cyclic, order 6, around the screen centre at the moment the gesture starts.
    - Each copy rotates the outline path (rounded to pixels) and then draws it, so filled copies have no holes. The brush itself is not rotated.
    - Freehand and airbrush copies appear while painting, shapes as their rubber band.
    - Not implemented: Text and Fill ignore symmetry; Mirror, Tile, Place and the Symmetry requester are missing.
  - **Be Square (Prefs, on by default):** uses the mode's displayed pixel shape (`DisplayMode::pixel_shape`, e.g. HiRes = 1:2). It squares circles (the radius is taken from the displayed distance, so a HiRes circle has half as many rows), Shift-squares and Shift-lines, symmetry rotation and the airbrush disc. Built-in brushes are not squared yet.
  - **Magnify (split screen, like DeluxePaint):**
    - Layout: the normal view on the left (a third of the paint area), a 3-UI-pixel divider, and the magnified view on the right up to the toolbox.
    - Factors: 2, 3, 4, 6, 8, 12, 16, 20. The manual says "up to 400 times", i.e. 20×20. The default is 4.
    - `m` magnifies around the pointer. Arrows move the magnified part by a quarter of its width, and `n` re-centres it on the pointer. The normal half scrolls to keep it in view.
    - Zoom: click or `>` enlarges; right-click or `<` shrinks.
    - Every tool works in both halves; the divider is not part of the page.
    - Deviations: the toolbox button magnifies around the screen centre (DeluxePaint first shows a box to place). While magnified, the mouse wheel and Shift+S are inactive. On a page no larger than the screen the normal half cannot scroll, so it shows the left third of the page even when the magnified spot lies further right.
  - **Toggles:** Grid, Symmetry and Magnify are drawn inverted while on (`toolbox::draw_toggle_on`), and the drawing tool stays selected. A toolbox click or tool key commits a pending polygon, curve or text first.
  - **`--commands FILE.json`** (snapshot option): a JSON array of commands run through the engine before `--snapshot` renders. Demo: `docs/design/screenshots/phase3-tools.json` → `phase3-tools.png`.
  - **Not implemented** (later, mostly need requesters): Ctrl traces, `i`-click filled + outlined, Shift on freehand and airbrush, the Spacing and Fill Type requesters (fills are solid), the airbrush size drag, ellipse rotation, Brush Selector, eyedropper.
- Verified:
  - dpx-core: pixel-exact tests for lines, outline and filled rectangles, circles of radius 1 and 3, a 4×2 ellipse, zero-radius ellipses, open, closed, filled and even-odd polygons, the curve through its bend point, 4-connected flood fill, symmetry copies (square and HiRes pixels), JSON shape, and the magnified view. Every case also checks that `paint_bounds` covers every changed pixel.
  - Engine tests: every shape kind equals its `paint` preview, is one undo step and redoes exactly; flood fill snapshots only its region; invalid shapes change nothing.
  - App flow tests (press/drag/release → one command → undo restores, redo repeats): line with Shift, filled rectangle/circle/ellipse, Be Square on HiRes, polygon by click and by Space, curve bend, filled freehand, fill and grid snapping, airbrush, symmetry, text typing, tool keys and Zoom right-click, and Magnify mapping, compose and painting in both halves. Also `--commands` with a bad command and bad JSON.
  - Toggle drawing test in dpx-ui.
  - Headless snapshot `docs/design/screenshots/phase3-tools.png`, checked by eye. The GUI starts.
  - Not tried by hand in the window: the drag feel, airbrush rate, typing, Magnify scrolling, HiRes Be Square.
- Consequences:
  - Fill types, spacing and brush pick-up can extend the same `shape` command (e.g. a fill-type field) without changing the preview mechanism.
  - Symmetry and grid settings move into requesters later; the commands already carry the order, centre and aspect.
