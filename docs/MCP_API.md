# MCP API

The technical reference of DeluxePainter's [Model Context Protocol](https://modelcontextprotocol.io) interface: transports, tools, resources, error codes, the security model and how to test it. For the two-minute set-up for non-developers see the README ("Connect to Claude in two minutes"). Decisions and their reasons are in the ADRs: [ADR-43](adr/0043-live-mcp.md) (live MCP, HTTP server, bridge, Prefs > AI & MCP..), [ADR-44](adr/0044-layers.md) (layers and the layer tools), [ADR-45](adr/0045-ai-image-generation.md) (image generation and its tools), [ADR-46](adr/0046-modes-presets-and-hardware-rules.md) (chipsets, presets, hardware rules), [ADR-24](adr/0024-mcp-tools-and-file-access.md) (approved directories).

Status (wave 8, 2026-11-13): this page describes the code after the wave-8 integration: the live MCP of ADR-43 plus the layer tools (ADR-44, "Integration (wave 8)") and the image generation tools (ADR-45, "Integration (wave 8)"). What was and was not verified is listed at the end; in short, **no real MCP client (Claude Code, Claude Desktop, MCP Inspector) and no real OpenAI request has been run**, and everything ran on macOS only.

## 1. Overview

```text
Claude Desktop / Claude Code ──stdio──► dpx-mcp-bridge ──HTTP 127.0.0.1──► DeluxePainter window
                                        (deluxepainter mcp-bridge)          (Prefs > AI & MCP.. On)

Scripts, tests ──stdio──► deluxepainter mcp            (headless engine, no window)
               ──HTTP───► deluxepainter mcp --http     (headless engine, no window)
```

- The **running app** serves MCP Streamable HTTP on `127.0.0.1` while MCP control is on (Prefs > AI & MCP.., off by default, or `--mcp` for one session). The tools act on the document in the window, through the same `dpx-core` commands as the mouse and keyboard; a change is an undo step of the page it changed (the result's `document_id`), which the user can take back with `U` on that page, unless its result says `undoable: false` (section 3).
- MCP clients launch stdio servers themselves, so they start the **bridge** (`dpx-mcp-bridge`, also `deluxepainter mcp-bridge`). It forwards every call to the app. It needs no arguments and no token in the client's configuration: it reads the app's endpoint file.
- The **headless** servers (`deluxepainter mcp`, `deluxepainter mcp --http`) run their own engine with a picture and a spare page in memory; nothing is shown. They have no image generator.
- ADR-43 supersedes ADR-5's plan of a local socket / named pipe: one protocol end to end (rmcp's server and client), no `interprocess` crate.

## 2. Transports

### 2.1 Streamable HTTP inside the app

