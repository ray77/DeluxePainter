//! End to end: an in-process headless HTTP server (the library's) and the real
//! `dpx-mcp-bridge` binary speaking newline-delimited JSON-RPC on its stdio.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;

use dpx_mcp::{FileAccess, HeadlessWorkspace, HttpOptions, HttpServer, ServerState, Settings};
use serde_json::{Value, json};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dpx-bridge-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn server(dir: &Path) -> (HttpServer, Arc<ServerState>) {
    let exec = dpx_mcp::jobs::spawn_headless(HeadlessWorkspace::default());
    let state = ServerState::new(Settings::headless(FileAccess::off()));
    let s = HttpServer::start(
        exec,
        state.clone(),
        HttpOptions {
            port: 0,
            token: None,
            endpoint_dir: Some(dir.to_path_buf()),
        },
    )
    .unwrap();
    (s, state)
}

struct Bridge {
    child: Child,
    stdin: Option<ChildStdin>,
    out: BufReader<ChildStdout>,
    next_id: u64,
}

impl Bridge {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dpx-mcp-bridge"))
            .env("DPX_CONFIG_DIR", dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn dpx-mcp-bridge");
        let stdin = child.stdin.take();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut b = Self {
            child,
            stdin,
            out,
            next_id: 1,
        };
        let init = b.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "bridge-test", "version": "0"}}),
        );
        assert_eq!(
            init["result"]["serverInfo"]["name"], "deluxepainter",
            "{init}"
        );
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert!(init["result"]["capabilities"]["resources"].is_object());
        b.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        b
    }

    fn send(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "bridge closed");
            let v: Value = serde_json::from_str(line.trim()).expect("stdout is protocol only");
            assert_eq!(v["jsonrpc"], "2.0");
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert!(r.get("error").is_none(), "{r}");
        r["result"].clone()
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let r = self.call(name, args);
        assert_eq!(r["isError"], false, "{r}");
        r["structuredContent"].clone()
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

#[test]
fn forwards_calls_and_resources_to_the_running_app() {
    let dir = scratch("forward");
    let (srv, state) = server(&dir);
    let mut b = Bridge::start(&dir);
    let list = b.request("tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, dpx_mcp::tools::names(), "the same definitions");
    let c = b.ok(
        "create_document",
        json!({"width": 320, "height": 240, "chipset": "ocs", "colors": 32}),
    );
    assert_eq!(c["document"]["display_mode"], "PAL:Low Res");
    b.ok(
        "draw_rectangle",
        json!({"x": 10, "y": 10, "width": 10, "height": 5, "color": 5, "filled": true}),
    );
    let p = b.ok(
        "get_pixels",
        json!({"rect": {"x": 12, "y": 12, "w": 1, "h": 1}}),
    );
    assert_eq!(p["pixels"], json!([[5]]));
    let s = b.ok("get_app_status", json!({}));
    assert_eq!(s["transport"], "http", "served by the app's HTTP server");
    // A tool error comes back as a result.
    let e = b.call("set_palette_color", json!({"index": 1, "color": "#123456"}));
    assert_eq!(e["isError"], true);
    assert_eq!(e["structuredContent"]["code"], "HARDWARE_LIMIT");
    // Resources.
    let r = b.request("resources/read", json!({"uri": "dpx://document/info"}));
    let text = r["result"]["contents"][0]["text"].as_str().unwrap();
    let info: Value = serde_json::from_str(text).unwrap();
    assert_eq!(info["height"], 240);
    let r = b.request("resources/list", json!({}));
    assert_eq!(r["result"]["resources"].as_array().unwrap().len(), 3);
    // The app sees who called.
    let client = state.activity().snapshot().client.unwrap();
    assert_eq!(client, "bridge-test via deluxepainter-bridge");
    drop(b);
    srv.stop();
    assert!(!dir.join("mcp-endpoint.json").exists(), "removed on stop");
}

#[test]
fn reconnects_to_a_restarted_app_with_a_new_port_and_token() {
    let dir = scratch("restart");
    let (a, _) = server(&dir);
    let port_a = a.port();
    let mut b = Bridge::start(&dir);
    b.ok("draw_pixel", json!({"x": 1, "y": 1, "color": 7}));
    let p = b.ok(
        "get_pixels",
        json!({"rect": {"x": 1, "y": 1, "w": 1, "h": 1}}),
    );
    assert_eq!(p["pixels"], json!([[7]]));
    a.stop();
    // While nothing runs: a clear error, no crash.
    let e = b.call("get_document_info", json!({}));
    assert_eq!(e["isError"], true, "{e}");
    assert_eq!(e["structuredContent"]["code"], "APP_NOT_RUNNING");
    let (c, _) = server(&dir);
    assert_ne!(c.port(), port_a);
    let p = b.ok(
        "get_pixels",
        json!({"rect": {"x": 1, "y": 1, "w": 1, "h": 1}}),
    );
    assert_eq!(p["pixels"], json!([[0]]), "the new app's fresh document");
    drop(b);
    c.stop();
}

#[test]
fn without_the_app_lists_locally_and_calls_fail_clearly() {
    let dir = scratch("absent");
    let mut b = Bridge::start(&dir);
    let list = b.request("tools/list", json!({}));
    assert!(list["result"]["tools"].as_array().unwrap().len() >= 20);
    let r = b.call("draw_pixel", json!({"x": 0, "y": 0, "color": 1}));
    assert_eq!(r["isError"], true);
    assert_eq!(r["structuredContent"]["code"], "APP_NOT_RUNNING");
    assert_eq!(
        r["structuredContent"]["message"],
        "DeluxePainter is not running. Start it and turn on Prefs > AI & MCP."
    );
    let r = b.request("tools/call", json!({"name": "nope", "arguments": {}}));
    assert!(
        r.get("error").is_some(),
        "unknown tools are protocol errors: {r}"
    );
}

#[test]
fn a_stale_endpoint_file_counts_as_not_running() {
    let dir = scratch("stale");
    // A port nothing listens on.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let e = dpx_mcp::endpoint::Endpoint::new(port, "x".repeat(64));
    dpx_mcp::endpoint::write_endpoint(&dir, &e).unwrap();
    let mut b = Bridge::start(&dir);
    let r = b.call("get_app_status", json!({}));
    assert_eq!(r["structuredContent"]["code"], "APP_NOT_RUNNING", "{r}");
}

#[test]
fn bad_arguments_are_refused() {
    let out = Command::new(env!("CARGO_BIN_EXE_dpx-mcp-bridge"))
        .arg("--nope")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
}
