//! The Brush menu (ADR-19): transforms of the custom brush and brush files.
//!
//! The pixel work lives in `dpx_core::brush_ops`; this module wires the menu ids to the
//! app's `custom_brush` / `picked_brush` and reads and writes brush files (IFF ILBM with a
//! transparent colour and the handle in GRAB).

use std::path::Path;

use dpx_core::{BrushImage, IndexedImage, Palette, brush_ops};
use dpx_iff::ilbm::{self, ColorRange, Compression, Ilbm, Masking};
use dpx_ui::FileKind;

use crate::tools::XformKind;
use crate::{PainterApp, files, note_requester};

/// Title of the note requester after Brush ▸ Load.
const BRUSH_LOADED: &str = "Brush loaded";

/// A brush as an ILBM file: BMHD masking 2 with the transparent colour, the handle in GRAB,
/// the palette in CMAP and the colour ranges in CRNG (brush files carry their palette and
/// cycling data, manual AGA p.242, p.267; ranges since ADR-39).
pub fn encode_brush(
    brush: &BrushImage,
    palette: &Palette,
    ranges: &[ColorRange],
) -> Result<Vec<u8>, String> {
    let mut file = Ilbm::from_indexed(brush.to_image(), palette.clone());
    file.ranges = ranges.to_vec();
    if let Some(t) = brush.transparent {
        file.masking = Masking::TransparentColor;
        file.transparent_color = u16::from(t);
    }
    let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    file.grab = Some((clamp(brush.handle.0), clamp(brush.handle.1)));
    ilbm::write(&file, Compression::ByteRun1).map_err(|e| e.to_string())
}

/// A brush from an ILBM file. Its colours are fitted to `page_palette` where the palettes
/// differ (the picture palette stays as it is); the second value is how many colours had
/// to change. A mask plane becomes a transparent colour that no opaque pixel uses. (The app
/// loads with [`read_brush`] and [`fit_brush`], ADR-39.)
#[cfg(test)]
pub fn decode_brush(bytes: &[u8], page_palette: &Palette) -> Result<(BrushImage, usize), String> {
    let (brush, palette, _) = read_brush(bytes)?;
    Ok(fit_brush(&brush, &palette, page_palette))
}

/// A brush file as it is (ADR-39): the brush in the file's colour numbers, the file's palette
/// and its colour ranges. A mask plane becomes a transparent colour that no opaque pixel
/// uses.
pub fn read_brush(bytes: &[u8]) -> Result<(BrushImage, Palette, Vec<ColorRange>), String> {
    let file = ilbm::read(bytes).map_err(|e| e.summary)?;
    let (width, height) = (file.image.width(), file.image.height());
    let mut pixels = file.image.pixels().to_vec();
    let mut transparent = match file.masking {
        Masking::TransparentColor | Masking::Lasso => u8::try_from(file.transparent_color).ok(),
        Masking::None | Masking::HasMask => None,
    };
    if let (Masking::HasMask, Some(mask)) = (file.masking, &file.mask) {
        let mut used = [false; 256];
        for (&p, &opaque) in pixels.iter().zip(mask) {
            if opaque {
                used[usize::from(p)] = true;
            }
        }
        if let Some(t) = (0..=255u8).find(|&c| !used[usize::from(c)]) {
            for (p, &opaque) in pixels.iter_mut().zip(mask) {
                if !opaque {
                    *p = t;
                }
            }
            transparent = Some(t);
        }
    }
    let image = IndexedImage::from_pixels(width, height, pixels).ok_or("bad brush size")?;
    let mut brush = BrushImage::from_image(&image, transparent);
    if let Some((x, y)) = file.grab {
        brush.handle = (
            i32::from(x).clamp(0, width as i32 - 1),
            i32::from(y).clamp(0, height as i32 - 1),
        );
    }
    Ok((brush, file.palette, file.ranges))
}

