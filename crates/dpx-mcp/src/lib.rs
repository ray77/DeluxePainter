//! DeluxePainter MCP server (ADR-5, ADR-21, ADR-24, ADR-43).
//!
//! - [`tools`]: the tools and resources. They never own editor state: they run jobs
//!   ([`jobs`]) on whichever thread owns a [`Workspace`] — the headless engine
//!   ([`HeadlessWorkspace`]) or the running GUI — and do file I/O and encoding themselves.
//! - [`server`]: the rmcp handler for both transports; [`seq`] keeps calls in arrival order.
//! - [`http`]: Streamable HTTP on `127.0.0.1` with bearer token, Host and Origin checks;
//!   [`endpoint`] writes the discovery files the [`bridge`] reads.
//! - [`bridge`]: the stdio server for Claude Desktop / Claude Code that forwards to the app.
//!
//! Every document change goes through [`dpx_core::Engine`] commands, exactly like the GUI.
//! File tools only touch approved directories ([`FileAccess`]). stdout carries protocol
//! messages only; diagnostics go to stderr.

pub mod access;
pub mod bridge;
pub mod endpoint;
pub mod error;
pub mod http;
pub mod jobs;
pub mod paths;
pub mod pictures;
pub mod seq;
pub mod server;
pub mod state;
pub mod tools;
pub mod view;
pub mod workspace;

use std::path::PathBuf;

use rmcp::ServiceExt;
use rmcp::transport::async_rw::AsyncRwTransport;

pub use access::FileAccess;
pub use error::ToolError;
pub use http::{HttpOptions, HttpServer};
pub use jobs::{Executor, JobQueue};
pub use server::McpService;
pub use state::{Overwrite, ServerState, Settings};
pub use tools::SERVER_NAME;
pub use workspace::{Brush, HeadlessWorkspace, PictureInfo, Question, Slot, Workspace};

const USAGE: &str = "usage: deluxepainter mcp [--allow-dir DIR]...\n       deluxepainter mcp --http [--port N] [--allow-dir DIR]...";

/// The environment variable that sets the HTTP server's bearer token (else a random one).
pub const TOKEN_ENV: &str = "DPX_MCP_TOKEN";

/// What `deluxepainter mcp ...` asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliOptions {
    pub dirs: Vec<PathBuf>,
    pub http: bool,
    pub port: Option<u16>,
}

/// Parses the arguments after `mcp`.
pub fn parse_cli(args: &[String]) -> Result<CliOptions, String> {
    let mut o = CliOptions {
        dirs: Vec::new(),
        http: false,
        port: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let port = |v: &str| {
            v.parse::<u16>()
                .map_err(|_| format!("--port needs a number from 0 to 65535, not {v:?}\n{USAGE}"))
        };
        if a == "--allow-dir" {
            let d = it
                .next()
                .ok_or_else(|| format!("--allow-dir needs a directory\n{USAGE}"))?;
            o.dirs.push(PathBuf::from(d));
        } else if let Some(d) = a.strip_prefix("--allow-dir=") {
            o.dirs.push(PathBuf::from(d));
        } else if a == "--http" {
            o.http = true;
        } else if a == "--port" {
            let v = it
                .next()
                .ok_or_else(|| format!("--port needs a number\n{USAGE}"))?;
            o.port = Some(port(v)?);
        } else if let Some(v) = a.strip_prefix("--port=") {
            o.port = Some(port(v)?);
        } else {
            return Err(format!("unknown argument {a}\n{USAGE}"));
        }
    }
    if o.port.is_some() && !o.http {
        return Err(format!("--port needs --http\n{USAGE}"));
    }
    Ok(o)
}

/// `deluxepainter mcp [--allow-dir DIR]...` runs the headless server on stdio;
/// `deluxepainter mcp --http [--port N] [--allow-dir DIR]...` on Streamable HTTP. Each
/// `--allow-dir` must be an existing directory.
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let o = parse_cli(args)?;
    let access = FileAccess::new(o.dirs)?;
    if o.http {
        run_http_blocking(access, o.port.unwrap_or(http::DEFAULT_PORT))
    } else {
        run_stdio_blocking(access).map_err(|e| e.to_string())
    }
}

