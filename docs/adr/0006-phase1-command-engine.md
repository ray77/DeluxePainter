# ADR-6: Phase 1 implementation of the command engine

- Status: accepted (Phase 1, 2026-11-10)
- Context: ADR-2 fixes one serialisable `Command` API for GUI, CLI and MCP. Phase 1 has a single indexed page and a palette, no layers or frames yet, and no tiled buffers.
- Decision:
  - `dpx_core::Engine` owns the `Document` (page + palette + revision). All changes go through `Engine::execute(Request { command, expected_revision, dry_run })`, `undo()` and `redo()`.
  - Commands so far: `Stroke` (dotted/continuous freehand with a pixel brush), `Clear`, `Flip`, `ReplaceColor`, `SwapColors`, `SetPalette` (same colour count). They are `serde` types with a stable JSON shape (`{"op":"flip","axis":"horizontal"}`), ready to become MCP tool parameters.
  - Validation happens before any mutation; errors carry stable codes (`INVALID_ARGUMENT`, `PALETTE_RANGE_INVALID`, `REVISION_CONFLICT`, `NOTHING_TO_UNDO`, `NOTHING_TO_REDO`). Size limits guard against absurd input (100 000 stroke points, 100×100 brush pixels, coordinates within ±1 000 000).
  - Undo stores, per step, a copy of the affected page rectangle (clipped stroke bounds or the whole page) or the previous palette. Redo replays the undone command, which is valid because commands are deterministic and the document is back in the state the command was validated against. The history keeps 64 steps; a new command clears the redo list.
  - The revision increases by one for every execute, undo and redo. A dry run validates and reports the affected area without changing the revision.
  - Freehand painting is previewed by the GUI on a copy of the page with the same `raster::stroke` function the engine uses, and committed as one `Stroke` command on button release. Stamping a point twice changes nothing, so preview and committed result are identical (tested).
  - GUI: `U` and a left click on UNDO undo, `Shift+U` and a right click on UNDO redo. DeluxePaint itself has a single undo level; history and redo are DeluxePainter additions (FEATURES.md, "Undo / redo (multi-level)").
- Not yet: transactions (`begin/commit/rollback`), copy-on-write tiles, document ids. They arrive with the MCP work and with layers/frames; the public `Request`/`Outcome` shapes are meant to stay.
- Consequences: menu commands BG -> FG and BG <-> FG are undoable (documented deviation). Every future tool adds a `Command` variant plus tests in `dpx-core`; the GUI only builds commands and previews.
