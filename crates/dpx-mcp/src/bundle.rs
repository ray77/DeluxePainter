//! Names shared by the Claude Desktop extension (`.mcpb`, built by `cargo run -p xtask --
//! mcpb`) and the app's connect flows (Prefs > AI & MCP.., ADR-43 part 3).

/// The name MCP clients know the server by: the extension's `name` and the Claude Code
/// registration (`claude mcp add … deluxe-painter …`).
pub const CLIENT_NAME: &str = "deluxe-painter";

/// The bridge binary inside the extension (`server/dpx-mcp-bridge`, `.exe` on Windows).
pub const BRIDGE_BIN: &str = "dpx-mcp-bridge";

/// The folder of `target/` the xtask writes the bundles to.
pub const MCPB_DIR: &str = "mcpb";

/// `deluxepainter-<os>-<arch>.mcpb`, with Rust's names (`macos`, `windows`, `linux`;
/// `aarch64`, `x86_64`).
pub fn mcpb_file_name(os: &str, arch: &str) -> String {
    format!("deluxepainter-{os}-{arch}.mcpb")
}

/// The bundle for the platform this binary was built for.
pub fn this_mcpb_file_name() -> String {
    mcpb_file_name(std::env::consts::OS, std::env::consts::ARCH)
}

/// The MCPB platform name (`process.platform`) of a Rust OS name.
pub fn mcpb_platform(os: &str) -> Option<&'static str> {
    match os {
        "macos" => Some("darwin"),
        "windows" => Some("win32"),
        "linux" => Some("linux"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            mcpb_file_name("macos", "aarch64"),
            "deluxepainter-macos-aarch64.mcpb"
        );
        assert!(this_mcpb_file_name().starts_with("deluxepainter-"));
        assert_eq!(mcpb_platform("windows"), Some("win32"));
        assert_eq!(mcpb_platform("macos"), Some("darwin"));
        assert_eq!(mcpb_platform("freebsd"), None);
        // Claude Code server names: letters, digits, '-' and '_' only.
        assert!(
            CLIENT_NAME
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }
}
