//! Spawns `deluxepainter mcp` and talks newline-delimited JSON-RPC over its stdio.

mod common;

use std::process::{Command, Stdio};

use common::{Client, EXE, base64, base64_decode, gradient_png, scratch};
use serde_json::{Value, json};

#[test]
fn initialize_list_and_call_tools() {
    let mut c = Client::mcp(&[]);
    let init = c.initialize();
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    assert!(
        init["result"]["capabilities"]["resources"].is_object(),
        "{init}"
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "deluxepainter");

    let list = c.request("tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for want in ["get_app_status", "create_document", "get_document_info"] {
        assert!(names.contains(&want), "{names:?}");
    }
    for old in [
        "app_status",
        "new_document",
        "get_document_state",
        "render_png",
        "save_picture",
    ] {
        assert!(!names.contains(&old), "{old} replaced");
    }

    let status = c.ok("get_app_status", json!({}));
    assert_eq!(status["name"], "deluxepainter");
    assert_eq!(status["transport"], "stdio");
    assert_eq!(status["document"]["width"], 320);

    let created = c.ok(
        "create_document",
        json!({"video": "ntsc", "resolution": "hi", "colors": 16}),
    );
    assert_eq!(created["document"]["width"], 640);
    assert_eq!(created["document"]["height"], 200);

    let state = c.ok("get_document_info", json!({}));
    let mut want = created["document"].clone();
    want["success"] = json!(true);
    assert_eq!(state, want);
    assert_eq!(state["colors"], 16);

    assert_eq!(
        c.code("create_document", json!({"colors": 33})),
        "PALETTE_RANGE_INVALID"
    );

    // Without --allow-dir every file tool is refused.
    for (tool, path) in [
        ("open_picture", "/etc/hosts"),
        ("export_image", "/tmp/x.iff"),
        ("save_project", "/tmp/x.dpxproj"),
        ("import_image", "/tmp/x.png"),
    ] {
        assert_eq!(
            c.code(tool, json!({"path": path})),
            "FILE_ACCESS_OFF",
            "{tool}"
        );
    }
    c.close();
}

#[test]
fn spec_example_320x240_ocs_and_refused_ocs_hires() {
    let mut c = Client::mcp(&[]);
    c.initialize();
    let d = c.ok(
        "create_document",
        json!({"width": 320, "height": 240, "chipset": "OCS", "palette_size": 32, "name": "Burglandschaft"}),
    );
    assert_eq!(d["document"]["display_mode"], "PAL:Low Res");
    assert_eq!(d["document"]["screen_height"], 240);
    assert_eq!(d["document"]["chipset"], "ocs");
    assert_eq!(d["document"]["colors"], 32);
    let r = c.call(
        "create_document",
        json!({"width": 640, "height": 256, "chipset": "ocs", "colors": 32}),
    );
    assert_eq!(r["isError"], true);
    let msg = r["structuredContent"]["message"].as_str().unwrap();
    assert!(msg.contains("at most 16 colours"), "{msg}");
    c.close();
}

