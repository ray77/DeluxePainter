# ADR-33: Settings file

- Status: accepted (Wave 6)
- Context: ADR-13 left "saving the pixel size" for a settings file. The Prefs toggles, tool settings and the last drawer were also lost at every start.
- Decision:
  - **File:** `settings.json` in the app's own config directory: macOS `~/Library/Application Support/DeluxePainter`, Windows `%APPDATA%\DeluxePainter`, elsewhere `$XDG_CONFIG_HOME` (or `~/.config`) `/DeluxePainter`. `DPX_CONFIG_DIR` overrides the directory. Only std and `serde_json` are used; no new dependency.
  - **Content (version 1):** `pixel_size`; `prefs` (one bool per Prefs item, by stable name, `Pref::key`); `drawers` (last drawer per requester kind; today only `picture`, used by Load/Save/Export, because the app has one drawer for all of them); `tools` (grid on and size, symmetry on and settings, spacing mode/N/airbrush radius, fill type and dither); `translucency` (on, percent); `light_table` (on and the choices). Not saved: anything that belongs to a document, the current tool, colours, brushes, stencil.
  - **Loading:** each field is read on its own; missing fields, wrong types and unknown fields fall back to the default or are ignored, numbers are clamped to the requesters' ranges. A file that does not parse as a JSON object is moved to `settings.json.bak` with a note on stderr, and the defaults are used. A remembered drawer that no longer exists is dropped. A file with a newer `version` is read as far as known and rewritten as version 1 on the next change.
  - **Saving:** every frame the current settings are compared with what was last written; once they have stopped changing for 1 s they are written (temp file `settings.json.<pid>.tmp`, then rename), and again at quit (`on_exit`). A write error is a stderr note, never a crash or a retry every frame.
  - **Who reads it:** only the GUI start. `--snapshot`, `info`, `convert`, `mcp` and `--font-preview` return before it is opened; `--no-settings` skips it for the GUI. Tests use `ConfigStore::open(dir)` with their own temp directories.
  - **Start window:** sized from the saved pixel size (then ADR-13's snapping applies).
- Update (ADR-47): two more top-level keys, still version 1: `ui_style` (`"classic"` or `"modern"`) and `theme` (`"system"`, `"dark"`, `"light"`). A file without them, or with a name this version does not know, reads as Classic and System.
- Not done: full-screen state and window position are not saved; the file is loaded once (no live reload; two running instances overwrite each other, last writer wins).
- Verified (tests, run twice): defaults without a file, round trip with every field, apply/capture agreement, broken file kept as `.bak`, unknown fields and bad values, debounce timing, unwritable directory.
- Not verified: the GUI by hand across a restart; Windows and Linux paths.
