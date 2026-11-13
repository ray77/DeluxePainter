# DeluxePainter - Dependency Research (P0-DEPS)

Retrieval date: **2026-11-09**. Toolchain under test: rustc/cargo 1.90.0 (Homebrew, macOS arm64). `rustup` is **not** installed (`which rustup` found nothing). `brew info rust` reports stable 1.99.1 as available (not installed; nothing was installed during this research).

Method: `cargo search`, the crates.io API (`https://crates.io/api/v1/crates/<name>`: version, `rust_version`, license), GitHub raw `Cargo.toml` per tag, and the MCP spec at modelcontextprotocol.io. In addition, a scratch project with `rust-version = "1.90"` and the recommended set was resolved (`cargo generate-lockfile`) and type-checked (`cargo check`) on 1.90.0 - it succeeded (see "Verification").

## 1. Crate table

"Recommended" = what resolves/builds on rustc 1.90. "Latest" = newest stable on crates.io. Licenses are as declared on crates.io.

| Crate | Recommended | Latest | MSRV (rec.) | License | Purpose | Source | Retrieved |
|---|---|---|---|---|---|---|---|
| eframe | **0.33.3** | 0.37.0 (MSRV 1.95) | 1.88 | MIT OR Apache-2.0 | App host (winit window, renderer, event loop) | https://crates.io/crates/eframe | 2026-11-09 |
| egui | **0.33.3** (via eframe) | 0.37.0 (MSRV 1.95) | 1.88 | MIT OR Apache-2.0 | Immediate-mode core, `Context`, textures, painter | https://crates.io/crates/egui | 2026-11-09 |
| egui_extras | not needed | 0.37.0 | - | MIT OR Apache-2.0 | Only for image loaders/tables; we upload textures ourselves | https://crates.io/crates/egui_extras | 2026-11-09 |
| wgpu (transitive) | 27.0.1 (with eframe 0.33.3) | 30.0.1 | 1.87 | MIT OR Apache-2.0 | Optional eframe renderer | https://crates.io/crates/wgpu | 2026-11-09 |
| image | **0.25.10** | 0.25.10 | 1.88 | MIT OR Apache-2.0 | Decode/encode PNG/JPEG etc. (use `default-features = false`, pick formats) | https://crates.io/crates/image | 2026-11-09 |
| png | **0.18.1** | 0.18.1 | 1.73 | MIT OR Apache-2.0 | Direct PNG control (indexed, chunks, metadata) | https://crates.io/crates/png | 2026-11-09 |
| jpeg-decoder | avoid | 0.3.2 | 1.61 | MIT OR Apache-2.0 | Older JPEG decoder; superseded by zune-jpeg (which `image` uses) | https://crates.io/crates/jpeg-decoder | 2026-11-09 |
| zune-jpeg | **0.5.15 (used directly since ADR-9)** | 0.5.16-rc3 (pre-release) | 1.75 | MIT OR Apache-2.0 OR Zlib | JPEG decode in `dpx-formats` (default features: std + x86/NEON SIMD); `image` is not used | https://crates.io/crates/zune-jpeg | 2026-11-10 |
| zip | **9.5.0** | 9.5.0 | 1.88 | MIT | Project container (.zip) | https://crates.io/crates/zip | 2026-11-09 |
| zstd | **0.14.3** | 0.14.3 (0.13.4 also published) | 1.64 | BSD-3-Clause (wrapper; bundles Zstandard C lib, BSD-3/GPLv2 dual - used under BSD) | Compression of layer data | https://crates.io/crates/zstd | 2026-11-09 |
| serde | **1.0.229** | 1.0.229 | 1.56 | MIT OR Apache-2.0 | Serialization framework | https://crates.io/crates/serde | 2026-11-09 |
| serde_json | **1.0.154** | 1.0.154 | 1.71 | MIT OR Apache-2.0 | JSON | https://crates.io/crates/serde_json | 2026-11-09 |
| ciborium | **0.2.2** | 0.2.2 (last release 2024-01) | 1.58 | Apache-2.0 | CBOR | https://crates.io/crates/ciborium | 2026-11-09 |
| thiserror | **2.0.21** | 2.0.21 | 1.77 | MIT OR Apache-2.0 | Library error enums | https://crates.io/crates/thiserror | 2026-11-09 |
| anyhow | **1.0.104** | 1.0.104 | 1.68 | MIT OR Apache-2.0 | Application-level errors | https://crates.io/crates/anyhow | 2026-11-09 |
| uuid | **1.29.2** | 1.29.2 | 1.89 | Apache-2.0 OR MIT | IDs for layers/objects | https://crates.io/crates/uuid | 2026-11-09 |
| schemars | **1.2.3** | 1.2.3 | 1.74 | MIT | JSON Schema for MCP tool I/O (JSON Schema 2020-12) | https://crates.io/crates/schemars | 2026-11-09 |
| tokio | **1.54.1** | 1.54.1 | 1.85 | MIT | Async runtime for rmcp / sockets | https://crates.io/crates/tokio | 2026-11-09 |
| rmcp | **3.9.0** | 3.9.0 | 1.88 | Apache-2.0 | Official MCP Rust SDK | https://crates.io/crates/rmcp | 2026-11-09 |
| interprocess | not used (ADR-43: the bridge speaks Streamable HTTP to the app) | 2.4.8 | 1.75 | 0BSD OR Apache-2.0 | Unix domain sockets / Windows named pipes (editor <-> MCP server IPC) | https://crates.io/crates/interprocess | 2026-11-09 |
| proptest | **1.12.0** | 1.12.0 | 1.88 | MIT OR Apache-2.0 | Property tests (dev-dependency) | https://crates.io/crates/proptest | 2026-11-09 |
| rfd | **0.17.2** | 0.18.0 (MSRV 1.92) | not declared (resolves fine on 1.90) | MIT | Native open/save file dialogs | https://crates.io/crates/rfd | 2026-11-09 |
| directories | **6.0.0** | 6.0.0 | not declared | MIT OR Apache-2.0 | Config/data dir locations per OS | https://crates.io/crates/directories | 2026-11-09 |

