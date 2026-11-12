//! Tool tests against the headless workspace, through the same `call` the servers use.

use super::*;
use crate::jobs::spawn_headless;
use crate::state::{Overwrite, Settings};
use crate::workspace::HeadlessWorkspace;
use serde_json::{Value, json};

struct T {
    rt: tokio::runtime::Runtime,
    ctx: ToolCtx,
}

impl T {
    fn new() -> Self {
        Self::with(Settings::headless(FileAccess::off()))
    }

    fn with(settings: Settings) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let exec = spawn_headless(HeadlessWorkspace::default());
        Self {
            rt,
            ctx: ToolCtx {
                exec,
                state: ServerState::new(settings),
                transport: "test",
            },
        }
    }

    fn in_dir(dir: &Path) -> Self {
        Self::with(Settings::headless(
            FileAccess::new([dir.to_path_buf()]).unwrap(),
        ))
    }

    fn call(&self, name: &str, args: Value) -> CallToolResult {
        self.rt
            .block_on(call(&self.ctx, name, args.as_object().cloned()))
            .expect("known tool")
    }

    fn ok(&self, name: &str, args: Value) -> Value {
        let r = self.call(name, args);
        assert_eq!(
            r.is_error,
            Some(false),
            "{name}: {:?}",
            r.structured_content
        );
        let s = r.structured_content.unwrap();
        assert_eq!(s["success"], true);
        s
    }

    fn err(&self, name: &str, args: Value) -> (String, String) {
        let r = self.call(name, args);
        assert_eq!(r.is_error, Some(true), "{name}: {:?}", r.structured_content);
        let s = r.structured_content.unwrap();
        assert_eq!(s["success"], false);
        (
            s["code"].as_str().unwrap().to_string(),
            s["message"].as_str().unwrap().to_string(),
        )
    }

    fn pixel(&self, x: i32, y: i32) -> u64 {
        let p = self.ok(
            "get_pixels",
            json!({"rect": {"x": x, "y": y, "w": 1, "h": 1}}),
        );
        p["pixels"][0][0].as_u64().unwrap()
    }
}

#[test]
fn definitions_are_complete_and_annotated() {
    let defs = definitions();
    let names: Vec<&str> = defs.iter().map(|t| t.name.as_ref()).collect();
    for want in [
        "get_app_status",
        "list_documents",
        "get_document_info",
        "create_document",
        "select_document",
        "get_palette",
        "set_palette_color",
        "draw_pixel",
        "draw_line",
        "draw_rectangle",
        "draw_ellipse",
        "fill_area",
        "import_image",
        "undo",
        "redo",
        "save_project",
        "export_image",
        "get_preview",
        "open_picture",
        "execute_command",
        "get_pixels",
    ] {
        assert!(names.contains(&want), "{want} missing");
    }
    for old in [
        "app_status",
        "new_document",
        "get_document_state",
        "render_png",
        "save_picture",
    ] {
        assert!(!names.contains(&old), "{old} must be gone");
    }
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "unique names");
    for t in &defs {
        let a = t.annotations.as_ref().unwrap();
        assert!(t.description.as_ref().unwrap().len() > 20, "{}", t.name);
        assert_eq!(t.input_schema["type"], "object", "{}", t.name);
        assert_eq!(a.read_only_hint, Some(is_read_only(&t.name)));
    }
    let get = |n: &str| defs.iter().find(|t| t.name == n).unwrap();
    assert_eq!(
        get("get_preview")
            .annotations
            .as_ref()
            .unwrap()
            .read_only_hint,
        Some(true)
    );
    assert_eq!(
        get("open_picture")
            .annotations
            .as_ref()
            .unwrap()
            .destructive_hint,
        Some(true)
    );
    assert_eq!(
        get("draw_pixel")
            .annotations
            .as_ref()
            .unwrap()
            .destructive_hint,
        Some(false)
    );
    let props = get("create_document").input_schema["properties"].clone();
    assert!(props.get("chipset").is_some() && props.get("preset").is_some());
}

#[test]
fn unknown_tools_and_bad_arguments() {
    let t = T::new();
    assert!(t.rt.block_on(call(&t.ctx, "nope", None)).is_none());
    let (code, msg) = t.err("draw_pixel", json!({"x": 1, "y": 1}));
    assert_eq!(code, "INVALID_ARGUMENT");
    assert!(msg.contains("color"), "{msg}");
    let (code, _) = t.err(
        "draw_pixel",
        json!({"x": 1, "y": 1, "color": 1, "colour": 2}),
    );
    assert_eq!(code, "INVALID_ARGUMENT", "unknown fields are refused");
    let (code, _) = t.err("execute_command", json!({"command": {"op": "nope"}}));
    assert_eq!(code, "INVALID_ARGUMENT");
}

