//! The app side of the Modern skin (ADR-47). For now the headless snapshot only: the
//! `--snapshot` flags of the Modern main window and the picture that goes into its quad. The
//! live window follows in the next slice.

use dpx_core::Document;
use dpx_ui::UiState;
use dpx_ui_modern::{Frame, Image, Model, Renderer, Spot, Theme, ThemeKind};

use crate::{View, arg_value, snapshot_rgba, write_png};

/// The window a Modern snapshot shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SnapshotOptions {
    pub theme: ThemeKind,
    /// Window size in points.
    pub size: (f32, f32),
    /// Physical pixels per point.
    pub device_scale: f32,
    /// A resting pointer, in window points.
    pub pointer: Option<(f32, f32)>,
}

/// Default window of a Modern snapshot, in points.
const DEFAULT_SIZE: (f32, f32) = (1280.0, 800.0);
/// Largest window side a snapshot accepts, in points.
const MAX_SIDE: f32 = 8192.0;

fn pair(spec: &str, separator: char) -> Option<(f32, f32)> {
    let (a, b) = spec.split_once(separator)?;
    let (a, b): (f32, f32) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a.is_finite() && b.is_finite()).then_some((a, b))
}

/// Reads `--ui modern` and its companions `--theme dark|light` (default dark, never the
/// machine's setting), `--size WxH` in points (default 1280x800), `--device-scale 1|2` and
/// `--pointer X,Y` (window points). `None` without `--ui modern`: a Classic snapshot, which
/// these flags do not touch.
pub(crate) fn snapshot_options(args: &[String]) -> Result<Option<SnapshotOptions>, String> {
    let modern = match arg_value(args, "--ui").as_deref() {
        None | Some("classic") => false,
        Some("modern") => true,
        Some(other) => return Err(format!("bad --ui '{other}' (expected classic or modern)")),
    };
    if !modern {
        for flag in ["--theme", "--size", "--device-scale", "--pointer"] {
            if args.iter().any(|a| a == flag) {
                return Err(format!("{flag} needs --ui modern"));
            }
        }
        return Ok(None);
    }
    let theme = match arg_value(args, "--theme").as_deref() {
        None | Some("dark") => ThemeKind::Dark,
        Some("light") => ThemeKind::Light,
        Some(other) => return Err(format!("bad --theme '{other}' (expected dark or light)")),
    };
    let size = match arg_value(args, "--size") {
        None => DEFAULT_SIZE,
        Some(spec) => pair(&spec, 'x')
            .filter(|&(w, h)| {
                let (min_w, min_h) = dpx_ui_modern::layout::MIN_WINDOW;
                w >= min_w && h >= min_h && w <= MAX_SIDE && h <= MAX_SIDE
            })
            .ok_or_else(|| {
                let (w, h) = dpx_ui_modern::layout::MIN_WINDOW;
                format!("bad --size '{spec}' (expected WxH in points, at least {w}x{h})")
            })?,
    };
    let device_scale = match arg_value(args, "--device-scale").as_deref() {
        None | Some("1") => 1.0,
        Some("2") => 2.0,
        Some(other) => return Err(format!("bad --device-scale '{other}' (expected 1 or 2)")),
    };
    let pointer = match arg_value(args, "--pointer") {
        None => None,
        Some(spec) => Some(
            pair(&spec, ',')
                .ok_or_else(|| format!("bad --pointer '{spec}' (expected X,Y in window points)"))?,
        ),
    };
    Ok(Some(SnapshotOptions {
        theme,
        size,
        device_scale,
        pointer,
    }))
}

/// The interface state the picture quad is composed with: the shared state without the
/// classic chrome, which the Modern skin draws itself. Requesters and the Animation Control
/// Panel stay: they are shown in the Classic look inside the quad until they are converted.
fn quad_ui(ui: &UiState) -> UiState {
    let mut bare = ui.clone();
    bare.title_visible = false;
    bare.toolbox_visible = false;
    bare.menu = dpx_ui::MenuState::default();
    bare.layer_panel = None;
    bare.tooltip = None;
    bare
}

/// The Modern main window as an image: width, height (physical pixels) and RGBA8.
pub(crate) fn snapshot_image(
    options: &SnapshotOptions,
    doc: &Document,
    view: View,
    scroll: (i32, i32),
    ui: &UiState,
    custom_brush: bool,
) -> (u32, u32, Vec<u8>) {
    // The classic screen without its chrome, one image pixel per displayed pixel (the pixel
    // shape of the display mode applied), exactly as a Classic snapshot at scale 1 makes it.
    let (pw, ph, picture) = snapshot_rgba(1, doc, view, scroll, &quad_ui(ui), false);
    let mut frame = Frame {
        size: options.size,
        scale: options.device_scale,
        theme: Theme::of(options.theme),
        model: Model {
            custom_brush,
            ..Model::new(ui, doc.palette())
        },
        picture: Image {
            width: pw,
            height: ph,
            rgba: &picture,
        },
    };
    if let Some((x, y)) = options.pointer
        && let Spot::Ui(hit) = dpx_ui_modern::hit(&frame.layout(), x, y)
    {
        frame.model.hover = Some(hit);
    }
    dpx_ui_modern::render_window(&frame, &mut Renderer::new())
}

