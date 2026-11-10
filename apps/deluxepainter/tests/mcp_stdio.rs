//! Spawns `deluxepainter mcp` and talks newline-delimited JSON-RPC over its stdio.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    fn start() -> Self {
        Self::start_with(&[])
    }

    fn start_with(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_deluxepainter"))
            .arg("mcp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn deluxepainter mcp");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            out,
            next_id: 1,
        }
    }

    fn send(&mut self, msg: &Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert!(self.out.read_line(&mut line).unwrap() > 0, "server closed");
            // Every stdout line must be a JSON-RPC message (nothing else may be printed).
            let v: Value = serde_json::from_str(line.trim()).expect("stdout is protocol only");
            assert_eq!(v["jsonrpc"], "2.0");
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        assert!(r.get("error").is_none(), "{r}");
        r["result"].clone()
    }
}

fn text_json(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn initialize_list_and_call_tools() {
    let mut c = Client::start();
    let init = c.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }),
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let list = c.request("tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for want in ["app_status", "new_document", "get_document_state"] {
        assert!(names.contains(&want), "{names:?}");
    }

    let status = text_json(&c.call("app_status", json!({})));
    assert_eq!(status["name"], "deluxepainter");
    assert_eq!(status["document"]["width"], 320);

    let created = text_json(&c.call(
        "new_document",
        json!({"video": "ntsc", "resolution": "hi", "colors": 16}),
    ));
    assert_eq!(created["width"], 640);
    assert_eq!(created["height"], 200);

    let state = text_json(&c.call("get_document_state", json!({})));
    assert_eq!(state, created);
    assert_eq!(state["palette_len"], 16);

    let bad = c.call("new_document", json!({"colors": 33}));
    assert_eq!(bad["isError"], true, "{bad}");
    assert!(
        bad["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("PALETTE_RANGE_INVALID"),
        "{bad}"
    );

    // Without --allow-dir every file tool is refused.
    for tool in ["open_picture", "save_picture"] {
        let r = c.call(tool, json!({"path": "/etc/hosts"}));
        assert_eq!(r["isError"], true, "{r}");
        assert!(
            r["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("FILE_ACCESS_OFF"),
            "{r}"
        );
    }

    drop(c.stdin);
    let _ = c.child.wait();
}

fn initialize(c: &mut Client) {
    c.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }),
    );
    c.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
}

fn base64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => panic!("bad base64 byte {c}"),
    };
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s.as_bytes().iter().filter(|&&c| c != b'=') {
        acc = (acc << 6) | u32::from(val(c));
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

fn error_text(r: &Value) -> &str {
    assert_eq!(r["isError"], true, "{r}");
    r["content"][0]["text"].as_str().unwrap()
}

#[test]
fn paint_look_undo_and_files() {
    let dir = std::env::temp_dir().join(format!("dpx-mcp-stdio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    let mut c = Client::start_with(&["--allow-dir", dir.to_str().unwrap()]);
    initialize(&mut c);

    let list = c.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    for want in [
        "execute_command",
        "undo",
        "redo",
        "get_palette",
        "get_pixels",
        "render_png",
        "open_picture",
        "save_picture",
    ] {
        let t = tools.iter().find(|t| t["name"] == want);
        let t = t.unwrap_or_else(|| panic!("tool {want} missing"));
        assert!(t["description"].as_str().unwrap().len() > 10);
        assert!(t["inputSchema"].is_object());
    }

    c.call("new_document", json!({}));
    let palette = text_json(&c.call("get_palette", json!({})));
    let red = palette["colors"][5].as_str().unwrap().to_string();

    // Paint a filled rectangle.
    let done = text_json(&c.call(
        "execute_command",
        json!({"command": {
            "op": "shape",
            "shape": {"kind": "rect", "from": [10, 10], "to": [19, 14]},
            "brush": [[0, 0]], "color": 5, "filled": true
        }}),
    ));
    assert_eq!(done["revision"], 1);
    assert_eq!(
        done["changed_rect"],
        json!({"x": 10, "y": 10, "w": 10, "h": 5})
    );

    // Look at it: decode the PNG and check a pixel.
    let img = c.call(
        "render_png",
        json!({"rect": {"x": 0, "y": 0, "w": 32, "h": 32}, "scale": 2}),
    );
    let block = &img["content"][0];
    assert_eq!(block["type"], "image", "{img}");
    assert_eq!(block["mimeType"], "image/png");
    let png_bytes = base64_decode(block["data"].as_str().unwrap());
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (64, 64));
    let px = |x: usize, y: usize| {
        let i = (y * 64 + x) * 3;
        format!("#{:02X}{:02X}{:02X}", buf[i], buf[i + 1], buf[i + 2])
    };
    assert_eq!(px(30, 26), red, "inside the rectangle (page 15,13)");
    assert_ne!(px(2, 2), red, "background");

    // Undo; the pixel is background again.
    let undone = text_json(&c.call("undo", json!({})));
    assert_eq!(undone["can_redo"], true);
    let px_rows = text_json(&c.call(
        "get_pixels",
        json!({"rect": {"x": 15, "y": 13, "w": 1, "h": 1}}),
    ));
    assert_eq!(px_rows["pixels"], json!([[0]]));

    // Save, refuse overwrite, refuse paths outside the approved directory.
    let saved = text_json(&c.call("save_picture", json!({"path": "t.iff"})));
    assert_eq!(saved["format"], "iff_ilbm");
    assert!(dir.join("t.iff").is_file());
    assert!(
        error_text(&c.call("save_picture", json!({"path": "t.iff"}))).starts_with("FILE_EXISTS")
    );
    assert!(
        error_text(&c.call("open_picture", json!({"path": "/etc/hosts"})))
            .starts_with("PATH_NOT_ALLOWED")
    );
    assert!(
        error_text(&c.call("open_picture", json!({"path": "../../etc/hosts"})))
            .starts_with("PATH_NOT_ALLOWED")
    );
    let opened = text_json(&c.call("open_picture", json!({"path": "t.iff"})));
    assert_eq!(opened["document"]["width"], 320);

    drop(c.stdin);
    let _ = c.child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_allow_dir_fails_at_startup() {
    let out = Command::new(env!("CARGO_BIN_EXE_deluxepainter"))
        .args(["mcp", "--allow-dir", "/definitely/not/a/dir"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "stdout stays protocol-only");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--allow-dir"));
}
