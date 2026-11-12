//! Helpers shared by the MCP integration tests: a newline-delimited JSON-RPC client over a
//! child's stdio, a raw HTTP/1.1 client and test pictures.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub const EXE: &str = env!("CARGO_BIN_EXE_deluxepainter");

/// A fresh, canonical scratch directory.
pub fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dpx-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

/// JSON-RPC over a child's stdin/stdout.
pub struct Client {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    out: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    pub fn spawn(cmd: &mut Command) -> Self {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let stdin = child.stdin.take();
        let out = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            out,
            next_id: 1,
        }
    }

    pub fn mcp(args: &[&str]) -> Self {
        Self::spawn(Command::new(EXE).arg("mcp").args(args))
    }

    pub fn send(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    /// The next id a request will use.
    pub fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The next message from stdout; every stdout line must be JSON-RPC.
    pub fn read(&mut self) -> Value {
        let mut line = String::new();
        assert!(self.out.read_line(&mut line).unwrap() > 0, "server closed");
        let v: Value = serde_json::from_str(line.trim()).expect("stdout is protocol only");
        assert_eq!(v["jsonrpc"], "2.0");
        v
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.take_id();
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let v = self.read();
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    pub fn initialize(&mut self) -> Value {
        let init = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
        );
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        init
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        assert!(r.get("error").is_none(), "{r}");
        r["result"].clone()
    }

    /// The structured content of a successful call.
    pub fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let r = self.call(name, arguments);
        assert_eq!(r["isError"], false, "{name}: {r}");
        let s = r["structuredContent"].clone();
        assert_eq!(s["success"], true, "{r}");
        // The text block carries the same JSON.
        let text = r["content"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|c| c["text"].as_str())
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(text).unwrap(), s);
        s
    }

    /// The error code of a failed call.
    pub fn code(&mut self, name: &str, arguments: Value) -> String {
        let r = self.call(name, arguments);
        assert_eq!(r["isError"], true, "{name}: {r}");
        assert_eq!(r["structuredContent"]["success"], false);
        r["structuredContent"]["code"].as_str().unwrap().to_string()
    }

    pub fn close(mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

/// A raw HTTP response.
#[derive(Debug)]
pub struct Http {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Http {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&self.body)))
    }
}

/// The bytes of one HTTP/1.1 request (`Connection: close`).
pub fn request_bytes(method: &str, port: u16, headers: &[(&str, String)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("{method} /mcp HTTP/1.1\r\n");
    let has_host = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host"));
    if !has_host {
        out.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
    }
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Reads one response: up to its `Content-Length`, else to the end of the connection.
pub fn read_response(stream: &mut TcpStream) -> Http {
    let mut raw = Vec::new();
    let mut buf = [0u8; 8192];
    let split = loop {
        if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        let n = stream.read(&mut buf).unwrap();
        assert!(n > 0, "connection closed before the headers ended");
        raw.extend_from_slice(&buf[..n]);
    };
    let length = String::from_utf8_lossy(&raw[..split])
        .split("\r\n")
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok());
    match length {
        Some(len) => {
            while raw.len() < split + 4 + len {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "connection closed inside the body");
                raw.extend_from_slice(&buf[..n]);
            }
        }
        None => {
            stream.read_to_end(&mut raw).unwrap();
        }
    }
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let mut body = raw[split + 4..].to_vec();
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    if chunked {
        let mut out = Vec::new();
        let mut rest = &body[..];
        loop {
            let end = rest.windows(2).position(|w| w == b"\r\n").unwrap();
            let size = usize::from_str_radix(std::str::from_utf8(&rest[..end]).unwrap().trim(), 16)
                .unwrap();
            rest = &rest[end + 2..];
            if size == 0 {
                break;
            }
            out.extend_from_slice(&rest[..size]);
            rest = &rest[size + 2..];
        }
        body = out;
    }
    Http {
        status,
        headers,
        body,
    }
}

pub fn http(method: &str, port: u16, headers: &[(&str, String)], body: &[u8]) -> Http {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s.write_all(&request_bytes(method, port, headers, body))
        .unwrap();
    read_response(&mut s)
}

/// The standard MCP headers with a bearer token.
pub fn mcp_headers(token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {token}")),
        ("Content-Type", "application/json".into()),
        ("Accept", "application/json, text/event-stream".into()),
        ("MCP-Protocol-Version", "2025-06-18".into()),
    ]
}

/// A tools/call request body.
pub fn call_body(id: u64, name: &str, arguments: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": name, "arguments": arguments}}))
    .unwrap()
}

/// Waits until `dir` holds an endpoint file (of a process other than `not_pid`).
pub fn wait_for_endpoint(dir: &Path, secs: u64) -> Value {
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Ok(b) = std::fs::read(dir.join("mcp-endpoint.json"))
            && let Ok(v) = serde_json::from_slice::<Value>(&b)
        {
            return v;
        }
        assert!(
            Instant::now() < until,
            "no endpoint file in {}",
            dir.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A headless HTTP server process with its port and token.
pub struct HttpChild {
    pub child: Child,
    pub port: u16,
    pub token: String,
    pub dir: PathBuf,
}

impl HttpChild {
    pub fn start(tag: &str, extra: &[&str]) -> Self {
        let dir = scratch(tag);
        let child = Command::new(EXE)
            .args(["mcp", "--http", "--port", "0"])
            .args(extra)
            .env("DPX_CONFIG_DIR", &dir)
            .env_remove("DPX_MCP_TOKEN")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let e = wait_for_endpoint(&dir, 20);
        assert_eq!(e["pid"], child.id(), "{e}");
        Self {
            port: e["port"].as_u64().unwrap() as u16,
            token: e["token"].as_str().unwrap().to_string(),
            child,
            dir,
        }
    }

    pub fn post(&self, body: &[u8]) -> Http {
        http("POST", self.port, &mcp_headers(&self.token), body)
    }

    /// A tool call's result (asserting a JSON-RPC result, not an error).
    pub fn call(&self, id: u64, name: &str, args: Value) -> Value {
        let r = self.post(&call_body(id, name, args));
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        assert_eq!(v["id"], id, "{v}");
        v["result"].clone()
    }
}

impl Drop for HttpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A true-colour RGBA PNG of `w`×`h`: a smooth colour gradient (slow to reduce).
pub fn gradient_png(w: u32, h: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&[
                (x * 255 / w.max(1)) as u8,
                (y * 255 / h.max(1)) as u8,
                ((x + y) * 127 / (w + h).max(1)) as u8,
                255,
            ]);
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    out
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(s: &str) -> Vec<u8> {
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