#[test]
fn status_and_documents() {
    let t = T::new();
    let s = t.ok("get_app_status", json!({}));
    assert_eq!(s["name"], "deluxepainter");
    assert_eq!(s["engine"], "headless");
    assert_eq!(s["transport"], "test");
    assert_eq!(s["permissions"]["read"], true);
    assert_eq!(s["file_access"]["enabled"], false);
    assert_eq!(s["document"]["width"], 320);
    assert_eq!(s["document"]["document_id"], "page1");
    let l = t.ok("list_documents", json!({}));
    assert_eq!(l["active"], "page1");
    assert_eq!(l["documents"][0]["active"], true);
    assert_eq!(l["documents"][1]["blank"], true);
    let i = t.ok("get_document_info", json!({}));
    assert_eq!(i["chipset"], "aga");
    assert_eq!(i["display_mode"], "PAL:Low Res");
    assert_eq!(i["colors"], 32);
    assert_eq!(i["max_colors"], 256);
    assert_eq!(i["modified"], true);
    let (code, _) = t.err("get_document_info", json!({"document_id": "page2"}));
    assert_eq!(code, "DOCUMENT_NOT_FOUND");
    let (code, _) = t.err("select_document", json!({"document_id": "spare"}));
    assert_eq!(code, "DOCUMENT_NOT_FOUND");
}

#[test]
fn select_document_swaps_once_and_ids_stay_with_their_page() {
    let t = T::new();
    t.ok("draw_pixel", json!({"x": 0, "y": 0, "color": 5}));
    let s = t.ok("select_document", json!({"document_id": "page2"}));
    assert_eq!(s["changed"], true);
    assert_eq!(s["document_id"], "page2");
    assert_eq!(t.pixel(0, 0), 0, "the spare is blank");
    let again = t.ok("select_document", json!({"document_id": "page2"}));
    assert_eq!(again["changed"], false, "idempotent");
    let other = t.ok("get_document_info", json!({"document_id": "page1"}));
    assert_eq!(other["role"], "spare");
    assert_eq!(other["active"], false);
    t.ok("select_document", json!({"document_id": "page1"}));
    assert_eq!(t.pixel(0, 0), 5);
}

fn info_of(t: &T, args: Value) -> Value {
    let c = t.ok("create_document", args);
    assert_eq!(c["document_id"], c["document"]["document_id"]);
    c
}

#[test]
fn create_320x240_ocs_32_colours() {
    let t = T::new();
    // Spec example: chipset in capitals and palette_size.
    let c = info_of(
        &t,
        json!({"width": 320, "height": 240, "chipset": "OCS", "palette_size": 32, "name": "Castle"}),
    );
    let d = &c["document"];
    assert_eq!(
        (d["width"].clone(), d["height"].clone()),
        (json!(320), json!(240))
    );
    assert_eq!(d["display_mode"], "PAL:Low Res");
    assert_eq!(
        (d["screen_width"].clone(), d["screen_height"].clone()),
        (json!(320), json!(240))
    );
    assert_eq!(d["custom_screen"], true);
    assert!(d["notes"][0].as_str().unwrap().contains("custom 320x240"));
    assert_eq!(d["chipset"], "ocs");
    assert_eq!(d["colors"], 32);
    assert_eq!(d["name"], "Castle");
    // AGA -> OCS replaces the document and says so.
    assert_eq!(c["undoable"], false);
    assert!(c["notes"][0].as_str().unwrap().contains("not undoable"));
    // The palette is the 12-bit start palette.
    let p = t.ok("get_palette", json!({}));
    assert_eq!(p["bits_per_channel"], 4);
    for c in p["colors"].as_array().unwrap() {
        let c = parse_color(c.as_str().unwrap()).unwrap();
        assert!(is_12_bit(c), "{c}");
    }
}