/// `brush` (colour numbers of `file_palette`) with its colours fitted to `page_palette`: numbers
/// whose colour differs from the page palette's go to the nearest page colour, the
/// transparent colour keeps its number, and no opaque colour goes to the transparent number
/// (a black outline must not turn into holes where page colour 0 is black too). The second
/// value is how many colours had to change.
pub fn fit_brush(
    brush: &BrushImage,
    file_palette: &Palette,
    page_palette: &Palette,
) -> (BrushImage, usize) {
    let transparent = brush.transparent;
    let mut map: [Option<u8>; 256] = [None; 256];
    let mut changed = 0;
    let mut out = brush.clone();
    for &p in &brush.pixels {
        let slot = &mut map[usize::from(p)];
        if slot.is_some() {
            continue;
        }
        let want = file_palette.get(p);
        let to = match want {
            Some(c) if page_palette.get(p) != Some(c) => {
                changed += usize::from(Some(p) != transparent);
                match transparent {
                    Some(t) => page_palette.nearest_excluding(c, &[t]),
                    None => page_palette.nearest(c),
                }
            }
            _ => p,
        };
        *slot = Some(if Some(p) == transparent { p } else { to });
    }
    for p in &mut out.pixels {
        *p = map[usize::from(*p)].unwrap_or(*p);
    }
    (out, changed)
}

impl PainterApp {
    /// Runs a `brush.*` menu command. Items that need a brush do nothing without one.
    pub(crate) fn brush_action(&mut self, id: &str) {
        self.finish_stroke();
        match id {
            "brush.load" => return self.open_file_requester(FileKind::LoadBrush),
            "brush.save" => {
                if self.custom_brush.is_some() {
                    self.open_file_requester(FileKind::SaveBrush);
                }
                return;
            }
            "brush.restore" => {
                if let Some(picked) = self.picked_brush.clone() {
                    self.custom_brush = Some(picked);
                    self.leave_anim_brush(); // ADR-36
                    self.dirty = true;
                }
                return;
            }
            // ADR-30: the drag gestures, the spare brush and Remap.
            "brush.stretch" => return self.arm_xform(XformKind::Stretch),
            "brush.rotate_any" => return self.arm_xform(XformKind::Rotate),
            "brush.shear" => return self.arm_xform(XformKind::Shear),
            "brush.bend_horizontal" => return self.arm_xform(XformKind::BendH),
            "brush.bend_vertical" => return self.arm_xform(XformKind::BendV),
            "brush.handle_place" => return self.arm_xform(XformKind::Place),
            "brush.to_spare" => {
                self.tools.spare_brush.clone_from(&self.custom_brush);
                return;
            }
            "brush.swap_spare" => {
                // Nothing to swap with: both stay as they were.
                if self.tools.spare_brush.is_some() && self.custom_brush.is_some() {
                    std::mem::swap(&mut self.tools.spare_brush, &mut self.custom_brush);
                    self.leave_anim_brush(); // ADR-36
                    self.dirty = true;
                }
                return;
            }
            "brush.remap" => return self.remap_brush(),
            _ => {}
        }
        let (fg, bg) = (self.ui.fg, self.ui.bg);
        let Some(b) = &self.custom_brush else {
            return;
        };
        let new = match id {
            "brush.flip_horizontal" => brush_ops::flip_horizontal(b),
            "brush.flip_vertical" => brush_ops::flip_vertical(b),
            "brush.rotate_90" => brush_ops::rotate_90(b),
            "brush.halve" => brush_ops::halve(b),
            "brush.double" => brush_ops::double(b, true, true),
            "brush.double_horizontal" => brush_ops::double(b, true, false),
            "brush.double_vertical" => brush_ops::double(b, false, true),
            "brush.outline" => brush_ops::outline(b, fg),
            "brush.trim" => match brush_ops::trim(b) {
                Some(t) => t,
                None => return,
            },
            "brush.handle_center" => brush_ops::handle_center(b),
            "brush.handle_corner" => brush_ops::handle_next_corner(b),
            "brush.bg_to_fg" => brush_ops::replace_color(b, bg, fg),
            "brush.swap_bg_fg" => brush_ops::swap_colors(b, bg, fg),
            "brush.change_transparency" => brush_ops::set_transparent(b, bg),
            _ => return,
        };
        self.custom_brush = Some(new);
        self.dirty = true;
    }

