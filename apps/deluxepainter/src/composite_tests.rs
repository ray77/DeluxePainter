//! ADR-44 in the app: a layered document shows, exports and saves as its visible layers
//! (the composite), while every gesture paints the active layer. These tests make layers
//! through the command engine, as `--commands` does, and drive the real `ui()` loop where the
//! screen is shown; the layer panel's tests are in `layers.rs`.

use std::borrow::Cow;

use dpx_core::{Command, IndexedImage, Layer, Palette, Shape};
use dpx_ui::Tool;
use eframe::egui;

use crate::tests::{key_event, pointer_button, ui_pass, ui_point};
use crate::{PainterApp, files, recovery};

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

/// The start page (PAL LoRes 320x256) in orange (4), and above it the layer "Ink" (active)
/// with a yellow (5) square at 10..=19.
fn layered_app() -> PainterApp {
    let mut app = PainterApp::new();
    app.engine.execute(Command::Clear { color: 4 }).unwrap();
    app.engine
        .execute(Command::AddLayer {
            name: Some("Ink".into()),
        })
        .unwrap();
    app.engine.execute(rect((10, 10), (19, 19), 5)).unwrap();
    app.dirty = true;
    app
}

/// Drags the left button over the page from `from` to `to` (page = UI pixels on LoRes) in
/// real `ui()` passes; `during` sees the app with the button still down.
fn drag(
    app: &mut PainterApp,
    ctx: &egui::Context,
    t: f64,
    from: (i32, i32),
    to: (i32, i32),
    during: impl FnOnce(&PainterApp),
) -> f64 {
    use egui::PointerButton::Primary;
    let (a, b) = (ui_point(app, from), ui_point(app, to));
    ui_pass(app, ctx, t, vec![egui::Event::PointerMoved(a)]);
    ui_pass(app, ctx, t + 0.02, vec![pointer_button(a, Primary, true)]);
    ui_pass(app, ctx, t + 0.04, vec![egui::Event::PointerMoved(b)]);
    during(app);
    ui_pass(app, ctx, t + 0.06, vec![pointer_button(b, Primary, false)]);
    t + 0.1
}

#[test]
fn the_screen_shows_every_visible_layer_while_gestures_paint_the_active_one() {
    let ctx = egui::Context::default();
    let mut app = layered_app();
    ui_pass(&mut app, &ctx, 0.0, vec![]);
    let screen = app.compose_screen();
    assert_eq!(screen.get(15, 15), Some(5), "the upper layer");
    assert_eq!(
        screen.get(60, 60),
        Some(4),
        "the bottom layer through colour 0"
    );
    // A freehand line on the upper layer: the preview shows over the bottom layer at once.
    app.select_tool(Tool::ContinuousFreehand, false);
    app.ui.fg = 2;
    let t = drag(&mut app, &ctx, 0.1, (40, 60), (80, 60), |app| {
        let shown = app.compose_screen();
        assert_eq!(shown.get(60, 60), Some(2), "the stroke preview");
        assert_eq!(shown.get(60, 70), Some(4), "the bottom layer around it");
        assert_eq!(shown.get(15, 15), Some(5), "the rest of the upper layer");
    });
    let doc = app.engine.document();
    assert_eq!(doc.active_layer(), 1);
    assert_eq!(doc.layer(1).unwrap().get(60, 60), Some(2), "painted above");
    assert!(
        doc.layer(0).unwrap().pixels().iter().all(|&p| p == 4),
        "the bottom layer is untouched"
    );
    // Colour 0 erases the upper layer: the bottom layer shows there again.
    app.ui.fg = 0;
    let t = drag(&mut app, &ctx, t, (5, 15), (30, 15), |_| {});
    let screen = app.compose_screen();
    assert_eq!(screen.get(15, 15), Some(4), "erased to the bottom layer");
    assert_eq!(screen.get(15, 16), Some(5));
    // Hidden layers do not show; `U` undoes the hiding.
    app.run(Command::SetLayerVisible {
        layer: 1,
        visible: false,
    });
    ui_pass(&mut app, &ctx, t, vec![]);
    assert_eq!(app.compose_screen().get(15, 16), Some(4));
    ui_pass(&mut app, &ctx, t + 0.1, vec![key_event(egui::Key::U, true)]);
    assert!(app.engine.document().layers()[1].visible);
    assert_eq!(app.compose_screen().get(15, 16), Some(5));
}

