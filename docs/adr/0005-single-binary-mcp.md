# ADR-5: One binary; MCP over stdio bridged to the running GUI

- Status: accepted (Phase 0)
- Decision: `deluxepainter` starts the GUI; `deluxepainter mcp` runs an rmcp stdio server that connects to a running GUI over a local socket / named pipe (interprocess crate) or falls back to an in-process headless engine. MCP is disabled by default; file access limited to approved directories; logs on stderr only.
- Consequences: one version to install and configure; GUI and MCP always share the same engine code.
