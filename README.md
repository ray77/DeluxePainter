<p align="center">
  <img src="assets/app/deluxepainter-logo.png" alt="DeluxePainter logo" width="256">
</p>

<h1 align="center">DeluxePainter</h1>

<p align="center">
  A pixel-art editor in the spirit of the classic Amiga paint programs —<br>
  real OCS/AGA hardware rules, IFF ILBM/ANIM files, layers, and an MCP interface for AI agents.<br>
  Native Rust · macOS · Windows · Linux
</p>

---

> **Status: pre-alpha (Phase 1 first paint, Phase 2 files started).** The window shows a classic Amiga LoRes paint screen (title bar, toolbox, palette, pull-down menus) rendered pixel-exactly; freehand painting, tool/brush/colour selection, undo/redo, CLR, paint-mode selection and a few menu commands work. IFF ILBM and indexed PNG pictures can be inspected, converted and opened from the command line. Everything else is still to come.

<p align="center">
  <img src="docs/design/screenshots/phase1-lores-default.png" alt="DeluxePainter start screen, 320×256 LoRes at 2×" width="400">
  <img src="docs/design/screenshots/phase1-menu-picture.png" alt="Picture menu open" width="400">
</p>

## What it will be

- **The classic look and feel**, rebuilt from scratch: menu bar, two-column toolbox, palette, requesters — rendered as crisp pixel art at integer scales (1×–5×, 4K-ready), never blurred.
- **Real Amiga targets**: OCS and AGA profiles (PAL/NTSC, LoRes/HiRes/SuperHiRes, interlace, EHB, HAM6, HAM8) are enforced document rules, not a visual filter.
- **Amiga files done right**: IFF ILBM import/export (ByteRun1, masks, CAMG, colour cycling) and IFF ANIM; PNG import/export; JPEG import.
- **Modern project editing**: the native `.dpxproj` format keeps layers, groups, frames, palettes and editor state. Saving is lossless; Amiga files are produced by an explicit, checked export.
- **Classic tools**: freehand, lines, curves, shapes, fills with gradients and dithering, airbrush, brushes with transform, stencil, symmetry, grid, magnify, spare page, colour cycling, animation with light table.
- **MCP server**: Claude and other MCP clients can create, draw, inspect and export documents through typed commands — the same command engine the GUI uses, no mouse automation.

See [`docs/FEATURES.md`](docs/FEATURES.md) for the prioritised feature set and [`docs/ROADMAP.md`](docs/ROADMAP.md) for the plan.

## Documentation

| Document | Content |
|---|---|
| [FEATURES.md](docs/FEATURES.md) | Condensed, prioritised feature list (MVP → later) |
| [ROADMAP.md](docs/ROADMAP.md) | Phases and milestones |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crates, document model, command engine, rendering, MCP |
| [adr/](docs/adr/) | Architecture decision records |
| [HARDWARE_MODES.md](docs/HARDWARE_MODES.md) | OCS/AGA display modes, palettes, EHB/HAM rules |
| [IFF_SUPPORT_MATRIX.md](docs/IFF_SUPPORT_MATRIX.md) | ILBM/ANIM chunk layouts and support plan |
| [research/](docs/research/) | Manual, video and dependency research with sources |
| [design/](docs/design/) | UI layout mockups |

## Building