    /// Color > Brush > Remap: the brush's colours (as the palette at pick-up or load had them)
    /// become the nearest colours of the current palette. A brush has no palette of its own
    /// here, so without that snapshot nothing happens (ADR-30).
    fn remap_brush(&mut self) {
        let (Some(b), Some(from)) = (&self.custom_brush, &self.tools.brush_palette) else {
            return;
        };
        let now = self.engine.document().palette().clone();
        let (new, _) = brush_ops::remap(b, from, &now);
        self.custom_brush = Some(new);
        self.tools.brush_palette = Some(now);
        self.dirty = true;
    }

    /// The file requester's answer for Brush ▸ Load.. and Save..
    pub(crate) fn brush_file_accept(&mut self, kind: FileKind, path: &Path, overwrite: bool) {
        match kind {
            FileKind::LoadBrush => self.load_brush(path),
            FileKind::SaveBrush => self.save_brush(path, overwrite),
            _ => {}
        }
    }

    fn load_brush(&mut self, path: &Path) {
        let result = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| read_brush(&b));
        match result {
            Ok((own, palette, ranges)) => {
                let (brush, changed) = fit_brush(&own, &palette, self.engine.document().palette());
                // ADR-39: Use Brush Palette needs the file's own palette and ranges.
                self.remember_loaded_brush(own, palette, ranges, brush.clone());
                self.custom_brush = Some(brush.clone());
                self.picked_brush = Some(brush);
                self.tools.brush_gen += 1;
                self.leave_anim_brush(); // ADR-36
                self.tools.brush_palette = Some(self.engine.document().palette().clone());
                self.ui.file_requester = None;
                let mut notes = Vec::new();
                if changed > 0 {
                    notes.push(format!(
                        "{changed} brush colours were fitted to the picture palette"
                    ));
                }
                self.ui.note_requester = note_requester(BRUSH_LOADED, &notes);
                self.dirty = true;
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e);
            }
        }
    }

    fn save_brush(&mut self, path: &Path, overwrite: bool) {
        if path.is_dir() {
            return self.requester_message("That is a drawer".into());
        }
        if path.exists() && !overwrite {
            if let Some(req) = &mut self.ui.file_requester {
                req.ask_overwrite();
            }
            return;
        }
        let Some(brush) = &self.custom_brush else {
            return;
        };
        let bytes = encode_brush(brush, self.engine.document().palette(), &self.extras.ranges);
        match bytes.and_then(|b| files::write_atomic(path, &b)) {
            Ok(()) => self.ui.file_requester = None,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                self.requester_message(e);
            }
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use dpx_core::paint::{PaintContext, plot};
    use dpx_core::{DrawMode, Rgb8};
    use dpx_ui::{Action, ReqEvent, ReqKey};

    use super::*;

    fn ctx() -> eframe::egui::Context {
        eframe::egui::Context::default()
    }

    fn brush() -> BrushImage {
        BrushImage {
            width: 3,
            height: 2,
            pixels: vec![0, 5, 6, 7, 0, 8],
            handle: (2, 0),
            transparent: Some(0),
        }
    }

    fn act(app: &mut PainterApp, id: &'static str) {
        app.run_action(Action::Command(id), &ctx());
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dpx-brush-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    fn type_into(app: &mut PainterApp, text: &str) {
        for c in text.chars() {
            app.file_requester_event(ReqEvent::Key(ReqKey::Char(c)), 0);
        }
    }

    #[test]
    fn menu_items_transform_the_custom_brush_and_restore_goes_back() {
        let mut app = PainterApp::new();
        act(&mut app, "brush.flip_horizontal");
        assert!(
            app.custom_brush.is_none(),
            "nothing happens without a brush"
        );
        app.custom_brush = Some(brush());
        app.picked_brush = Some(brush());
        act(&mut app, "brush.flip_horizontal");
        assert_eq!(
            app.custom_brush.as_ref().unwrap().pixels,
            vec![6, 5, 0, 8, 0, 7]
        );
        act(&mut app, "brush.rotate_90");
        let b = app.custom_brush.clone().unwrap();
        assert_eq!((b.width, b.height), (2, 3));
        app.ui.fg = 9;
        act(&mut app, "brush.outline");
        assert_eq!(app.custom_brush.as_ref().unwrap().width, 4);
        act(&mut app, "brush.trim");
        assert_eq!(app.custom_brush.as_ref().unwrap().width, 2);
        act(&mut app, "brush.restore");
        assert_eq!(app.custom_brush, Some(brush()));
        app.ui.bg = 3;
        act(&mut app, "brush.change_transparency");
        assert_eq!(app.custom_brush.as_ref().unwrap().transparent, Some(3));
        act(&mut app, "brush.handle_corner");
        assert_eq!(app.custom_brush.as_ref().unwrap().handle, (2, 1));
    }

    #[test]
    fn brush_file_round_trips_through_the_file_requester() {
        let dir = temp_dir("roundtrip");
        let mut app = PainterApp::new();
        app.last_drawer = Some(dir.to_string_lossy().into_owned());
        app.custom_brush = Some(brush());
        act(&mut app, "brush.save");
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().kind(),
            FileKind::SaveBrush
        );
        type_into(&mut app, "Test.brush");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none(), "saved and closed");
        let bytes = std::fs::read(dir.join("Test.brush")).unwrap();
        let file = ilbm::read(&bytes).unwrap();
        assert_eq!(file.masking, Masking::TransparentColor);
        assert_eq!(file.transparent_color, 0);
        assert_eq!(file.grab, Some((2, 0)));

        app.custom_brush = None;
        app.picked_brush = None;
        act(&mut app, "brush.load");
        assert_eq!(
            app.ui.file_requester.as_ref().unwrap().kind(),
            FileKind::LoadBrush
        );
        type_into(&mut app, "Test.brush");
        app.file_requester_event(ReqEvent::Key(ReqKey::Enter), 0);
        assert!(app.ui.file_requester.is_none());
        assert_eq!(app.custom_brush, Some(brush()));
        assert_eq!(app.picked_brush, Some(brush()), "Restore returns to it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_fits_foreign_palette_colours() {
        let mut other = Palette::dpaint_default_32().colors().to_vec();
        other[5] = Rgb8::new(1, 2, 3);
        let other = Palette::new(other).unwrap();
        let bytes = encode_brush(&brush(), &other, &[]).unwrap();
        let page = Palette::dpaint_default_32();
        let (b, changed) = decode_brush(&bytes, &page).unwrap();
        assert_eq!(changed, 1);
        assert_eq!(b.transparent, Some(0));
        assert_eq!(b.get(0, 0), Some(0), "transparent index kept");
        // The nearest colour that is not the transparent one (page colour 0 is black too).
        let want = page.nearest_excluding(Rgb8::new(1, 2, 3), &[0]);
        assert_ne!(want, 0);
        assert_eq!(b.get(1, 0), Some(want));
        assert_eq!(b.get(2, 0), Some(6));
    }

    #[test]
    fn no_opaque_colour_is_fitted_onto_the_transparent_colour() {
        // A sprite with its own palette (pink = transparent, black outline, white, red) on a
        // page whose colour 0 is black: the outline must stay opaque.
        let own = Palette::new(vec![
            Rgb8::new(255, 0, 255),
            Rgb8::BLACK,
            Rgb8::WHITE,
            Rgb8::new(255, 0, 0),
        ])
        .unwrap();
        let page = Palette::dpaint_default_32();
        assert_eq!(page.get(0), Some(Rgb8::BLACK));
        let sprite = BrushImage {
            width: 4,
            height: 1,
            pixels: vec![0, 1, 2, 3],
            handle: (0, 0),
            transparent: Some(0),
        };
        let (fitted, changed) = fit_brush(&sprite, &own, &page);
        assert_eq!(
            fitted.pixels[0], 0,
            "the transparent colour keeps its number"
        );
        for (i, &p) in fitted.pixels.iter().enumerate().skip(1) {
            assert_ne!(p, 0, "opaque pixel {i} is no hole");
            let want = own.get(sprite.pixels[i]).unwrap();
            assert_eq!(p, page.nearest_excluding(want, &[0]), "pixel {i}");
        }
        assert_eq!(changed, 3);
        // An opaque brush may use colour 0 like any other.
        let opaque = BrushImage {
            transparent: None,
            ..sprite
        };
        let (fitted, _) = fit_brush(&opaque, &own, &page);
        assert_eq!(fitted.pixels[1], 0, "black is page colour 0");
    }

    #[test]
    fn a_mask_plane_becomes_a_transparent_colour() {
        let img = IndexedImage::from_pixels(2, 1, vec![3, 4]).unwrap();
        let mut file = Ilbm::from_indexed(img, Palette::dpaint_default_32());
        file.masking = Masking::HasMask;
        file.mask = Some(vec![true, false]);
        let bytes = ilbm::write(&file, Compression::ByteRun1).unwrap();
        let (b, _) = decode_brush(&bytes, &Palette::dpaint_default_32()).unwrap();
        assert_eq!(b.transparent, Some(0));
        assert_eq!(b.pixels, vec![3, 0]);
    }

    #[test]
    fn brush_items_are_ghosted_until_there_is_a_brush() {
        let mut app = PainterApp::new();
        let enabled = |app: &PainterApp, id: &'static str| {
            let mut ui = app.ui.clone();
            ui.has_brush = app.custom_brush.is_some();
            ui.can_restore = app.picked_brush.is_some();
            Action::Command(id).enabled(&ui.menu_context())
        };
        assert!(enabled(&app, "brush.load"));
        assert!(!enabled(&app, "brush.flip_vertical"));
        app.custom_brush = Some(brush());
        assert!(enabled(&app, "brush.flip_vertical"));
    }

    #[test]
    fn spare_brush_copies_and_swaps() {
        let mut app = PainterApp::new();
        app.custom_brush = Some(brush());
        act(&mut app, "brush.swap_spare");
        assert_eq!(app.custom_brush, Some(brush()), "no spare: nothing happens");
        assert!(app.tools.spare_brush.is_none());
        act(&mut app, "brush.to_spare");
        act(&mut app, "brush.rotate_90");
        let rotated = app.custom_brush.clone().unwrap();
        act(&mut app, "brush.swap_spare");
        assert_eq!(app.custom_brush, Some(brush()));
        assert_eq!(app.tools.spare_brush, Some(rotated));
    }

    #[test]
    fn remap_fits_the_brush_to_the_changed_palette() {
        let mut app = PainterApp::new();
        let old = app.engine.document().palette().clone();
        app.custom_brush = Some(brush());
        act(&mut app, "brush.remap");
        assert_eq!(
            app.custom_brush,
            Some(brush()),
            "no snapshot: nothing happens"
        );
        app.tools.brush_palette = Some(old.clone());
        act(&mut app, "brush.remap");
        assert_eq!(
            app.custom_brush,
            Some(brush()),
            "same palette: nothing changes"
        );
        let mut colors = old.colors().to_vec();
        colors[5] = Rgb8::new(1, 2, 3);
        let new = Palette::new(colors).unwrap();
        app.run(dpx_core::Command::SetPalette {
            colors: new.colors().to_vec(),
        });
        assert_eq!(app.engine.document().palette(), &new);
        act(&mut app, "brush.remap");
        let want = new.nearest(old.get(5).unwrap());
        let b = app.custom_brush.clone().unwrap();
        assert_eq!(b.get(1, 0), Some(want));
        assert_eq!(b.get(0, 0), Some(0), "transparent stays");
        assert_eq!(app.tools.brush_palette.as_ref(), Some(&new));
    }

    /// The mode demo behind docs/design/screenshots/phase3-modes.png. Set `DPX_MODES_PNG` to a
    /// path to write it; the checks run either way.
    #[test]
    fn mode_demo() {
        // 64 colours: the default 32 plus their half-bright twins (EHB).
        let mut colors = Palette::dpaint_default_32().colors().to_vec();
        let half: Vec<Rgb8> = colors
            .iter()
            .map(|c| Rgb8::new(c.r / 2, c.g / 2, c.b / 2))
            .collect();
        colors.extend(half);
        let pal = Palette::new(colors).unwrap();
        let (pw, ph, cols, scale) = (56u32, 40u32, 4u32, 4u32);
        let mut sheet = IndexedImage::new(cols * (pw + 2) + 2, 2 * (ph + 2) + 2, 0);
        let modes = [
            None,
            Some(DrawMode::Smear),
            Some(DrawMode::Shade),
            Some(DrawMode::Blend),
            Some(DrawMode::Cycle),
            Some(DrawMode::Smooth),
            Some(DrawMode::Mix),
            Some(DrawMode::HBrite),
        ];
        let mut changed = Vec::new();
        for (n, mode) in modes.iter().enumerate() {
            // Vertical bars of colours 4..=15 plus a white and a dark block.
            let mut panel = IndexedImage::new(pw, ph, 0);
            for x in 0..pw as i32 {
                panel.fill_rect(x, 0, 1, ph as i32, 4 + (x as u8 / 5) % 12);
            }
            panel.fill_rect(0, 30, pw as i32, 10, 1);
            panel.fill_rect(20, 30, 16, 10, 2);
            let before = panel.clone();
            if let Some(mode) = *mode {
                let ctx_base = |motion| PaintContext {
                    palette: &pal,
                    range: Some((4, 15)),
                    stencil: None,
                    area: None,
                    before: &before,
                    motion,
                    right: false,
                };
                let mut last = (6, 6);
                for t in 0..=40 {
                    let (px, py) = (6 + t * 44 / 40, 6 + t * 20 / 40);
                    let motion = (px - last.0, py - last.1);
                    last = (px, py);
                    let c = ctx_base(motion);
                    for dy in -5i32..=5 {
                        for dx in -5i32..=5 {
                            if dx * dx + dy * dy <= 25 {
                                plot(&mut panel, px + dx, py + dy, 17, mode, &c);
                            }
                        }
                    }
                }
                assert_ne!(panel, before, "{mode:?} changed the picture");
            }
            changed.push(panel.clone());
            let (ox, oy) = (
                2 + (n as u32 % cols) * (pw + 2),
                2 + (n as u32 / cols) * (ph + 2),
            );
            sheet.blit(&panel, ox as i32, oy as i32);
        }
        assert_eq!(changed.len(), 8);
        if let Ok(path) = std::env::var("DPX_MODES_PNG") {
            let (w, h) = (sheet.width() * scale, sheet.height() * scale);
            let mut big = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    big.push(sheet.get((x / scale) as i32, (y / scale) as i32).unwrap());
                }
            }
            let big = IndexedImage::from_pixels(w, h, big).unwrap();
            let png = dpx_formats::write_png(&big, &pal).unwrap();
            std::fs::write(path, png).unwrap();
        }
    }
}
