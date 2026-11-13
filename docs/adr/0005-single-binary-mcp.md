# ADR-5: One binary; MCP over stdio bridged to the running GUI

- Status: superseded by [ADR-43](0043-live-mcp.md) (wave 8, 2026-11-12) for the bridge: `deluxepainter mcp-bridge` (and the separate small binary `dpx-mcp-bridge`) speaks MCP Streamable HTTP to the running GUI on `127.0.0.1` instead of a local socket / named pipe, and the `interprocess` crate is not used. Still valid: one `deluxepainter` binary for the GUI, the CLI and the headless MCP server (`deluxepainter mcp`); MCP off by default; approved directories; logs on stderr. Accepted in Phase 0.
- Decision: `deluxepainter` starts the GUI; `deluxepainter mcp` runs an rmcp stdio server that connects to a running GUI over a local socket / named pipe (interprocess crate) or falls back to an in-process headless engine. MCP is disabled by default; file access limited to approved directories; logs on stderr only.
- Consequences: one version to install and configure; GUI and MCP always share the same engine code.