#[test]
fn ocs_hires_with_32_colours_is_refused_clearly() {
    let t = T::new();
    let before = t.ok("get_document_info", json!({}));
    let (code, msg) = t.err(
        "create_document",
        json!({"width": 640, "height": 256, "chipset": "ocs", "colors": 32}),
    );
    assert_eq!(code, "PALETTE_RANGE_INVALID");
    assert!(
        msg.contains("OCS PAL:High Res shows at most 16 colours"),
        "{msg}"
    );
    assert!(msg.contains("AGA"), "{msg}");
    let (code, msg) = t.err(
        "create_document",
        json!({"chipset": "ocs", "resolution": "super_hi"}),
    );
    assert_eq!(code, "UNSUPPORTED_MODE");
    assert!(msg.contains("AGA"), "{msg}");
    let (code, _) = t.err("create_document", json!({"colors": 12}));
    assert_eq!(code, "PALETTE_RANGE_INVALID");
    let (code, _) = t.err("create_document", json!({"chipset": "ecs"}));
    assert_eq!(code, "INVALID_ARGUMENT", "no ECS");
    let (code, _) = t.err(
        "create_document",
        json!({"preset": "pal_lores", "video": "ntsc"}),
    );
    assert_eq!(code, "INVALID_ARGUMENT");
    assert_eq!(t.ok("get_document_info", json!({})), before, "unchanged");
}

#[test]
fn sizes_choose_the_matching_mode() {
    let t = T::new();
    for (w, h, mode, screen) in [
        (320, 200, "NTSC:Low Res", (320, 200)),
        (320, 256, "PAL:Low Res", (320, 256)),
        (640, 200, "NTSC:High Res", (640, 200)),
        (640, 256, "PAL:High Res", (640, 256)),
        (640, 400, "NTSC:High Res Laced", (640, 400)),
        (640, 512, "PAL:High Res Laced", (640, 512)),
        (640, 480, "PAL:High Res Laced", (640, 480)),
        (360, 280, "PAL:Low Res", (360, 280)),
        // Larger than any screen: a page on the guessed mode's standard screen.
        (1000, 900, "PAL:High Res Laced", (640, 512)),
    ] {
        t.ok("create_document", json!({"width": w, "height": h}));
        let d = t.ok("get_document_info", json!({}));
        assert_eq!(d["display_mode"], mode, "{w}x{h}");
        assert_eq!(
            (
                d["screen_width"].as_u64().unwrap(),
                d["screen_height"].as_u64().unwrap()
            ),
            (screen.0, screen.1),
            "{w}x{h}"
        );
        assert_eq!(
            (d["width"].clone(), d["height"].clone()),
            (json!(w), json!(h))
        );
    }
    // Smaller than every screen of the guessed mode: refused with the reason.
    let (_, msg) = t.err("create_document", json!({"width": 300, "height": 150}));
    assert!(msg.contains("smaller"), "{msg}");
}

#[test]
fn presets_and_explicit_modes() {
    let t = T::new();
    for (preset, mode, size) in [
        ("ntsc_lores", "NTSC:Low Res", (320, 200)),
        ("pal_lores", "PAL:Low Res", (320, 256)),
        ("pixelart_320x240", "PAL:Low Res", (320, 240)),
        ("ntsc_hires", "NTSC:High Res", (640, 200)),
        ("pal_hires", "PAL:High Res", (640, 256)),
        ("ntsc_interlace", "NTSC:High Res Laced", (640, 400)),
        ("pal_interlace", "PAL:High Res Laced", (640, 512)),
    ] {
        let c = t.ok("create_document", json!({"preset": preset}));
        let d = &c["document"];
        assert_eq!(d["display_mode"], mode, "{preset}");
        assert_eq!(
            (d["width"].clone(), d["height"].clone()),
            (json!(size.0), json!(size.1))
        );
    }
    let c = t.ok(
        "create_document",
        json!({"video": "ntsc", "resolution": "hi", "colors": 16}),
    );
    assert_eq!(c["document"]["display_mode"], "NTSC:High Res");
    assert_eq!(c["document"]["colors"], 16);
    // No size: the current format, default 32 colours (16 on OCS High Res).
    let c = t.ok("create_document", json!({"chipset": "ocs"}));
    assert_eq!(c["document"]["colors"], 16);
}

#[test]
fn create_document_keeping_the_chipset_is_undoable() {
    let t = T::new();
    t.ok("draw_pixel", json!({"x": 3, "y": 3, "color": 7}));
    let c = t.ok(
        "create_document",
        json!({"width": 320, "height": 200, "colors": 16}),
    );
    assert_eq!(c["undoable"], true);
    assert_eq!(c["document"]["colors"], 16);
    assert_eq!(t.pixel(3, 3), 0);
    t.ok("undo", json!({}));
    let d = t.ok("get_document_info", json!({}));
    assert_eq!(d["display_mode"], "PAL:Low Res");
    assert_eq!(t.pixel(3, 3), 7);
}