/// Writes the Modern main window to the PNG `path`.
pub(crate) fn snapshot(
    path: &str,
    options: &SnapshotOptions,
    doc: &Document,
    view: View,
    scroll: (i32, i32),
    ui: &UiState,
    custom_brush: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (w, h, rgba) = snapshot_image(options, doc, view, scroll, ui, custom_brush);
    write_png(path, w, h, &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{DisplayMode, Palette};
    use dpx_ui::Tool;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn without_ui_modern_the_snapshot_stays_classic() {
        assert_eq!(snapshot_options(&args(&["--snapshot", "a.png"])), Ok(None));
        assert_eq!(snapshot_options(&args(&["--ui", "classic"])), Ok(None));
        // The Modern flags alone are a mistake, not silently ignored.
        for flag in ["--theme", "--size", "--device-scale", "--pointer"] {
            let e = snapshot_options(&args(&[flag, "x"])).expect_err(flag);
            assert!(e.contains("--ui modern"), "{e}");
        }
        assert!(snapshot_options(&args(&["--ui", "retro"])).is_err());
    }

    #[test]
    fn the_modern_flags_have_defaults_and_are_checked() {
        assert_eq!(
            snapshot_options(&args(&["--ui", "modern"])),
            Ok(Some(SnapshotOptions {
                theme: ThemeKind::Dark,
                size: (1280.0, 800.0),
                device_scale: 1.0,
                pointer: None,
            }))
        );
        let all = [
            "--ui",
            "modern",
            "--theme",
            "light",
            "--size",
            "1440x900",
            "--device-scale",
            "2",
            "--pointer",
            "1210,150",
        ];
        assert_eq!(
            snapshot_options(&args(&all)),
            Ok(Some(SnapshotOptions {
                theme: ThemeKind::Light,
                size: (1440.0, 900.0),
                device_scale: 2.0,
                pointer: Some((1210.0, 150.0)),
            }))
        );
        for bad in [
            ["--theme", "sepia"],
            ["--size", "1280"],
            ["--size", "100x100"],
            ["--size", "99999x800"],
            ["--size", "NaNx800"],
            ["--device-scale", "3"],
            ["--device-scale", "1.5"],
            ["--pointer", "12"],
        ] {
            let mut list = vec!["--ui", "modern"];
            list.extend(bad);
            assert!(snapshot_options(&args(&list)).is_err(), "{bad:?}");
        }
    }

    fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2]]
    }

    /// The quad holds the page and nothing of the classic chrome: where the Classic skin has
    /// its title bar and toolbox, the Modern picture shows page pixels.
    #[test]
    fn the_picture_quad_is_the_page_without_classic_chrome() {
        let (w, h) = DisplayMode::PAL_LORES.size();
        let palette = Palette::dpaint_default_32();
        let mut doc = Document::new(w, h, palette.clone());
        // Colour 5 everywhere: the classic title bar and toolbox would not be colour 5.
        let mut engine = dpx_core::Engine::new(doc.clone());
        engine
            .execute(dpx_core::Command::Clear { color: 5 })
            .expect("clear");
        doc = engine.document().clone();
        let view = View::of_target(doc.target());
        let ui = UiState {
            tool: Tool::Line,
            ..UiState::default()
        };
        let options = SnapshotOptions {
            theme: ThemeKind::Dark,
            size: (1280.0, 800.0),
            device_scale: 1.0,
            pointer: Some((1210.0, 105.0)),
        };
        let (iw, ih, rgba) = snapshot_image(&options, &doc, view, (0, 0), &ui, false);
        assert_eq!((iw, ih), (1280, 800));
        let c = palette.get(5).expect("colour 5");
        let page = [c.r, c.g, c.b];
        // The quad is 960 x 768 at (114, 31): its corners, where Classic draws chrome.
        for (x, y) in [
            (114, 31),
            (1073, 31),
            (114, 798),
            (1073, 798),
            (600, 400),
            (1060, 300),
        ] {
            assert_eq!(pixel(&rgba, iw, x, y), page, "({x}, {y})");
        }
        // The chrome is the Modern one: the selected Line button is on the accent, and the
        // pointer raises the button it rests on (Dotted Freehand, top left of the tools).
        let theme = Theme::dark();
        let accent = [theme.accent.r, theme.accent.g, theme.accent.b];
        let hover = [theme.hover.r, theme.hover.g, theme.hover.b];
        assert_eq!(pixel(&rgba, iw, 1199, 141), accent);
        assert_eq!(pixel(&rgba, iw, 1199, 99), hover);
    }
}
