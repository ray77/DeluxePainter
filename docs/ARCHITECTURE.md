# Architecture

Status: Phase 0 draft, 2026-11-09; sections 2, 3, 7, 7a, 8 and 9 brought up to date for wave 8 (2026-11-13: live MCP, layers, AI image generation, hardware rules); sections 4–6 still show parts of the Phase-0 sketch (see the ADRs for what was built). Decisions marked **ADR-n** are recorded in `docs/adr/`.

## 1. Goals that shape the architecture

1. One engine, many front-ends: the GUI, the CLI and the MCP server all mutate documents **only** through the same validated `Command` API.
2. Pixel-exact, deterministic results: the same command sequence produces byte-identical pixels and files on every platform.
3. The classic UI is pixel art, not a skinned widget toolkit: rendered by our own code into a low-resolution framebuffer, shown with nearest-neighbour scaling at integer factors.
4. Editable project (`.dpxproj`) ≠ hardware output (IFF ILBM/ANIM, PNG). Export is a separate, checked conversion.
5. Core crates compile and test without a GPU or window system.

## 2. Workspace layout

As built (wave 8):

```text
crates/
  dpx-core      Document model (frames, layers, palette, HardwareTarget), Command enum,
                Engine (execute/undo/redo, revisions, dry runs), raster algorithms
                (`raster`), colour reduction and resampling (`quantize`, `resample`),
                import and generated-picture conversion (`import`), hardware rules and
                export notes (`hardware`), display modes and screen presets (`mode`)
  dpx-iff       IFF container, ILBM (BMHD/CMAP/CAMG/BODY/CRNG/CCRT/GRAB…), ByteRun1,
                ANIM op 5
  dpx-formats   PNG import/export, JPEG import (decoders only), EHB/HAM6/HAM8 codecs, the
                .dpxproj project (`project.rs`: ZIP + JSON manifest + one indexed PNG per
                layer and frame)
  dpx-ui        RetroUi: framebuffer, bitmap font, widgets, toolbox, menus, requesters
                (incl. the layer panel, Prefs > AI & MCP.., Generate, Image Generator setup)
  dpx-mcp       MCP (rmcp): tools and resources, the Workspace trait and job queue, the
                Streamable HTTP server on 127.0.0.1, endpoint files, the stdio bridge,
                the .mcpb names; feature `imagegen` for the generation tools
  dpx-imagegen  AI image generation: ImageGenerationProvider trait, OpenAI Images (ureq,
                native TLS), API keys (keychain via `keyring`), worker-thread jobs
apps/
  deluxepainter  The GUI (eframe) by default; `deluxepainter mcp [--http]` = headless MCP
                 server; `deluxepainter mcp-bridge` = the stdio bridge; `info`/`convert` CLI
  dpx-mcp-bridge The stdio bridge alone (no eframe), the binary in the .mcpb extension
xtask           `cargo run -p xtask -- mcpb` packs the Claude Desktop extension
```

Planned in Phase 0 but not split out: `dpx-raster` (in `dpx_core::raster`), `dpx-project` (in `dpx-formats`), `dpx-render` (compositing is `dpx_core::layer`, the screen is drawn by `dpx-ui` and the app).

Dependency direction (no cycles): `core ← iff/formats ← app`, `core ← ui ← app`, `core/formats ← imagegen ← app`, `core/formats/iff ← mcp ← app/bridge/xtask` (`mcp` → `imagegen` only with its `imagegen` feature, which the app turns on and the bridge leaves off). `dpx-ui` never touches files directly; it issues commands. No pixel logic outside `dpx-core`.

One binary for the GUI, the CLI and the headless MCP server (ADR-5); since ADR-43 there is also the small `dpx-mcp-bridge` binary, so a client configuration can start a process without the GUI toolkit (the same bridge is `deluxepainter mcp-bridge`).

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