| Item | Value |
|---|---|
| Address | `http://127.0.0.1:<port>/mcp`, bound to `127.0.0.1` only |
| Port | 8765 by default (setting `mcp.port`); if it is taken, the next nine (8766–8774), then any free port (`dpx_mcp::http::bind`: ten ports from the preferred one on, that one included). The status line says "(port 8765 was in use)" after a fallback |
| Mode | stateless (no `Mcp-Session-Id`), JSON responses (no SSE stream); rmcp 3.9 |
| Lifetime | starts when MCP control is turned on (or with `--mcp`), stops when it is turned off and when the app quits; queued calls then fail with `APP_UNAVAILABLE` |
| Requests | `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (else 403); any `Origin` header is refused (403); `Authorization: Bearer <token>` compared in constant time (else 401 with `WWW-Authenticate: Bearer`); bodies over 16 MiB are refused (413) |
| One per user | one server per config directory answers MCP clients: it holds `mcp-endpoint.lock`, and a second one for the same config directory does not start ("another DeluxePainter is already serving MCP clients"). `mcp-app.lock` does something else: it ends a second `--mcp` start and tells the bridge that a GUI runs (section 2.4) |

Calls are served in arrival order, one at a time: `tools/call` and `resources/read` take a ticket when they arrive and keep their turn for the whole call (pipelined calls too). `initialize`, `tools/list` and `resources/list` take no ticket.

### 2.2 The stdio bridge

`dpx-mcp-bridge` (a small binary without the GUI, so it starts fast) or `deluxepainter mcp-bridge`; no arguments. It is what Claude Code's `claude mcp add` entry and the Claude Desktop extension (`.mcpb`) start.

- It answers `initialize`, `tools/list` and `resources/list` itself from the same definitions as the app, so a client sees every tool while the app is not running.
- It forwards `tools/call` and `resources/read` to `127.0.0.1` only, with the token from the endpoint file, through its own HTTP client (no proxy from `HTTP_PROXY`/`ALL_PROXY`, no redirects). It trusts an endpoint file only while the server that wrote it holds `mcp-endpoint.lock`; a file left behind by a crash is removed and nothing is sent to its port.
- With no reachable server:
  - DeluxePainter runs, MCP control off: `APP_UNAVAILABLE` "MCP control is off in DeluxePainter. …".
  - DeluxePainter runs, MCP control on but no answer within 3 s: `APP_UNAVAILABLE` "DeluxePainter is running, but its MCP server does not answer. …".
  - DeluxePainter does not run and `mcp-app.json` says MCP control was on: the bridge starts the app recorded there (`<app> --mcp`, detached, no standard streams) and waits up to 20 s for its server; without one by then the call is `APP_NOT_RUNNING` ("DeluxePainter was started but did not offer its MCP server within 20 s. …").
  - DeluxePainter does not run and MCP control was turned off: `APP_NOT_RUNNING`, the app is not started.
  - No record at all (or no config directory): `APP_NOT_RUNNING` "DeluxePainter is not running. Start it and turn on Prefs > AI & MCP."
- A read-only call (and every resource read) is retried once after a lost connection; a changing call is not (it may have run) and gives `APP_CONNECTION_LOST`.
- The bridge's own failures reach a client as tool results (`isError`, with the codes above); for a resource read they are JSON-RPC errors (internal error, message `CODE: text`).
- The app sees the client as "<client name> via deluxepainter-bridge".
- `dpx-mcp-bridge` built alone (`cargo build --release -p dpx-mcp-bridge`, as `cargo run -p xtask -- mcpb` does for the `.mcpb`) has no image generation code (no HTTP client for OpenAI, no TLS or keychain code). `deluxepainter mcp-bridge`, which Claude Code's `claude mcp add` entry runs, is the full app binary with that code; and a `dpx-mcp-bridge` built in one cargo run with the app (`cargo build --workspace`, `cargo test --workspace`, or `-p deluxepainter -p dpx-mcp-bridge` as in section 9) has it too, because cargo unifies `dpx-mcp`'s `imagegen` feature. Either way the bridge lists the generation tools and forwards their calls to the window; it generates nothing itself.

### 2.3 Headless servers

```bash
deluxepainter mcp [--allow-dir DIR]...                          # stdio
deluxepainter mcp --http [--port N] [--allow-dir DIR]...         # Streamable HTTP, port 8765 by default, 0 = any
```

- One engine in memory with a picture (`page1`) and a spare page (`page2`); no window, no autosave.
- Read, edit and overwrite are allowed (`overwrite: true` replaces a file without asking: there is nobody to ask).
- File tools work only inside the `--allow-dir` directories (repeatable; each must exist); without one they fail with `FILE_ACCESS_OFF`.
- The HTTP mode prints the URL and the endpoint file's path on stderr (never the token) and stops on Ctrl-C or SIGTERM, removing the endpoint file. Its token is random per start, or `DPX_MCP_TOKEN` (at least 16 visible ASCII characters); without a config directory it needs `DPX_MCP_TOKEN`.
- The four image generation tools answer `IMAGE_GENERATION_UNAVAILABLE` after their arguments are checked.
- stdout carries protocol messages only; diagnostics go to stderr.

### 2.4 Files in the config directory

The config directory is the one `settings.json` uses: `DPX_CONFIG_DIR` if set, else `~/Library/Application Support/DeluxePainter` (macOS), `%APPDATA%\DeluxePainter` (Windows), `$XDG_CONFIG_HOME/DeluxePainter` or `~/.config/DeluxePainter` (elsewhere).

| File | Written by | Content and rules |
|---|---|---|
| `mcp-token` | the app | the bearer token clients configured once keep using (64 hex digits, 32 random bytes); 0600 on Unix; replaced when damaged or readable by group/others; Revoke access writes a new one |
| `mcp-endpoint.json` | a running server | `{version: 1, port, token, pid, app_path, url}`, written atomically, 0600 on Unix; its `token` is a second token made for this start only (accepted besides `mcp-token`'s); removed when the server stops normally |
| `mcp-endpoint.lock` | a running server | held (exclusive OS lock) while it serves; the bridge trusts the endpoint file only while it is held |
| `mcp-app.json` | the app | `{version, app_path, args, enabled}`: what the bridge may start (`args` `["--mcp"]`) and whether it may (`enabled` follows MCP control; a record without `enabled` counts as off); it stays when the app quits |
| `mcp-app.lock` | a running GUI | held while the GUI runs; a `--mcp` start while another GUI holds it ends at once |

Windows has no 0600 equivalent here: the files rely on the profile directory's permissions (unverified).

### 2.5 Settings

`settings.json` (ADR-33), section `mcp` (missing keys keep their defaults):

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | the server runs while the app runs |
| `port` | `8765` | port tried first (`0`: any free port) |
| `allow_read` | `true` | clients may read the document (off: only `get_app_status` answers) |
| `allow_edit` | `true` | clients may change the document (needs `allow_read` too) |
| `overwrite_without_asking` | `false` | `overwrite: true` replaces an existing file without the "Replace File?" question (needs read and edit) |
| `approved_dirs` | `[<Pictures>/DeluxePainter]` | the only directories file tools may use; an explicit empty list turns file access off. Pictures is `~/Pictures` (macOS), `%USERPROFILE%\Pictures` (Windows), `$XDG_PICTURES_DIR` or `~/Pictures` (elsewhere). The default directory is created when the server starts |

Section `image_generator`: `{"provider": "none" | "openai", "openai": {"model", "quality", "timeout_secs", "connect_timeout_secs"}}`. Defaults: provider `none`, model `gpt-image-2.6-flare`, quality `medium`, timeouts 180 s and 20 s. **The API key is never in this file**: it is kept in the system keychain (service `DeluxePainter`, account `openai`), or for this session only when there is no usable keychain, or read from the `OPENAI_API_KEY` environment variable.

## 3. Conventions

- **Results.** Every result is a JSON object with `success: true`, and `document_id` and `revision` where a document is involved. It is the structured content and, serialised, the first text block (for clients that ignore structured content). `get_preview` and `get_generation_status` (when done) put a PNG image block first.
- **Errors.** A failed call is an `isError` result `{"success": false, "code": "...", "message": "..."}` with a stable upper-case code (section 6); a refused engine command keeps the `dpx-core` code (`INVALID_ARGUMENT`, `PALETTE_RANGE_INVALID`, `REVISION_CONFLICT`, `UNSUPPORTED_MODE`, `NOTHING_TO_UNDO`, `NOTHING_TO_REDO`). An unknown tool is a JSON-RPC `invalid_params` error, not a result. Arguments are strict: an unknown field is `INVALID_ARGUMENT`. The one exception is the `command` object of `execute_command`: it is the engine's own JSON form, and unknown fields inside it are ignored, so their names must be exact (section 4.5). Resource reads fail with JSON-RPC errors (section 5).
- **Documents.** DeluxePainter has two pages: the picture and the spare page. Their ids are `page1` and `page2`; an id stays with its page across Picture > Spare > Swap (`j`). The tools work on the active (shown) page; `select_document` swaps.
- **Layers.** Numbered from 0, the bottom layer. Layer 0 is opaque; on every layer above it palette colour 0 is transparent, so drawing with colour 0 there erases to the layers below. Drawing tools, `fill_area` and the page commands of `execute_command` paint the **active** layer. Previews, pixels and exports read the picture as it shows (the visible layers flattened).
- **Colours.** Drawing tools take a palette index (`color`), checked against the palette size. Colours are written `#RRGGBB` (or `#RGB`, each digit doubled). OCS documents take 12-bit colours only (each channel a multiple of 0x11).
- **Undo.** Every change is one undo step of the page it changed, the result's `document_id`, unless the result says `undoable: false`. That is the active page except for the spare-page targets of `import_image` and `apply_generated_image` (`page2` while `page1` is shown, and the other way round). `undo` and `redo` step the active page only: to take back a change to the spare page, `select_document` that page first. The user's `U` takes back an MCP change and MCP `undo` takes back the user's last change on the same page.
- **Revisions.** `revision` counts the page's changes; `execute_command` can require one (`expected_revision`, else `REVISION_CONFLICT`). Slow work (imports, generated pictures) checks that the page did not change meanwhile.
- **In the window.** A changing call is a menu command: a gesture without a held button (polygon corners, typed text) is committed first, Stencil > Paint ends, the area stencil (ADR-39) wraps strokes, shapes, fills and CLR, CLR on a fixed background restores it, and frame commands move the area stencil with their frame. While a requester is open (any but Prefs > AI & MCP..) changing calls and questions are refused with `EDITOR_BUSY` ("… reading still works. Try again after the user has closed it."); while the user holds a mouse button on the picture they wait, at most 10 s, then `EDITOR_BUSY`. Reading calls always answer. A job not answered within 60 s gives `APP_BUSY` and never runs later.
- **Activity log.** Every call writes one line (tool, ok or the error code, what it did, e.g. `draw_rectangle 10,20 20x10 colour 5 filled`); Prefs > AI & MCP.. shows the newest ones (the last 50 are kept).

