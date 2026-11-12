//! The app side of ADR-46 (chipset, screen presets and hardware rules):
//!
//! - Picture ▸ Export.. and Anim ▸ Save.. refuse an IFF file the display mode cannot show on
//!   its chipset with a note ([`PainterApp::refuse_unshowable`]); the rules are
//!   `dpx_core::hardware`'s, the check is `files::check_hardware`.
//! - Palette colours that come from elsewhere (Palette ▸ Load.., Restore Palette) are rounded
//!   to 12 bits on an OCS picture, with a note ([`PainterApp::chipset_colors`]); the Screen
//!   Format requester names the colours a change to OCS rounds ([`rounded_note`], counted
//!   by the engine in `Outcome::rounded_colors`).
//! - `--chipset` and `--preset` set up the picture of a `--snapshot` ([`snapshot_format`]);
//!   `--requester-keys` drives the Screen Format requester ([`snapshot_requester`]) and
//!   `--export-note` shows what Picture ▸ Export.. would say ([`snapshot_export_note`]).

use std::path::Path;

use dpx_core::mode::{PRESETS, Preset};
use dpx_core::{Chipset, Command, Document, Engine, Rgb8, hardware};
use dpx_ui::NoteRequester;
use dpx_ui::requester::{ReqEvent, ReqKey, ReqResponse, ScreenFormatRequester};

use crate::files::{self, PictureExtras, SaveFormat};
use crate::{PainterApp, arg_value};

/// Note title after a Screen Format change that rounded palette colours.
pub(crate) const FORMAT_CHANGED: &str = "Screen format changed";
/// Note titles of a refused IFF export.
pub(crate) const NOT_EXPORTED: &str = "Picture not exported";
pub(crate) const ANIM_NOT_SAVED: &str = "Animation not saved";

/// The note after a Screen Format change that rounded `n` palette colours, if any (the
/// engine counts them: `Outcome::rounded_colors`).
pub(crate) fn rounded_note(n: u16) -> Option<NoteRequester> {
    (n > 0).then(|| NoteRequester::new(FORMAT_CHANGED, &[hardware::rounded_note(n.into())]))
}

/// `--chipset ocs|aga` and `--preset ID` of a `--snapshot`: one SetFormat that keeps the
/// picture (the page becomes the preset's screen, or grows to the screen), with the palette
/// size limited to the mode's maximum.
pub(crate) fn snapshot_format(args: &[String], engine: &mut Engine) -> Result<(), String> {
    let chipset = match arg_value(args, "--chipset").as_deref() {
        None => None,
        Some("ocs") => Some(Chipset::Ocs),
        Some("aga") => Some(Chipset::Aga),
        Some(other) => return Err(format!("--chipset expects ocs or aga, not '{other}'")),
    };
    let preset = match arg_value(args, "--preset") {
        None => None,
        Some(id) => Some(Preset::by_id(&id).and_then(|p| p.format).ok_or_else(|| {
            let ids: Vec<&str> = PRESETS
                .iter()
                .filter(|p| p.format.is_some())
                .map(|p| p.id)
                .collect();
            format!("--preset {id}: expected one of {}", ids.join(", "))
        })?),
    };
    if chipset.is_none() && preset.is_none() {
        return Ok(());
    }
    let doc = engine.document();
    let t = doc.target();
    let chipset = chipset.unwrap_or(t.chipset);
    let (mode, screen) = preset.unwrap_or((t.mode, t.screen));
    let max = mode.max_colors(chipset).unwrap_or(2);
    let colors = (doc.palette().len().clamp(2, 256) as u16)
        .next_power_of_two()
        .min(max);
    let (width, height) = match preset {
        Some(_) => screen,
        None => (
            doc.page().width().max(screen.0),
            doc.page().height().max(screen.1),
        ),
    };
    engine
        .execute(Command::SetFormat {
            mode,
            colors,
            width,
            height,
            retain: true,
            screen: Some(screen),
            chipset: Some(chipset),
        })
        .map(|_| ())
        .map_err(|e| format!("--chipset/--preset: {}", e.message))
}