#[test]
fn palette_colours_follow_the_chipset() {
    let t = T::new();
    t.ok(
        "create_document",
        json!({"width": 320, "height": 240, "chipset": "ocs"}),
    );
    let (code, msg) = t.err("set_palette_color", json!({"index": 1, "color": "#123456"}));
    assert_eq!(code, "HARDWARE_LIMIT");
    assert!(
        msg.contains("#123456") && msg.contains("nearest 12-bit colour is #113355"),
        "{msg}"
    );
    let r = t.ok("set_palette_color", json!({"index": 1, "color": "#F80"}));
    assert_eq!(r["color"], "#FF8800");
    assert_eq!(r["palette_changed"], true);
    assert_eq!(r["document_id"], "page1");
    assert_eq!(t.ok("get_palette", json!({}))["colors"][1], "#FF8800");
    let (code, _) = t.err("set_palette_color", json!({"index": 32, "color": "#000"}));
    assert_eq!(code, "PALETTE_RANGE_INVALID");
    let (code, _) = t.err("set_palette_color", json!({"index": 1, "color": "red"}));
    assert_eq!(code, "INVALID_ARGUMENT");
    t.ok("undo", json!({}));
    assert_ne!(t.ok("get_palette", json!({}))["colors"][1], "#FF8800");
    // AGA takes any 24-bit colour.
    t.ok("create_document", json!({"chipset": "aga"}));
    t.ok("set_palette_color", json!({"index": 2, "color": "123456"}));
    assert_eq!(t.ok("get_palette", json!({}))["colors"][2], "#123456");
}

#[test]
fn drawing_tools_paint_through_the_engine() {
    let t = T::new();
    let r = t.ok(
        "draw_rectangle",
        json!({"x": 10, "y": 10, "width": 10, "height": 5, "color": 5, "filled": true}),
    );
    assert_eq!(r["revision"], 1);
    assert_eq!(
        r["changed_rect"],
        json!({"x": 10, "y": 10, "w": 10, "h": 5})
    );
    assert_eq!(r["can_undo"], true);
    assert_eq!(t.pixel(19, 14), 5);
    assert_eq!(t.pixel(20, 14), 0);
    t.ok(
        "draw_rectangle",
        json!({"x": 40, "y": 40, "width": 5, "height": 5, "color": 6}),
    );
    assert_eq!(t.pixel(40, 42), 6, "outline");
    assert_eq!(t.pixel(42, 42), 0, "hollow");
    t.ok(
        "draw_line",
        json!({"x1": 0, "y1": 100, "x2": 9, "y2": 100, "color": 3}),
    );
    assert_eq!(t.pixel(9, 100), 3);
    t.ok("draw_pixel", json!({"x": 200, "y": 200, "color": 4}));
    assert_eq!(t.pixel(200, 200), 4);
    t.ok(
        "draw_ellipse",
        json!({"cx": 100, "cy": 150, "rx": 10, "ry": 6, "color": 2, "filled": true}),
    );
    assert_eq!(t.pixel(100, 150), 2);
    assert_eq!(t.pixel(100, 157), 0);
    t.ok(
        "draw_ellipse",
        json!({"cx": 250, "cy": 150, "rx": 10, "ry": 10, "color": 2}),
    );
    assert_eq!(t.pixel(250, 150), 0, "outline only");
    assert_eq!(t.pixel(260, 150), 2);
    t.ok("fill_area", json!({"x": 300, "y": 10, "color": 9}));
    assert_eq!(t.pixel(0, 0), 9);
    assert_eq!(t.pixel(12, 12), 5, "inside the filled rectangle stays");
    let (code, _) = t.err("draw_pixel", json!({"x": 1, "y": 1, "color": 200}));
    assert_eq!(code, "PALETTE_RANGE_INVALID");
    let (code, _) = t.err(
        "draw_rectangle",
        json!({"x": 1, "y": 1, "width": 0, "height": 3, "color": 1}),
    );
    assert_eq!(code, "INVALID_ARGUMENT");
    // Undo and redo, one step each.
    let u = t.ok("undo", json!({}));
    assert_eq!(u["can_redo"], true);
    assert_eq!(t.pixel(0, 0), 0);
    t.ok("redo", json!({}));
    assert_eq!(t.pixel(0, 0), 9);
    let (code, _) = t.err("redo", json!({}));
    assert_eq!(code, "NOTHING_TO_REDO");
}