## 4. Tools

33 tools: 21 of ADR-43, 4 image generation tools (ADR-45) and 8 layer tools (ADR-44).

Permission: **none** = always answers; **read** = needs "Read document info"; **edit** = needs "Read document info" and "Edit pictures and palettes". Hints are the MCP tool annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`); `openWorldHint` is false for every tool except `generate_image`.

| Tool | Permission | Hints |
|---|---|---|
| `get_app_status` | none | read-only |
| `list_documents` | read | read-only |
| `get_document_info` | read | read-only |
| `create_document` | edit | |
| `select_document` | edit | |
| `get_palette` | read | read-only |
| `set_palette_color` | edit | idempotent |
| `draw_pixel`, `draw_line`, `draw_rectangle`, `draw_ellipse`, `fill_area` | edit | |
| `import_image` | edit | destructive |
| `open_picture` | edit | destructive |
| `undo`, `redo` | edit | |
| `save_project` | read | destructive |
| `export_image` | read | destructive |
| `get_preview`, `get_pixels` | read | read-only |
| `execute_command` | edit | |
| `generate_image` | edit | open world |
| `get_generation_status` | read | read-only |
| `apply_generated_image` | edit | destructive |
| `cancel_generation` | read | idempotent |
| `list_layers` | read | read-only |
| `create_layer`, `move_layer`, `merge_layer_down` | edit | |
| `select_layer`, `set_layer_visibility`, `rename_layer` | edit | idempotent |
| `delete_layer` | edit | destructive |

`save_project` and `export_image` need only the Read permission (they change no picture), but replacing an existing file always needs `overwrite: true`, and the user's click unless "Overwrite files without asking" is on or the server is headless (section 7).

### 4.1 Status and documents

**`get_app_status`** (no arguments). `{name: "deluxepainter", version, engine: "gui" | "headless", transport: "stdio" | "http", permissions: {read, edit, overwrite_without_asking}, file_access: {enabled, approved_dirs}, document, brush}`. `document` (as `get_document_info`) only while reading is allowed; `brush` is `[width, height]` of the custom brush or null.

**`list_documents`** (no arguments). `{active: "page1" | "page2", documents: [...]}`: both pages as `get_document_info` describes them; a spare page not used yet is `{document_id, role: "spare", active: false, blank: true, notes}`.

**`get_document_info`** `{document_id?}` (default: the active page). `document_id`, `role` (`picture` for the shown page, `spare` for the other one), `active`, `name`, `width`, `height` (the page), `chipset` (`ocs`/`aga`), `display_mode` (e.g. `PAL:Low Res`), `screen_width`, `screen_height`, `custom_screen`, `colors` (palette entries), `max_colors` (the mode's maximum on this chipset), `frames`, `current_frame` (from 0), `layer_count`, `active_layer`, `layers` (as `list_layers`), `revision`; for the active page also `can_undo`, `can_redo`, `modified` (unsaved changes); `notes` (e.g. a custom screen size). A spare page not used yet is `DOCUMENT_NOT_FOUND`.

**`create_document`** — a new blank picture on the active page.

| Parameter | Default | Notes |
|---|---|---|
| `width`, `height` | the preset's or mode's screen; with neither, the current screen | page size in pixels |
| `chipset` | the current document's | `ocs` (12-bit colours; 32 colours in Low Res, 16 in High Res, no Super-High Res) or `aga` (24-bit colours, 256) |
| `colors` (alias `palette_size`) | 32 (16 on OCS High Res) | a power of two from 2 to the mode's maximum |
| `preset` | | `ntsc_lores` (320×200), `pal_lores` (320×256), `pixelart_320x240` (alias `pixel_art_320x240`: PAL Low Res with a custom 320×240 screen), `ntsc_hires` (640×200), `pal_hires` (640×256), `ntsc_interlace` (alias `ntsc_hires_laced`, 640×400), `pal_interlace` (alias `pal_hires_laced`, 640×512); not together with `video`/`resolution`/`interlace` |
| `video`, `resolution`, `interlace` | PAL, `lo`, false | an explicit mode: `pal`/`ntsc`, `lo` (320)/`hi` (640)/`super_hi` (1280, AGA only), laced |
| `name` | | at most 200 characters; shown as the title until saved |

Without a preset or explicit mode the mode follows the size: a standard screen size gets its mode; otherwise the first mode that can show a custom screen of that size, in the order Low Res, High Res, Super-High Res, not laced before laced, PAL before NTSC (so 320×240 is PAL Low Res with a custom 320×240 screen, and the result notes it). Hardware limits are refused, nothing is reduced: e.g. OCS High Res with 32 colours is `PALETTE_RANGE_INVALID` "OCS PAL:High Res shows at most 16 colours at once, not 32. OCS High Res allows 16 colours; use 16 colours, Low Res (32 colours) or AGA (256 colours)."; OCS Super-High Res is `UNSUPPORTED_MODE`. A one-frame document changes with one undoable `SetFormat`, also when the chipset changes (to OCS the kept colours are rounded to 12 bits; `rounded_colors` and a note count them); an animation is replaced by a blank one-frame picture (`undoable: false`, with a note). Result: `{document_id, revision, undoable, document, rounded_colors (only when colours were rounded), notes}`.

**`select_document`** `{document_id}` (`page1`/`page2`). Makes that page the shown one, like Picture > Spare > Swap; selecting the active page changes nothing. `{changed, document_id, document}`.

### 4.2 Palette

**`get_palette`** (no arguments). `{document_id, revision, chipset, size, max_colors, bits_per_channel (4 OCS, 8 AGA), colors: ["#RRGGBB", ...]}`.

**`set_palette_color`** `{index, color}`. One undo step. An index outside the palette is `PALETTE_RANGE_INVALID`. On OCS a colour that is not 12-bit is refused with `HARDWARE_LIMIT`, naming the nearest 12-bit colour (it is not rounded). Result: the edit result (below) plus `index` and `color`.

**Edit result** (drawing tools, `set_palette_color`, `execute_command`, `undo`, `redo`): `{document_id, revision, changed_rect: {x, y, w, h} | null, palette_changed, format_changed, can_undo, can_redo, dry_run (only when true), rounded_colors (only when a set_format to OCS rounded kept colours)}`.

### 4.3 Drawing

All draw with a one-pixel brush in a palette index, on the active layer, one undo step each; coordinates are page pixels and may lie partly outside the page.

| Tool | Parameters |
|---|---|
| `draw_pixel` | `x`, `y`, `color` |
| `draw_line` | `x1`, `y1`, `x2`, `y2`, `color` |
| `draw_rectangle` | `x`, `y` (top left), `width`, `height` (at least 1), `color`, `filled` (default false: a 1-pixel outline) |
| `draw_ellipse` | `cx`, `cy`, `rx`, `ry` (radii in pixels, not corrected for the pixel aspect), `color`, `filled` (default false) |
| `fill_area` | `x`, `y`, `color`: the 4-connected area of the colour at (x, y) takes `color` |

### 4.4 Layers

| Tool | Parameters | Engine command | Notes |
|---|---|---|---|
| `list_layers` | none | | `{document_id, revision, layer_count, active_layer, layers: [{index, name, visible, active}], transparency}`, bottom first |
| `create_layer` | `name?` | `add_layer` | a transparent layer above the active one, in every frame; it becomes active; default name "Layer N" (smallest unused N); 32 layers at most (a 33rd is the engine's refusal, `INVALID_ARGUMENT` "32 layers: at most 32 allowed"; the memory limit of all frames' pictures applies too) |
| `select_layer` | `layer` | `select_layer` | a setting, not an undo step (the revision still moves) |
| `delete_layer` | `layer` | `delete_layer` | the only layer cannot be deleted; when the active layer goes the one below becomes active |
| `set_layer_visibility` | `layer`, `visible` | `set_layer_visible` | hidden layers do not show and are not exported; they can still be painted |
| `rename_layer` | `layer`, `name` | `rename_layer` | 1 to 255 characters, not only spaces, no control characters (non-ASCII shows as `?` in the pixel font) |
| `move_layer` | `from`, `to` | `move_layer` | the others close up; the active layer stays the same layer; a layer moved to the bottom becomes opaque |
| `merge_layer_down` | `layer` (1 or above) | `merge_layer_down` | its pixels other than colour 0 cover the layer below in every frame; the lower layer keeps its name and visibility |

Changing layer tools return `{document_id, revision, changed, undoable, can_undo, can_redo, layer, layer_count, active_layer, layers}`. `changed` is false when the call asked for what already was (selecting the active layer, the same visibility or name, `from == to`): no command runs and no undo step is made. `layer` is the layer the call was about as numbered afterwards. A layer number outside the stack is `LAYER_NOT_FOUND` naming what there is; deleting the only layer and merging layer 0 are `INVALID_ARGUMENT`.

There is no Flatten tool; `execute_command` runs `{"op": "flatten_layers"}` (hidden layers go, every frame becomes one layer showing what it showed).

### 4.5 Undo, redo and raw commands

**`undo`**, **`redo`** (no arguments). Step the active page's history (in the window after ending Stencil > Paint, so they never undo a mask stroke; a generated picture's page details come back with its step). A change `import_image` or `apply_generated_image` made to the spare page is undone after `select_document` of that page. `NOTHING_TO_UNDO` / `NOTHING_TO_REDO`. Edit result.

**`execute_command`** `{command, expected_revision?, dry_run?}` (advanced). Runs one raw `dpx-core` command, given as JSON tagged by `op` (snake_case), as one undo step (`select_layer` and `goto_frame` are settings). Ops: `stroke`, `shape` (`line`, `curve`, `rect`, `ellipse`, `polygon`, `dots`), `flood_fill`, `clear`, `flip`, `replace_color`, `swap_colors`, `set_palette`, `set_format` (`chipset` optional; to OCS the kept colours are rounded and counted in `rounded_colors`), `set_page_size`, `set_page`, `merge_page`, the frame ops (`set_frame_count`, `add_frames`, `copy_frame`, `insert_frame_copies`, `delete_frames`, `goto_frame`, `set_frame_rate`), the layer ops (`add_layer`, `duplicate_layer`, `delete_layer`, `move_layer`, `select_layer`, `set_layer_visible`, `rename_layer`, `merge_layer_down`, `flatten_layers`), `arrange_palette`, `remap_colors`, and also the whole-picture commands `replace_picture`, `set_picture` and `add_picture_layer` (validated like the others; on OCS `replace_picture` needs 12-bit colours). Points and brush pixels are `[x, y]` pairs; a one-pixel brush is `[[0, 0]]`. The `command` object is not strict: a field the op does not know is ignored (a misspelt `colour` for `color`, or `expected_revision` put inside `command`, has no effect and gives no error), so field names must be exact. Example:

```json
{"command": {"op": "shape", "shape": {"kind": "rect", "from": [10, 10], "to": [50, 40]},
             "brush": [[0, 0]], "color": 5, "filled": true}}
