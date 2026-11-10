# Architecture

Status: Phase 0 draft, 2026-11-09. Decisions marked **ADR-n** are recorded in `docs/adr/`.

## 1. Goals that shape the architecture

1. One engine, many front-ends: the GUI, the CLI and the MCP server all mutate documents **only** through the same validated `Command` API.
2. Pixel-exact, deterministic results: the same command sequence produces byte-identical pixels and files on every platform.
3. The classic UI is pixel art, not a skinned widget toolkit: rendered by our own code into a low-resolution framebuffer, shown with nearest-neighbour scaling at integer factors.
4. Editable project (`.dpxproj`) ≠ hardware output (IFF ILBM/ANIM, PNG). Export is a separate, checked conversion.
5. Core crates compile and test without a GPU or window system.

## 2. Workspace layout

```text
crates/
  dpx-core      Document model, HardwareTarget, Palette, Layers/Frames/Cels,
                Command enum, Engine (execute/undo/redo/transactions), errors
  dpx-raster    Pure pixel algorithms on index buffers: line, ellipse, polygon,
                flood fill, brush stamping, spacing, symmetry, gradients, dither
                (until it is split out: `dpx_core::raster`; colour reduction and
                resampling of true-colour pictures: `dpx_core::quantize`/`resample`)
  dpx-iff       IFF container, ILBM (BMHD/CMAP/CAMG/BODY/CRNG/CCRT/GRAB…),
                ByteRun1, EHB/HAM6/HAM8 codecs, later ANIM
  dpx-formats   PNG import/export, JPEG import (decoders only), ExportAnalyzer
  dpx-project   .dpxproj read/write/migrate (ZIP + JSON manifest + zstd blobs)
  dpx-render    Compositing layers → indexed/RGBA frame, colour-cycling preview
  dpx-ui        RetroUi: framebuffer, bitmap font, widgets, toolbox, menus,
                requesters; tool interaction state machines; eframe adapter
  dpx-mcp       MCP server (rmcp), IPC bridge to a running GUI, tool schemas
apps/
  deluxepainter Single binary: GUI by default; `deluxepainter mcp` = MCP stdio
                server; `deluxepainter cli …` = headless inspect/convert
```

Dependency direction (no cycles): `core ← raster ← render ← ui ← app`, `core ← iff/formats/project ← app`, `core ← mcp ← app`. `dpx-ui` never touches files directly; it issues commands.

One binary instead of a separate MCP binary (ADR-5): simpler install, one version, and the MCP client config just runs `deluxepainter mcp`.

## 3. Document model (ADR-1)

```rust
struct Document {
    id: DocumentId,               // UUID
    revision: u64,                // +1 per committed command/transaction
    canvas: CanvasSpec,           // width, height, pixel aspect
    target: HardwareTarget,       // chipset, tv standard, resolution class,
                                  // interlace, bitplanes, encoding (Indexed/Ehb/Ham6/Ham8)
    palette: Palette,             // up to 256 entries, RGB8 + original RGB4 if imported
    color_cycles: Vec<ColorCycle>,// range, rate, direction, active
    layers: LayerTree,            // groups + layers (name, visible, locked, opacity, blend)
    frames: Vec<Frame>,           // duration
    cels: CelStore,               // (LayerId, FrameId) -> PixelBuffer
    metadata: ProjectMetadata,    // import provenance, preserved IFF chunks, UI state
}
```

- **Palette is per document** in V1 (DPaint behaviour). Per-frame palettes are an ANIM-import concern and are deferred; the data model reserves `Frame::palette_override: Option<PaletteId>`.
- **Pixel buffers** are either `Indexed` (u8 index + 1-bit transparency mask) or `Rgba8` (imported references). Indexed is the normal painting format so palette edits and colour cycling stay live.
- Buffers are stored as **64×64 tiles behind `Arc`** (copy-on-write). Duplicating a frame or snapshotting for undo copies pointers, not pixels.
- HAM/EHB: the editable data is always indices + palette. HAM is an *output encoding*; a HAM import keeps the decoded RGB as an RGBA layer plus the original raw planes in `metadata` for lossless re-export (ADR-4).

## 4. Command engine (ADR-2)

Phase 1 status: implemented for a single page + palette; see ADR-6 for what exists today (snapshot undo, redo by replay, no transactions or tiles yet).

```rust
enum Command {
    NewDocument{..}, SetHardwareTarget{..}, ResizeCanvas{..},
    SetPixel{..}, DrawLine{..}, DrawRect{..}, DrawEllipse{..}, DrawPolyline{..},
    FloodFill{..}, StampBrush{..}, PastePixels{..},
    SetPaletteColor{..}, SetColorCycle{..},
    CreateLayer{..}, RenameLayer{..}, ReorderLayer{..}, SetLayerVisibility{..}, …
    CreateFrame{..}, DuplicateFrame{..}, SetFrameDuration{..}, …
}
struct Request { document_id, expected_revision: Option<u64>, dry_run: bool, command: Command }
struct Outcome { new_revision, affected_rect: Option<Rect>, warnings: Vec<Warning>, undo_available: bool }
```