#[test]
fn execute_command_dry_run_and_revision_guard() {
    let t = T::new();
    let cmd = json!({"op": "shape", "shape": {"kind": "rect", "from": [1, 1], "to": [4, 4]}, "brush": [[0, 0]], "color": 5, "filled": true});
    let dry = t.ok("execute_command", json!({"command": cmd, "dry_run": true}));
    assert_eq!(dry["dry_run"], true);
    assert_eq!(dry["revision"], 0);
    assert_eq!(t.pixel(2, 2), 0);
    t.ok("execute_command", json!({"command": cmd}));
    let (code, _) = t.err(
        "execute_command",
        json!({"command": cmd, "expected_revision": 0}),
    );
    assert_eq!(code, "REVISION_CONFLICT");
}

#[test]
fn permissions_are_enforced() {
    let mut s = Settings::headless(FileAccess::off());
    s.read = false;
    s.edit = false;
    let t = T::with(s);
    let st = t.ok("get_app_status", json!({}));
    assert_eq!(st["permissions"]["read"], false);
    assert!(st["document"].is_null(), "no document details without read");
    for (tool, args) in [
        ("get_palette", json!({})),
        ("get_preview", json!({})),
        ("draw_pixel", json!({"x": 1, "y": 1, "color": 1})),
        ("undo", json!({})),
        ("export_image", json!({"path": "a.iff"})),
    ] {
        let (code, msg) = t.err(tool, args);
        assert_eq!(code, "PERMISSION_DENIED", "{tool}");
        assert!(msg.contains("Prefs > AI & MCP"), "{msg}");
    }
    // Read only.
    let mut s = Settings::headless(FileAccess::off());
    s.edit = false;
    let t = T::with(s);
    t.ok("get_palette", json!({}));
    let (code, _) = t.err("set_palette_color", json!({"index": 1, "color": "#000"}));
    assert_eq!(code, "PERMISSION_DENIED");
    // Settings change live.
    t.ctx
        .state
        .set_settings(Settings::headless(FileAccess::off()));
    t.ok("set_palette_color", json!({"index": 1, "color": "#000"}));
}

#[test]
fn reading_off_refuses_changes_too_and_every_call_is_logged() {
    let mut s = Settings::headless(FileAccess::off());
    s.read = false;
    let t = T::with(s);
    let (code, msg) = t.err("draw_pixel", json!({"x": 1, "y": 1, "color": 1}));
    assert_eq!(code, "PERMISSION_DENIED");
    assert!(msg.starts_with("reading the document"), "{msg}");
    t.ok("get_app_status", json!({}));
    let log = t.ctx.state.activity().log();
    assert_eq!(log.len(), 2);
    assert_eq!(
        (log[0].tool.as_str(), log[0].ok, log[0].code.as_deref()),
        ("draw_pixel", false, Some("PERMISSION_DENIED"))
    );
    assert!(log[1].ok && log[1].tool == "get_app_status");
}

#[test]
fn the_overwrite_question_names_the_client() {
    let q = overwrite_question(Some("claude-ai"), "castle.iff");
    assert_eq!(q.text, "Claude wants to replace castle.iff. Replace?");
    assert_eq!((q.yes.as_str(), q.no.as_str()), ("Replace", "Keep"));
    let q = overwrite_question(None, "a.png");
    assert!(q.text.starts_with("An AI client wants"));
}