```

`dry_run: true` validates and reports what would change without changing anything. The colour stencil (Effect > Stencil) is not applied to MCP commands (menu commands do not use it either); a command can carry its own `stencil`. The area stencil applies in the window (section 3).

### 4.6 Looking

**`get_preview`** `{rect?, scale?, layer?}`. A PNG of the picture as it shows (or of `rect`, clipped to the page), palette applied, nearest-neighbour scaled by `scale` 1–8 (default: up to 4, keeping the longer side within 1024 pixels; at most 4 194 304 pixels). With `layer` it shows that layer alone (shown or hidden): layer 0 opaque, a layer above it with colour 0 transparent (alpha 0 in an RGBA PNG); `layer_note` says so and whether the layer shows. Content: the image block, then `{document_id, revision, rect, scale, image_width, image_height, page_width, page_height, mime_type: "image/png", layer?, layer_note?}`.

**`get_pixels`** `{rect?, encoding?}`. Palette indices of the picture as it shows, at most 65 536 pixels per call (default: the whole page if it has no more). `encoding` `rows` (default: an array of rows) or `base64` (raw index bytes, row by row). `{document_id, x, y, w, h, encoding, pixels, revision}`. There is no `layer` argument.

### 4.7 Files

Paths must lie in an approved directory; relative paths start at the first one. Every path is canonicalised (symlinks followed) and must stay inside; a file to write must not be a symlink; files up to 64 MiB are read. A new file is created exclusively; replacing goes through a temporary file and a rename.

**`open_picture`** `{path}`. Opens an IFF ILBM, IFF ANIM, indexed PNG or `.dpxproj` (with its spare page) and replaces the active page (not undoable; as Picture > Load). True-colour PNG and JPEG are refused with a pointer to `import_image`. An older OCS project whose colours are not 12-bit opens with them as stored and notes which entries `export_image` will refuse and how to fix them. `{document_id, revision, undoable: false, document, notes}`.

**`import_image`** — converts a PNG or JPEG into the document's format (ADR-42's options).

| Parameter | Default | Notes |
|---|---|---|
| `path` or `data` | | a file in an approved directory, or base64 (a `data:` URL is accepted), at most 12 MiB decoded and 64 Mi pixels |
| `target` | `picture` | `picture` (replaces the page, keeps its mode, screen and page size; not undoable), `spare` (the spare page, in the picture's format and palette; one undo step on the spare page; a spare page with a format of its own, from `new_document`, takes the picture's format with it, and undo on that page brings its own format back), `brush` (the custom brush, the picture's palette, its own size; transparent colour 0 when the source has transparency; not undoable: the brush is no document) |
| `position` | `centre` (alias `center`) | or `top_left`; the rest of the page is the BG colour |
| `palette` | `new` for the picture, `current` otherwise | `new` is refused for `spare` and `brush` |
| `colors` | the document's palette size | colours of a new palette: a power of two up to the mode's maximum (on OCS rounded to 12 bits) |
| `dither` | `floyd_steinberg` | or `none`, `ordered` |

`{document_id, revision, target, width, height, at: [x, y], size: [w, h], palette, colors, undoable, notes}`; `document_id` and `revision` are those of the page it went to (the spare page for `spare`). A change of the page during the conversion gives `REVISION_CONFLICT`.

**`save_project`** `{path, overwrite?}`. Writes the active page losslessly as `.dpxproj` (every frame and layer; in the window exactly what Picture > Save writes, without the spare page; headless with the spare page) and marks it saved if it is still the same picture. Another extension is `UNSUPPORTED_FILE_TYPE`.

**`export_image`** `{path, format?, overwrite?}`. IFF ILBM (`.iff`, `.ilbm`, `.lbm`), IFF ANIM (`.anim`, every frame) or indexed PNG (`.png`), by the extension or `format` (`iff_ilbm`, `iff_anim`, `png`; a mismatch with the extension is `INVALID_ARGUMENT`). The document (every frame and layer) is checked against the chipset's limits before any encoder runs (`HARDWARE_LIMIT`; nothing is reduced silently; over MCP this check also covers PNG); the file holds the picture as it shows (visible layers flattened). The notes name a custom or overscan screen (e.g. "320x240 is a custom screen size on PAL:Low Res (standard 320x256); exact hardware compatibility is not guaranteed"), that an IFF file stores no chipset (an OCS picture reopens as AGA), and how many layers were flattened.

Both return `{document_id, revision, path, format, bytes, notes}`. An existing file is `FILE_EXISTS` unless `overwrite: true`; then the user is asked, unless "Overwrite files without asking" is on or the server is headless (section 7).

### 4.8 Image generation

Only in the running window, with a generator set up by the user (Prefs > AI & MCP.. > Set up.., or Picture > Generate.. > Set up..). They share the Generate requester's generator, key store, job manager and conversion. Jobs that MCP clients start are visible to MCP only (the requester's own jobs are `JOB_NOT_FOUND` for MCP). The plans of the 16 newest MCP jobs are kept, but the shared job manager keeps only its 8 newest finished jobs (the requester's and MCP's together). A job's picture stays with its plan once `get_generation_status` or `apply_generated_image` has fetched it; a finished job whose picture was never fetched is lost when the job manager drops it, and then gives `JOB_NOT_FOUND` "gen-N is no longer kept: generate it again" although its plan is among the 16.

**`generate_image`** — starts a job and answers at once. **The request is sent immediately**: the description goes over the internet to the provider and is billed to the user's own account (section 8).

| Parameter | Default | Notes |
|---|---|---|
| `prompt` | (required) | 1–1000 characters; a style sentence is added |
| `preset`, `width`, `height` | the Generate requester's resolution of this session, else the shown picture's display mode and screen size | as `create_document` (320×240: PAL Low Res with a custom screen) |
| `chipset` | as above | `ocs` or `aga` |
| `colors` (alias `palette_size`) | the requester's choice, else the picture's palette size, lowered to the mode's maximum | a power of two up to the mode's maximum |
| `dithering` | `none` | `none`, `ordered`, `floyd_steinberg` (alias `diffusion`) |
| `palette` | `new` | `new` or `current` (the picture's palette; refused if the mode cannot show it) |
| `style` | `pixel_art` | `pixel_art`, `painted`, `none` |
| `background` | `opaque` | or `transparent` (transparent pixels become colour 0) |
| `target` | `spare` | `spare`, `picture`, `brush`, `new_layer`, `new_document` (the spare page with a format of its own) |

Options left out take the Generate requester's choices of this session when the user has made any (typed a description or changed an option there; opening it and closing it unchanged is no choice), else the defaults in the table; this holds for the resolution (preset or size), chipset, colours, dithering, palette, style and background, not for `target`, which is always `spare` when left out (the requester's Target does not change it). Framing and scaler have no arguments: they are the requester's (default Crop and the Pixel art scaler). `new_layer` takes the picture's page, chipset and palette: any `preset`, a `width`, `height`, `chipset` or `colors` other than the picture's, or `palette: new` is `INVALID_ARGUMENT`, a picture with 32 layers, or one whose frames have no memory left for another layer (all frames' pictures stay within 128 MiB, the limit `create_layer` meets), `LAYER_LIMIT`, a palette its mode cannot show `PALETTE_RANGE_INVALID` — all before anything is sent. Result: `{job_id ("gen-N"), status: "running", target, width, height, chipset, display_mode, colors, palette, dithering, style, background, provider: {id, name, network, model}, cost, notes}`.

**`get_generation_status`** `{job_id}`. `{job_id, status: "running" | "done" | "failed" | "cancelled", target, elapsed_seconds, provider}`; failed: `error: {code, message, retryable}`; cancelled: a billing note; done: a PNG image block of the converted picture (palette colours, integer-scaled) and `width`, `height`, `chipset`, `colors_used`, `palette_size`, `palette_bits` (4 OCS, 8 AGA), `preview_scale`, `rounded_colors` (see below), `notes`. For the brush target it shows the brush as Apply makes it (fitted to the shown picture's palette). For `new_layer` it shows the new layer alone, as `get_preview` with `layer` shows a layer above the bottom one: an RGBA PNG whose colour 0 is transparent (alpha 0), where the layers below will show; a note says so.

**`apply_generated_image`** `{job_id, target?}` (default: the target `generate_image` was given). Applies the finished picture as the Generate requester's Apply does, as one undo step where it lands: `spare`/`new_document` (the spare page becomes the picture in the format it was made for; asks first when the spare page holds a picture), `picture` (replaces every frame and layer; always asks first), `brush` (fitted to the picture's palette; no question, not undoable: the brush is no document), `new_layer` (a layer "Generated", "Generated 2", … above the active one, picture in the current frame only; no question). `{job_id, target, document_id, revision, width, height, colors_used, undoable, document, rounded_colors, notes}`. Unfinished: `JOB_NOT_DONE`; the user keeps the picture or nobody answers within 45 s: `APPLY_DECLINED`; the page changed in a way the conversion depends on: `REVISION_CONFLICT`.

**`cancel_generation`** `{job_id}`. Stops waiting for the job (needs Read only). A request already sent may still be billed. `{job_id, status, cancelled, notes}`; `status` is the state `get_generation_status` reports (a picture already fetched stays `done` and appliable).

Rounded colours (ADR-46): with `palette: current` for an OCS target, entries of the current palette that are not 12-bit (an AGA picture's, or an older OCS project's) are rounded. `get_generation_status` and `apply_generated_image` then give `rounded_colors` and a note such as "2 colours rounded to 12-bit OCS values of the current palette for OCS", as the Generate requester's Apply note ("Picture generated") does; both leave `rounded_colors` out when nothing was rounded. Only the targets that take the result's palette count it (`spare`, `new_document`, `picture`); `new_layer` and `brush` keep the picture's palette as it is stored and are matched against those colours, not against rounded ones. On an older OCS project whose colours are not 12-bit their notes name those entries (as `open_picture` does: `export_image` refuses the picture until they are 12-bit) and how to fix them, and `get_generation_status` gives `palette_bits` 8; on a 12-bit OCS palette the notes say "OCS: every palette entry is a 12-bit colour" and `palette_bits` is 4.

At most **2** requests are in flight at once for the requester and MCP together (cancelled ones count until the provider has answered); a third gets `BUSY`.

## 5. Resources

| URI | MIME | Content |
|---|---|---|
| `dpx://document/info` | `application/json` | the active page as `get_document_info` describes it |
| `dpx://document/palette` | `application/json` | as `get_palette` |
| `dpx://document/preview.png` | `image/png` | the picture as `get_preview` shows it by default (base64 blob) |