### MSRV verdict for rustc 1.90.0

- **Blocked on 1.90:** eframe/egui **0.34 - 0.37** (0.34-0.35 need 1.92; 0.36-0.37 need 1.95) and rfd **0.18.0** (needs 1.92). Newest eframe usable on 1.90 is **0.33.3** (MSRV 1.88, released 2025-12-11).
- Everything else, including rmcp 3.9.0 (1.88), zip 9.5.0 (1.88), uuid 1.29.2 (1.89), image 0.25.10 (1.88), proptest 1.12.0 (1.88), builds on 1.90.
- To use eframe 0.37 / egui 0.37: **rustc >= 1.95** is required (the egui repo pins `rust-toolchain` to 1.95.0). rfd 0.18 needs >= 1.92. Upgrade path: `brew upgrade rust` (stable 1.99.1 is offered) or install rustup. Not done here.
- Note: with `rust-version = "1.90"` in Cargo.toml and `resolver = "3"` (edition 2024), cargo 1.90 automatically picks MSRV-compatible versions ("Locking 501 packages to latest Rust 1.90 compatible versions").

### License compatibility for a public GitHub repo

**Decision (2026-11-09):** the project is licensed under **PolyForm Noncommercial 1.0.0** (see `LICENSE.md`). The table below was written while the license was still open; its conclusion holds: all direct dependencies are permissive and may be combined with a noncommercial source-available project, provided their license texts/notices are shipped (`cargo about` / `cargo deny` before the first binary release).

| Finding | Detail |
|---|---|
| No crate forces a copyleft license | All direct dependencies are permissive (MIT, Apache-2.0, BSD-3-Clause, 0BSD, Zlib). None is GPL/LGPL/AGPL/MPL. |
| Apache-2.0-only crates: **rmcp**, **ciborium** | Fine inside an MIT/Apache-2.0 project and inside a GPL-3.0 project. Apache-2.0 is compatible with GPL-3.0 but **not** with GPL-2.0-only. If the project were MIT-only, the combined binary still carries the Apache-2.0 terms (NOTICE/patent clause) - acceptable, just keep third-party notices. |
| MIT-only crates: zip, schemars, tokio, rfd | Compatible with everything above. |
| zstd | Wrapper is BSD-3-Clause (0.13.3 and earlier were MIT); the bundled C library is BSD-3-Clause/GPL-2.0 dual-licensed, used under BSD. No GPL obligation. |
| Transitive license set | Not audited by name. Before publishing run `cargo deny check licenses` or `cargo about` (not installed; do not auto-install) to generate THIRD-PARTY notices. eframe pulls fonts (Ubuntu-Light, Hack, emoji icons) under OFL/Ubuntu Font Licence/MIT - redistribution is fine but notices are needed if `default_fonts` stay on. |
| If the project chose GPL-3.0 | All of the above are GPL-3.0 compatible (Apache-2.0 and MIT/BSD are one-way compatible into GPL-3.0). Choosing GPL-3.0 would make the project less reusable but is not forced by any dependency. |

## 2. eframe/egui pixel rendering

Facts below were checked in the source of tags 0.33.3 and 0.37.0.

**Backend defaults (important difference).**
- eframe **0.33.3** default features include `glow` (OpenGL via glutin) - `wgpu` is opt-in (`features = ["wgpu"]`, "overrides glow").
- eframe **0.37.0** default features include `wgpu`; `glow` is opt-in. wgpu 30 is used there, wgpu 27.0.1 with eframe 0.33.3.
- For a pixel editor that only paints textured quads either works. glow = fewer/lighter dependencies, on macOS it uses OpenGL (deprecated by Apple but functional); wgpu = Metal on macOS, D3D12/Vulkan elsewhere, heavier build. Recommendation: start with eframe's default (glow) on 0.33.3; switch to `wgpu` if GL issues arise. Select at runtime/feature via `NativeOptions::renderer` if both enabled.