fn decode_png(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

#[test]
fn preview_is_a_png_image_and_pixels_are_capped() {
    let t = T::new();
    t.ok(
        "draw_rectangle",
        json!({"x": 10, "y": 10, "width": 10, "height": 5, "color": 5, "filled": true}),
    );
    let r = t.call(
        "get_preview",
        json!({"rect": {"x": 8, "y": 8, "w": 4, "h": 4}, "scale": 3}),
    );
    assert_eq!(r.is_error, Some(false));
    let img = r.content[0].as_image().expect("image first");
    assert_eq!(img.mime_type, "image/png");
    let (w, h, rgb) = decode_png(&view::from_base64(&img.data).unwrap());
    assert_eq!((w, h), (12, 12));
    let s = r.structured_content.unwrap();
    assert_eq!(s["scale"], 3);
    assert_eq!(s["image_width"], 12);
    let want = t.ok("get_palette", json!({}))["colors"][5]
        .as_str()
        .unwrap()
        .to_string();
    let at = |x: usize, y: usize| {
        let i = (y * 12 + x) * 3;
        format!("#{:02X}{:02X}{:02X}", rgb[i], rgb[i + 1], rgb[i + 2])
    };
    assert_eq!(at(11, 11), want);
    assert_ne!(at(0, 0), want);
    // Default: the whole page, scaled 3x (longer side within 1024).
    let s = t.call("get_preview", json!({})).structured_content.unwrap();
    assert_eq!(
        (s["image_width"].clone(), s["image_height"].clone()),
        (json!(960), json!(768))
    );
    let (code, _) = t.err("get_preview", json!({"scale": 9}));
    assert_eq!(code, "INVALID_ARGUMENT");
    let (code, _) = t.err("get_pixels", json!({}));
    assert_eq!(code, "INVALID_ARGUMENT", "320x256 exceeds the pixel cap");
    let p = t.ok(
        "get_pixels",
        json!({"rect": {"x": 300, "y": 250, "w": 100, "h": 100}, "encoding": "base64"}),
    );
    assert_eq!((p["w"].clone(), p["h"].clone()), (json!(20), json!(6)));
    assert_eq!(p["document_id"], "page1");
}

/// A `w`×`h` RGBA PNG: left half red, right half blue, the top-left pixel transparent if asked.
fn rgba_png(w: u32, h: u32, transparent_corner: bool) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let a = if transparent_corner && x == 0 && y == 0 {
                0
            } else {
                255
            };
            if x < w / 2 {
                data.extend_from_slice(&[255, 0, 0, a]);
            } else {
                data.extend_from_slice(&[0, 0, 255, a]);
            }
        }
    }
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    out
}

#[test]
fn import_image_into_picture_spare_and_brush() {
    let t = T::new();
    let data = view::to_base64(&rgba_png(64, 32, false));
    let r = t.ok(
        "import_image",
        json!({"data": data, "colors": 4, "dither": "none"}),
    );
    assert_eq!(r["target"], "picture");
    assert_eq!(r["colors"], 4);
    assert_eq!(r["undoable"], false);
    assert_eq!(
        (r["width"].clone(), r["height"].clone()),
        (json!(320), json!(256))
    );
    // 64x32 fitted into 320x256 keeping its shape: 320x160, centred (48 lines above).
    assert_eq!(r["size"], json!([320, 160]));
    assert_eq!(r["at"], json!([0, 48]));
    let p = t.ok("get_palette", json!({}));
    assert_eq!(p["size"], 4);
    let red = t.pixel(10, 100);
    let blue = t.pixel(300, 100);
    assert_ne!(red, blue);
    assert_eq!(p["colors"][red as usize], "#FF0000");
    assert_eq!(p["colors"][blue as usize], "#0000FF");
    let d = t.ok("get_document_info", json!({}));
    assert_eq!(d["display_mode"], "PAL:Low Res", "the format stays");

    // Spare: the picture's palette; a new palette is refused.
    let (code, _) = t.err(
        "import_image",
        json!({"data": data, "target": "spare", "palette": "new"}),
    );
    assert_eq!(code, "INVALID_ARGUMENT");
    let r = t.ok(
        "import_image",
        json!({"data": data, "target": "spare", "position": "top_left"}),
    );
    assert_eq!(r["palette"], "current");
    assert_eq!(r["undoable"], true);
    t.ok("select_document", json!({"document_id": "page2"}));
    assert_eq!(t.pixel(10, 10), red, "spare top-left in the shared palette");
    t.ok("select_document", json!({"document_id": "page1"}));

    // Brush: its own pixel size, transparent pixels become colour 0.
    let small = view::to_base64(&rgba_png(8, 6, true));
    let r = t.ok("import_image", json!({"data": small, "target": "brush"}));
    assert_eq!(
        (r["width"].clone(), r["height"].clone()),
        (json!(8), json!(6))
    );
    assert!(r["notes"][0].as_str().unwrap().contains("transparent"));
    let s = t.ok("get_app_status", json!({}));
    assert_eq!(s["brush"], json!([8, 6]));

    // Errors.
    let (code, _) = t.err("import_image", json!({}));
    assert_eq!(code, "INVALID_ARGUMENT");
    let (code, _) = t.err("import_image", json!({"data": data, "colors": 3}));
    assert_eq!(code, "PALETTE_RANGE_INVALID", "not a power of two");
}