Reading a resource needs the Read permission and takes its turn in arrival order like a tool call. No subscriptions. A failed read is a JSON-RPC error, not a result: an unknown URI is MCP's resource-not-found error (`-32002`) with the message "no resource …"; any other failure (e.g. reading turned off) is an invalid-request error whose message starts with the code (`PERMISSION_DENIED: reading the document is turned off …`).

## 6. Error codes

| Code | Meaning |
|---|---|
| `INVALID_ARGUMENT` | a bad or unknown argument, a refused command (also a 33rd layer from `create_layer` or `add_layer`) |
| `PALETTE_RANGE_INVALID` | a colour count or index the mode, chipset or palette does not allow; a colour that is not 12-bit on OCS in a raw `set_palette` or `replace_picture` of `execute_command` |
| `HARDWARE_LIMIT` | `set_palette_color` with a colour that is not 12-bit on OCS; an `export_image` beyond the chipset's or mode's rules |
| `UNSUPPORTED_MODE` | a mode the chipset cannot show (Super-High Res on OCS) |
| `DOCUMENT_NOT_FOUND` | a `document_id` other than `page1`/`page2`, or a spare page not used yet |
| `LAYER_NOT_FOUND` | a layer number outside the stack |
| `LAYER_LIMIT` | `generate_image` or `apply_generated_image` with the target `new_layer` for a picture that has 32 layers or no memory left for another layer |
| `REVISION_CONFLICT` | `expected_revision` did not match, or the page changed while an import or a generated picture was converted |
| `NOTHING_TO_UNDO`, `NOTHING_TO_REDO` | empty history |
| `PERMISSION_DENIED` | reading or changing is turned off for AI clients (Prefs > AI & MCP) |
| `FILE_ACCESS_OFF` | no approved directory |
| `PATH_NOT_ALLOWED` | a path outside the approved directories |
| `FILE_EXISTS` | the file exists and `overwrite` was not given |
| `OVERWRITE_DECLINED` | the user kept the file, or nobody answered within 45 s |
| `FILE_NOT_FOUND`, `DIR_NOT_FOUND`, `NOT_A_REGULAR_FILE` | the path does not lead to a usable file or directory |
| `INVALID_FILE`, `UNSUPPORTED_FILE_TYPE` | a file that cannot be read, or a format the tool does not take |
| `IO_ERROR` | the file system refused |
| `EDITOR_BUSY` | a requester is open in the window, or the user has been painting for 10 s (changes only; reading works) |
| `APP_BUSY` | the app did not take the call within 60 s; it never runs later |
| `APP_UNAVAILABLE` | the server stopped, or (bridge) the app runs but MCP control is off or does not answer |
| `APP_NOT_RUNNING` | (bridge) DeluxePainter is not running and is not started |
| `APP_CONNECTION_LOST` | (bridge) the connection broke during a changing call, which may have run |
| `RESOURCE_NOT_FOUND` | an unknown resource URI (sent as the JSON-RPC resource-not-found error, section 5) |
| `IMAGE_GENERATOR_NOT_CONFIGURED` | no generator set up, or OpenAI without a key; nothing was sent |
| `IMAGE_GENERATION_UNAVAILABLE` | the headless server (no generator) |
| `JOB_NOT_FOUND`, `JOB_NOT_DONE` | an unknown or no longer kept job; a job still running |
| `APPLY_DECLINED`, `APPLY_FAILED` | the user kept the picture or did not answer; Apply failed in the window |
| `BUSY` | two generation requests are already in flight |
| `INVALID_REQUEST`, `AUTH_FAILED`, `RATE_LIMITED`, `QUOTA_EXCEEDED`, `CONTENT_POLICY`, `NETWORK`, `TIMEOUT`, `SERVER_ERROR`, `BAD_RESPONSE` | the provider's errors (at once from `generate_image`, or in a failed status's `error` with `retryable`) |
| `INTERNAL_ERROR` | a bug (a panicking job: the call fails, the editor state is kept and the app keeps running) |