Requires [rustup](https://rustup.rs); the toolchain version is pinned in `rust-toolchain.toml`. A non-rustup Rust (for example Homebrew's `rust` formula) ignores that file; if `which cargo` does not point to `~/.cargo/bin/cargo`, put `~/.cargo/bin` first in `PATH` or remove the other Rust.

```bash
cargo run -p deluxepainter --release
```

Headless snapshot of the start screen (used for visual comparison tests):

```bash
cargo run -p deluxepainter -- --snapshot screen.png --scale 2
```

Controls so far: left mouse = FG colour, right mouse = BG colour; click toolbox buttons, brushes and palette cells. Every toolbox drawing tool works (ADR-14): freehand (dotted, continuous, filled), line and curve (drag, release, move to bend, click), rectangle/circle/ellipse/polygon as outline (upper-left half of the button) or filled (lower-right half), fill, airbrush and text; Shift makes lines horizontal/vertical and rectangles square; a polygon closes when you click its first corner or press Space/Return; Grid (`g`), Symmetry (`/`) and Magnify (`m`, split screen, Zoom with `>` `<`) are toggles. Tool keys as in DeluxePaint: `s`, `d`/`D`, `v`, `q`, `f`, `r`/`R`, `c`/`C`, `e`/`E`, `t`, `K`. Hold the right mouse button on the title bar for the menus (release on an item to select it, click the left button to select several Prefs at once); `F1`–`F8` paint modes; `U` or UNDO undo, `Shift+U` or right-click on UNDO redo (64 steps); `F10` hide/show toolbox and title bar, `F9` title bar only. The window snaps to the picture's screen in whole and half steps when you resize it; Prefs ▸ Pixel Size (or Cmd/Ctrl + `+`, `−`, `0` for half a step bigger, smaller, largest) sets the step; Ctrl+Cmd+F (macOS) or F11 switches full screen. DeluxePainter remembers the pixel size, the Prefs menu, the last drawer and the tool settings in `settings.json` in its config directory (macOS `~/Library/Application Support/DeluxePainter`, Windows `%APPDATA%`, Linux `$XDG_CONFIG_HOME`); `--no-settings` starts without it (ADR-33).

Pictures: IFF ILBM and indexed PNG in and out; true-colour PNG and JPEG in (HAM/EHB come later). True-colour pictures get a new palette with the mode's colour count and Floyd–Steinberg dithering; pictures bigger than any screen are scaled to the current screen; transparent pixels become pink colour 0 (ADR-9). `info` shows how a picture will open:

```bash
cargo run -p deluxepainter --release -- info picture.iff
```

```bash
cargo run -p deluxepainter --release -- convert picture.iff picture.png
```

```bash
cargo run -p deluxepainter --release -- --open picture.iff
```

Picture ▸ Load.. and Save.. open a DeluxePaint-style file requester (click a `<DIR>` to open it, click a file to choose it, double-click to load/save; Drawer, Pattern with AmigaDOS wildcards such as `#?.iff`, Disks, Assigns, Parent). Save writes IFF ILBM with the display mode (CAMG), pixel aspect and colour cycling ranges; a name ending in `.png` writes PNG. Whatever a load or save could not keep (unknown chunks, mask plane, what PNG cannot store) or had to change (scaled, colours reduced, page enlarged) is listed in a requester with Ok afterwards.

Pages larger than the screen: Picture ▸ Page Size.. (Standard, Full Page, Overscan or any size up to 1008×1024); arrow keys scroll (Alt+arrows by a screen), `n` centres the spot under the pointer, the mouse wheel or trackpad scrolls too; Picture ▸ Show Page (Shift+S) shows the whole page — drag the rectangle and release to jump there.

Display modes: Picture ▸ Screen Format.. chooses PAL/NTSC LoRes, HiRes and SuperHiRes (laced or not), the screen size (Standard, Overscan or a Custom width/height), palette size, page size and whether to keep the picture.

Menus that work (everything else is ghosted until it exists, ADR-15): Picture ▸ Load.., Save.. (the `.dpxproj` project), Export.. (IFF ILBM, or PNG for a `.png` name), Delete.. (asks Yes/No), Flip, Spare (Swap `j`, Copy to Spare Ctrl+`j`, Merge in Front/Back with colour 0 transparent, Delete This Page — each page keeps its own undo), Page Size.., Show Page, Screen Format.., About.. (`?`), Quit (asks "Quit without saving?" when the picture has unsaved changes; unsaved work is autosaved to a recovery file and offered again at the next start, ADR-34); Color ▸ Cycle (Tab: cycles the colour ranges loaded with the picture on screen only; it keeps running under requesters and menus and during animation playback, where Tab also toggles it, ADR-35), Palette ▸ Restore / Default / Load.. / Save.. (DeluxePaint palette files: ILBM with BMHD + CMAP + CRNG, no BODY; Load also takes the colours of any ILBM picture), BG -> FG, BG <-> FG; Effect ▸ Background ▸ Fix / Free (CLR and `k` then go back to the fixed picture); Mode, Prefs and Pixel Size.

<p align="center">
  <img src="docs/design/screenshots/phase3-menus.png" alt="Color menu with the Palette submenu; unavailable items ghosted" width="400">
</p>

Color ▸ Ranges.. opens the Range requester (ADR-22): pick one of eight ranges, click its first and last colour (or drag across the palette), set its speed, direction and Active; a new picture starts with two inactive default ranges. Ok keeps the ranges (saved as CRNG chunks), Cancel drops the edits.

<p align="center">
  <img src="docs/design/screenshots/phase3-ranges.png" alt="The Range requester over the default palette" width="400">
</p>

Palette operations and effects (ADR-31): Color ▸ Palette ▸ Arrange (`P`) reorders the palette by dragging swatches (Ex exchanges) and keeps the picture looking the same in one undoable step; Color ▸ Remap re-points the pixels after a palette change; Effect ▸ Translucency, Process and AntiAlias (title-bar flags `T`, `P`, `a`/`A`), Stencil ▸ Show, Load.. and Save.. work. The snapshot below was made with `--commands docs/design/screenshots/phase3-palette-ops.json --arrange-requester --moves 9:0,31:2,20:12`.

![Translucent shapes and the Arrange requester](docs/design/screenshots/phase3-palette-ops.png)

Toolbox settings (ADR-20): right-click Grid, Symmetry or Fill, or the freehand, line, curve, outline-shape and Airbrush buttons, to open the Gridding, Symmetry, Fill Type or Spacing requester (Return = Ok, Esc = Cancel, Tab = next field; `F` and `V` open Fill Type and Spacing). Fill can paint gradients through the colour range of the foreground colour, Spacing lays brush stamps along lines and outlines (N total, every Nth pixel, airbrushed), Symmetry can mirror, and Effect ▸ Stencil ▸ Make.. protects colours.

<p align="center">
  <img src="docs/design/screenshots/phase3-requesters.png" alt="The Fill Type requester" width="400">
</p>

Custom brushes (ADR-18): the Brush Selector (`b`) picks up the part of the page you drag over (right button: cut, leaving BG behind, one undo step; Shift: square); the BG colour becomes transparent and the mode switches to Matte, then the previous tool comes back. Every brush tool stamps the custom brush, which also follows the pointer over the page. Matte paints its own colours, Color its shape in FG (BG with the right button), Replace the whole rectangle. `B` or a right-click on the Brush Selector brings the last brush back; clicking a built-in brush drops it. The snapshot below was made with `--commands docs/design/screenshots/phase3-brushes.json --pick 24,34,25,25 --brush-commands docs/design/screenshots/phase3-brushes-stamps.json` (`--pick X,Y,W,H` picks up a brush after `--commands`; `--brush-commands FILE.json` then runs commands that stamp it).

Brush transforms (ADR-30): Brush > Size > Stretch, Rotate > Any Angle / Shear, Bend > Horiz / Vert and Handle > Place are drag gestures: choose the item, the brush follows the pointer, press and drag on the page to preview, release to keep (Esc cancels, Brush > Restore returns to the picked brush). Brush > Spare and Remap work on the app's brush; none of this touches the page. Snapshot: `--commands docs/design/screenshots/phase3-brushes.json --pick 24,34,25,25 --brush-xforms 'stretch:60,30@6,160;rotate:30@85,152;shear:14@150,160;bend_h:12,12@200,160;bend_v:12,12@255,165;rotate:75@150,205'` (`--brush-xforms` stamps transformed copies of the picked brush; ops `stretch:W,H`, `rotate:DEG`, `shear:DX`, `bend_h:AMOUNT,LINE`, `bend_v:AMOUNT,LINE`, each `@X,Y`).

![Stretched, rotated, sheared and bent brushes](docs/design/screenshots/phase3-brush-transforms.png)

<p align="center">
  <img src="docs/design/screenshots/phase3-brushes.png" alt="A picked-up brush stamped in Matte, Replace and Color, with Dotted Freehand and a Color-mode curve" width="400">
</p>

Paint modes and the Brush menu (ADR-19): `dpx_core::paint::plot` implements Smear, Shade, Blend, Cycle, Smooth, Mix and HBrite. The Brush menu flips, rotates by 90 degrees, doubles/halves, outlines/trims and recolours the custom brush, moves its handle, and loads/saves brush files (IFF ILBM). Items that need a drag (Stretch, Any Angle, Shear, Bend, Place) stay ghosted. `DPX_MODES_PNG=out.png cargo test -p deluxepainter mode_demo` renders the mode demo `docs/design/screenshots/phase3-modes.png`.

Animation (ADR-28): Anim ▸ Frames ▸ Set #.. makes frames (new ones copy the current frame), Add Frames.., Copy Frames.. (to all, to a range, or copies inserted before a frame) and Delete Frames.. change them; `1`/`2` step back/on, `3` Go to Frame, `4` Play, `5` Play once, `6` Ping-pong (Esc, Space or a click stops; Left/Right change the speed; Control ▸ Set Rate.. sets frames per second). Painting and every picture command work on the current frame, and undo steps back to the frame it changed. With several frames the title bar shows `frame/total`. Effect ▸ LightTable (`l`) shows the previous frame (Ctrl+`1`–`4`: two back, previous, next, spare page) dimmed behind the current one; Merge copies them in (undoable). Save keeps every frame (project format version 2; a single picture is still version 1); Export writes the current frame, or all frames for a `.anim` name. Anim ▸ Load.. / Save.. read and write IFF ANIM files (ADR-32); `deluxepainter info x.anim` and `convert a.anim b.dpxproj` work too. The snapshot below was made with `--frames 3 --commands docs/design/screenshots/phase6-anim.json --frame 2 --light-table previous,next,dim`.

<p align="center">
  <img src="docs/design/screenshots/phase6-anim.png" alt="Frame 2 of 3 with the previous and next frames dimmed behind it by the Light Table" width="400">
</p>

Snapshot options: `--palette gold16`, `--fg N`, `--tool NAME[:filled]`, `--menu M[:I[:S]]` (open menu M, highlight item I and sub item S), `--open picture.iff` (also PNG/JPEG), `--requester` (Screen Format requester), `--file-requester load|save|export|load-anim|save-anim` with `--drawer DIR`, `--file NAME`, `--drawer-as TEXT` (Load/Save/Export requester), `--page WxH`, `--scroll X,Y`, `--commands FILE.json` (run a JSON array of commands first, e.g. `docs/design/screenshots/phase3-tools.json`), `--show-page`, `--page-size-requester`, `--fill-requester`, `--grid-requester`, `--symmetry-requester`, `--spacing-requester`, `--stencil-requester [--protect 1,2,3]`, `--palette-requester`, `--range-requester`, `--arrange-requester [--moves 9:0,31:2]`, `--translucency-requester`, `--notes` (the notes of `--open`), `--frames N` (before `--commands`), `--frame I`, `--light-table [previous,next,two_back,spare,dim]`, `--anim-requester count|add|copy|delete|goto|rate`, `--font-preview out.png`.

## MCP server (spike)

`deluxepainter mcp` runs a headless [MCP](https://modelcontextprotocol.io) server on stdin/stdout (rmcp; stdout carries protocol messages only, logs go to stderr). It is off unless a client starts it. Tools: `app_status`, `new_document`, `get_document_state` (ADR-21), `execute_command` (any engine command as JSON, with `expected_revision` and `dry_run`), `undo`, `redo`, `get_palette`, `get_pixels`, `render_png` (the page as an image, so an agent can look at its work), `open_picture` and `save_picture` (ADR-24). The file tools work only inside directories you approve with `--allow-dir DIR` (repeatable); without it they fail with `FILE_ACCESS_OFF`. They read and write indexed IFF ILBM and indexed PNG; true-colour PNG and JPEG are not opened over MCP. Example client configuration:

```json
{
  "mcpServers": {
    "deluxepainter": {
      "command": "/path/to/deluxepainter",
      "args": ["mcp", "--allow-dir", "/path/to/pictures"]
    }
  }
}
```

## Project files (.dpxproj)

The lossless project format is a ZIP with a JSON manifest and one indexed PNG per layer; see [`docs/PROJECT_FORMAT.md`](docs/PROJECT_FORMAT.md). On the command line, `deluxepainter info x.dpxproj` describes a project and `deluxepainter convert a.iff b.dpxproj` (and back) converts. Wiring it into the Save and Export menus is still to do.

## License

Copyright © 2026 Dominik Schmidt. DeluxePainter is source-available under the [PolyForm Noncommercial License 1.0.0](LICENSE.md). You may use, modify and share it for any **noncommercial** purpose. Commercial use requires a separate license from the author.

## Disclaimer

DeluxePainter is an independent project. It is not affiliated with, endorsed by, or connected to Electronic Arts. "Deluxe Paint" / "DeluxePaint" is a trademark of Electronic Arts Inc.; Amiga is a trademark of its respective owner. No original program code, artwork, fonts or documentation of DeluxePaint are included in this repository.
