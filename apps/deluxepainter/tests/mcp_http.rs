//! The headless HTTP mode (`deluxepainter mcp --http`) over raw HTTP/1.1, and the stdio
//! bridge (`deluxepainter mcp-bridge`) in front of it (ADR-43).

mod common;

use std::io::Write;
use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{
    Client, EXE, HttpChild, base64, call_body, gradient_png, http, mcp_headers, read_response,
    request_bytes, scratch, wait_for_endpoint,
};
use serde_json::{Value, json};

#[test]
fn handshake_list_and_calls() {
    let s = HttpChild::start("http-basic", &[]);
    let e = wait_for_endpoint(&s.dir, 1);
    assert_eq!(e["version"], 1);
    assert_eq!(e["url"], format!("http://127.0.0.1:{}/mcp", s.port));
    assert_eq!(e["token"].as_str().unwrap().len(), 64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(s.dir.join("mcp-endpoint.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "only the owner may read the token");
    }

    let init = s.post(
        &serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "http-test", "version": "0"}}}))
        .unwrap(),
    );
    assert_eq!(init.status, 200);
    assert!(
        init.header("content-type")
            .unwrap()
            .starts_with("application/json")
    );
    let v = init.json();
    assert_eq!(v["result"]["serverInfo"]["name"], "deluxepainter", "{v}");
    assert!(v["result"]["capabilities"]["tools"].is_object());
    assert!(init.header("mcp-session-id").is_none(), "stateless");

    let note = s.post(
        &serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .unwrap(),
    );
    assert_eq!(note.status, 202);

    let list = s
        .post(
            &serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
                .unwrap(),
        )
        .json();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"get_preview") && names.contains(&"import_image"),
        "{names:?}"
    );

    let st = s.call(3, "get_app_status", json!({}));
    assert_eq!(st["structuredContent"]["transport"], "http");
    assert_eq!(st["structuredContent"]["engine"], "headless");
    let c = s.call(
        4,
        "create_document",
        json!({"width": 320, "height": 240, "chipset": "ocs", "colors": 32}),
    );
    assert_eq!(
        c["structuredContent"]["document"]["display_mode"],
        "PAL:Low Res"
    );
    let r = s.call(
        5,
        "draw_rectangle",
        json!({"x": 0, "y": 0, "width": 4, "height": 4, "color": 3, "filled": true}),
    );
    assert_eq!(r["structuredContent"]["revision"], 1);
    let u = s.call(6, "undo", json!({}));
    assert_eq!(u["structuredContent"]["can_redo"], true);
    let p = s.call(7, "get_preview", json!({}));
    assert_eq!(p["content"][0]["type"], "image");
    assert_eq!(p["structuredContent"]["image_width"], 960);
}