## 7. Security model

- **Off by default, local only.** The server runs only while MCP control is on (Prefs > AI & MCP.., or `--mcp` for one session) and listens on `127.0.0.1` only.
- **Every HTTP request is checked.** `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (against DNS rebinding); any `Origin` header is refused (browsers send one, local MCP clients do not); `Authorization: Bearer` with the 32-byte random token, compared in constant time; bodies over 16 MiB are refused.
- **The token never reaches a client configuration.** It lives in `mcp-token` and in the endpoint file (0600 on Unix); the bridge reads the endpoint file. It is not shown on screen, not in the `claude mcp add` command, not on the clipboard, not in a project or `settings.json`. The bridge sends it only to a server that holds the endpoint lock, through no proxy. A crash leaves an endpoint file whose token no longer works.
- **Permissions, applied to the next call:** Read document info (off: only `get_app_status` answers), Edit pictures and palettes (needs Read), Overwrite files without asking (needs both). File tools work only in the approved directories (default `~/Pictures/DeluxePainter`).
- **Overwriting asks.** An existing file is replaced only with `overwrite: true`, and then the user is asked in the window: "Replace File?" — "Claude wants to replace NAME. Replace?" with Replace and Keep. Return and Esc keep the file; only a click replaces it; no answer within 45 s keeps it (`OVERWRITE_DECLINED`). "Overwrite files without asking" skips the question.
- **Generated pictures never replace work unasked.** `apply_generated_image` to the picture always asks ("Replace Picture?"), to the spare page when it holds a picture ("Replace Spare Page?"); a new layer and the brush replace nothing. This is independent of the overwrite setting.
- **Revoke access** (Prefs > AI & MCP..) writes a new token (the old one and the endpoint file's old one get 401 at once) and turns the three permissions off. **Off** stops the server and keeps the bridge from starting the app. **Disconnect** removes the Claude Code entry (`claude mcp remove deluxe-painter -s user`, after asking); a Claude Desktop extension is removed in Claude Desktop's Settings > Extensions.
- **No shell.** No tool runs a shell command. The connect flows in Prefs > AI & MCP.. run Claude Code's `claude` (a read-only `claude mcp get` check first; `claude mcp add` or `remove` only after the user confirmed the exact command shown) and the OS opener for the `.mcpb` (Claude Desktop then shows its own install dialog), all without a shell.
- **The honest limit.** The token keeps out the network, browsers and other users of the computer, not the user's own programs: any process running as the same user can read the token file (as it can read the user's pictures and `settings.json`) and call the app. Revoke access therefore also turns the permissions off.

## 8. Image generation: cost and consent

- Nothing is generated until the user has chosen a generator and given it a key (OpenAI Images is the only provider; default `none`). Without one, the tools answer `IMAGE_GENERATOR_NOT_CONFIGURED` and nothing is made up; the test pattern used by tests and snapshots is never offered to users.
- OpenAI bills every picture to the user's own API account; a request already sent may still be billed when it is cancelled (UNVERIFIED whether it is; some texts assume it is). There is no free tier and DeluxePainter quotes no price (OpenAI does not publish a per-picture price for the current models).
- In the window's Generate requester the cost note is shown before Generate. **Over MCP the request is sent at once, without a question in the app**: the cost and the network use are stated beforehand in `generate_image`'s description and its `openWorldHint` (a hint clients may use when they ask the user to approve a tool call; how real clients show it was never checked, section 10), the provider's cost note is in the result's `cost`, and the activity log names every request. Whether the app should also ask (once per session or per request) is an open owner decision (ADR-45, "Integration (wave 8)").
- The API key is never part of a result, a log line or an error; provider messages are cleaned (anything like an `sk-` key fragment is replaced by `[key]`).

## 9. Testing

Gate (as CI runs it): `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

