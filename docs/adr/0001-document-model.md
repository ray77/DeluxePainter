# ADR-1: Indexed, tiled, layered document model independent of file formats

- Status: accepted (Phase 0)
- Context: DPaint works on one indexed bitmap per frame; users also want Photoshop-like layers. IFF must not dictate the in-memory model.
- Decision: `Document` = palette + hardware target + layer tree + frames + cels `(layer, frame) → PixelBuffer`. Buffers are `Indexed` (u8 + transparency mask) or `Rgba8`; stored as 64×64 copy-on-write tiles. One palette per document in V1; per-frame palette override reserved.
- Consequences: palette edits/colour cycling stay live; frame duplication and undo snapshots are cheap; export must composite and quantise explicitly.
