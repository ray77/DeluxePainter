//! `dpx-mcp-bridge`: the stdio MCP server that Claude Desktop (`.mcpb`) or Claude Code
//! starts; it forwards every tool call to the running DeluxePainter (ADR-43). The same code
//! runs as `deluxepainter mcp-bridge`; this binary has no GUI dependencies, so it can ship
//! on its own inside an extension bundle.
//!
//! It finds the app through `mcp-endpoint.json` in the DeluxePainter config directory
//! (`DPX_CONFIG_DIR` overrides it) and starts the app named in `mcp-app.json` when it is not
//! running. stdout carries protocol messages only.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = dpx_mcp::bridge::run_cli(&args) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
