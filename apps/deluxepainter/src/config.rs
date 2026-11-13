//! The settings file (ADR-33): what DeluxePainter remembers between runs. A small JSON file in
//! the platform config directory holds the pixel size, the Prefs menu, the last drawers and the
//! tool settings that are not document data. Loading never fails (a missing or broken file
//! gives the defaults; a broken one is kept as `settings.json.bak`), unknown fields are
//! ignored, and saving writes a temporary file and renames it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dpx_core::{GradientDirection, LightTable};
use dpx_ui::tool_requesters::{
    AIRBRUSH_RANGE, FillSettings, GRID_RANGE, GridSettings, SPACING_RANGE, SYMMETRY_RANGE,
    SpacingMode, SpacingSettings, SymmetrySettings, TRANSLUCENCY_RANGE,
};
use dpx_ui::{PixelSize, Pref, Prefs, ThemeChoice, UiStyle};
use serde_json::{Map, Value, json};

use crate::PainterApp;
use crate::import::{self, ImportSettings};
use crate::mcp::McpSettings;

/// Version written to the file. Newer files are read as far as their fields are known.
pub const VERSION: u64 = 1;
/// File name inside the config directory.
const FILE: &str = "settings.json";
/// A change is written this many seconds after the last change.
const DEBOUNCE: f64 = 1.0;
/// Name of the key in `drawers` for the Load/Save/Export requesters.
pub const DRAWER_PICTURE: &str = "picture";

/// Everything that is remembered.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The skin of the main window (ADR-47): key `ui_style`, `"classic"` or `"modern"`.
    pub ui_style: UiStyle,
    /// The Modern skin's theme (ADR-47): key `theme`, `"system"`, `"dark"` or `"light"`.
    pub theme: ThemeChoice,
    pub pixel_size: u8,
    pub prefs: Prefs,
    /// Last drawer per requester kind.
    pub drawers: BTreeMap<String, String>,
    pub grid: bool,
    pub grid_size: GridSettings,
    pub symmetry: bool,
    pub symmetry_settings: SymmetrySettings,
    pub spacing: SpacingSettings,
    pub fill: FillSettings,
    pub translucency_on: bool,
    pub translucency_percent: u8,
    pub light_on: bool,
    pub light: LightTable,
    /// The Import requester's last choices (ADR-42).
    pub import: ImportSettings,
    /// The image generator, its model and quality (ADR-45; the API key is never here).
    pub image_generator: crate::generate::ImageGenSettings,
    /// The MCP server of the running app (ADR-43).
    pub mcp: McpSettings,
}

impl Default for Config {
    /// What a fresh [`PainterApp`] starts with.
    fn default() -> Self {
        let tools = crate::tools::ToolState::default();
        let effects = crate::effects::Effects::default();
        let anim = crate::anim::AnimState::default();
        Self {
            ui_style: UiStyle::default(),
            theme: ThemeChoice::default(),
            pixel_size: PixelSize::default().0,
            prefs: Prefs::default(),
            drawers: BTreeMap::new(),
            grid: tools.grid,
            grid_size: tools.grid_size,
            symmetry: tools.symmetry,
            symmetry_settings: tools.symmetry_settings,
            spacing: tools.spacing,
            fill: tools.fill,
            translucency_on: effects.translucency_on,
            translucency_percent: effects.percent,
            light_on: anim.light_on,
            light: anim.light,
            import: ImportSettings::default(),
            image_generator: crate::generate::ImageGenSettings::default(),
            mcp: McpSettings::default(),
        }
    }
}