#[test]
fn paint_look_undo_and_files() {
    let dir = scratch("stdio-files");
    let mut c = Client::mcp(&["--allow-dir", dir.to_str().unwrap()]);
    c.initialize();

    let list = c.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    for want in [
        "execute_command",
        "undo",
        "redo",
        "get_palette",
        "get_pixels",
        "get_preview",
        "open_picture",
        "save_project",
        "export_image",
    ] {
        let t = tools.iter().find(|t| t["name"] == want);
        let t = t.unwrap_or_else(|| panic!("tool {want} missing"));
        assert!(t["description"].as_str().unwrap().len() > 10);
        assert!(t["inputSchema"].is_object());
        assert!(t["annotations"]["readOnlyHint"].is_boolean(), "{t}");
    }

    c.ok("create_document", json!({}));
    let palette = c.ok("get_palette", json!({}));
    let red = palette["colors"][5].as_str().unwrap().to_string();

    let done = c.ok(
        "draw_rectangle",
        json!({"x": 10, "y": 10, "width": 10, "height": 5, "color": 5, "filled": true}),
    );
    assert_eq!(
        done["changed_rect"],
        json!({"x": 10, "y": 10, "w": 10, "h": 5})
    );

    // Look at it: decode the PNG and check a pixel.
    let img = c.call(
        "get_preview",
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
    assert_eq!(img["structuredContent"]["image_width"], 64);

    // Undo; the pixel is background again.
    let undone = c.ok("undo", json!({}));
    assert_eq!(undone["can_redo"], true);
    let px_rows = c.ok(
        "get_pixels",
        json!({"rect": {"x": 15, "y": 13, "w": 1, "h": 1}}),
    );
    assert_eq!(px_rows["pixels"], json!([[0]]));
    c.ok("redo", json!({}));

    // Export, refuse overwrite, refuse paths outside the approved directory.
    let saved = c.ok("export_image", json!({"path": "t.iff"}));
    assert_eq!(saved["format"], "iff_ilbm");
    assert!(dir.join("t.iff").is_file());
    assert_eq!(
        c.code("export_image", json!({"path": "t.iff"})),
        "FILE_EXISTS"
    );
    c.ok("export_image", json!({"path": "t.iff", "overwrite": true}));
    assert_eq!(
        c.code("open_picture", json!({"path": "/etc/hosts"})),
        "PATH_NOT_ALLOWED"
    );
    assert_eq!(
        c.code("open_picture", json!({"path": "../../etc/hosts"})),
        "PATH_NOT_ALLOWED"
    );
    let p = c.ok("save_project", json!({"path": "t.dpxproj"}));
    assert_eq!(p["format"], "dpxproj");
    c.ok("create_document", json!({"colors": 8}));
    for path in ["t.iff", "t.dpxproj"] {
        let opened = c.ok("open_picture", json!({"path": path}));
        assert_eq!(opened["document"]["width"], 320, "{path}");
        let px = c.ok(
            "get_pixels",
            json!({"rect": {"x": 15, "y": 13, "w": 1, "h": 1}}),
        );
        assert_eq!(px["pixels"], json!([[5]]), "{path}");
    }
    c.close();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn argument_errors_are_coded_results() {
    let mut c = Client::mcp(&[]);
    c.initialize();
    let r = c.call("create_document", json!({"colors": 70000}));
    assert_eq!(r["isError"], true);
    assert_eq!(r["structuredContent"]["code"], "INVALID_ARGUMENT", "{r}");
    assert_eq!(
        c.code(
            "get_pixels",
            json!({"rect": {"x": 0, "y": 0, "w": -1, "h": 1}})
        ),
        "INVALID_ARGUMENT"
    );
    let r = c.request(
        "tools/call",
        json!({"name": "no_such_tool", "arguments": {}}),
    );
    assert!(
        r.get("error").is_some(),
        "unknown tools are protocol errors: {r}"
    );
    c.close();
}

#[test]
fn ham_and_ehb_pictures_over_stdio() {
    let dir = scratch("stdio-special");
    let data = format!(
        "{}/../../crates/dpx-iff/tests/data",
        env!("CARGO_MANIFEST_DIR")
    );
    for f in ["ehb-16x2.iff", "ham6-16x2.iff"] {
        std::fs::copy(format!("{data}/{f}"), dir.join(f)).unwrap();
    }
    let mut c = Client::mcp(&["--allow-dir", dir.to_str().unwrap()]);
    c.initialize();
    assert_eq!(
        c.code("open_picture", json!({"path": "ham6-16x2.iff"})),
        "UNSUPPORTED_FILE_TYPE"
    );
    c.ok("open_picture", json!({"path": "ehb-16x2.iff"}));
    c.ok("export_image", json!({"path": "out.png"}));
    assert_eq!(
        c.code(
            "open_picture",
            json!({"path": "//attacker.invalid/s/a.iff"})
        ),
        "PATH_NOT_ALLOWED"
    );
    c.close();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_allow_dir_fails_at_startup() {
    let out = Command::new(EXE)
        .args(["mcp", "--allow-dir", "/definitely/not/a/dir"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "stdout stays protocol-only");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--allow-dir"));
}

/// Requests written at once, without waiting for answers, run in the order they were
/// written: a slow import first, then quick drawing calls that must land on the imported
/// picture (run before it, the import would wipe them).
#[test]
fn pipelined_calls_run_in_arrival_order() {
    let mut c = Client::mcp(&[]);
    c.initialize();
    let data = base64(&gradient_png(1600, 1200));
    let mut ids = Vec::new();
    let import_id = c.take_id();
    c.send(
        &json!({"jsonrpc": "2.0", "id": import_id, "method": "tools/call",
        "params": {"name": "import_image", "arguments": {"data": data, "colors": 32}}}),
    );
    for i in 0..24u64 {
        let id = c.take_id();
        ids.push(id);
        c.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "draw_pixel", "arguments": {"x": i, "y": 0, "color": 1 + i % 30}}}));
    }
    let mut answers = std::collections::HashMap::new();
    while answers.len() < 25 {
        let v = c.read();
        answers.insert(v["id"].as_u64().unwrap(), v);
    }
    let import = &answers[&import_id]["result"];
    assert_eq!(import["isError"], false, "{import}");
    assert_eq!(
        import["structuredContent"]["revision"], 0,
        "a fresh picture"
    );
    for (n, id) in ids.iter().enumerate() {
        let r = &answers[id]["result"]["structuredContent"];
        assert_eq!(
            r["revision"],
            n as u64 + 1,
            "call {n} ran in its place: {r}"
        );
    }
    let row = c.ok(
        "get_pixels",
        json!({"rect": {"x": 0, "y": 0, "w": 24, "h": 1}}),
    );
    let want: Vec<Value> = (0..24u64).map(|i| json!(1 + i % 30)).collect();
    assert_eq!(row["pixels"][0], Value::Array(want));
    c.close();
}

#[test]
fn resources_over_stdio() {
    let mut c = Client::mcp(&[]);
    c.initialize();
    let list = c.request("resources/list", json!({}));
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert_eq!(
        uris,
        [
            "dpx://document/info",
            "dpx://document/palette",
            "dpx://document/preview.png"
        ]
    );
    let r = c.request("resources/read", json!({"uri": "dpx://document/palette"}));
    let text = r["result"]["contents"][0]["text"].as_str().unwrap();
    let p: Value = serde_json::from_str(text).unwrap();
    assert_eq!(p["size"], 32);
    let r = c.request(
        "resources/read",
        json!({"uri": "dpx://document/preview.png"}),
    );
    assert_eq!(r["result"]["contents"][0]["mimeType"], "image/png");
    assert!(r["result"]["contents"][0]["blob"].as_str().unwrap().len() > 100);
    let r = c.request("resources/read", json!({"uri": "dpx://nope"}));
    assert!(r.get("error").is_some(), "{r}");
    c.close();
}
