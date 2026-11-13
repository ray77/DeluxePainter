//! DeluxePainter build tasks.
//!
//! `cargo run -p xtask -- mcpb [--bridge FILE] [--out DIR]` builds the Claude Desktop
//! extension `target/mcpb/deluxepainter-<os>-<arch>.mcpb` (ADR-43, part 3): it builds the
//! release `dpx-mcp-bridge` (unless `--bridge` names a built one) and packs it with a
//! `manifest.json` (MCPB manifest version 0.3) into a ZIP:
//!
//! ```text
//! manifest.json
//! server/dpx-mcp-bridge        (server/dpx-mcp-bridge.exe on Windows), Unix mode 0755
//! ```
//!
//! The bundle is for the platform it is built on (its `compatibility.platforms` names only
//! that one); CI builds it on macOS and Windows. It has no icon (branding is the owner's
//! call) and is not signed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use dpx_mcp::bundle;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, System, ZipWriter};

const USAGE: &str = "usage: cargo run -p xtask -- mcpb [--bridge FILE] [--out DIR]";

/// The MCPB manifest version written (the schema `mcpb` 2.1 calls "latest").
pub const MANIFEST_VERSION: &str = "0.3";

const DESCRIPTION: &str = "Lets Claude paint in DeluxePainter on this computer: the \
    extension forwards MCP tool calls to the running app (Prefs > AI & MCP).";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("mcpb") => mcpb(&args[1..]).map(|p| println!("{}", p.display())),
        _ => Err(USAGE.to_string()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn arg(args: &[String], name: &str) -> Option<PathBuf> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

/// The workspace's `target` directory.
fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or(Path::new("."))
            .join("target"),
    }
}

fn mcpb(args: &[String]) -> Result<PathBuf, String> {
    if let Some(bad) = args
        .iter()
        .enumerate()
        .find(|&(i, a)| i % 2 == 0 && a != "--bridge" && a != "--out")
    {
        return Err(format!("unknown option {}\n{USAGE}", bad.1));
    }
    let os = std::env::consts::OS;
    let bridge = match arg(args, "--bridge") {
        Some(b) => b,
        None => build_bridge()?,
    };
    let out = arg(args, "--out").unwrap_or_else(|| target_dir().join(bundle::MCPB_DIR));
    let bytes =
        std::fs::read(&bridge).map_err(|e| format!("cannot read {}: {e}", bridge.display()))?;
    let path = out.join(bundle::this_mcpb_file_name());
    let tools = tool_list();
    let manifest = manifest(env!("CARGO_PKG_VERSION"), &author(), os, &tools)?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    write_mcpb(&path, &manifest, os, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// `cargo build --release -p dpx-mcp-bridge`; the binary it made.
fn build_bridge() -> Result<PathBuf, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "--release", "-p", "dpx-mcp-bridge"])
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if !status.success() {
        return Err("cargo build of dpx-mcp-bridge failed".into());
    }
    Ok(target_dir().join("release").join(format!(
        "{}{}",
        bundle::BRIDGE_BIN,
        std::env::consts::EXE_SUFFIX
    )))
}

fn author() -> String {
    env!("CARGO_PKG_AUTHORS")
        .split(':')
        .next()
        .unwrap_or("DeluxePainter")
        .to_string()
}

/// The tools the bridge offers (name, first sentence of the description), from the same
/// definitions the server lists.
fn tool_list() -> Vec<(String, String)> {
    dpx_mcp::tools::definitions()
        .into_iter()
        .map(|t| {
            let d = t.description.as_deref().unwrap_or_default();
            (t.name.to_string(), first_sentence(d).to_string())
        })
        .collect()
}

/// The first sentence of `text`: up to the first ". " outside parentheses that does not end
/// an abbreviation such as "e.g." (wave-8 integration review: generate_image's description
/// has "(...; e.g. OpenAI Images)" before its first full stop), else all of it.
fn first_sentence(text: &str) -> &str {
    const ABBREVIATIONS: [&str; 3] = ["e.g", "i.e", "etc"];
    let mut depth = 0i32;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '.' if depth == 0 && text[i + 1..].starts_with(' ') => {
                let word = text[..i].rsplit([' ', '(']).next().unwrap_or_default();
                if !ABBREVIATIONS.contains(&word) {
                    return text[..=i].trim();
                }
            }
            _ => {}
        }
    }
    text.trim()
}

/// The binary's path inside the bundle for `os`.
fn entry_point(os: &str) -> String {
    let exe = if os == "windows" { ".exe" } else { "" };
    format!("server/{}{exe}", bundle::BRIDGE_BIN)
}

