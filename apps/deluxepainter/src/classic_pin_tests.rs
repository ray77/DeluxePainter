//! The Classic guard of ADR-47: the classic screen ([`PainterApp::compose_screen`]) and the
//! classic snapshot ([`snapshot_rgba`]) of a few fixed scenes are pinned by hash. Work on the
//! Modern skin must leave every value here alone. A hash changes only when the classic look of
//! its scene changes on purpose; then the new value is printed by the failing assertion and is
//! replaced together with the regenerated screenshots.
//!
//! The scenes avoid the Prefs menu, which ADR-47 makes two rows taller.

use dpx_core::{Command, Shape};
use dpx_ui::{Pref, ThemeChoice, Tool, UiStyle};
use eframe::egui;

use crate::tests::ui_pass;
use crate::{PainterApp, page_size_requester, snapshot_rgba};

/// FNV-1a, 64 bit: enough to notice any changed pixel, and no dependency.
fn fnv(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for &b in *part {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Hashes of the classic screen as the window shows it and of the snapshot at scale 2.
pub(crate) fn pins(app: &PainterApp) -> (u64, u64) {
    let screen = app.compose_screen();
    let composed = fnv(&[
        &screen.width().to_le_bytes(),
        &screen.height().to_le_bytes(),
        screen.pixels(),
    ]);
    let doc = app.engine.document();
    let (w, h, rgba) = snapshot_rgba(2, doc, app.view(), app.scroll, &app.ui, false);
    let snapshot = fnv(&[&w.to_le_bytes(), &h.to_le_bytes(), &rgba]);
    (composed, snapshot)
}

fn rect(from: (i32, i32), to: (i32, i32), color: u8) -> Command {
    Command::Shape {
        shape: Shape::Rect { from, to },
        brush: vec![],
        color,
        filled: true,
        symmetry: None,
        custom: None,
        mode: Default::default(),
        range: None,
        stencil: None,
        right: false,
        fx: Default::default(),
    }
}

/// The start screen after one real `ui()` pass.
pub(crate) fn scene_start() -> PainterApp {
    let mut app = PainterApp::new();
    ui_pass(&mut app, &egui::Context::default(), 0.0, vec![]);
    app
}

/// A painted page, the filled rectangle tool, other colours, Coords on, and the Mode menu
/// held open on its second item.
pub(crate) fn scene_painted_with_menu() -> PainterApp {
    let mut app = PainterApp::new();
    app.engine.execute(Command::Clear { color: 2 }).unwrap();
    app.engine.execute(rect((20, 30), (120, 90), 7)).unwrap();
    app.engine.execute(rect((100, 70), (250, 200), 12)).unwrap();
    app.ui.tool = Tool::Rectangle;
    app.ui.filled = true;
    app.ui.brush = 3;
    app.ui.fg = 9;
    app.ui.bg = 5;
    app.ui.prefs.toggle(Pref::Coords);
    app.dirty = true;
    ui_pass(&mut app, &egui::Context::default(), 0.0, vec![]);
    app.ui.coords = Some((123, 45));
    app.ui.menu.active = true;
    app.ui.menu.menu = Some(2);
    app.ui.menu.item = Some(1);
    app
}

/// Two layers, the layer panel (Prefs > Layers) and Magnify.
pub(crate) fn scene_layers_and_magnify() -> PainterApp {
    let mut app = PainterApp::new();
    app.engine.execute(Command::Clear { color: 4 }).unwrap();
    app.engine
        .execute(Command::AddLayer {
            name: Some("Ink".into()),
        })
        .unwrap();
    app.engine.execute(rect((10, 10), (19, 19), 5)).unwrap();
    app.ui.prefs.toggle(Pref::LayerPanel);
    app.tools.magnify = Some((15, 15));
    app.dirty = true;
    ui_pass(&mut app, &egui::Context::default(), 0.0, vec![]);
    app
}

/// A requester over the screen: Picture > Page Size.
pub(crate) fn scene_requester() -> PainterApp {
    let mut app = scene_start();
    app.ui.page_size_requester = Some(page_size_requester(app.engine.document()));
    app
}

/// Every pinned scene with its name and its expected hashes.
pub(crate) fn scenes() -> [(&'static str, PainterApp, (u64, u64)); 4] {
    [
        (
            "start",
            scene_start(),
            (0x4447_d11c_950f_38b8, 0xc9be_e553_c6f0_0a05),
        ),
        (
            "painted_with_menu",
            scene_painted_with_menu(),
            (0x4a35_3612_e314_424a, 0x4473_07e0_e747_1fb5),
        ),
        (
            "layers_and_magnify",
            scene_layers_and_magnify(),
            (0xd9bc_1d4d_2641_300a, 0x8219_4712_b104_ddd5),
        ),
        (
            "requester",
            scene_requester(),
            (0x9739_7969_a6df_09f4, 0xaf0b_f364_9356_2da5),
        ),
    ]
}

#[test]
fn classic_screen_and_snapshot_are_pinned() {
    // All scenes are compared before failing, so one run prints every changed value.
    let changed: Vec<String> = scenes()
        .iter()
        .filter_map(|(name, app, expected)| {
            let got = pins(app);
            (got != *expected)
                .then(|| format!("(\"{name}\", .., ({:#018x}, {:#018x}))", got.0, got.1))
        })
        .collect();
    assert!(
        changed.is_empty(),
        "classic scenes changed:\n{}",
        changed.join("\n")
    );
}

#[test]
fn the_scenes_differ_and_the_hash_sees_one_pixel() {
    let all: Vec<(u64, u64)> = scenes().iter().map(|(_, app, _)| pins(app)).collect();
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a.0, b.0);
            assert_ne!(a.1, b.1);
        }
    }
    // One changed page pixel changes both hashes.
    let mut app = scene_start();
    let before = pins(&app);
    app.engine.execute(rect((50, 50), (50, 50), 3)).unwrap();
    app.dirty = true;
    let after = pins(&app);
    assert_ne!(before.0, after.0);
    assert_ne!(before.1, after.1);
}

#[test]
fn switching_to_modern_and_back_leaves_classic_alone() {
    // Classic -> Modern -> Classic, with real `ui()` passes in between; then every scene
    // still has its pinned look. The theme belongs to the Modern skin only.
    let ctx = egui::Context::default();
    for (name, mut app, _) in scenes() {
        let before = pins(&app);
        let (menu, requester) = (app.ui.menu, app.ui.page_size_requester.take());
        app.ui.menu.close();
        let coords = app.ui.coords;
        for (i, (style, theme)) in [
            (UiStyle::Modern, ThemeChoice::Light),
            (UiStyle::Modern, ThemeChoice::Dark),
            (UiStyle::Classic, ThemeChoice::Dark),
        ]
        .into_iter()
        .enumerate()
        {
            app.ui_style = style;
            app.theme = theme;
            app.dirty = true;
            ui_pass(&mut app, &ctx, 1.0 + i as f64, vec![]);
        }
        assert_eq!(app.ui_style, UiStyle::Classic);
        app.ui.menu = menu;
        app.ui.page_size_requester = requester;
        app.ui.coords = coords;
        assert_eq!(
            pins(&app),
            before,
            "classic scene '{name}' after the round trip"
        );
    }
}
