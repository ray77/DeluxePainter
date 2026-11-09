# DeluxePainter — agent instructions

Read `docs/FEATURES.md`, `docs/ARCHITECTURE.md` and `docs/ROADMAP.md` before working. Everything in the repository is **English** (code, docs, commits). The owner communicates in German.

## Non-negotiables

- **Rust only** for product code. Desktop host: `eframe`/`egui`, but the visible UI is our own software-rendered pixel UI (bitmap font, bevels, icons) drawn into a framebuffer and shown with nearest-neighbour scaling at integer factors. No standard egui widgets in the painting UI.
- **Design basis: DeluxePaint IV AGA (v4.5).** Don't mix in DPaint III/V behaviour without documenting it. All window/menu/requester corners are square.
- OCS/AGA hardware limits are document rules, validated on export. Never silently reduce size, palette, colours, transparency or frames.
- Save = `.dpxproj` (lossless project). Export = IFF ILBM / IFF ANIM / PNG. JPEG is import-only. No other output formats.
- One `Command` engine for GUI, CLI and MCP. No pixel logic outside `dpx-core`/`dpx-raster`.
- MCP: rmcp, stdio first, disabled by default, logs to stderr only, file access only in approved directories.

## Reference material

`Manuals, Pics/` (repo root, git-ignored) holds copyrighted manuals and screenshots of the original program. **Read-only. Never modify, commit or redistribute it**, and never copy manual text, original artwork or fonts into the repo. Research summaries live in `docs/research/`.

## Working rules

- Small vertical slices; every milestone ends with a working build, tests and real screenshots/CLI output.
- Delegate small, well-scoped tasks to Sonnet subagents with explicit file scope and acceptance criteria; review and test every result before integrating.
- Ask the owner only for visible UI/UX decisions, project-format breaking changes and branding; decide internal details yourself and document them (ADR in `docs/adr/`).
- Never claim tests, platforms or format compatibility that were not actually verified.
- CI gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- License: PolyForm Noncommercial 1.0.0. Only add dependencies with permissive licenses (MIT/Apache-2.0/BSD/Zlib/0BSD).
