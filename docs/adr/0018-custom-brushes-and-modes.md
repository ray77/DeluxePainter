# ADR-18: Custom brushes and drawing modes in the painting code

- Status: accepted (Phase 3, 2026-11-10)
- Context: The drawing tools (ADR-14) stamped only the built-in brushes, and the Mode menu changed nothing but the title bar. The shared foundation (`dpx_core::paint`: `BrushImage`, `DrawMode`, `PaintContext`, `plot`) was added for wave 2. This step makes custom brushes work with every tool and sends every painted pixel through `plot`. The mode algorithms inside `plot` (Smear, Shade, …), the Stencil requester, the Brush menu and gradient fills are separate work. Behaviour comes from `docs/research/manual-ui-notes.md` §1.5, §4 and §5.
- Decision:
  - **Commands.** `stroke` and `shape` gain five optional fields: `custom` (a `BrushImage`), `mode` (`matte` by default), `range` (`[low, high]`), `stencil` (256 booleans) and `right`. Unset fields are left out of the JSON, so old command files and their output stay the same. `stroke.brush` may now be omitted when `custom` is set.
  - **Validation.** The brush must be 1×1 up to the target's largest page, with exactly `width × height` pixels. Its colours and transparent colour must be inside the palette, and the handle within ±8192. The range must have `low ≤ high < palette size`. The stencil must have 256 entries.
  - **One paint path.** `raster::Pen` holds the brush, colour, mode and a `PaintContext`. `raster::Stamper` follows a path and gives each stamp `motion` = the step from the previous stamp of the same path (or of the same symmetric copy).
    - Every stamp of freehand, line, curve, shape outlines, dots (airbrush, symmetric freehand, text) and every pixel of a filled shape goes through `paint::plot`. Each filled pixel is plotted exactly once.
    - `before` is the page before the command. A gesture previewed in parts passes the page before the gesture.
    - The new signature is `Command::paint(img, before, palette)`. `Command::pen(before, palette)` builds the pen. `Command::apply` clones the page as `before`.
  - **Symmetric copies** are visited point by point: point 1 of every copy, then point 2, and so on. Painting a shape in parts therefore gives the same stamps in the same order as the whole shape.
  - **Pixel colour.** `color` is FG, or BG with the right button.

    | | Built-in brush | Custom brush, left button | Custom brush, right button |
    |---|---|---|---|
    | Matte (and the other modes) | `color` | own colours, transparent pixels skipped | shape in BG |
    | Color | `color` | shape in FG | shape in BG |
    | Replace | `color` | every pixel in its own colour | whole rectangle in BG |

    Filled shapes are always solid `color` and never use the custom brush.
  - **Preview = result.** In Matte, Color and Replace a freehand stroke or spray is painted part by part on top of the preview. In the other modes the whole preview is painted again for each new part, because those modes depend on the order and motion of every stamp.
  - **Brush Selector** (`b`):
    - Dragging shows a frame in each pixel's complementary colour (`colours − 1 − c`, i.e. XOR on a power-of-two palette).
    - Release picks up the rectangle (corners inclusive, clipped to the page) with the handle in the centre. The transparent colour is BG; with Prefs ▸ AutoTransp it is the colour all four corners share. With Grid and Prefs ▸ ExclBrush the right and bottom border is left out.
    - After the pick-up the mode becomes Matte and the tool selected before the Brush Selector comes back (Continuous Freehand if there is none).
    - Right-button drag cuts: the same pick-up plus a filled BG rectangle as one command, so one undo step. Shift = square. Grid snaps the corners. Symmetry does not apply.
  - **Brushes.**
    - `B` restores the brush last picked up (`picked_brush`). A right-click on the Brush Selector toggles between that brush and the built-in brush. Clicking a built-in brush drops the custom brush.
    - While a custom brush is the brush, no built-in brush is shown selected (`toolbox::draw_brush_unselected`).
    - A custom brush with colours the palette no longer has (after Screen Format) is not used.
  - **Pointer preview.** While a tool that stamps the brush is selected and no gesture is running, the custom brush is drawn at the (grid-snapped) pointer into the page preview (`Kind::Hover`), in the current mode. It is never committed. Brush-stamping tools are freehand, line, curve, outline shapes and airbrush.
  - **Mode wiring.**
    - `PaintMode` maps one to one to `DrawMode`.
    - `range` is the first loaded colour range (CRNG) that holds FG and lies inside the palette, else none.
    - `stencil` is `PainterApp::stencil`.
    - The 1-pixel feedback outline of filled freehand shapes and filled polygons stays plain: Matte, no stencil.
  - **Snapshots.** `--pick X,Y,W,H` picks up a brush after `--commands` (colour 0 transparent). `--brush-commands FILE` then runs commands. In those commands every stroke and outline without its own `custom` brush uses the picked brush.
- Deviations from DeluxePaint IV:
  - **Right button with a custom brush.** The notes do not say what Matte and Replace do. We paint the shape (Replace: the rectangle) in BG.
  - **Replace** paints transparent pixels in their own colour, which is the BG colour at pick-up time, not the current BG.
  - **Not done:** polygon or lasso pick-up (double-click in DeluxePaint; our toolbox has no lasso variant), AnimBrush pick-up (Alt), handle keys, Brush menu items. The cut ignores the stencil. Flood fill does not go through `plot` yet.
  - **Not checked against DeluxePaint:** the exact look of the frame, the pointer preview (a page preview rather than a sprite), the built-in brush losing its highlight, and the fallback tool after a pick-up.
- Verified (`cargo test --workspace`, headless snapshot):
  - Core: Matte, Color and Replace with both buttons; transparency; the stencil on stamps, outlines and fills; a custom brush painted in parts (dots and open polygon, 5-fold symmetry) equals the whole shape; bounds cover every stamp; a custom-brush stroke is one undo step and replays; validation errors; old JSON gives the same commands and serialises unchanged; new fields round-trip.
  - App: the Brush Selector gesture (frame while dragging, nothing committed, brush size, handle and transparency, Matte, back to the previous tool, Shift square); right-drag cut is exactly one undo step; AutoTransp; stamping in all three modes with freehand and line (preview = result); the pointer preview is shown, moves, hides off the page and is never committed; a built-in brush click drops the custom brush; `B` and the right-click toggle; mode, range, stencil and `right` reach the commands; `--brush-commands`.
  - `docs/design/screenshots/phase3-brushes.png` was rendered with the options in README.md and looked at.
  - Not verified: the running GUI by hand (mouse, real pointer movement), performance with very large brushes.