| Target | What it covers |
|---|---|
| `cargo test -p dpx-mcp` | tools, arguments, permissions, hardware rules, files, resources, the sequencer, job queue, endpoint files, HTTP guard; the generation tools with `--features imagegen` (as the workspace builds it; without the feature they are unavailable after the argument checks) |
| `cargo test -p deluxepainter --test mcp_stdio` | the real `deluxepainter mcp` over stdio: handshake, tools, files, arrival order |
| `cargo test -p deluxepainter --test mcp_http` | the real `deluxepainter mcp --http`: 401/403/413, arrival order, approved directory, SIGTERM, the bridge forwarding to it and starting the app |
| `cargo test -p dpx-mcp-bridge` | the real bridge binary against the HTTP server: forwarding, reconnect after a restart, stale endpoint files, proxies ignored, MCP control off |
| `cargo test -p deluxepainter mcp` | ui-loop tests of the GUI workspace (`mcp::tests`, the generation tools in `mcp::tests::gen_tests`, Prefs > AI & MCP.. in `mcp_prefs::tests`): calls landing in the window's document, `U`, busy rules, the questions, the real HTTP server starting and stopping, the connect flows with a fake `claude` |
| `cargo test -p dpx-imagegen` | provider, request body, error mapping, keys (in-memory and fake keychains only), jobs, against a local HTTP server; never the network |
| `cargo run -p xtask -- mcpb` | builds the release bridge and `target/mcpb/deluxepainter-<os>-<arch>.mcpb` (xtask's own tests check the bundle's layout and manifest) |