#[test]
fn import_image_on_ocs_rounds_to_12_bit_and_refuses_too_many_colours() {
    let t = T::new();
    t.ok(
        "create_document",
        json!({"width": 320, "height": 240, "chipset": "ocs"}),
    );
    let data = view::to_base64(&rgba_png(16, 16, false));
    let (code, msg) = t.err("import_image", json!({"data": data, "colors": 64}));
    assert_eq!(code, "PALETTE_RANGE_INVALID");
    assert!(msg.contains("32"), "{msg}");
    let r = t.ok("import_image", json!({"data": data, "colors": 32}));
    assert_eq!(r["colors"], 32);
    for c in t.ok("get_palette", json!({}))["colors"].as_array().unwrap() {
        assert!(is_12_bit(parse_color(c.as_str().unwrap()).unwrap()));
    }
    let (code, _) = t.err("import_image", json!({"data": "aGVsbG8="}));
    assert_eq!(code, "UNSUPPORTED_FILE_TYPE");
}

#[test]
fn files_save_open_export_and_overwrite_rules() {
    let dir = crate::access::tests::scratch("tools-files");
    let t = T::in_dir(&dir);
    t.ok(
        "create_document",
        json!({"width": 320, "height": 240, "chipset": "ocs", "colors": 32}),
    );
    t.ok(
        "draw_rectangle",
        json!({"x": 10, "y": 10, "width": 10, "height": 5, "color": 5, "filled": true}),
    );
    let s = t.ok("save_project", json!({"path": "castle.dpxproj"}));
    assert_eq!(s["format"], "dpxproj");
    assert_eq!(t.ok("get_document_info", json!({}))["modified"], false);
    let (code, _) = t.err("save_project", json!({"path": "castle.dpxproj"}));
    assert_eq!(code, "FILE_EXISTS");
    t.ok(
        "save_project",
        json!({"path": "castle.dpxproj", "overwrite": true}),
    );
    let (code, _) = t.err("save_project", json!({"path": "castle.iff"}));
    assert_eq!(code, "UNSUPPORTED_FILE_TYPE");

    for (path, format) in [
        ("c.iff", "iff_ilbm"),
        ("c.png", "png"),
        ("c.anim", "iff_anim"),
    ] {
        let e = t.ok("export_image", json!({"path": path}));
        assert_eq!(e["format"], format);
        assert!(
            e["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n.as_str().unwrap().contains("custom 320x240"))
        );
        assert!(dir.join(path).is_file());
    }
    let e = t.ok(
        "export_image",
        json!({"path": "noext", "format": "iff_ilbm"}),
    );
    assert_eq!(e["format"], "iff_ilbm");
    let (code, _) = t.err(
        "export_image",
        json!({"path": "c.png", "format": "iff_ilbm"}),
    );
    assert_eq!(code, "INVALID_ARGUMENT");
    let (code, _) = t.err("export_image", json!({"path": "c.iff"}));
    assert_eq!(code, "FILE_EXISTS");
    for p in ["c.jpg", "c.dpxproj"] {
        let (code, _) = t.err("export_image", json!({"path": p}));
        assert_eq!(code, "UNSUPPORTED_FILE_TYPE", "{p}");
    }
    let (code, _) = t.err("export_image", json!({"path": "../escape.iff"}));
    assert_eq!(code, "PATH_NOT_ALLOWED");

    // Re-open the project and the ILBM: size, colours, pixels and palette survive.
    t.ok("create_document", json!({"chipset": "aga"}));
    for path in ["castle.dpxproj", "c.iff"] {
        let o = t.ok("open_picture", json!({"path": path}));
        let d = &o["document"];
        assert_eq!(o["undoable"], false);
        assert_eq!(
            (d["width"].clone(), d["height"].clone()),
            (json!(320), json!(240)),
            "{path}"
        );
        assert_eq!(d["colors"], 32);
        assert_eq!(d["modified"], false);
        assert_eq!(t.pixel(12, 12), 5, "{path}");
    }
    let o = t.ok("open_picture", json!({"path": "castle.dpxproj"}));
    assert_eq!(o["document"]["chipset"], "ocs", "the project keeps OCS");

    // With Ask, the headless workspace cannot ask: the file is kept.
    let mut s = t.ctx.state.settings();
    s.overwrite = Overwrite::Ask;
    t.ctx.state.set_settings(s);
    let before = std::fs::read(dir.join("c.png")).unwrap();
    let (code, _) = t.err("export_image", json!({"path": "c.png", "overwrite": true}));
    assert_eq!(code, "OVERWRITE_DECLINED");
    assert_eq!(std::fs::read(dir.join("c.png")).unwrap(), before);
    // A new file needs no question.
    t.ok(
        "export_image",
        json!({"path": "new.png", "overwrite": true}),
    );
    // "Without asking" needs both permissions: with editing off the user is asked (the
    // headless workspace answers no), as the GUI ghosts that box then.
    let mut s = t.ctx.state.settings();
    s.overwrite = Overwrite::Allowed;
    s.edit = false;
    t.ctx.state.set_settings(s.clone());
    let (code, _) = t.err("export_image", json!({"path": "c.png", "overwrite": true}));
    assert_eq!(code, "OVERWRITE_DECLINED");
    assert_eq!(std::fs::read(dir.join("c.png")).unwrap(), before);
    let st = t.ok("get_app_status", json!({}));
    assert_eq!(st["permissions"]["overwrite_without_asking"], false);
    s.edit = true;
    t.ctx.state.set_settings(s);
    let st = t.ok("get_app_status", json!({}));
    assert_eq!(st["permissions"]["overwrite_without_asking"], true);

    // Import from a path.
    std::fs::write(dir.join("in.png"), rgba_png(32, 32, false)).unwrap();
    t.ok("import_image", json!({"path": "in.png", "colors": 2}));
    let (code, _) = t.err("import_image", json!({"path": "c.iff"}));
    assert_eq!(code, "UNSUPPORTED_FILE_TYPE");
    let (code, _) = t.err("open_picture", json!({"path": "in.png"}));
    assert_eq!(
        code, "UNSUPPORTED_FILE_TYPE",
        "true colour goes through import_image"
    );
}

#[test]
fn file_tools_are_off_without_approved_directories() {
    let t = T::new();
    for (tool, args) in [
        ("open_picture", json!({"path": "x.iff"})),
        ("save_project", json!({"path": "x.dpxproj"})),
        ("export_image", json!({"path": "x.iff"})),
        ("import_image", json!({"path": "x.png"})),
    ] {
        let (code, _) = t.err(tool, args);
        assert_eq!(code, "FILE_ACCESS_OFF", "{tool}");
    }
}

#[test]
fn resources_read_info_palette_and_preview() {
    let t = T::new();
    let names: Vec<String> = resources().iter().map(|r| r.uri.clone()).collect();
    assert_eq!(names, [RESOURCE_INFO, RESOURCE_PALETTE, RESOURCE_PREVIEW]);
    let r = t.rt.block_on(read_resource(&t.ctx, RESOURCE_INFO)).unwrap();
    let text = match &r.contents[0] {
        ResourceContents::TextResourceContents { text, .. } => text.clone(),
        other => panic!("{other:?}"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["width"], 320);
    let r =
        t.rt.block_on(read_resource(&t.ctx, RESOURCE_PREVIEW))
            .unwrap();
    match &r.contents[0] {
        ResourceContents::BlobResourceContents {
            blob, mime_type, ..
        } => {
            assert_eq!(mime_type.as_deref(), Some("image/png"));
            let (w, h, _) = decode_png(&view::from_base64(blob).unwrap());
            assert_eq!((w, h), (960, 768));
        }
        other => panic!("{other:?}"),
    }
    let e =
        t.rt.block_on(read_resource(&t.ctx, "dpx://nope"))
            .unwrap_err();
    assert_eq!(e.code, "RESOURCE_NOT_FOUND");
}

#[test]
fn activity_counts_calls_and_the_log_records_changes() {
    let t = T::new();
    t.ok("draw_pixel", json!({"x": 1, "y": 2, "color": 3}));
    t.ok("get_palette", json!({}));
    assert_eq!(t.ctx.state.activity().snapshot().tool_calls, 2);
    let log =
        t.rt.block_on(t.ctx.exec.run(|ws| {
            // The headless workspace's own log through the trait object.
            let mut lines = Vec::new();
            ws.log("probe");
            lines.push(ws.kind().to_string());
            lines
        }))
        .unwrap();
    assert_eq!(log, ["headless"]);
}
