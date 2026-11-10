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
        let mut child = Command::new(env!("CARGO_BIN_EXE_deluxepainter"))
            .arg("mcp")
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

    drop(c.stdin);
    let _ = c.child.wait();
}