- Every command is `serde`-serialisable → the same structs are the MCP tool parameters (via `schemars`) and the command log.
- `Engine::execute` validates first (bounds, palette range, locked layer, mode limits) and only then mutates. Errors carry stable codes (`OUT_OF_BOUNDS`, `REVISION_CONFLICT`, `PALETTE_RANGE_INVALID`, `UNSUPPORTED_MODE`, …).
- Undo stores, per command, the previous tiles of the affected rect (cheap with COW tiles) plus inverse metadata ops.
- Transactions: `begin/commit/rollback`; a batch is one undo step and all-or-nothing.
- **Interactive strokes**: the GUI accumulates a freehand stroke as one `StampBrush{points}` command committed on mouse-up; the in-progress stroke is drawn into a preview overlay, not into the document. This keeps undo granular and MCP-equivalent.

## 5. Rendering and the RetroUi (ADR-3)

```text
            ┌─────────────────────── eframe window ───────────────────────┐
 input ───► │ physical px → logical UI px (÷ integer scale) → canvas px    │
            │                                                              │
            │  RetroUi::frame(state) → Framebuffer RGBA8 (e.g. 640×512)    │
            │     menu bar, toolbox, palette, requesters, canvas viewport  │
            │  upload changed rows → egui texture (TextureOptions::NEAREST) │
            │  draw one textured rect at integer scale, snapped to device  │
            │  pixels; the window snaps to screen × pixel size (ADR-13)    │
            └──────────────────────────────────────────────────────────────┘
```

- The **whole visible UI**, including the zoomed canvas viewport, is software-rendered into one RGBA framebuffer at "Amiga screen" resolution. egui only provides the window, input, clipboard and the GPU blit.
- Pixel size in whole and half points per UI pixel (Prefs ▸ Pixel Size 1x–8x), drawn in **physical** pixels (Retina: every step sharp; 1× displays: half steps give uneven nearest-neighbour pixels, never blur).
- Screen size: exactly the document's screen; the window snaps to screen × pixel size after a resize. Full screen centres it at the largest step that fits (ADR-13).
- Because rendering is pure CPU code, **UI screenshots are deterministic** and testable headless (golden PNGs) and available to MCP (`get_ui_screenshot`).
- Colour cycling is a display-time palette rotation; document pixels never change while cycling.
- Font: an **original** 8×8 / 8×9 bitmap font drawn for this project (Topaz and other Amiga ROM fonts are not redistributable). Icons are original pixel art drawn in the spirit of DPaint IV, not copied.

## 6. Files

| Path | Crate | Notes |
|---|---|---|
| `.dpxproj` | dpx-formats `project.rs` (ADR-23; the version 1 layout in `docs/PROJECT_FORMAT.md` is simpler than this sketch) | ZIP: `project.json` (schema_version, canvas, target, layer tree, frames), `palette.json`, `cels/<layer>/<frame>.bin` (zstd, tiled), `preview.png`, `source/` (preserved import chunks). Atomic save: temp file in same dir → fsync → rename. Zip-bomb/traversal/duplicate-entry checks. |
| IFF ILBM | dpx-iff | Import all documented variants incl. EHB/HAM6/HAM8, CRNG/CCRT; export via `ExportAnalyzer` report → explicit confirmation if lossy. |
| IFF ANIM | dpx-iff | Phase 6: op 5 first (most common, DPaint), verified against foreign files. |
| PNG | dpx-formats | Import (indexed as is; true colour reduced by `dpx_core::quantize`, ADR-9; later also as RGBA layer), export composited native resolution. |
| JPEG | dpx-formats | Import only (zune-jpeg, EXIF orientation), reduced like true-colour PNG (ADR-9). |

## 7. MCP (ADR-5)

```text
MCP client ──stdio──► deluxepainter mcp ──local socket / named pipe──► running GUI (Engine)
                                    └── no GUI running → in-process headless Engine
```

- rmcp 3.9.x (pinned), protocol versions negotiated per the current MCP spec (stateless `2026-07-28` era plus legacy `initialize` clients — see `docs/research/dependencies.md`).
- Tools = thin wrappers mapping JSON params → `Command`, returning `Outcome` + stable error codes. Resources for document state, palette, layers, preview PNG.
- MCP is **off by default**; enabled by preference or `--mcp`. File access restricted to user-approved directories (canonicalised, symlink-safe). Logs to stderr only.

## 8. Testing strategy

- Unit + property tests (proptest) for raster algorithms, ByteRun1, IFF parsing with fuzzed bytes.
- Golden files: ILBM fixtures we generate ourselves + a small set of redistributable third-party files (licence checked), compared pixel-by-pixel after decode.
- UI golden screenshots rendered headless by `dpx-ui`.
- MCP acceptance scenario (`docs/MCP_API.md`, Bauplan §7.5) as an integration test against the headless engine.
- CI: `fmt --check`, `clippy -D warnings`, `test`, release build on macOS, Windows, Linux.

## 9. Toolchain

Current eframe/egui (0.37) needs rustc ≥ 1.95; the local Homebrew toolchain is 1.90. The repo will pin the toolchain in `rust-toolchain.toml`; how it gets installed (rustup vs. Homebrew upgrade) is an open owner decision. Fallback: eframe 0.33.3 on 1.90 behind a thin adapter (the `App` trait changed between 0.33 and 0.37).