fn report_access(access: &FileAccess) {
    if access.is_on() {
        for d in access.dirs() {
            eprintln!("deluxepainter mcp: approved directory {}", d.display());
        }
    } else {
        eprintln!("deluxepainter mcp: file access off (use --allow-dir DIR)");
    }
}

/// Runs the headless server on stdin/stdout until the client disconnects. Blocking; builds
/// its own tokio runtime so the (synchronous) app entry point can call it directly.
pub fn run_stdio_blocking(access: FileAccess) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        report_access(&access);
        let exec = jobs::spawn_headless(HeadlessWorkspace::default());
        let state = ServerState::new(Settings::headless(access));
        let seq = seq::Sequencer::new();
        let service = McpService::new(exec, state, seq.clone(), "stdio");
        let (stdin, stdout) = rmcp::transport::stdio();
        let transport = server::Ticketed::new(AsyncRwTransport::new_server(stdin, stdout), seq);
        eprintln!("deluxepainter mcp: stdio server ready");
        let running = service.serve(transport).await?;
        running.waiting().await?;
        Ok(())
    })
}

/// A bearer token from [`TOKEN_ENV`]: at least 16 visible ASCII characters without spaces.
fn token_from_env() -> Result<Option<String>, String> {
    let Some(v) = std::env::var_os(TOKEN_ENV).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let v = v
        .into_string()
        .map_err(|_| format!("{TOKEN_ENV} must be ASCII"))?;
    if v.len() < 16 || !v.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(format!(
            "{TOKEN_ENV} must be at least 16 visible ASCII characters without spaces"
        ));
    }
    Ok(Some(v))
}

/// Runs the headless HTTP server until Ctrl-C (or SIGTERM on Unix). The bearer token comes
/// from [`TOKEN_ENV`] or is random; it is written, with the port, to the endpoint file in
/// the config directory and never printed.
pub fn run_http_blocking(access: FileAccess, port: u16) -> Result<(), String> {
    let token = token_from_env()?;
    let dir = paths::config_dir();
    if dir.is_none() && token.is_none() {
        return Err(format!(
            "no config directory for the endpoint file: set DPX_CONFIG_DIR or {TOKEN_ENV}"
        ));
    }
    report_access(&access);
    let exec = jobs::spawn_headless(HeadlessWorkspace::default());
    let state = ServerState::new(Settings::headless(access));
    let server = HttpServer::start(
        exec,
        state,
        HttpOptions {
            port,
            token,
            endpoint_dir: dir,
        },
    )
    .map_err(|e| format!("deluxepainter mcp: cannot start the HTTP server: {e}"))?;
    eprintln!("deluxepainter mcp: HTTP server ready on {}", server.url());
    match server.endpoint_file() {
        Some(f) => eprintln!(
            "deluxepainter mcp: port and bearer token in {}",
            f.display()
        ),
        None => eprintln!("deluxepainter mcp: bearer token from {TOKEN_ENV}"),
    }
    wait_for_shutdown_signal();
    eprintln!("deluxepainter mcp: stopping");
    server.stop();
    Ok(())
}

fn wait_for_shutdown_signal() {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    rt.block_on(async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            match signal(SignalKind::terminate()) {
                Ok(mut term) => {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {}
                        _ = term.recv() => {}
                    }
                }
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cli_arguments() {
        let o = parse_cli(&args(&["--http", "--port", "0", "--allow-dir=/a"])).unwrap();
        assert!(o.http);
        assert_eq!(o.port, Some(0));
        assert_eq!(o.dirs, vec![PathBuf::from("/a")]);
        assert!(
            parse_cli(&args(&["--port", "1"])).is_err(),
            "--port needs --http"
        );
        assert!(parse_cli(&args(&["--http", "--port", "x"])).is_err());
        assert!(parse_cli(&args(&["--nope"])).is_err());
        let o = parse_cli(&[]).unwrap();
        assert!(!o.http && o.dirs.is_empty());
    }
}