Snapshot flags (real renders of the pixel UI, `deluxepainter --snapshot out.png ...`): `--mcp-calls FILE.json` runs a JSON array of `{"tool": NAME, "arguments": {...}}` on the GUI workspace first (read and edit allowed, no file access; each result on stderr; e.g. `docs/design/screenshots/wave8-mcp-gui-calls.json`, `wave8-mcp-layers.json`); `--mcp-prefs STATE` shows Prefs > AI & MCP.. (`off`, `ready`, `connected`, `fallback`, `error`, `code-confirm`, `code-replace`, `code-missing`, `desktop-missing`, `disconnect`, `generator`, `imagegen-setup`); `--mcp-question NAME` shows the "Replace File?" question; `--mcp-gen-question picture|spare` shows the question before an MCP Apply. Example:

```bash
cargo run -p deluxepainter -- --snapshot out.png --scale 2 \
  --mcp-calls docs/design/screenshots/wave8-mcp-gui-calls.json --mcp-prefs connected
```

**A manual check without a window** (the headless HTTP server and the bridge, with a scratch config directory; Unix shell):

```bash
cargo build -p deluxepainter -p dpx-mcp-bridge
export DPX_CONFIG_DIR=$(mktemp -d)
mkdir -p "$DPX_CONFIG_DIR/pictures"
target/debug/deluxepainter mcp --http --port 0 --allow-dir "$DPX_CONFIG_DIR/pictures" &
SERVER=$!
sleep 1
printf '%s\n' \
 '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"manual","version":"1"}}}' \
 '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
 '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_document","arguments":{"width":320,"height":240,"chipset":"ocs","colors":32}}}' \
 '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"draw_rectangle","arguments":{"x":10,"y":10,"width":50,"height":30,"color":5,"filled":true}}}' \
 '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"export_image","arguments":{"path":"box.iff"}}}' \
 | target/debug/dpx-mcp-bridge
kill $SERVER
```

Each answer is one JSON line on stdout; `box.iff` (2984 bytes, with the custom-screen and chipset notes in the answer) lands in `$DPX_CONFIG_DIR/pictures`, and the endpoint file is gone after the `kill`. This exact sequence was run on macOS (Apple Silicon, debug build of main at `cc920a7`) for this page. With the window instead: start `target/debug/deluxepainter --mcp` (with the same `DPX_CONFIG_DIR`; add `--no-settings` to leave your settings alone), and the same lines through the bridge draw into the window; files then go to the approved folder Prefs > AI & MCP.. shows (`~/Pictures/DeluxePainter` by default, created when the server starts), not to `--allow-dir`. In the window MCP calls go to the shown picture; the image generation tools need a generator set up in that window.

**The live OpenAI test** (ignored by default: it needs a key and the internet, and it costs money, one picture at quality `low`):

```bash
read -rs OPENAI_API_KEY && export OPENAI_API_KEY   # paste your key, then Return (not shown, not in the history)
cargo test -p dpx-imagegen --test openai_live -- --ignored --nocapture
```

`DPX_OPENAI_MODEL` picks another model id. The key is read from `OPENAI_API_KEY` only and kept in memory (the keychain is not touched); without it the test sends nothing and passes. It asks for one 1024×768 picture (quality `low`) for an OCS 320×240 page, converts it with the Pixel art scaler, checks the result (exactly 320×240 pixels, a 32-entry palette of 12-bit colours, every pixel inside it) and writes it to `target/tmp/openai_live_320x240.png`. It has never been run. OpenAI bills the picture to that key's account.

## 10. Verified and not verified

Verified (macOS on Apple Silicon, `cargo test --workspace` and debug runs; details in the ADRs): the tools and resources through the headless workspace and through the GUI workspace driven by the real `ui()` loop; the real `deluxepainter mcp` over stdio and `--http`; the real bridge binary against the HTTP server; a debug run of the real GUI with `--mcp` driven over HTTP by a Python client and through `deluxepainter mcp-bridge` (endpoint, token and launch files 0600); the bundled bridge from `cargo run -p xtask -- mcpb` answering `initialize` and `tools/list` and forwarding to a debug run of the GUI; the generation tools with the debug test pattern and gated test providers only. The fixes of the wave-8 seam review (how the layer and generation strands work together; ADR-45 "Integration (wave 8)", "Seam review") each have a test that fails without the fix. Over the headless `deluxepainter mcp` (stdio, debug build, a Python client) the brush and spare-page results of `import_image`, `create_document`'s `rounded_colors` and a 150-character non-ASCII name, and the instructions were checked, and the `.mcpb` manifest built by `cargo run -p xtask -- mcpb` (with a placeholder bridge file) lists `generate_image`'s whole first sentence; the window-only fixes ran in the Rust tests only.

Not verified:

- Real MCP clients: Claude Code, Claude Desktop and MCP Inspector never connected, neither through the bridge nor directly over HTTP; whether their HTTP stacks send an `Origin` header (it would get 403); how they show `openWorldHint` and tool descriptions; whether they wait 45 s for a question. No real client called the layer or generation tools.
- The bridge forwarding the layer and generation tools to a running window: the debug runs of the bridge against the GUI were made with ADR-43's 21 tools, before the integration strands; the bridge lists the 33 tools from the same `tools::definitions` as the app.
- Claude Desktop installing the `.mcpb` (unsigned, with an unsigned and not notarized macOS binary), and `mcpb validate`; a real `claude mcp add/get/remove` (the connect flows were tested with a fake `claude`).
- A real OpenAI request (no key was available): latency, price, transparent backgrounds, whether a cancelled request is billed, real error texts; HTTPS through native TLS; the real system keychain on any OS.
- Windows and Linux: builds and tests there, file permissions of the token and endpoint files, the lock files, the Claude Desktop and `claude` paths, opening a `.mcpb`.
- The stateless MCP `2026-07-28` protocol era (only `initialize`-based clients were exercised).
- The window as a person sees it: calls arriving in a visible window, the questions clicked by a person, the layer panel and the requesters by hand (the owner declines remote control; ui-loop tests and snapshots only).