**API drift to expect when moving 0.33 -> 0.37.** In 0.33.3 `App` has `fn update(&mut self, ctx: &egui::Context, frame: &mut Frame)`. In 0.37.0 the trait has `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame)` plus an optional `logic(&mut self, ctx, frame)`. Keep app code behind a thin adapter so the upgrade is mechanical.

**Nearest filtering.** `egui::TextureOptions::NEAREST` (both min and mag nearest, `ClampToEdge`, no mipmaps; constant exists in both versions):

```rust
let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba_bytes);
let tex = ctx.load_texture("canvas", img, egui::TextureOptions::NEAREST);
// update later in place: tex.set(new_image, egui::TextureOptions::NEAREST);
```

**Pixels per point / native pixel size.**
- `ctx.pixels_per_point() -> f32` current scale; `ctx.set_pixels_per_point(f32)` overrides it; `ctx.native_pixels_per_point() -> Option<f32>` is the OS-reported scale (e.g. 2.0 on Retina). Physical pixel size of the drawable area = `ctx.content_rect()` (or `screen_rect`) size in points x `pixels_per_point()`.
- Per-frame input also exposes the viewport info: `ctx.input(|i| i.viewport().native_pixels_per_point)`.

**Full-window texture with integer scaling (recipe).**
1. Let `ppp = ctx.pixels_per_point()`; available physical size `avail_px = rect_points.size() * ppp`.
2. `scale = floor(min(avail_px.x / canvas_w, avail_px.y / canvas_h)).max(1)` (integer).
3. Size in points = `canvas_size * scale / ppp` (so each canvas pixel covers exactly `scale` physical pixels); center it and round the min corner to a physical pixel: `(p * ppp).round() / ppp`.
4. Paint: `ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(pos2(0.,0.), pos2(1.,1.)), Color32::WHITE)`. Use `Painter::rect_filled` for letterbox bars. Keep `Color32::WHITE` tint (no premultiplied-alpha surprises).
5. Use `egui::CentralPanel::default().frame(Frame::NONE)` so no panel padding shifts the image off the pixel grid.
6. Avoid non-integer `ppp` (e.g. 1.25 on Windows): integer canvas scale cannot be pixel-exact there; report effective scale in UI or force `set_pixels_per_point` to an integer.

**Viewport/icon setup.**
```rust
let icon = egui::IconData { rgba: rgba_bytes, width: 256, height: 256 };
let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([640.0, 400.0])
        .with_title("DeluxePainter")
        .with_icon(std::sync::Arc::new(icon)),
    ..Default::default()
};
eframe::run_native("DeluxePainter", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
```
`with_icon` takes `impl Into<Arc<IconData>>`; also available: `with_resizable`, `with_fullsize_content_view`, `with_titlebar_shown`, `with_title_shown`. `egui::include_image!`-style loaders live in `egui_extras` and are not needed. macOS dock icon from `with_icon` works only at runtime; bundle `.icns` for packaged apps.

**Other.** `persistence` feature (ron/serde) is optional and not needed if the app stores state itself. `accesskit`, `wayland`, `x11` are default features and are needed on Linux.

## 3. rmcp MCP server

- **Version:** rmcp **3.9.0** (2026-11-05), license **Apache-2.0**, MSRV **1.88** (workspace `rust-version`). Repo: https://github.com/modelcontextprotocol/rust-sdk ; docs: https://rust.sdk.modelcontextprotocol.io/ and https://docs.rs/rmcp. 3.x is a major jump from the older 0.x series; the README links a 3.x migration guide, so old blog posts/snippets may be wrong.
- **Protocol support:** implements MCP **2026-07-28** and stays compatible with 2025-11-25 and earlier (per README).
- **schemars:** rmcp depends on `schemars = "1.0"` (dev-dependencies use 1.1); resolved: **1.2.3**. Use `schemars` 1.x, not 0.8. rmcp re-exports it as `rmcp::schemars`. Schemas are JSON Schema 2020-12.
- **Default features:** `["base64", "macros", "server"]`. Relevant flags:
  - `server` - server side (pulls schemars, uuid, pastey)
  - `macros` - `#[tool]`, `#[tool_router]`, `#[tool_handler]`, `#[prompt]`, ... (default)
  - `transport-io` - **stdio** transport (`rmcp::transport::stdio`)
  - `transport-streamable-http-server` - Streamable HTTP server as a Tower service (needs an HTTP stack such as axum/hyper in the app; pulls `server-side-http`)
  - `transport-async-rw` - generic AsyncRead/AsyncWrite transport (included by `server`; usable for our own local-socket/named-pipe stream)
  - `transport-child-process`, `transport-streamable-http-client-reqwest` (+ `reqwest` TLS flags) - client side
  - `transport-worker` - in-process transport (tests)
  - also: `elicitation`, `auth`, `request-state`, `skills-fs`, `local`.