/// `--requester` of a `--snapshot`: the Screen Format requester for `doc` after the keys of
/// `--requester-keys` (comma-separated: `up`, `down`, `tab`, `enter`, or one character such
/// as `h` for Chipset, `p` for Preset, `r`, `u`). A Use the engine refuses shows its reason,
/// as in the program; an accepted Use or Cancel would close it and is an error here.
pub(crate) fn snapshot_requester(
    args: &[String],
    doc: &Document,
) -> Result<ScreenFormatRequester, String> {
    let page = (doc.page().width(), doc.page().height());
    let mut req = ScreenFormatRequester::new(doc.target(), page, doc.palette().len());
    let Some(spec) = arg_value(args, "--requester-keys") else {
        return Ok(req);
    };
    for token in spec.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let key = match token {
            "up" => ReqKey::Up,
            "down" => ReqKey::Down,
            "tab" => ReqKey::Tab,
            "enter" => ReqKey::Enter,
            t if t.chars().count() == 1 => ReqKey::Char(t.chars().next().unwrap_or(' ')),
            t => return Err(format!("--requester-keys: unknown key '{t}'")),
        };
        match req.handle(ReqEvent::Key(key)) {
            ReqResponse::Continue => {}
            ReqResponse::Cancel => {
                return Err("--requester-keys: the requester was cancelled".into());
            }
            ReqResponse::Use(command) => match Engine::new(doc.clone()).execute(command) {
                Ok(_) => {
                    return Err("--requester-keys: Use was accepted (the requester closes)".into());
                }
                Err(e) => req.set_message(e.message),
            },
        }
    }
    Ok(req)
}

/// `--export-note NAME` of a `--snapshot`: the note Picture ▸ Export.. shows for `doc` under
/// that name, without writing anything: the refusal of an IFF file the display mode cannot
/// show, or the notes of a written one (`None` when there is nothing to say).
pub(crate) fn snapshot_export_note(
    name: &str,
    doc: &Document,
    extras: &PictureExtras,
) -> Result<Option<NoteRequester>, String> {
    let path = Path::new(name);
    let format = files::save_format(path)?;
    if let Err(rule) = files::check_hardware(doc, format) {
        let title = if format == SaveFormat::Anim {
            ANIM_NOT_SAVED
        } else {
            NOT_EXPORTED
        };
        return Ok(Some(NoteRequester::new(
            title,
            &[rule, files::REFUSED_HINT.to_string()],
        )));
    }
    let (_, notes) = files::encode_document(doc, extras, path)?;
    let title = if format == SaveFormat::Anim {
        crate::ANIM_SAVED
    } else {
        crate::EXPORTED
    };
    Ok(crate::note_requester(title, &notes))
}

impl PainterApp {
    /// ADR-46: an IFF ILBM or ANIM export the picture's display mode cannot show is refused:
    /// the file requester closes and a note names the broken rule. Returns whether it was
    /// refused (nothing is written then).
    pub(crate) fn refuse_unshowable(&mut self, path: &Path) -> bool {
        let Ok(format) = files::save_format(path) else {
            return false;
        };
        let Err(rule) = files::check_hardware(self.engine.document(), format) else {
            return false;
        };
        eprintln!("{}: {}", path.display(), files::refused(&rule));
        self.ui.file_requester = None;
        let title = if format == SaveFormat::Anim {
            ANIM_NOT_SAVED
        } else {
            NOT_EXPORTED
        };
        self.ui.note_requester = Some(NoteRequester::new(
            title,
            &[rule, files::REFUSED_HINT.to_string()],
        ));
        true
    }