#[test]
fn magnify_light_table_and_show_page_show_the_composite() {
    let ctx = egui::Context::default();
    let mut app = layered_app();
    // Magnify: the right half shows the page as it looks.
    app.tools.magnify = Some((15, 15));
    let view = app.view();
    let m = app.magnify_layout(view).expect("magnify is on");
    let screen = app.compose_screen();
    let at = |px: i32, py: i32| {
        let (sx, sy) = (
            m.start + (px - m.source.x) * m.zoom,
            (py - m.source.y) * m.zoom,
        );
        screen.get(sx, sy)
    };
    assert_eq!(at(15, 15), Some(5));
    assert_eq!(at(8, 15), Some(4), "the bottom layer beside the square");
    app.tools.magnify = None;
    // The Light Table: frame 2 shows frame 1's upper layer through its BG pixels.
    app.engine.execute(Command::AddFrames { count: 1 }).unwrap();
    app.engine.execute(Command::Clear { color: 0 }).unwrap();
    app.engine
        .execute(Command::SelectLayer { layer: 0 })
        .unwrap();
    app.engine.execute(Command::Clear { color: 0 }).unwrap();
    app.anim.light_on = true;
    app.anim.light.dim = false;
    app.prepare_anim_view();
    let shown = app.anim.shown.take().expect("the Light Table picture");
    assert_eq!(shown.get(15, 15), Some(5), "frame 1's upper layer");
    assert_eq!(shown.get(60, 60), Some(4), "frame 1's bottom layer");
    app.anim.light_on = false;
    // Show Page renders the composite too (through `page()`).
    assert!(
        matches!(app.page(), Cow::Owned(_)),
        "two layers: a new picture"
    );
    app.engine.execute(Command::GotoFrame { frame: 0 }).unwrap();
    assert_eq!(app.page().get(15, 15), Some(5));
    ui_pass(&mut app, &ctx, 0.0, vec![]);
}