- **Tool API (current):** `#[tool_router]` on an `impl` block with methods marked `#[tool(description = "...")]`; parameters arrive as `Parameters<T>` where `T: Deserialize + schemars::JsonSchema`. For a tools-only server, `#[tool_router(server_handler)]` generates the `ServerHandler` impl. Otherwise keep a `tool_router: ToolRouter<Self>` field initialized with `Self::tool_router()` and add `#[tool_handler]` (optionally with `name`, `version`, `instructions`) on `impl ServerHandler`. Methods may return `String`, `Result<CallToolResult, McpError>` (`McpError = rmcp::ErrorData`), or typed/structured output; `CallToolResult::success(vec![ContentBlock::text(..)])`, `CallToolResult::error(..)`. A `RequestContext<RoleServer>` parameter is also accepted.
- **Resources:** implement `list_resources`, `read_resource` (and optionally `list_resource_templates`) on `ServerHandler`; declare the capability in `get_info()` with `ServerCapabilities::builder().enable_resources()` (also `.enable_tools()`, `.enable_prompts()`). Contents: `ResourceContents::text(text, uri)` / `ResourceContents::blob(base64, uri)`; unknown URI -> `McpError::resource_not_found`. Change events: `notify_resource_list_changed`, `notify_resource_updated`. URI templates like `scheme://{id}/x`.
- **Prompts:** `#[prompt_router]` on an impl block, `#[prompt(name = "...", description = "...")]` per method, `#[prompt_handler]` on `impl ServerHandler`, with a `prompt_router: PromptRouter<Self>` field (`Self::prompt_router()`). Argument structs derive `JsonSchema` (`#[schemars(description = ...)]`); return `Vec<PromptMessage>`, `GetPromptResult` or `Result<_, McpError>`. Note: MCP types prompt arguments as string maps, so use lenient deserialization for non-string args (the official example does this).
- **Minimal stdio server skeleton** (compiled successfully on rustc 1.90.0 with `rmcp = { version = "3.9", features = ["server", "transport-io"] }`, `tokio` (rt-multi-thread, macros), `serde` (derive), `schemars = "1"`, `anyhow`):

```rust
use rmcp::{ServiceExt, handler::server::wrapper::Parameters, tool, tool_router, transport::stdio};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct SumArgs { /** first operand */ a: i32, /** second operand */ b: i32 }

#[derive(Clone)]
struct Painter;

#[tool_router(server_handler)]
impl Painter {
    #[tool(description = "Add two integers")]
    fn sum(&self, Parameters(SumArgs { a, b }): Parameters<SumArgs>) -> String {
        (a + b).to_string()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdout carries protocol messages: log to stderr only.
    let service = Painter.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```
  Running it in a plain terminal appears to hang - `serve()` waits for client input. Test with `npx @modelcontextprotocol/inspector <binary>`.
- **Bridging editor <-> MCP server:** run the MCP server as a separate stdio binary that talks to the running editor over a local socket (`interprocess` 2.4.8: Unix domain socket on macOS/Linux, named pipe on Windows; tokio support via the `tokio` feature). Alternatively expose the same tools via Streamable HTTP (bind to 127.0.0.1 only, validate Origin) - heavier, adds an HTTP stack.

## 4. MCP protocol version

Source: https://modelcontextprotocol.io/specification (-> /specification/2026-07-28/...), retrieved 2026-11-09.

- **Current version string: `2026-07-28`** (spec based on `schema/2026-07-28/schema.ts`). Previous: `2025-11-25` (and older), called "legacy" in the new spec.
- **Big change - no handshake in the modern era.** MCP 2026-07-28 is **stateless**: no `initialize`/`initialized` session setup. Every request carries its context in `_meta`:
  - `io.modelcontextprotocol/protocolVersion` (required), e.g. `"2026-07-28"`
  - `io.modelcontextprotocol/clientCapabilities` (required)
  - `io.modelcontextprotocol/clientInfo` (recommended), `io.modelcontextprotocol/logLevel` (optional)
  - Servers SHOULD put `io.modelcontextprotocol/serverInfo` in result `_meta`.
  - A request missing required fields -> JSON-RPC `-32602` (HTTP 400).
- **Version negotiation:** server accepts or rejects each request; unsupported version -> error `-32022 UnsupportedProtocolVersion` with `data: { supported: [...], requested: "..." }`; client retries with a mutual version. Other new codes: `-32020 HeaderMismatch`, `-32021 MissingRequiredClientCapability` (`data.requiredCapabilities`). On HTTP the version is also in the `MCP-Protocol-Version` header.
- **Discovery:** servers MUST implement `server/discover` (returns supported versions and server capabilities); clients MAY call it first. Results carry `resultType` (`"complete"` or `"input_required"`; absent = complete). `input_required` implements Multi Round-Trip Requests (server asks for sampling/elicitation/roots input).
- **Capabilities** are plain objects, e.g. server `{ "tools": {}, "resources": {}, "prompts": {} }`, client `{ "roots": {}, "extensions": { ... } }`. Optional **extensions** (Tasks `io.modelcontextprotocol/tasks`, MCP Apps `io.modelcontextprotocol/ui`, Skills over MCP) are negotiated through the `extensions` map in capabilities; both sides must support them or fall back to core behavior.
- **Legacy compatibility (2025-11-25):** a dual-era server answers `initialize` (client sends `protocolVersion`, `capabilities`, `clientInfo`; server replies with chosen version, `capabilities`, `serverInfo`, then client sends `notifications/initialized`). A dual-era client on stdio probes with `server/discover` and falls back to `initialize` on non-modern errors. Legacy clients cannot talk to modern-only servers.
- **Practical consequence:** rmcp 3.9 handles both eras per its README. Many installed clients (Claude Desktop, IDEs, Inspector) may still speak 2025-11-25 - do not make the server modern-only. Verify behavior with real clients before release.
- JSON Schema dialect: 2020-12 default; `$ref` to network URIs must not be auto-dereferenced.