- **Frames (ADR-28, implemented)**: `Document` holds 1..N frames of one size, one page each; the current frame is `page()` and every page command acts on it; the other frames sit behind `Arc` (shared by undo snapshots and copies). Frame commands are undo steps; `GotoFrame`/`SetFrameRate` are settings without one.
- **Layers (ADR-44, implemented; the sketch above is not what was built)**: one flat layer stack per document, shared by every frame (`Layer { name, visible }`, bottom first, at most 32); every frame holds one indexed picture per layer, all in the document palette. `page()` is the active layer of the current frame, so every page command paints it. Above the bottom layer colour 0 is transparent; the bottom layer is opaque. `Document::composite()` flattens the visible layers (borrowed, no copy, for a single visible layer); the screen, previews, MCP reads and every export use it. Layers are shown or hidden only (no opacity, lock, blend modes or groups: owner decision). Layer commands are undo steps; `select_layer` is a setting. Project format version 3 stores them.
- **Chipset and hardware rules (ADR-46)**: the target's chipset (OCS or AGA) is a document rule: OCS shows 32 colours in Low Res, 16 in High Res, no Super-High Res, and its palette entries are 12-bit. `dpx_core::hardware` holds the rules and the export notes; `dpx_core::mode::PRESETS` is the one preset table for Screen Format, Generate and MCP.
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
- Because rendering is pure CPU code, **UI screenshots are deterministic** and testable headless (golden PNGs, `deluxepainter --snapshot`). MCP clients see the picture (`get_preview`, `dpx://document/preview.png`), not the UI; the planned `get_ui_screenshot` tool was not built.
- Colour cycling is a display-time palette rotation; document pixels never change while cycling.
- Font: an **original** 8×8 / 8×9 bitmap font drawn for this project (Topaz and other Amiga ROM fonts are not redistributable). Icons are original pixel art drawn in the spirit of DPaint IV, not copied.

## 6. Files