    /// Palette colours from elsewhere as the picture's chipset shows them: rounded to 12 bits
    /// on OCS, unchanged on AGA (ADR-46). Returns the note to show when any was rounded.
    pub(crate) fn chipset_colors(&self, colors: &mut [Rgb8]) -> Option<String> {
        let chipset = self.engine.document().target().chipset;
        let n = hardware::snap_colors(chipset, colors);
        (n > 0).then(|| hardware::rounded_note(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{click_ui, key_event, menu_item_path, pointer_button, ui_pass, ui_point};
    use dpx_core::{DisplayMode, IndexedImage, Palette, Resolution, Target, Video};
    use dpx_ui::requester::{self, ScreenFormatRequester};
    use eframe::egui;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-adr46-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    /// Picks the top-level menu item `id` as on the Amiga: right button down on the title,
    /// along to the item, up over it.
    fn pick(app: &mut PainterApp, ctx: &egui::Context, t: f64, id: &'static str) {
        use egui::PointerButton::Secondary;
        let [title, item] = menu_item_path(app, id).map(|p| ui_point(app, p));
        ui_pass(
            app,
            ctx,
            t,
            vec![
                egui::Event::PointerMoved(title),
                pointer_button(title, Secondary, true),
            ],
        );
        ui_pass(app, ctx, t + 0.02, vec![egui::Event::PointerMoved(item)]);
        ui_pass(
            app,
            ctx,
            t + 0.04,
            vec![pointer_button(item, Secondary, false)],
        );
    }

    /// A click on the middle of a Screen Format requester gadget (requester pixels on the
    /// HiRes requester screen; UI pixels are LoRes, two requester pixels wide).
    fn click_gadget(app: &mut PainterApp, ctx: &egui::Context, t: f64, g: dpx_core::Rect) {
        let (ux, uy) = requester::SCREEN.ui_scale();
        let at = ((g.x + g.w / 2) / ux as i32, (g.y + g.h / 2) / uy as i32);
        click_ui(app, ctx, t, at, egui::PointerButton::Primary);
    }

    fn typed(app: &mut PainterApp, ctx: &egui::Context, t: f64, text: &str) {
        ui_pass(app, ctx, t, vec![egui::Event::Text(text.into())]);
    }

    fn enter(app: &mut PainterApp, ctx: &egui::Context, t: f64) {
        ui_pass(app, ctx, t, vec![key_event(egui::Key::Enter, true)]);
        ui_pass(app, ctx, t + 0.02, vec![key_event(egui::Key::Enter, false)]);
    }

    #[test]
    fn screen_format_chipset_and_preset_through_the_ui_loop() {
        let ctx = egui::Context::default();
        let mut app = PainterApp::new();
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pick(&mut app, &ctx, 0.1, "picture.screen_format");
        let req = app.requester.as_ref().expect("the Screen Format requester");
        assert_eq!(
            (req.chipset(), req.preset().id),
            (Chipset::Aga, "pal_lores")
        );
        // Chipset: OCS; Preset: next is "Pixel art 320x240 (custom)".
        click_gadget(&mut app, &ctx, 0.3, ScreenFormatRequester::chipset_gadget());
        click_gadget(&mut app, &ctx, 0.5, ScreenFormatRequester::preset_gadget());
        let req = app.requester.as_ref().unwrap();
        assert_eq!(req.chipset(), Chipset::Ocs);
        assert_eq!(req.preset().label, "Pixel art 320x240 (custom)");
        assert_eq!(
            (req.screen_size(), req.page_size()),
            ((320, 240), (320, 240))
        );
        click_gadget(&mut app, &ctx, 0.7, ScreenFormatRequester::use_button());
        assert!(app.requester.is_none(), "Use closed it");
        let doc = app.engine.document();
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES).with_screen(320, 240);
        assert_eq!(doc.target(), ocs);
        assert_eq!((doc.page().width(), doc.page().height()), (320, 240));
        assert_eq!(doc.palette().len(), 32);
        assert!(hardware::validate(doc).is_ok());
        assert!(
            app.ui.note_requester.is_none(),
            "the default palette is 12-bit"
        );
        assert_eq!(app.view().ui_size(), (320, 240), "the screen follows");
        // Keys: P cycles the preset, H the chipset; Enter = Use. One undo step each.
        pick(&mut app, &ctx, 1.0, "picture.screen_format");
        assert_eq!(
            app.requester.as_ref().unwrap().preset().id,
            "pixel_art_320x240"
        );
        let text = |c: &str| vec![egui::Event::Text(c.into())];
        ui_pass(&mut app, &ctx, 1.2, text("p"));
        ui_pass(&mut app, &ctx, 1.3, text("h"));
        let req = app.requester.as_ref().unwrap();
        assert_eq!(
            (req.preset().id, req.chipset()),
            ("ntsc_hires", Chipset::Aga)
        );
        enter(&mut app, &ctx, 1.4);
        let t = app.engine.document().target();
        assert_eq!(
            (t.chipset, t.mode.name()),
            (Chipset::Aga, "NTSC:High Res".into())
        );
        app.undo();
        assert_eq!(app.engine.document().target(), ocs);
        app.undo();
        assert_eq!(app.engine.document().target(), Target::default());
    }

    #[test]
    fn a_change_to_ocs_names_the_rounded_colours() {
        let mut app = PainterApp::new();
        let mut colors = Palette::dpaint_default_32().colors().to_vec();
        colors[3] = Rgb8::new(0x10, 0x20, 0x31);
        colors[9] = Rgb8::new(1, 2, 3);
        app.engine.execute(Command::SetPalette { colors }).unwrap();
        app.open_screen_format();
        app.requester_event(dpx_ui::ReqEvent::Key(dpx_ui::ReqKey::Char('h')));
        app.requester_event(dpx_ui::ReqEvent::Key(dpx_ui::ReqKey::Enter));
        assert!(app.requester.is_none());
        let doc = app.engine.document();
        assert_eq!(doc.target().chipset, Chipset::Ocs);
        assert_eq!(doc.palette().get(3), Some(Rgb8::new(0x11, 0x22, 0x33)));
        let note = app.ui.note_requester.as_ref().expect("a note");
        assert_eq!(note.title(), FORMAT_CHANGED);
        assert_eq!(note.notes(), ["2 colours rounded to 12-bit OCS values"]);
        // A refusal keeps the requester open with the engine's reason.
        app.ui.note_requester = None;
        app.engine.execute(Command::Clear { color: 31 }).unwrap();
        app.open_screen_format();
        let hi = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        while app.requester.as_ref().unwrap().mode() != hi {
            app.requester_event(dpx_ui::ReqEvent::Key(dpx_ui::ReqKey::Up));
        }
        app.requester_event(dpx_ui::ReqEvent::Key(dpx_ui::ReqKey::Enter));
        assert!(
            app.requester.is_some(),
            "refused: 31 is used, OCS HiRes has 16"
        );
        assert_eq!(app.engine.document().target().mode, DisplayMode::PAL_LORES);
    }

    #[test]
    fn export_refuses_what_the_mode_cannot_show_and_names_custom_screens() {
        let ctx = egui::Context::default();
        let dir = temp_dir("export");
        // OCS PAL HiRes with 32 colours: made around the engine (it refuses this format), as
        // an older project could have it.
        let hi = DisplayMode::new(Video::Pal, Resolution::Hi, false);
        let bad = Document::from_picture(
            IndexedImage::new(640, 256, 0),
            Palette::dpaint_default_32(),
            Target::new(Chipset::Ocs, hi),
        );
        let mut app = PainterApp::new();
        app.engine = Engine::new(bad);
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        // Files of those names exist: the refusal comes before the overwrite question, and
        // they are left as they are.
        let old = b"not a picture".to_vec();
        for name in ["Hi.iff", "Hi.anim"] {
            std::fs::write(dir.join(name), &old).unwrap();
        }
        ui_pass(&mut app, &ctx, 0.0, vec![]);
        pick(&mut app, &ctx, 0.1, "picture.export");
        assert!(app.ui.file_requester.is_some());
        typed(&mut app, &ctx, 0.2, "Hi.iff");
        enter(&mut app, &ctx, 0.3);
        assert!(
            app.ui.file_requester.is_none(),
            "closed, no overwrite question"
        );
        assert_eq!(
            std::fs::read(dir.join("Hi.iff")).unwrap(),
            old,
            "nothing written"
        );
        let note = app.ui.note_requester.take().expect("the refusal");
        assert_eq!(note.title(), NOT_EXPORTED);
        assert_eq!(
            note.notes(),
            [
                "32 colours exceed the 16 of PAL:High Res on Ocs",
                "An IFF picture must fit its display mode: change it with Picture > Screen \
                 Format, or export PNG"
            ]
        );
        // PNG is a free exchange format: written.
        pick(&mut app, &ctx, 0.5, "picture.export");
        typed(&mut app, &ctx, 0.6, "Hi.png");
        enter(&mut app, &ctx, 0.7);
        assert!(dir.join("Hi.png").exists());
        // Anim ▸ Save.. is refused the same way.
        app.ui.note_requester = None;
        app.open_file_requester(dpx_ui::FileKind::SaveAnim);
        typed(&mut app, &ctx, 0.9, "Hi");
        enter(&mut app, &ctx, 1.0);
        assert!(
            app.ui.file_requester.is_none(),
            "closed, no overwrite question"
        );
        assert_eq!(std::fs::read(dir.join("Hi.anim")).unwrap(), old);
        assert_eq!(
            app.ui.note_requester.as_ref().unwrap().title(),
            ANIM_NOT_SAVED
        );

        // The pixel-art preset on OCS exports, and the note says what it is.
        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        let mut args: Vec<String> = ["--chipset", "ocs", "--preset", "pixel_art_320x240"]
            .map(String::from)
            .to_vec();
        snapshot_format(&args, &mut app.engine).unwrap();
        ui_pass(&mut app, &ctx, 2.0, vec![]);
        pick(&mut app, &ctx, 2.1, "picture.export");
        typed(&mut app, &ctx, 2.2, "Px.iff");
        enter(&mut app, &ctx, 2.3);
        assert!(dir.join("Px.iff").exists());
        let note = app.ui.note_requester.as_ref().expect("the notes");
        assert_eq!(note.title(), "Picture exported");
        assert_eq!(
            note.notes(),
            [
                "320x240 is a custom screen size on PAL:Low Res (standard 320x256); exact \
                 hardware compatibility is not guaranteed",
                "IFF files store no chipset: this OCS picture reopens as AGA"
            ]
        );
        // Bad snapshot values are named.
        args[1] = "ecs".into();
        let e = snapshot_format(&args, &mut app.engine).unwrap_err();
        assert_eq!(e, "--chipset expects ocs or aga, not 'ecs'");
        let e = snapshot_format(
            &["--preset".to_string(), "custom".to_string()],
            &mut app.engine,
        )
        .unwrap_err();
        assert!(e.starts_with("--preset custom: expected one of ntsc_lores, pal_lores"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn palette_load_and_restore_round_to_12_bit_on_ocs() {
        let dir = temp_dir("palette");
        let mut app = PainterApp::new();
        snapshot_format(
            &["--chipset".to_string(), "ocs".to_string()],
            &mut app.engine,
        )
        .unwrap();
        assert_eq!(app.engine.document().target().chipset, Chipset::Ocs);
        // A palette file (an ILBM CMAP) with 24-bit colours.
        let colors: Vec<Rgb8> = (0..32).map(|i| Rgb8::new(i * 8, 100, 3)).collect();
        let file = dpx_iff::Ilbm::from_indexed(
            IndexedImage::new(1, 1, 0),
            Palette::new(colors.clone()).unwrap(),
        );
        let path = dir.join("p.iff");
        std::fs::write(
            &path,
            dpx_iff::ilbm::write(&file, dpx_iff::Compression::ByteRun1).unwrap(),
        )
        .unwrap();
        let revision = app.engine.document().revision();
        app.menu_file_accept(dpx_ui::FileKind::LoadPalette, &path, false);
        let doc = app.engine.document();
        assert_eq!(doc.revision(), revision + 1, "loaded, not refused");
        assert!(
            doc.palette()
                .colors()
                .iter()
                .all(|&c| hardware::is_12_bit(c))
        );
        assert_eq!(doc.palette().get(2), Some(Rgb8::new(0x11, 0x66, 0x00)));
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.notes(), ["32 colours rounded to 12-bit OCS values"]);
        // Restore Palette with a 24-bit palette from before the change to OCS.
        app.menu_data.loaded_palette = Some(Palette::new(colors).unwrap());
        app.run(Command::Clear { color: 1 });
        app.engine
            .execute(Command::SetPalette {
                colors: Palette::dpaint_default_32().colors().to_vec(),
            })
            .unwrap();
        app.restore_palette();
        let doc = app.engine.document();
        assert_eq!(doc.palette().get(2), Some(Rgb8::new(0x11, 0x66, 0x00)));
        let note = app.ui.note_requester.take().expect("a note");
        assert_eq!(note.title(), "Palette restored");
        // AGA takes the file as it is, without a note.
        let mut aga = PainterApp::new();
        aga.menu_file_accept(dpx_ui::FileKind::LoadPalette, &path, false);
        assert_eq!(
            aga.engine.document().palette().get(2),
            Some(Rgb8::new(16, 100, 3))
        );
        assert!(aga.ui.note_requester.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The engine counts the rounded kept entries (dpx-core tests); the note names them.
    #[test]
    fn rounded_note_names_the_engines_count() {
        assert!(rounded_note(0).is_none());
        let note = rounded_note(1).unwrap();
        assert_eq!(note.title(), FORMAT_CHANGED);
        assert_eq!(note.notes(), ["1 colour rounded to a 12-bit OCS value"]);
    }
}