/// `manifest.json` for a bundle of `os` (Rust's OS name).
pub fn manifest(
    version: &str,
    author: &str,
    os: &str,
    tools: &[(String, String)],
) -> Result<Value, String> {
    let platform =
        bundle::mcpb_platform(os).ok_or_else(|| format!("no Claude Desktop platform for {os}"))?;
    let command = |p: &str| format!("${{__dirname}}/{p}");
    Ok(json!({
        "manifest_version": MANIFEST_VERSION,
        "name": bundle::CLIENT_NAME,
        "display_name": "DeluxePainter",
        "version": version,
        "description": DESCRIPTION,
        "author": { "name": author },
        "license": "PolyForm-Noncommercial-1.0.0",
        "server": {
            "type": "binary",
            "entry_point": entry_point(os),
            "mcp_config": {
                "command": command(&entry_point("macos")),
                "args": [],
                "platform_overrides": {
                    "win32": { "command": command(&entry_point("windows")) }
                }
            }
        },
        "tools": tools
            .iter()
            .map(|(name, description)| json!({ "name": name, "description": description }))
            .collect::<Vec<_>>(),
        "compatibility": { "platforms": [platform] }
    }))
}

/// Writes the bundle: `manifest.json` and the bridge (`bridge`, mode 0755) at its entry
/// point. Entries are dated 1980-01-01, so the same input gives the same bytes.
pub fn write_mcpb(path: &Path, manifest: &Value, os: &str, bridge: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut zip = ZipWriter::new(std::io::BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default())
        .system(System::Unix);
    zip.start_file("manifest.json", options.unix_permissions(0o644))?;
    let text = serde_json::to_string_pretty(manifest).map_err(std::io::Error::other)?;
    zip.write_all(text.as_bytes())?;
    zip.write_all(b"\n")?;
    // Claude Desktop starts the entry point itself: it must be executable (as `mcpb pack`
    // makes it).
    zip.start_file(entry_point(os), options.unix_permissions(0o755))?;
    zip.write_all(bridge)?;
    zip.finish()?.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    // The MCPB manifest 0.3 schema (modelcontextprotocol/mcpb, `mcpb-manifest-latest`):
    // every object has `additionalProperties: false`, so only these keys may appear.
    const TOP: &[&str] = &[
        "$schema",
        "dxt_version",
        "manifest_version",
        "name",
        "display_name",
        "version",
        "description",
        "long_description",
        "author",
        "repository",
        "homepage",
        "documentation",
        "support",
        "icon",
        "icons",
        "screenshots",
        "localization",
        "server",
        "tools",
        "tools_generated",
        "prompts",
        "prompts_generated",
        "keywords",
        "license",
        "privacy_policies",
        "compatibility",
        "user_config",
        "_meta",
    ];
    const REQUIRED: &[&str] = &["name", "version", "description", "author", "server"];
    const SERVER: &[&str] = &["type", "entry_point", "mcp_config"];
    const MCP_CONFIG: &[&str] = &["command", "args", "env", "platform_overrides"];
    const OVERRIDE: &[&str] = &["command", "args", "env"];
    const AUTHOR: &[&str] = &["name", "email", "url"];
    const TOOL: &[&str] = &["name", "description"];
    const COMPATIBILITY: &[&str] = &["claude_desktop", "platforms", "runtimes"];

    fn keys_within(v: &Value, allowed: &[&str], what: &str) {
        let o = v
            .as_object()
            .unwrap_or_else(|| panic!("{what} is an object"));
        for k in o.keys() {
            assert!(allowed.contains(&k.as_str()), "{what}: unknown key {k}");
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dpx-xtask-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn check_manifest(m: &Value, os: &str) {
        keys_within(m, TOP, "manifest");
        for k in REQUIRED {
            assert!(m.get(*k).is_some(), "required {k}");
        }
        assert_eq!(m["manifest_version"], "0.3");
        assert_eq!(m["name"], "deluxe-painter");
        assert_eq!(m["version"], env!("CARGO_PKG_VERSION"));
        keys_within(&m["author"], AUTHOR, "author");
        assert!(m["author"]["name"].as_str().is_some_and(|n| !n.is_empty()));
        let server = &m["server"];
        keys_within(server, SERVER, "server");
        for k in SERVER {
            assert!(server.get(*k).is_some(), "server.{k}");
        }
        assert_eq!(server["type"], "binary");
        let config = &server["mcp_config"];
        keys_within(config, MCP_CONFIG, "mcp_config");
        assert_eq!(config["command"], "${__dirname}/server/dpx-mcp-bridge");
        assert_eq!(config["args"], json!([]));
        let win = &config["platform_overrides"]["win32"];
        keys_within(win, OVERRIDE, "win32 override");
        assert_eq!(win["command"], "${__dirname}/server/dpx-mcp-bridge.exe");
        keys_within(&m["compatibility"], COMPATIBILITY, "compatibility");
        let platforms = m["compatibility"]["platforms"].as_array().unwrap();
        assert_eq!(platforms.len(), 1);
        assert_eq!(platforms[0], bundle::mcpb_platform(os).unwrap());
        assert!(
            platforms
                .iter()
                .all(|p| ["darwin", "win32", "linux"].contains(&p.as_str().unwrap()))
        );
        let tools = m["tools"].as_array().unwrap();
        for t in tools {
            keys_within(t, TOOL, "tool");
            let d = t["description"].as_str().unwrap();
            assert!(!d.is_empty());
            // A whole sentence: parentheses closed, not cut at an abbreviation.
            assert_eq!(d.matches('(').count(), d.matches(')').count(), "{d}");
            assert!(
                !d.ends_with("e.g.") && !d.ends_with("i.e.") && d.ends_with('.'),
                "{d}"
            );
        }
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, dpx_mcp::tools::names(), "the server's own tools");
        assert!(
            m.get("icon").is_none(),
            "no icon: branding is the owner's call"
        );
    }

    /// Wave-8 integration review: generate_image's first sentence holds "(...; e.g. ...)".
    #[test]
    fn the_first_sentence_skips_abbreviations_and_parentheses() {
        assert_eq!(first_sentence("One. Two."), "One.");
        assert_eq!(first_sentence("No full stop"), "No full stop");
        assert_eq!(
            first_sentence("Ask (a; e.g. B) for it. More (x. y)."),
            "Ask (a; e.g. B) for it."
        );
        assert_eq!(first_sentence("Use i.e. this. Then"), "Use i.e. this.");
        let generate = tool_list()
            .into_iter()
            .find(|(n, _)| n == "generate_image")
            .unwrap()
            .1;
        assert!(
            generate.ends_with("for a picture from a description."),
            "{generate}"
        );
    }

    #[test]
    fn bundle_layout_modes_and_manifest_follow_mcpb_0_3() {
        let dir = scratch("layout");
        let bridge = b"\x7fELF fake bridge";
        for (os, entry) in [
            ("macos", "server/dpx-mcp-bridge"),
            ("windows", "server/dpx-mcp-bridge.exe"),
        ] {
            let m = manifest("0.1.0", "Someone", os, &tool_list()).unwrap();
            assert_eq!(m["server"]["entry_point"], entry);
            let path = dir.join(format!("{os}.mcpb"));
            write_mcpb(&path, &m, os, bridge).unwrap();
            let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
            let names: Vec<String> = (0..zip.len())
                .map(|i| zip.by_index(i).unwrap().name().unwrap().into_owned())
                .collect();
            assert_eq!(names, ["manifest.json", entry], "{os}");
            {
                let mut f = zip.by_name(entry).unwrap();
                assert_eq!(f.unix_mode().map(|m| m & 0o777), Some(0o755), "executable");
                let mut got = Vec::new();
                f.read_to_end(&mut got).unwrap();
                assert_eq!(got, bridge);
            }
            let mut text = String::new();
            zip.by_name("manifest.json")
                .unwrap()
                .read_to_string(&mut text)
                .unwrap();
            let read: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(read, m);
            let mut checked = read.clone();
            checked["version"] = json!(env!("CARGO_PKG_VERSION"));
            check_manifest(&checked, os);
            // The entry point is in the bundle.
            assert!(names.contains(&read["server"]["entry_point"].as_str().unwrap().into()));
        }
        // The same input gives the same bytes.
        let m = manifest("0.1.0", "Someone", "macos", &tool_list()).unwrap();
        let (a, b) = (dir.join("a.mcpb"), dir.join("b.mcpb"));
        write_mcpb(&a, &m, "macos", bridge).unwrap();
        write_mcpb(&b, &m, "macos", bridge).unwrap();
        assert_eq!(std::fs::read(a).unwrap(), std::fs::read(b).unwrap());
        assert!(manifest("0.1.0", "x", "freebsd", &[]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcpb_with_a_built_bridge_writes_the_platform_bundle() {
        let dir = scratch("cli");
        let bridge = dir.join("bridge-bin");
        std::fs::write(&bridge, b"bridge").unwrap();
        let out = dir.join("out");
        let args = [
            "--bridge".to_string(),
            bridge.display().to_string(),
            "--out".into(),
            out.display().to_string(),
        ];
        let path = mcpb(&args).unwrap();
        assert_eq!(path, out.join(bundle::this_mcpb_file_name()));
        let zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        assert!(
            mcpb(&["--nope".into()])
                .unwrap_err()
                .starts_with("unknown option")
        );
        assert_eq!(
            author(),
            env!("CARGO_PKG_AUTHORS").split(':').next().unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