#[test]
fn token_host_and_origin_are_checked() {
    let s = HttpChild::start("http-guard", &[]);
    let body = call_body(1, "get_app_status", json!({}));
    let base = |token: Option<&str>| {
        let mut h = mcp_headers(token.unwrap_or(""));
        if token.is_none() {
            h.retain(|(k, _)| *k != "Authorization");
        }
        h
    };
    // 401 without or with a wrong token.
    let r = http("POST", s.port, &base(None), &body);
    assert_eq!(r.status, 401);
    assert_eq!(r.header("www-authenticate"), Some("Bearer"));
    assert_eq!(
        http("POST", s.port, &base(Some("wrong")), &body).status,
        401
    );
    let last = if s.token.ends_with('0') { '1' } else { '0' };
    let near = format!("{}{last}", &s.token[..63]);
    assert_eq!(http("POST", s.port, &base(Some(&near)), &body).status, 401);
    // 403 for a foreign Host, even with the token.
    for host in [
        "evil.example".to_string(),
        format!("evil.example:{}", s.port),
        format!("127.0.0.1:{}", s.port.wrapping_add(1)),
    ] {
        let mut h = base(Some(&s.token));
        h.push(("Host", host.clone()));
        assert_eq!(http("POST", s.port, &h, &body).status, 403, "{host}");
    }
    // localhost:<port> is fine.
    let mut h = base(Some(&s.token));
    h.push(("Host", format!("localhost:{}", s.port)));
    assert_eq!(http("POST", s.port, &h, &body).status, 200);
    // 403 for any browser Origin, also a local one, also without a token.
    for origin in ["http://evil.example", "http://127.0.0.1", "null"] {
        let mut h = base(Some(&s.token));
        h.push(("Origin", origin.to_string()));
        assert_eq!(http("POST", s.port, &h, &body).status, 403, "{origin}");
        let mut h = base(None);
        h.push(("Origin", origin.to_string()));
        assert_eq!(http("POST", s.port, &h, &body).status, 403, "{origin}");
    }
    // The right request still works afterwards.
    assert_eq!(s.post(&body).status, 200);
    // GET (an SSE stream) is not offered by the stateless server.
    let r = http("GET", s.port, &base(Some(&s.token)), b"");
    assert_eq!(r.status, 405);
    // Too large a body: refused from its announced length, before it is read.
    let mut st = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
    st.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut head = format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", s.port);
    for (k, v) in base(Some(&s.token)) {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", 17 * 1024 * 1024));
    st.write_all(head.as_bytes()).unwrap();
    assert_eq!(read_response(&mut st).status, 413);
}

/// Calls sent on separate connections without waiting run in the order they arrived: a slow
/// import first, then drawing calls that must land on the imported picture.
#[test]
fn pipelined_calls_run_in_arrival_order() {
    let s = HttpChild::start("http-order", &[]);
    let data = base64(&gradient_png(1600, 1200));
    let mut streams = Vec::new();
    let send = |id: u64, name: &str, args: Value| {
        let mut st = TcpStream::connect(("127.0.0.1", s.port)).unwrap();
        st.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        st.write_all(&request_bytes(
            "POST",
            s.port,
            &mcp_headers(&s.token),
            &call_body(id, name, args),
        ))
        .unwrap();
        st
    };
    streams.push((
        0,
        send(100, "import_image", json!({"data": data, "colors": 32})),
    ));
    for i in 0..10u64 {
        std::thread::sleep(Duration::from_millis(15));
        streams.push((
            i + 1,
            send(
                101 + i,
                "draw_pixel",
                json!({"x": i, "y": 0, "color": 1 + i}),
            ),
        ));
    }
    for (n, mut st) in streams {
        let r = read_response(&mut st);
        assert_eq!(r.status, 200);
        let v = r.json();
        assert_eq!(v["result"]["isError"], false, "{v}");
        assert_eq!(
            v["result"]["structuredContent"]["revision"], n,
            "call {n} ran in its place"
        );
    }
    let row = s.call(
        200,
        "get_pixels",
        json!({"rect": {"x": 0, "y": 0, "w": 10, "h": 1}}),
    );
    let want: Vec<Value> = (0..10u64).map(|i| json!(1 + i)).collect();
    assert_eq!(row["structuredContent"]["pixels"][0], Value::Array(want));
}

#[test]
fn files_over_http_stay_in_the_approved_directory() {
    let files = scratch("http-files-dir");
    let s = HttpChild::start("http-files", &["--allow-dir", files.to_str().unwrap()]);
    let r = s.call(1, "export_image", json!({"path": "a.png"}));
    assert_eq!(r["isError"], false, "{r}");
    assert!(files.join("a.png").is_file());
    let r = s.call(2, "export_image", json!({"path": "/etc/x.png"}));
    assert_eq!(r["structuredContent"]["code"], "PATH_NOT_ALLOWED");
}

#[cfg(unix)]
#[test]
fn sigterm_stops_the_server_and_removes_the_endpoint_file() {
    let mut s = HttpChild::start("http-term", &[]);
    let file = s.dir.join("mcp-endpoint.json");
    assert!(file.exists());
    let st = Command::new("kill")
        .args(["-TERM", &s.child.id().to_string()])
        .status()
        .unwrap();
    assert!(st.success());
    let until = Instant::now() + Duration::from_secs(10);
    while s.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < until, "did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!file.exists(), "endpoint file removed");
}

#[test]
fn the_http_mode_needs_a_place_for_the_token() {
    let out = Command::new(EXE)
        .args(["mcp", "--http", "--port", "0"])
        .env_remove("DPX_CONFIG_DIR")
        .env_remove("HOME")
        .env_remove("APPDATA")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("DPX_MCP_TOKEN")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("DPX_MCP_TOKEN"));
    let out = Command::new(EXE)
        .args(["mcp", "--http"])
        .env("DPX_MCP_TOKEN", "short")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let out = Command::new(EXE)
        .args(["mcp", "--port", "1"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "--port needs --http");
}

#[test]
fn the_bridge_forwards_stdio_to_the_http_server() {
    let s = HttpChild::start("http-bridge", &[]);
    let mut b = Client::spawn(
        Command::new(EXE)
            .arg("mcp-bridge")
            .env("DPX_CONFIG_DIR", &s.dir),
    );
    let init = b.initialize();
    assert_eq!(init["result"]["serverInfo"]["name"], "deluxepainter");
    b.ok(
        "create_document",
        json!({"preset": "pixelart_320x240", "chipset": "ocs"}),
    );
    b.ok(
        "draw_ellipse",
        json!({"cx": 160, "cy": 120, "rx": 20, "ry": 20, "color": 4, "filled": true}),
    );
    // The HTTP server has the change: ask it directly.
    let r = s.call(
        1,
        "get_pixels",
        json!({"rect": {"x": 160, "y": 120, "w": 1, "h": 1}}),
    );
    assert_eq!(r["structuredContent"]["pixels"], json!([[4]]));
    let st = b.ok("get_app_status", json!({}));
    assert_eq!(st["transport"], "http");
    b.close();
}

#[test]
fn the_bridge_starts_the_app_named_in_mcp_app_json() {
    let dir = scratch("http-launch");
    // A launch record that starts the headless HTTP server (the GUI would get `--mcp`).
    std::fs::write(
        dir.join("mcp-app.json"),
        serde_json::to_vec(&json!({"version": 1, "app_path": EXE,
            "args": ["mcp", "--http", "--port", "0"]}))
        .unwrap(),
    )
    .unwrap();
    let mut b = Client::spawn(
        Command::new(EXE)
            .arg("mcp-bridge")
            .env("DPX_CONFIG_DIR", &dir),
    );
    b.initialize();
    let st = b.ok("get_app_status", json!({}));
    assert_eq!(st["transport"], "http");
    let e = wait_for_endpoint(&dir, 1);
    b.close();
    // Stop the started server.
    let pid = e["pid"].as_u64().unwrap().to_string();
    #[cfg(unix)]
    let _ = Command::new("kill").args(["-TERM", &pid]).status();
    #[cfg(windows)]
    let _ = Command::new("taskkill").args(["/PID", &pid, "/F"]).status();
}