#[test]
fn a_single_layer_page_is_shown_without_a_copy() {
    let app = PainterApp::new();
    let shown = app.page();
    assert!(
        matches!(shown, Cow::Borrowed(p) if std::ptr::eq(p, app.engine.document().page())),
        "the page itself"
    );
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dpx-composite-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn picture_exports_hold_the_composite_and_say_so() {
    let mut app = layered_app();
    app.engine
        .execute(Command::AddLayer {
            name: Some("Hidden".into()),
        })
        .unwrap();
    app.engine.execute(Command::Clear { color: 9 }).unwrap();
    app.engine
        .execute(Command::SetLayerVisible {
            layer: 2,
            visible: false,
        })
        .unwrap();
    let doc = app.engine.document().clone();
    let shown = doc.composite().into_owned();
    assert_eq!((shown.get(15, 15), shown.get(60, 60)), (Some(5), Some(4)));
    let dir = temp("export");
    for name in ["a.png", "a.iff", "a.anim"] {
        let path = dir.join(name);
        let (bytes, notes) = files::encode_document(&doc, &app.extras, &path).unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("3 layers saved as one picture, as shown, 1 hidden")),
            "{name}: {notes:?}"
        );
        files::write_atomic(&path, &bytes).unwrap();
        let loaded = files::load(&path).unwrap();
        let (doc_back, _, _) = files::open_picture(loaded.picture, doc.target());
        assert_eq!(doc_back.page(), &shown, "{name}");
        assert_eq!(doc_back.layer_count(), 1);
    }
    // The project keeps every layer and says nothing.
    let path = dir.join("a.dpxproj");
    let (bytes, notes) = files::encode_document(&doc, &app.extras, &path).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    files::write_atomic(&path, &bytes).unwrap();
    let (back, _, _) = files::open_picture(files::load(&path).unwrap().picture, doc.target());
    assert_eq!(back, doc.with_palette(doc.palette().clone()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_recovery_project_keeps_every_layer() {
    let dir = temp("recovery");
    let mut app = layered_app();
    app.attach_recovery(recovery::Store::at(&dir));
    app.engine.execute(Command::AddFrames { count: 2 }).unwrap();
    app.engine.execute(rect((40, 40), (49, 49), 7)).unwrap();
    app.write_recovery();
    let saved = recovery::Store::at(&dir).project_path();
    let loaded = files::load(&saved).unwrap();
    let (back, _, _) = files::open_picture(loaded.picture, app.engine.document().target());
    let doc = app.engine.document();
    assert_eq!(back.layers(), doc.layers());
    assert_eq!((back.frame_count(), back.current_frame()), (3, 2));
    assert_eq!(back.active_layer(), 1);
    assert_eq!(
        back.cels().collect::<Vec<_>>(),
        doc.cels().collect::<Vec<_>>()
    );
    app.finish_session();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_small_layered_picture_grows_to_the_screen_on_every_layer() {
    let pal = Palette::dpaint_default_32();
    let frames = (0..2u8)
        .map(|f| {
            vec![
                IndexedImage::new(100, 50, 3 + f),
                IndexedImage::new(100, 50, 6),
            ]
        })
        .collect();
    let layers = vec![Layer::background(), Layer::new("Top")];
    let doc =
        dpx_core::Document::from_layers(frames, layers.clone(), 1, 1, 30, pal, Default::default())
            .unwrap();
    let (grown, note) = files::page_at_least_screen(doc);
    assert!(note.unwrap().contains("enlarged"));
    assert_eq!(grown.layers(), layers);
    assert_eq!((grown.current_frame(), grown.active_layer()), (1, 1));
    for f in 0..2 {
        let bottom = grown.cel(f, 0).unwrap();
        let top = grown.cel(f, 1).unwrap();
        assert_eq!((bottom.width(), bottom.height()), (320, 256));
        assert_eq!(
            (bottom.get(99, 49), bottom.get(100, 0)),
            (Some(3 + f as u8), Some(0))
        );
        assert_eq!((top.get(99, 49), top.get(100, 0)), (Some(6), Some(0)));
    }
}

#[test]
fn remap_repaints_every_frame_and_layer_and_keeps_colour_0_transparent() {
    let looks = |app: &PainterApp| -> Vec<Vec<dpx_core::Rgb8>> {
        let doc = app.engine.document();
        doc.composite_frames()
            .map(|f| {
                f.pixels()
                    .iter()
                    .map(|&p| doc.palette().get(p).unwrap())
                    .collect()
            })
            .collect()
    };
    let swapped = |app: &mut PainterApp, a: usize, b: usize| {
        app.refresh_effects();
        let mut colors = app.palette().colors().to_vec();
        colors.swap(a, b);
        app.run(Command::SetPalette { colors });
        app.refresh_effects();
    };
    // Colours 5 and 9 exchanged in the palette: Remap repaints both frames and layers.
    let mut app = layered_app();
    app.engine.execute(Command::AddFrames { count: 1 }).unwrap();
    let before = looks(&app);
    swapped(&mut app, 5, 9);
    assert_ne!(looks(&app), before);
    app.effect_action("color.remap");
    assert_eq!(looks(&app), before, "every frame looks as before");
    let doc = app.engine.document();
    for f in 0..2 {
        assert_eq!(doc.cel(f, 1).unwrap().get(15, 15), Some(9), "frame {f}");
        assert_eq!(doc.cel(f, 1).unwrap().get(60, 60), Some(0));
        assert_eq!(doc.cel(f, 0).unwrap().get(15, 15), Some(4));
    }
    app.undo();
    assert_eq!(
        app.engine.document().cel(0, 1).unwrap().get(15, 15),
        Some(5)
    );
    // Colours 0 and 5 exchanged: above the bottom layer 0 stays transparent and the yellow
    // goes to the nearest other entry, never to 0.
    let mut app = layered_app();
    app.engine.execute(Command::AddFrames { count: 1 }).unwrap();
    swapped(&mut app, 0, 5);
    app.effect_action("color.remap");
    let doc = app.engine.document();
    for f in 0..2 {
        let top = doc.cel(f, 1).unwrap();
        assert_eq!(top.get(60, 60), Some(0), "transparent stays transparent");
        assert_ne!(top.get(15, 15), Some(0), "nothing opaque turns transparent");
        assert_ne!(top.get(15, 15), Some(5), "repainted");
        assert_eq!(doc.cel(f, 0).unwrap().get(60, 60), Some(4));
    }
}

#[test]
fn a_single_layer_stroke_preview_is_shown_without_a_copy() {
    use egui::PointerButton::Primary;
    let ctx = egui::Context::default();
    let mut app = PainterApp::new();
    app.select_tool(Tool::ContinuousFreehand, false);
    app.ui.fg = 2;
    ui_pass(&mut app, &ctx, 0.0, vec![]);
    let (a, b) = (ui_point(&app, (40, 60)), ui_point(&app, (80, 60)));
    ui_pass(&mut app, &ctx, 0.1, vec![egui::Event::PointerMoved(a)]);
    ui_pass(&mut app, &ctx, 0.12, vec![pointer_button(a, Primary, true)]);
    ui_pass(&mut app, &ctx, 0.14, vec![egui::Event::PointerMoved(b)]);
    let stroke = app.stroke.as_ref().expect("a stroke in progress");
    assert!(
        matches!(app.page(), Cow::Borrowed(p) if std::ptr::eq(p, &stroke.preview)),
        "every redraw shows the preview itself (ADR-40)"
    );
    assert_eq!(app.page().get(60, 60), Some(2));
}

// ---- The spare page, the Light Table, the fixed background and Arrange with layers.

fn act(app: &mut PainterApp, id: &'static str) {
    app.run_action(dpx_ui::Action::Command(id), &egui::Context::default());
}

/// What `doc` shows at (`x`, `y`), as a colour.
fn looks(doc: &dpx_core::Document, x: i32, y: i32) -> dpx_core::Rgb8 {
    let p = doc.composite().get(x, y).unwrap();
    doc.palette().get(p).unwrap()
}

#[test]
fn arrange_changes_nothing_when_a_layered_spare_cannot_follow() {
    let mut app = PainterApp::new();
    // The spare: CLR 3, and an upper layer in colour 5.
    act(&mut app, "spare.swap");
    app.run(Command::Clear { color: 3 });
    app.run(Command::AddLayer { name: None });
    app.run(Command::Clear { color: 5 });
    act(&mut app, "spare.swap");
    fn spare_doc(app: &PainterApp) -> &dpx_core::Document {
        app.menu_data.spare.as_ref().unwrap().engine.document()
    }
    let spare_look = looks(spare_doc(&app), 5, 5);
    let before = app.engine.document().clone();
    // Entries 0 and 5 exchanged: colour 5 would become the transparent 0 on the spare's
    // upper layer. Nothing is arranged, neither page.
    let mut order = dpx_core::remap::identity(32);
    order.swap(0, 5);
    app.apply_arrange(&order);
    assert_eq!(
        app.engine.document(),
        &before,
        "the shown page is not arranged"
    );
    assert_eq!(spare_doc(&app).palette(), before.palette());
    act(&mut app, "spare.swap");
    assert_eq!(
        looks(app.engine.document(), 5, 5),
        spare_look,
        "the spare looks the same"
    );
    act(&mut app, "spare.swap");
    // An order the spare can take: both pages follow and keep their look.
    let mut order = dpx_core::remap::identity(32);
    order.swap(3, 5);
    app.apply_arrange(&order);
    assert_ne!(app.engine.document().palette(), before.palette());
    assert_eq!(spare_doc(&app).palette(), app.engine.document().palette());
    act(&mut app, "spare.swap");
    assert_eq!(looks(app.engine.document(), 5, 5), spare_look);
}

#[test]
fn light_table_merge_puts_what_showed_into_the_active_layer() {
    let mut app = PainterApp::new();
    let e = &mut app.engine;
    // Frame 1 in colour 1; frame 2 in colour 2 with a hole of colour 0 (BG) at 0..=9.
    e.execute(Command::Clear { color: 1 }).unwrap();
    e.execute(Command::AddFrames { count: 1 }).unwrap();
    e.execute(Command::Clear { color: 2 }).unwrap();
    e.execute(rect((0, 0), (9, 9), 0)).unwrap();
    // An upper layer (active) with one yellow pixel.
    e.execute(Command::AddLayer { name: None }).unwrap();
    e.execute(rect((50, 50), (50, 50), 5)).unwrap();
    let bottom = e.document().layer(0).unwrap().clone();
    app.ui.bg = 0;
    app.anim.light_on = true;
    app.anim.light.dim = false;
    app.prepare_anim_view();
    let view = app.anim.shown.take().expect("the Light Table picture");
    assert_eq!(
        (view.get(5, 5), view.get(50, 50), view.get(100, 100)),
        (Some(1), Some(5), Some(2))
    );
    act(&mut app, "light_table.merge");
    let doc = app.engine.document();
    assert_eq!(
        &*doc.composite(),
        &view,
        "the frame now shows what the table showed"
    );
    let upper = doc.layer(1).unwrap();
    assert_eq!(
        upper.get(5, 5),
        Some(1),
        "the hole filled on the active layer"
    );
    assert_eq!(
        upper.get(100, 100),
        Some(0),
        "the rest of it stays transparent"
    );
    assert_eq!(doc.layer(0).unwrap(), &bottom);
    app.undo();
    assert_eq!(app.engine.document().layer(1).unwrap().get(5, 5), Some(0));
}

#[test]
fn copy_to_spare_and_delete_this_page_carry_every_layer() {
    // Delete This Page: a layered page is replaced by the spare, all of it, in one step.
    let mut app = PainterApp::new();
    act(&mut app, "spare.swap");
    app.run(Command::Clear { color: 7 });
    act(&mut app, "spare.swap");
    app.run(Command::Clear { color: 1 });
    app.run(Command::AddLayer { name: None });
    app.run(Command::Clear { color: 4 });
    app.run(Command::SelectLayer { layer: 0 });
    act(&mut app, "spare.delete");
    let doc = app.engine.document();
    assert_eq!((doc.layer_count(), doc.composite().get(0, 0)), (1, Some(7)));
    assert!(app.menu_data.spare.is_none());
    app.undo();
    let doc = app.engine.document();
    assert_eq!((doc.layer_count(), doc.composite().get(0, 0)), (2, Some(4)));
    // Copy to Spare onto a layered spare: the spare becomes the shown page, colour 0 too
    // (it no longer lands on the spare's transparent upper layer).
    let mut app = PainterApp::new();
    act(&mut app, "spare.swap");
    app.run(Command::Clear { color: 3 });
    app.run(Command::AddLayer { name: None });
    act(&mut app, "spare.swap");
    act(&mut app, "spare.copy");
    act(&mut app, "spare.swap");
    let doc = app.engine.document();
    assert_eq!(
        (doc.layer_count(), doc.composite().get(100, 100)),
        (1, Some(0))
    );
    app.undo();
    let doc = app.engine.document();
    assert_eq!(
        (doc.layer_count(), doc.composite().get(100, 100)),
        (2, Some(3)),
        "one undo step on the spare brings its picture back"
    );
    // The backup workflow keeps the layers: Copy to Spare, paint, Delete This Page.
    let mut app = layered_app();
    let kept = app.engine.document().clone();
    act(&mut app, "spare.copy");
    app.run(Command::Clear { color: 9 });
    app.run(Command::DeleteLayer { layer: 0 });
    act(&mut app, "spare.delete");
    let doc = app.engine.document();
    assert_eq!(doc.layers(), kept.layers());
    assert_eq!(
        doc.cels().collect::<Vec<_>>(),
        kept.cels().collect::<Vec<_>>()
    );
    assert_eq!(doc.active_layer(), kept.active_layer());
}

#[test]
fn merge_in_back_fills_only_what_shows_colour_0() {
    // The spare in colour 7; the page in colour 1 with a hole of colour 0 at (2, 2), and an
    // empty upper layer (active).
    let mut app = PainterApp::new();
    act(&mut app, "spare.swap");
    app.run(Command::Clear { color: 7 });
    act(&mut app, "spare.swap");
    app.run(Command::Clear { color: 1 });
    app.run(rect((2, 2), (2, 2), 0));
    app.run(Command::AddLayer { name: None });
    act(&mut app, "spare.merge_back");
    let doc = app.engine.document();
    assert_eq!(
        doc.composite().get(100, 100),
        Some(1),
        "the picture stays in front"
    );
    assert_eq!(
        doc.composite().get(2, 2),
        Some(7),
        "the spare shows through its hole"
    );
    assert_eq!(
        doc.layer(1).unwrap().get(2, 2),
        Some(7),
        "on the active layer"
    );
    assert_eq!(doc.layer(0).unwrap().get(2, 2), Some(0));
    app.undo();
    assert_eq!(app.engine.document().composite().get(2, 2), Some(0));
    // In front the spare covers the picture, on the active layer.
    act(&mut app, "spare.merge_front");
    assert_eq!(app.engine.document().composite().get(100, 100), Some(7));
}

#[test]
fn clr_restores_the_fixed_background_only_on_its_layer() {
    let mut app = PainterApp::new();
    app.ui.bg = 0;
    app.run(Command::Clear { color: 1 });
    app.fix_background();
    app.run(Command::AddLayer { name: None });
    app.run(Command::Clear { color: 9 });
    app.run(Command::AddLayer { name: None });
    app.run(rect((5, 5), (5, 5), 6));
    // CLR on the top layer clears it (BG 0: transparent); the middle layer shows.
    app.run(Command::Clear { color: 0 });
    let doc = app.engine.document();
    assert_eq!(doc.layer(2).unwrap().get(5, 5), Some(0));
    assert_eq!(doc.composite().get(5, 5), Some(9));
    // On the layer it was fixed on, CLR restores it.
    app.run(Command::SelectLayer { layer: 0 });
    app.run(rect((7, 7), (7, 7), 3));
    // Lock FG finds what was painted on that layer since Fix, whichever layer is active.
    app.run(Command::SelectLayer { layer: 2 });
    act(&mut app, "background.lock_fg");
    let area = app.active_area().expect("an area stencil");
    assert!(area.protects(7, 7) && !area.protects(5, 5));
    app.menu_data.palette_stencil.area = None;
    app.run(Command::SelectLayer { layer: 0 });
    app.run(Command::Clear { color: 0 });
    let doc = app.engine.document();
    assert_eq!(
        doc.layer(0).unwrap().get(7, 7),
        Some(1),
        "the fixed picture again"
    );
    assert_eq!(doc.composite().get(7, 7), Some(9));
}