| Path | Crate | Notes |
|---|---|---|
| `.dpxproj` | dpx-formats `project.rs` (ADR-23; the version 1 layout in `docs/PROJECT_FORMAT.md` is simpler than this sketch) | ZIP: `project.json` (schema_version, canvas, target, layer tree, frames), `palette.json`, `cels/<layer>/<frame>.bin` (zstd, tiled), `preview.png`, `source/` (preserved import chunks). Atomic save: temp file in same dir → fsync → rename. Zip-bomb/traversal/duplicate-entry checks. |
| IFF ILBM | dpx-iff | Import all documented variants incl. EHB/HAM6/HAM8, CRNG/CCRT; export via `ExportAnalyzer` report → explicit confirmation if lossy. |
| IFF ANIM | dpx-iff | Phase 6: op 5 first (most common, DPaint), verified against foreign files. |
| PNG | dpx-formats | Import (indexed as is; true colour reduced by `dpx_core::quantize`, ADR-9; the GUI's Picture ▸ Load.. goes through the Import requester and `dpx_core::import`, ADR-42; later also as RGBA layer), export composited native resolution. |
| JPEG | dpx-formats | Import only (zune-jpeg, EXIF orientation), reduced like true-colour PNG (ADR-9; Import requester in the GUI, ADR-42). |

## 7. MCP (ADR-43; supersedes ADR-5's socket plan)

```text
Claude Desktop / Claude Code ──stdio──► dpx-mcp-bridge ──Streamable HTTP, 127.0.0.1, bearer token──► running GUI
                                         (reads <config dir>/mcp-endpoint.json)                       (JobQueue in App::logic)
scripts / tests ──stdio or HTTP──► deluxepainter mcp [--http] ──► headless engine (worker thread)
```

- ADR-5 planned `deluxepainter mcp` to reach the GUI over a local socket / named pipe (`interprocess`). ADR-43 replaced that: the GUI runs rmcp's Streamable HTTP server itself (axum, `127.0.0.1` only, stateless, JSON responses) while MCP control is on, and a stdio bridge forwards to it with rmcp's own HTTP client. One protocol end to end, no `interprocess` dependency; the HTTP endpoint can also be used directly.
- **Tools never own editor state.** `dpx_mcp::Workspace` is the editor state a tool needs (the shown page's engine, the spare page, the brush, the picture's name, questions, the image generator). A tool sends boxed jobs over a channel; the owner thread runs them: the GUI in `App::logic` (every pass, also while the window is hidden), the headless server on its worker thread. File I/O, encoding, decoding and conversions run on the server's threads, so a frame never waits for disk or a long conversion. Calls are serialised in arrival order (`dpx_mcp::seq`).
- **Same commands as the GUI.** Every change is a `dpx-core` command run as a menu command (gestures committed first, Stencil > Paint ended), one undo step of the page it changed (the result's `document_id`: the shown page, or the spare page for `import_image` and `apply_generated_image` to the spare page) unless the result says `undoable: false` (`open_picture`, `import_image` onto the picture or as the brush, `create_document` on an animation, a generated picture applied as the brush; `select_layer` and `goto_frame` are settings); `U` undoes MCP changes and MCP `undo` the user's, each on the page shown. Changes are refused (`EDITOR_BUSY`) while a requester is open and held while the user paints.
- **Security:** off by default; `127.0.0.1` only; Host and Origin checks; a 32-byte bearer token kept in the config directory (0600 on Unix) and never in a client configuration (the bridge reads the endpoint file, trusted only while its server holds `mcp-endpoint.lock`); separate read and edit permissions; file tools only in approved directories; replacing a file needs `overwrite: true` and the user's click; Revoke access rotates the token and turns the permissions off; no tool runs a shell command.
- **Connecting:** Prefs > AI & MCP.. runs `claude mcp add --scope user deluxe-painter -- <app> mcp-bridge` after the user confirmed it, or opens the `.mcpb` extension (built by `xtask`, one platform per bundle) so Claude Desktop shows its install dialog.
- rmcp 3.9; only `initialize`-based (2025 protocol era) clients have been exercised. Tools, parameters, error codes and the security model in detail: [`MCP_API.md`](MCP_API.md).

## 7a. AI image generation (ADR-45)

- `dpx-imagegen`: the `ImageGenerationProvider` trait (requests, capabilities, typed errors with stable codes), OpenAI Images as the first provider (blocking `ureq` with native TLS on a worker thread; rustls is avoided: rustls itself allows MIT, but every rustls build pulls the ISC-licensed `rustls-webpki` and `untrusted`, and its default crypto crate `ring` is Apache-2.0 AND ISC), API keys in the OS keychain (`keyring`) or for the session only, never in a file, and a `JobManager` (worker threads, cancel, at most 2 requests in flight) shared by the GUI and MCP.
- The conversion of a generated picture to the page (crop or fit, scalers, colour reduction, 12-bit OCS palettes) is `dpx_core::import::convert` (a new layer: `convert_for_layer`, the brush: `convert_for_brush`, which match the picture's palette as stored; pixel logic stays in `dpx-core`); applying it to the picture, a new layer or the spare page is one undo step (`Command::ReplacePicture`, `Command::AddPictureLayer`, the spare page's engine); as the brush it only replaces the custom brush, which is no undo step.
- Picture > Generate.. and the MCP generation tools (in the window only, through `Workspace::image_generation`) use the same generator, jobs and conversion. Without a set-up generator nothing is generated; a test pattern exists for tests and snapshots only.

## 8. Testing strategy

- Unit + property tests (proptest) for raster algorithms, ByteRun1, IFF parsing with fuzzed bytes.
- Golden files: ILBM fixtures we generate ourselves + a small set of redistributable third-party files (licence checked), compared pixel-by-pixel after decode.
- UI golden screenshots rendered headless by `dpx-ui`.
- MCP: tool tests on the headless workspace, ui-loop tests on the GUI workspace, integration tests against the real `deluxepainter mcp` (stdio and HTTP) and the real bridge binary; see [`MCP_API.md`](MCP_API.md) §9.
- CI: `fmt --check`, `clippy -D warnings`, `test`, release build on macOS, Windows, Linux.

## 9. Toolchain

eframe/egui 0.37 needs rustc ≥ 1.95, the workspace's `rust-version`. The toolchain is pinned in `rust-toolchain.toml` (Rust 1.99.1, installed with rustup; owner decision, see ROADMAP "Open owner decisions"). The Phase-0 fallback (eframe 0.33.3 on the Homebrew 1.90 toolchain) was not needed.