fn get_bool(o: &Map<String, Value>, key: &str, default: bool) -> bool {
    o.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn get_int(o: &Map<String, Value>, key: &str, range: (i32, i32), default: i32) -> i32 {
    match o.get(key).and_then(Value::as_i64) {
        Some(v) => v.clamp(i64::from(range.0), i64::from(range.1)) as i32,
        None => default,
    }
}

fn section(o: &Map<String, Value>, key: &str) -> Map<String, Value> {
    o.get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

impl Config {
    /// Reads the fields it knows; anything missing or of the wrong type keeps its default.
    /// `None` if `v` is not a JSON object.
    pub fn from_json(v: &Value) -> Option<Config> {
        let o = v.as_object()?;
        let d = Config::default();
        let mut c = d.clone();
        // ADR-47: files from before it lack both keys and read as Classic and System; so does
        // a name this version does not know.
        let name = |key: &str| o.get(key).and_then(Value::as_str);
        c.ui_style = name("ui_style")
            .and_then(UiStyle::from_key)
            .unwrap_or(d.ui_style);
        c.theme = name("theme")
            .and_then(ThemeChoice::from_key)
            .unwrap_or(d.theme);
        c.pixel_size = get_int(
            o,
            "pixel_size",
            (i32::from(PixelSize::MIN), i32::from(PixelSize::MAX)),
            i32::from(d.pixel_size),
        ) as u8;
        let p = section(o, "prefs");
        let mut prefs = d.prefs;
        for pref in Pref::ALL {
            prefs.set(pref, get_bool(&p, pref.key(), d.prefs.get(pref)));
        }
        c.prefs = prefs;
        if let Some(dr) = o.get("drawers").and_then(Value::as_object) {
            c.drawers = dr
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect();
        }
        let t = section(o, "tools");
        c.grid = get_bool(&t, "grid", d.grid);
        let g = section(&t, "grid_size");
        c.grid_size = GridSettings {
            x: get_int(&g, "x", GRID_RANGE, d.grid_size.x),
            y: get_int(&g, "y", GRID_RANGE, d.grid_size.y),
        };
        c.symmetry = get_bool(&t, "symmetry", d.symmetry);
        let s = section(&t, "symmetry_settings");
        c.symmetry_settings = SymmetrySettings {
            mirror: get_bool(&s, "mirror", d.symmetry_settings.mirror),
            order: get_int(
                &s,
                "order",
                SYMMETRY_RANGE,
                i32::from(d.symmetry_settings.order),
            ) as u8,
        };
        let sp = section(&t, "spacing");
        let mode = match sp.get("mode").and_then(Value::as_str) {
            Some("n_total") => SpacingMode::NTotal,
            Some("every_n") => SpacingMode::EveryN,
            Some("airbrush") => SpacingMode::Airbrush,
            Some("continuous") => SpacingMode::Continuous,
            _ => d.spacing.mode,
        };
        c.spacing = SpacingSettings {
            mode,
            n: get_int(&sp, "n", SPACING_RANGE, d.spacing.n),
            airbrush_radius: get_int(
                &sp,
                "airbrush_radius",
                AIRBRUSH_RANGE,
                d.spacing.airbrush_radius,
            ),
        };
        let f = section(&t, "fill");
        c.fill = FillSettings {
            gradient: match f.get("gradient") {
                None => d.fill.gradient,
                Some(Value::Null) => None,
                Some(g) => serde_json::from_value::<GradientDirection>(g.clone())
                    .ok()
                    .or(d.fill.gradient),
            },
            dither: get_bool(&f, "dither", d.fill.dither),
        };
        let tr = section(o, "translucency");
        c.translucency_on = get_bool(&tr, "on", d.translucency_on);
        c.translucency_percent = get_int(
            &tr,
            "percent",
            TRANSLUCENCY_RANGE,
            i32::from(d.translucency_percent),
        ) as u8;
        let l = section(o, "light_table");
        c.light_on = get_bool(&l, "on", d.light_on);
        c.light = LightTable {
            two_back: get_bool(&l, "two_back", d.light.two_back),
            previous: get_bool(&l, "previous", d.light.previous),
            next: get_bool(&l, "next", d.light.next),
            spare: get_bool(&l, "spare", d.light.spare),
            dim: get_bool(&l, "dim", d.light.dim),
        };
        let im = section(o, "import");
        let text = |key: &str| im.get(key).and_then(Value::as_str);
        c.import = ImportSettings {
            position: text("position")
                .and_then(import::parse_position)
                .unwrap_or(d.import.position),
            palette: text("palette")
                .and_then(import::parse_palette)
                .unwrap_or(d.import.palette),
            // Whole bitplanes, 2-256; the screen's maximum applies when the requester opens.
            colors: im
                .get("colors")
                .and_then(Value::as_i64)
                .map(|n| dpx_core::import::color_count(n.clamp(2, 256) as u16, 256))
                .or(d.import.colors),
            dither: text("dither")
                .and_then(import::parse_dither)
                .unwrap_or(d.import.dither),
        };
        c.image_generator =
            crate::generate::ImageGenSettings::from_json(&section(o, "image_generator"));
        c.mcp = McpSettings::from_json(&section(o, "mcp"), &d.mcp);
        Some(c)
    }

    pub fn to_json(&self) -> Value {
        let prefs: Map<String, Value> = Pref::ALL
            .into_iter()
            .map(|p| (p.key().to_owned(), Value::Bool(self.prefs.get(p))))
            .collect();
        let mode = match self.spacing.mode {
            SpacingMode::Continuous => "continuous",
            SpacingMode::NTotal => "n_total",
            SpacingMode::EveryN => "every_n",
            SpacingMode::Airbrush => "airbrush",
        };
        json!({
            "version": VERSION,
            "ui_style": self.ui_style.key(),
            "theme": self.theme.key(),
            "pixel_size": self.pixel_size,
            "prefs": prefs,
            "drawers": self.drawers,
            "tools": {
                "grid": self.grid,
                "grid_size": { "x": self.grid_size.x, "y": self.grid_size.y },
                "symmetry": self.symmetry,
                "symmetry_settings": {
                    "mirror": self.symmetry_settings.mirror,
                    "order": self.symmetry_settings.order,
                },
                "spacing": {
                    "mode": mode,
                    "n": self.spacing.n,
                    "airbrush_radius": self.spacing.airbrush_radius,
                },
                "fill": {
                    "gradient": self.fill.gradient.and_then(|g| serde_json::to_value(g).ok()),
                    "dither": self.fill.dither,
                },
            },
            "translucency": { "on": self.translucency_on, "percent": self.translucency_percent },
            "import": {
                "position": import::position_name(self.import.position),
                "palette": import::palette_name(self.import.palette),
                "colors": self.import.colors,
                "dither": import::dither_name(self.import.dither),
            },
            "image_generator": self.image_generator.to_json(),
            "light_table": {
                "on": self.light_on,
                "two_back": self.light.two_back,
                "previous": self.light.previous,
                "next": self.light.next,
                "spare": self.light.spare,
                "dim": self.light.dim,
            },
            "mcp": self.mcp.to_json(),
        })
    }

    /// The settings of a running app.
    pub fn capture(app: &PainterApp) -> Config {
        let mut drawers = BTreeMap::new();
        if let Some(d) = &app.last_drawer {
            drawers.insert(DRAWER_PICTURE.to_owned(), d.clone());
        }
        Config {
            ui_style: app.ui_style,
            theme: app.theme,
            pixel_size: app.ui.pixel_size.0,
            prefs: app.ui.prefs,
            drawers,
            grid: app.tools.grid,
            grid_size: app.tools.grid_size,
            symmetry: app.tools.symmetry,
            symmetry_settings: app.tools.symmetry_settings,
            spacing: app.tools.spacing,
            fill: app.tools.fill,
            translucency_on: app.menu_data.effects.translucency_on,
            translucency_percent: app.menu_data.effects.percent,
            light_on: app.anim.light_on,
            light: app.anim.light,
            import: app.import.settings,
            image_generator: app.imagegen.settings.clone(),
            mcp: app.mcp.settings.clone(),
        }
    }

    /// Puts the settings into a fresh app. A remembered drawer that no longer exists is
    /// dropped.
    pub fn apply(&self, app: &mut PainterApp) {
        app.ui_style = self.ui_style;
        app.theme = self.theme;
        app.ui.pixel_size = PixelSize(self.pixel_size);
        app.ui.prefs = self.prefs;
        app.last_drawer = self
            .drawers
            .get(DRAWER_PICTURE)
            .filter(|d| Path::new(d).is_dir())
            .cloned();
        app.tools.grid = self.grid;
        app.tools.grid_size = self.grid_size;
        app.tools.symmetry = self.symmetry;
        app.tools.symmetry_settings = self.symmetry_settings;
        app.tools.spacing = self.spacing;
        app.tools.fill = self.fill;
        app.menu_data.effects.translucency_on = self.translucency_on;
        app.menu_data.effects.percent = self.translucency_percent;
        app.anim.light_on = self.light_on;
        app.anim.light = self.light;
        app.import.settings = self.import;
        app.imagegen.settings = self.image_generator.clone();
        app.mcp.settings = self.mcp.clone();
    }
}

/// The directory for the settings file: `DPX_CONFIG_DIR` if set, else the platform's config
/// directory (macOS `~/Library/Application Support`, Windows `%APPDATA%`, elsewhere
/// `$XDG_CONFIG_HOME` or `~/.config`) plus `DeluxePainter`. The rule lives in
/// `dpx_mcp::paths` (ADR-43) so the MCP endpoint files and the stdio bridge use the same
/// directory.
pub fn default_dir() -> Option<PathBuf> {
    dpx_mcp::paths::config_dir()
}

/// The settings file and what was last written to it.
#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    saved: Config,
    /// A change waiting for the debounce: the settings and the time of the last change.
    pending: Option<(Config, f64)>,
}

impl ConfigStore {
    /// Opens `dir`'s settings file. Never fails: a missing file gives the defaults, a broken
    /// one is moved to `settings.json.bak` (with a note on stderr) and the defaults are used.
    pub fn open(dir: &Path) -> (ConfigStore, Config) {
        let path = dir.join(FILE);
        let config = Self::read(&path).unwrap_or_default();
        let store = ConfigStore {
            path,
            saved: config.clone(),
            pending: None,
        };
        (store, config)
    }

    fn read(path: &Path) -> Option<Config> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                eprintln!("note: settings file {} not read: {e}", path.display());
                return None;
            }
        };
        let parsed = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| Config::from_json(&v));
        if parsed.is_none() {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            match std::fs::rename(path, &bak) {
                Ok(()) => eprintln!(
                    "note: settings file {} is broken; kept as {}, using defaults",
                    path.display(),
                    Path::new(&bak).display()
                ),
                Err(e) => eprintln!(
                    "note: settings file {} is broken and could not be moved aside ({e}); using defaults",
                    path.display()
                ),
            }
        }
        parsed
    }

    /// Writes `config` through a temporary file and a rename.
    fn write(&mut self, config: &Config) {
        let result = (|| -> std::io::Result<()> {
            let dir = self.path.parent().unwrap_or(Path::new("."));
            std::fs::create_dir_all(dir)?;
            let tmp = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
            let mut text = serde_json::to_string_pretty(&config.to_json())?;
            text.push('\n');
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, &self.path).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        })();
        if let Err(e) = result {
            eprintln!("note: settings file {} not saved: {e}", self.path.display());
        }
        // Even after a failure, wait for the next change instead of retrying every frame.
        self.saved = config.clone();
        self.pending = None;
    }

    /// Called every frame with the current settings and the time in seconds. Writes once
    /// they have stopped changing for [`DEBOUNCE`]; returns how long to wait for that.
    pub fn update(&mut self, current: Config, now: f64) -> Option<f64> {
        if current == self.saved {
            self.pending = None;
            return None;
        }
        let since = match &self.pending {
            Some((p, t)) if *p == current => *t,
            _ => now,
        };
        if now - since >= DEBOUNCE {
            self.write(&current);
            None
        } else {
            self.pending = Some((current, since));
            Some(DEBOUNCE - (now - since))
        }
    }

    /// Writes at once if anything changed (on quit).
    pub fn flush(&mut self, current: Config) {
        if current != self.saved {
            self.write(&current);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dpx-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn changed() -> Config {
        let mut c = Config {
            ui_style: UiStyle::Modern,
            theme: ThemeChoice::Light,
            pixel_size: 9,
            ..Config::default()
        };
        c.prefs.toggle(Pref::Coords);
        c.prefs.toggle(Pref::BeSquare);
        c.drawers.insert(
            "picture".into(),
            std::env::temp_dir().to_string_lossy().into(),
        );
        c.grid = true;
        c.grid_size = GridSettings { x: 12, y: 24 };
        c.symmetry = true;
        c.symmetry_settings = SymmetrySettings {
            mirror: true,
            order: 6,
        };
        c.spacing = SpacingSettings {
            mode: SpacingMode::EveryN,
            n: 7,
            airbrush_radius: 20,
        };
        c.fill = FillSettings {
            gradient: Some(GradientDirection::HorizontalLine),
            dither: true,
        };
        c.translucency_on = true;
        c.translucency_percent = 25;
        c.light_on = true;
        c.light = LightTable {
            two_back: true,
            previous: false,
            next: true,
            spare: true,
            dim: false,
        };
        c.import = ImportSettings {
            position: dpx_core::import::Position::TopLeft,
            palette: dpx_core::import::PaletteChoice::Current,
            colors: Some(8),
            dither: dpx_core::quantize::Dither::Ordered,
        };
        c
    }

    #[test]
    fn import_settings_load_from_old_files_and_clamp_colours() {
        let d = dir("import");
        // A file from before ADR-42: the defaults (colours: the document's).
        std::fs::write(d.join(FILE), r#"{"version": 1, "pixel_size": 6}"#).unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(c.import, ImportSettings::default());
        assert_eq!(c.import.colors, None);
        // Odd counts become whole bitplanes, out of range is clamped, bad names fall back.
        std::fs::write(
            d.join(FILE),
            r#"{"import": {"position": "top_left", "palette": "sideways",
                          "colors": 100, "dither": "none"}}"#,
        )
        .unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(c.import.position, dpx_core::import::Position::TopLeft);
        assert_eq!(c.import.palette, dpx_core::import::PaletteChoice::New);
        assert_eq!(c.import.colors, Some(64));
        assert_eq!(c.import.dither, dpx_core::quantize::Dither::None);
        std::fs::write(d.join(FILE), r#"{"import": {"colors": 100000}}"#).unwrap();
        assert_eq!(ConfigStore::open(&d).1.import.colors, Some(256));
        std::fs::write(d.join(FILE), r#"{"import": {"colors": -3}}"#).unwrap();
        assert_eq!(ConfigStore::open(&d).1.import.colors, Some(2));
    }

    #[test]
    fn skin_and_theme_round_trip_and_old_files_are_classic() {
        // ADR-47. Every combination survives the file.
        for ui_style in UiStyle::ALL {
            for theme in ThemeChoice::ALL {
                let d = dir("skin");
                let c = Config {
                    ui_style,
                    theme,
                    ..Config::default()
                };
                let v = c.to_json();
                assert_eq!(v["ui_style"], ui_style.key());
                assert_eq!(v["theme"], theme.key());
                assert_eq!(Config::from_json(&v), Some(c.clone()));
                // Through the file (the defaults are what a missing file gives, so they
                // are not written).
                let (mut store, _) = ConfigStore::open(&d);
                store.flush(c.clone());
                assert_eq!(d.join(FILE).exists(), c != Config::default());
                assert_eq!(ConfigStore::open(&d).1, c);
            }
        }
        // A first start and a file from before ADR-47: Classic, theme from the system.
        assert_eq!(Config::default().ui_style, UiStyle::Classic);
        assert_eq!(Config::default().theme, ThemeChoice::System);
        let d = dir("skin-old");
        std::fs::write(d.join(FILE), r#"{"version": 1, "pixel_size": 6}"#).unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(
            (c.ui_style, c.theme),
            (UiStyle::Classic, ThemeChoice::System)
        );
        // Each key is read on its own.
        std::fs::write(d.join(FILE), r#"{"ui_style": "modern"}"#).unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(
            (c.ui_style, c.theme),
            (UiStyle::Modern, ThemeChoice::System)
        );
        std::fs::write(d.join(FILE), r#"{"theme": "dark"}"#).unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!((c.ui_style, c.theme), (UiStyle::Classic, ThemeChoice::Dark));
        // Unknown names and wrong types fall back and do not break the file.
        std::fs::write(
            d.join(FILE),
            r#"{"ui_style": "neon", "theme": 3, "pixel_size": 10}"#,
        )
        .unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert!(d.join(FILE).exists(), "readable file is not moved");
        assert_eq!(
            (c.ui_style, c.theme),
            (UiStyle::Classic, ThemeChoice::System)
        );
        assert_eq!(c.pixel_size, 10);
        std::fs::write(d.join(FILE), r#"{"ui_style": true, "theme": "LIGHT"}"#).unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(
            (c.ui_style, c.theme),
            (UiStyle::Classic, ThemeChoice::System)
        );
    }

    #[test]
    fn skin_and_theme_reach_the_app_and_come_back() {
        let mut app = PainterApp::new();
        assert_eq!(
            (app.ui_style, app.theme),
            (UiStyle::Classic, ThemeChoice::System),
            "a fresh app is Classic"
        );
        let c = Config {
            ui_style: UiStyle::Modern,
            theme: ThemeChoice::Dark,
            ..Config::default()
        };
        c.apply(&mut app);
        assert_eq!(
            (app.ui_style, app.theme),
            (UiStyle::Modern, ThemeChoice::Dark)
        );
        assert_eq!(Config::capture(&app), c);
    }

    #[test]
    fn defaults_without_a_file() {
        let d = dir("none");
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(c, Config::default());
        assert!(!d.join(FILE).exists(), "opening writes nothing");
        // A fresh app's own state equals the defaults.
        assert_eq!(Config::capture(&PainterApp::new()), Config::default());
    }

    #[test]
    fn round_trip_through_the_file() {
        let d = dir("round");
        let (mut store, _) = ConfigStore::open(&d);
        store.flush(changed());
        let text = std::fs::read_to_string(d.join(FILE)).unwrap();
        assert!(text.contains("\"version\": 1"));
        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            leftovers,
            [std::ffi::OsString::from(FILE)],
            "no temp file left"
        );
        let (_, back) = ConfigStore::open(&d);
        assert_eq!(back, changed());
    }

    #[test]
    fn apply_and_capture_agree() {
        let c = changed();
        let mut app = PainterApp::new();
        c.apply(&mut app);
        assert_eq!(Config::capture(&app), c);
        // A drawer that is gone is dropped.
        let mut gone = c.clone();
        gone.drawers
            .insert("picture".into(), "/nonexistent-dpx-drawer".into());
        let mut app = PainterApp::new();
        gone.apply(&mut app);
        assert_eq!(app.last_drawer, None);
    }

    #[test]
    fn broken_file_is_kept_as_bak() {
        let d = dir("broken");
        std::fs::write(d.join(FILE), "{ not json").unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(c, Config::default());
        assert!(!d.join(FILE).exists());
        assert_eq!(
            std::fs::read_to_string(d.join("settings.json.bak")).unwrap(),
            "{ not json"
        );
        // Valid JSON that is not an object is broken too.
        std::fs::write(d.join(FILE), "[1,2]").unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert_eq!(c, Config::default());
        assert_eq!(
            std::fs::read_to_string(d.join("settings.json.bak")).unwrap(),
            "[1,2]"
        );
    }

    #[test]
    fn unknown_fields_are_ignored_and_bad_values_fall_back() {
        let d = dir("unknown");
        std::fs::write(
            d.join(FILE),
            r#"{"version": 7, "future": {"x": 1}, "pixel_size": 10,
                "prefs": {"coords": true, "from_the_future": true, "be_square": "yes"},
                "tools": {"grid": true, "grid_size": {"x": 99999, "y": "a", "z": 1},
                          "fill": {"gradient": "sideways"}, "spacing": {"mode": "zigzag"}},
                "translucency": {"percent": 500}}"#,
        )
        .unwrap();
        let (_, c) = ConfigStore::open(&d);
        assert!(d.join(FILE).exists(), "readable file is not moved");
        let def = Config::default();
        assert_eq!(c.pixel_size, 10);
        assert!(c.prefs.get(Pref::Coords));
        assert_eq!(c.prefs.get(Pref::BeSquare), def.prefs.get(Pref::BeSquare));
        assert!(c.grid);
        assert_eq!(
            c.grid_size,
            GridSettings {
                x: GRID_RANGE.1,
                y: def.grid_size.y
            }
        );
        assert_eq!(c.fill, def.fill);
        assert_eq!(c.spacing, def.spacing);
        assert_eq!(c.translucency_percent, 100);
        assert_eq!(c.light, def.light);
    }

    #[test]
    fn writes_are_debounced() {
        let d = dir("debounce");
        let (mut store, _) = ConfigStore::open(&d);
        assert_eq!(store.update(Config::default(), 0.0), None);
        let mut c = Config {
            grid: true,
            ..Config::default()
        };
        assert!(store.update(c.clone(), 10.0).is_some());
        assert!(!d.join(FILE).exists());
        c.symmetry = true;
        assert!(
            store.update(c.clone(), 10.6).is_some(),
            "a new change waits again"
        );
        assert!(store.update(c.clone(), 11.0).is_some());
        assert!(!d.join(FILE).exists());
        assert_eq!(store.update(c.clone(), 11.7), None);
        let (_, back) = ConfigStore::open(&d);
        assert_eq!(back, c);
    }

    #[test]
    fn unwritable_directory_does_not_crash() {
        let d = dir("unwritable");
        // A file where the directory should be.
        let blocker = d.join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let (mut store, _) = ConfigStore::open(&blocker.join("sub"));
        store.flush(changed());
    }

    #[test]
    fn config_dir_name() {
        // Only reads the environment; does not set it (tests share the process).
        if std::env::var_os("DPX_CONFIG_DIR").is_none() && std::env::var_os("HOME").is_some() {
            let d = default_dir().expect("a home directory");
            assert!(d.ends_with("DeluxePainter"));
        }
    }

    #[test]
    fn settings_and_mcp_endpoint_files_share_one_directory() {
        // ADR-43: the bridge finds the endpoint file next to the settings file.
        assert_eq!(default_dir(), dpx_mcp::paths::config_dir());
        if let Some(d) = default_dir() {
            assert_eq!(
                dpx_mcp::endpoint::endpoint_path(&d).parent(),
                Some(d.join(FILE).parent().unwrap())
            );
        }
    }
}