## 5. Platform notes (macOS/Windows/Linux)

- **macOS (Apple Silicon):** eframe window via winit; Retina `pixels_per_point` is usually 2.0, so integer canvas scale is straightforward. glow = OpenGL 4.1 core (deprecated but works); wgpu = Metal. App bundle needs `.icns`; `with_icon` sets the Dock icon at runtime only. Native file dialog = NSOpenPanel/NSSavePanel (rfd). Config dir via `directories`: `~/Library/Application Support/...`. Local socket: Unix domain socket - keep the path short (< 104 bytes) and in a per-user directory with 0700 permissions.
- **Windows:** `pixels_per_point` is often 1.25/1.5 -> integer scaling is not pixel-exact unless forced; handle via a UI note or `set_pixels_per_point`. IPC via named pipes (`interprocess` supports them; set a per-user ACL/unique name). rfd uses the native Win32 dialogs. Stdio MCP servers need a console-less binary carefully: ensure stdout is binary-safe (no CRLF translation; Rust's stdio is fine).
- **Linux:** needs X11 and/or Wayland (eframe default features `x11` and `wayland`); packages for GL (glow) or Vulkan (wgpu). rfd uses xdg-desktop-portal (async) or GTK3 depending on features - check the chosen backend and its system packages. HiDPI scale on Wayland can be fractional. Sockets: `$XDG_RUNTIME_DIR`.
- **File dialogs:** `rfd` (Rusty File Dialog) opens **native OS dialogs** (NSOpenPanel on macOS, Win32 on Windows, portal/GTK on Linux), with an async API and a blocking `FileDialog` API. Native dialogs are the standard choice for editors; they are not rendered by egui and so do not use the pixel-art UI. Blocking dialogs on the UI thread freeze the frame loop - use the async variant or a worker thread. rfd 0.17.2 (not 0.18) is what builds on 1.90.
- **CI/target:** pin `rust-version` in `Cargo.toml` and run CI on all three OSes; `Cargo.lock` should be committed (binary project).

## 6. Risks and open points

1. **eframe is pinned one big step behind** (0.33.3 vs 0.37.0) because of MSRV. Security/bug fixes in 0.34-0.37 are not available until rustc >= 1.95. Decide: stay on 1.90 + eframe 0.33.3, or upgrade the toolchain (`brew upgrade rust` gives 1.99.1) and use 0.37.0. The `update` -> `ui` trait change means app code should be isolated. Recommendation: since greenfield, upgrading the toolchain first and starting directly on eframe 0.37 avoids a later migration; if the toolchain is fixed at 1.90, use 0.33.3.
2. **rmcp 3.x is new and fast-moving** (3.4 -> 3.9 in about 2 months, Apache-2.0 only). Pin an exact minor (`rmcp = "=3.9.0"` or `~3.9`) and re-check release notes before each bump. Docs of the macros (`tool_router(server_handler)`) differ from older 0.x tutorials.
3. **Spec transition risk:** MCP 2026-07-28 is stateless and removes the initialize handshake; client support in the wild is uncertain. Test against Claude Desktop/Code, VS Code and MCP Inspector on both eras.
4. **ciborium** is effectively unmaintained (last release 2024-01-24). Acceptable for a stable format, but consider `minicbor` or plain `serde_json`/binary chunks if CBOR is not essential.
5. **zip 9.x** is released very frequently (9.2.1 -> 9.5.0 within days, 9.5.0 published today). Pin and review changelog; keep a project-file format versioned independent of the zip crate. Fuzz/limit untrusted archive input (zip bombs, path traversal) when loading project files.
6. **zstd** switched license from MIT (<=0.13.3) to BSD-3-Clause (0.14); both fine, but re-run license audit.
7. **uuid 1.29.x MSRV is 1.89** - already within 1.90, but the margin is one minor version.
8. **Fractional DPI** breaks pixel-exact integer scaling (Windows/Linux); needs a defined policy.
9. **rfd 0.18 needs rustc 1.92**; rfd 0.17.2 does not declare an MSRV (resolver cannot protect us), though it checks fine on 1.90.
10. **Transitive licenses and bundled fonts** not audited; run a license tool before the public release and ship a THIRD-PARTY notice. (2026-11-13: the whole graph was scanned by licence expression, section 7.3; the notices are still to be generated.)
11. **Streamable HTTP** (if used) adds a network attack surface: bind 127.0.0.1, check Origin/Host, consider auth; stdio + local socket is safer for a local editor.
12. **Not verified:** runtime behavior of the editor on Windows/Linux (research done only on macOS); actual rendering with glow vs wgpu; rmcp prompts/resources snippets were taken from the README/examples and not compiled by us (only the tool skeleton above was).

## Verification (what was actually run)

- `cargo search <crate> --limit 1` works online for all crates; crates.io API for MSRV/license.
- Scratch project (outside the repo) with `rust-version = "1.90"`, `resolver = "3"`, dependencies: eframe 0.33, image 0.25 (png, jpeg), png 0.18, zip 9, zstd 0.14, serde, serde_json, ciborium 0.2, thiserror 2, anyhow, uuid 1 (v4, serde), schemars 1, tokio 1, rmcp 3 (server, transport-io, transport-streamable-http-server), interprocess 2 (tokio), rfd 0.17, directories 6, proptest 1: `cargo generate-lockfile` + `cargo check` succeeded on rustc 1.90.0. Resolver notes: eframe 0.37.0 (needs 1.95) and rfd 0.18.0 (needs 1.92) were skipped automatically.
- Separate scratch project: the rmcp stdio skeleton above passed `cargo check`.

## 7. Wave 8 additions and the licence scan (2026-11-13)

Method: `cargo metadata --format-version 1 --locked` on main at `cc920a7` (the whole graph, every target platform: 551 packages, 9 of them workspace crates), the same with `--filter-platform` for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`, and the `Cargo.lock` before wave 8 (`300257a`, 476 packages) for comparison; licence expressions as the packages declare them. The two wave-8 integration branches (MCP generation tools at `a82f22a`, MCP layer tools at `a1b5a9e`) add no package: their `Cargo.lock` files hold the same 551 name/version pairs as main's; the first only adds the dependency edge `dpx-mcp` → `dpx-imagegen` (optional, feature `imagegen`, which the app turns on), the second leaves `Cargo.lock` unchanged. That edge reaches no flagged package that `dpx-mcp` did not reach already (of the flagged ones `dpx-imagegen` reaches only `unicode-ident`), so the scan result below holds for the merge as well. The scan was made with a small script over the `cargo metadata` output (an expression parser for `OR`, `AND`, `/`, `WITH` and parentheses, and a breadth-first search from every workspace crate for the paths), not with `cargo deny` or `cargo about`.

### 7.1 New workspace crates and direct dependencies

| Crate | Direct dependencies added (version, licence) |
|---|---|
| `dpx-mcp` (ADR-43) | `axum` 0.8.9 (MIT; features `tokio`, `http1`), `reqwest` 0.13.5 (MIT OR Apache-2.0; `default-features = false`, the bridge's own client), `getrandom` 0.3.4 (MIT OR Apache-2.0, already in the tree), `tokio-util` 0.7.20 (MIT, already in the tree); `rmcp` 3.9.0 (Apache-2.0) gained the features `transport-streamable-http-server`, `client`, `transport-streamable-http-client-reqwest`, and `tokio` the features `net`, `sync`, `time`, `signal`; optional `dpx-imagegen` (workspace) behind the feature `imagegen` (integration strand A) |
| `dpx-imagegen` (new, ADR-45) | `ureq` 3.5.2 (MIT OR Apache-2.0; `default-features = false`, feature `native-tls-no-default`), `keyring` 4.2.0 (MIT OR Apache-2.0), `base64` 0.23.1 (MIT OR Apache-2.0, already in the tree); `dpx-core`, `dpx-formats`, `serde`, `serde_json` |
| `dpx-mcp-bridge` (new app, ADR-43) | `dpx-mcp` (workspace) only |
| `xtask` (new, ADR-43) | `dpx-mcp`, `serde_json`, `zip` (all already in the tree) |
| `deluxepainter` | `dpx-imagegen` (workspace); `dpx-mcp` with the feature `imagegen` (integration strand A) |

### 7.2 Every package new in `Cargo.lock` (72)

"all" = resolved for all four platforms above.

| Package | Version | Licence | Reached from | Platforms |
|---|---|---|---|---|
| aes | 0.9.4 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| aho-corasick | 1.1.5 | Unlicense OR MIT | dpx-imagegen | Windows |
| apple-native-keyring-store | 1.0.2 | MIT OR Apache-2.0 | dpx-imagegen | macOS |
| axum | 0.8.9 | MIT | dpx-mcp | all |
| axum-core | 0.5.6 | MIT | dpx-mcp | all |
| base64ct | 1.8.3 | Apache-2.0 OR MIT | dpx-imagegen | all |
| block-padding | 0.4.2 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| byteorder | 1.5.0 | Unlicense OR MIT | dpx-imagegen | Windows |
| cbc | 0.2.1 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 | dpx-mcp | all |
| cipher | 0.5.2 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| cmov | 0.5.4 | Apache-2.0 OR MIT | dpx-imagegen, dpx-mcp | all |
| core-foundation | 0.10.1 (0.9.4 was already there) | MIT OR Apache-2.0 | dpx-imagegen | macOS |
| cpubits | 0.1.1 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| ctutils | 0.4.8 | Apache-2.0 OR MIT | dpx-imagegen, dpx-mcp | all |
| der | 0.8.3 | Apache-2.0 OR MIT | dpx-imagegen | all |
| foreign-types | 0.3.2 (0.5.0 was already there) | MIT/Apache-2.0 | dpx-imagegen | Linux |
| foreign-types-shared | 0.1.1 (0.3.1 was already there) | MIT/Apache-2.0 | dpx-imagegen | Linux |
| hkdf | 0.13.0 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| hmac | 0.13.0 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| http | 1.5.0 | MIT OR Apache-2.0 | dpx-imagegen, dpx-mcp | all |
| http-body | 1.1.0 | MIT | dpx-mcp | all |
| http-body-util | 0.1.5 | MIT | dpx-mcp | all |
| httparse | 1.10.1 | MIT OR Apache-2.0 | dpx-imagegen, dpx-mcp | all |
| httpdate | 1.0.3 | MIT OR Apache-2.0 | dpx-mcp | all |
| hyper | 1.12.0 | MIT | dpx-mcp | all |
| hyper-util | 0.1.21 | MIT | dpx-mcp | all |
| inout | 0.2.6 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| ipnet | 2.12.2 | MIT OR Apache-2.0 | dpx-mcp | all |
| keyring | 4.2.0 | MIT OR Apache-2.0 | dpx-imagegen | all |
| keyring-core | 1.0.1 | MIT OR Apache-2.0 | dpx-imagegen | all |
| matchit | 0.8.4 | MIT AND BSD-3-Clause | dpx-mcp | all |
| mime | 0.3.18 | MIT OR Apache-2.0 | dpx-mcp | all |
| native-tls | 0.2.18 | MIT OR Apache-2.0 | dpx-imagegen | all |
| num | 0.4.3 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| num-bigint | 0.4.8 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| num-complex | 0.4.6 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| num-integer | 0.1.47 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| num-iter | 0.1.46 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| num-rational | 0.4.2 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| openssl | 0.10.82 | Apache-2.0 | dpx-imagegen | Linux |
| openssl-macros | 0.1.1 | MIT/Apache-2.0 | dpx-imagegen | Linux |
| openssl-probe | 0.2.2 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| openssl-sys | 0.9.118 | MIT | dpx-imagegen | Linux |
| pem-rfc7468 | 1.0.0 | Apache-2.0 OR MIT | dpx-imagegen | all |
| rand | 0.10.3 | MIT OR Apache-2.0 | dpx-mcp | all |
| rand_core | 0.10.1 | MIT OR Apache-2.0 | several workspace crates | all |
| regex | 1.13.1 | MIT OR Apache-2.0 | dpx-imagegen | Windows |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 | dpx-imagegen | Windows |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | dpx-imagegen | Windows |
| reqwest | 0.13.5 | MIT OR Apache-2.0 | dpx-mcp | all |
| rustls-pki-types | 1.16.0 | MIT OR Apache-2.0 | dpx-imagegen (types only, no rustls) | all |
| schannel | 0.1.30 | MIT | dpx-imagegen | Windows |
| secret-service | 5.2.0 | MIT OR Apache-2.0 | dpx-imagegen | Linux |
| security-framework | 3.7.0 | MIT OR Apache-2.0 | dpx-imagegen | macOS |
| security-framework-sys | 2.17.0 | MIT OR Apache-2.0 | dpx-imagegen | macOS |
| sse-stream | 0.2.6 | MIT OR Apache-2.0 | dpx-mcp | all |
| sync_wrapper | 1.0.2 | Apache-2.0 | dpx-mcp | all |
| tokio-stream | 0.1.19 | MIT | dpx-mcp | all |
| tower | 0.5.3 | MIT | dpx-mcp | all |
| tower-http | 0.6.11 | MIT | dpx-mcp | all |
| tower-layer | 0.3.3 | MIT | dpx-mcp | all |
| tower-service | 0.3.3 | MIT | dpx-mcp | all |
| try-lock | 0.2.5 | MIT | dpx-mcp | all |
| ureq | 3.5.2 | MIT OR Apache-2.0 | dpx-imagegen | all |
| ureq-proto | 0.7.1 | MIT OR Apache-2.0 | dpx-imagegen | all |
| utf8-zero | 0.8.1 | MIT OR Apache-2.0 | dpx-imagegen | all |
| vcpkg | 0.2.15 | MIT/Apache-2.0 | dpx-imagegen (build time) | Linux |
| want | 0.3.2 | MIT | dpx-mcp | all |
| wasm-streams | 0.5.0 | MIT OR Apache-2.0 | dpx-mcp (reqwest on wasm32 only) | none of the four |
| windows-native-keyring-store | 1.1.0 | MIT OR Apache-2.0 | dpx-imagegen | Windows |
| zbus-secret-service-keyring-store | 1.0.1 | MIT OR Apache-2.0 | dpx-imagegen | Linux |

Each of them offers MIT, Apache-2.0 or BSD-3-Clause (`aho-corasick` and `byteorder` are Unlicense OR MIT; `matchit` is MIT AND BSD-3-Clause, both allowed). No `ring`, `rustls`, `rustls-webpki`, `untrusted`, `webpki-roots` or `aws-lc-*` crate is in the lock (ADR-45 chose native TLS for that reason). The OpenSSL crates are Linux only; the OpenSSL library itself is a system library, not a crate. No existing crate changed its version.

### 7.3 Licence scan of the whole graph

Rule: a package passes when its licence expression can be satisfied with MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, Zlib or 0BSD alone (OR picks one side, AND needs both, `/` counts as OR). **24 packages do not pass. None of them is new in wave 8: all 24 were in `Cargo.lock` before it, at the same versions.** Wave 8 added new paths to the Unicode-3.0 crates (through `reqwest` and `rmcp`'s client in `dpx-mcp`), so the bridge binary and `xtask` now carry them too.

| Package | Version | Licence | Workspace crate and path | Platforms |
|---|---|---|---|---|
| libloading | 0.8.9 | ISC | `deluxepainter` via `eframe` → `glutin` and `eframe` → `wgpu` → `wgpu-hal` (its direct users: `glutin`, `wgpu-hal`, `ash`, `khronos-egl`, and `dlib` and `x11rb` outside Windows) | all |
| clipboard-win | 5.4.3 | BSL-1.0 | `deluxepainter` via `eframe` → `egui-winit` → `arboard` | Windows |
| error-code | 3.4.0 | BSL-1.0 | `deluxepainter` via `eframe` → `egui-winit` → `arboard` → `clipboard-win` | Windows |
| epaint_default_fonts | 0.37.0 | (MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0 | `deluxepainter` via `eframe` → `egui` → `epaint` | all |
| unicode-properties | 0.1.5 | (MIT OR Apache-2.0) AND Unicode-3.0 | `deluxepainter` via `eframe` → `egui` → `epaint` | all |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | every workspace crate, through `proc-macro2` (build time, inside proc macros such as `serde_derive`), also `wasm-bindgen-shared` | all |
| icu_collections, icu_locale_core, icu_normalizer, icu_normalizer_data, icu_properties, icu_properties_data, icu_provider | 2.4.0 | Unicode-3.0 | `url` 2.5.8 → `idna` 1.1.0 → `idna_adapter` 1.2.2, reached from `deluxepainter` via `eframe` → `egui-winit` → `webbrowser` → `url` (before wave 8, same versions) and from `dpx-mcp` via `reqwest` (directly, under `rmcp`'s HTTP client, and through its `tower-http`) → `url` (wave 8), so also from `deluxepainter`, `dpx-mcp-bridge` and `xtask` through `dpx-mcp` | all |
| litemap | 0.8.3 | Unicode-3.0 | as the ICU crates | all |
| potential_utf | 0.1.6 | Unicode-3.0 | as the ICU crates | all |
| tinystr | 0.8.5 | Unicode-3.0 | as the ICU crates | all |
| writeable | 0.6.4 | Unicode-3.0 | as the ICU crates | all |
| yoke, yoke-derive | 0.8.4 | Unicode-3.0 | as the ICU crates (`yoke-derive` is a proc macro) | all |
| zerofrom, zerofrom-derive | 0.1.8 | Unicode-3.0 | as the ICU crates (`zerofrom-derive` is a proc macro) | all |
| zerotrie | 0.2.5 | Unicode-3.0 | as the ICU crates | all |
| zerovec | 0.11.9 | Unicode-3.0 | as the ICU crates | all |
| zerovec-derive | 0.11.7 | Unicode-3.0 | as the ICU crates (a proc macro) | all |

Notes: ADR-45's sentence "The only flagged crate in the graph is `unicode-ident`" holds for the graph of `dpx-imagegen` (whose own dependencies reach no other flagged package), not for the workspace; ADR-43's "No ISC-only crate was added" holds for what `dpx-mcp` added (it added Unicode-3.0 paths, but no package). This is the first scan over the whole graph. ISC, BSL-1.0, OFL-1.1, Ubuntu-font-1.0 and Unicode-3.0 are permissive licences but not on CLAUDE.md's list; they all come through `eframe` or through `url` (which `eframe` already used), except `unicode-ident` (Unicode-3.0), which every workspace crate reaches through `proc-macro2` (`serde_derive` and other proc macros; `dpx-core`, `dpx-formats`, `dpx-iff`, `dpx-ui` and `dpx-imagegen` reach it without `eframe` or `url`), and whether they are acceptable is the owner's call (no dependency was changed). `epaint_default_fonts` holds egui's built-in fonts; DeluxePainter draws its own bitmap font. The third-party notices for a binary release still have to be generated (`cargo about` or `cargo deny`, section 1).
