//! The lossless project format `.dpxproj` (ADR-23, `docs/PROJECT_FORMAT.md`).
//!
//! A ZIP container with a JSON manifest and one indexed PNG per layer. The reader validates
//! everything it is given (versions, entry names, sizes, PNG kind and palette) and the writer
//! is deterministic, so the same project always gives the same bytes. The app maps its
//! picture-file extras (colour ranges, grab point, transparent colour) from and to
//! [`ProjectExtras`]; this module knows nothing about the app.

use std::collections::HashSet;
use std::fmt;
use std::io::{Cursor, Read, Write};

use dpx_core::{Document, IndexedImage, Palette, Rgb8, Target};
use serde::{Deserialize, Serialize};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::png_io::{PngImage, read_png, write_png};

/// The format name in the manifest.
pub const FORMAT: &str = "dpxproj";
/// The newest format version this build reads and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

const MANIFEST: &str = "manifest.json";
const SPARE_PATH: &str = "spare/page.png";
const LAYER_NAME: &str = "Background";

/// What the reader accepts. [`Limits::default`] is for real use; tests lower the numbers.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Most entries in the archive.
    pub max_entries: usize,
    /// Most bytes of all entries together, once uncompressed.
    pub max_total_bytes: u64,
    /// Most bytes of the manifest.
    pub max_manifest_bytes: u64,
    /// Most pixels of one image (as for plain PNG import).
    pub max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_manifest_bytes: 1024 * 1024,
            max_pixels: 64 * 1024 * 1024,
        }
    }
}

/// One colour-cycling range, kept exactly as the ILBM CRNG chunk stores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRange {
    /// 16384 = 60 steps per second.
    pub rate: u16,
    /// Bit 0 active, bit 1 reverse; other bits are kept as they are.
    pub flags: u16,
    pub low: u8,
    pub high: u8,
}

/// Picture details next to the document that the project keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectExtras {
    pub ranges: Vec<ProjectRange>,
    pub grab: Option<(i16, i16)>,
    /// Palette entry that counts as transparent in brushes.
    pub transparent_color: Option<u16>,
}

/// A project: the document, its extras and the optional spare page (same palette).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub document: Document,
    pub extras: ProjectExtras,
    pub spare: Option<IndexedImage>,
}

impl Project {
    pub fn new(document: Document, extras: ProjectExtras) -> Self {
        Self {
            document,
            extras,
            spare: None,
        }
    }
}

/// Why a project cannot be read or written. Displays as a short first line (at most 34
/// characters, for the file requester) followed by details (ADR-17).
#[derive(Debug)]
pub enum ProjectError {
    /// Not a ZIP archive or not a `.dpxproj` at all.
    NotAProject(String),
    /// A `format_version` newer than [`FORMAT_VERSION`].
    NewerVersion(u64),
    /// A size limit was exceeded.
    TooLarge(String),
    /// The project is damaged or breaks a format rule.
    Invalid(String),
    /// The project cannot be written as described.
    Write(String),
}

impl ProjectError {
    pub fn summary(&self) -> String {
        match self {
            ProjectError::NotAProject(_) => "Not a DeluxePainter project".into(),
            ProjectError::NewerVersion(_) => "Project is from a newer version".into(),
            ProjectError::TooLarge(_) => "Project file is too large".into(),
            ProjectError::Invalid(_) => "Project file is damaged".into(),
            ProjectError::Write(_) => "Cannot write project".into(),
        }
    }

    pub fn details(&self) -> String {
        match self {
            ProjectError::NewerVersion(v) => format!(
                "The file has project format version {v}; this build reads up to version \
                 {FORMAT_VERSION}."
            ),
            ProjectError::NotAProject(d)
            | ProjectError::TooLarge(d)
            | ProjectError::Invalid(d)
            | ProjectError::Write(d) => d.clone(),
        }
    }
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{}", self.summary(), self.details())
    }
}

impl std::error::Error for ProjectError {}

fn invalid(what: impl Into<String>) -> ProjectError {
    ProjectError::Invalid(what.into())
}

// ---- manifest ----

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    format_version: u32,
    app_version: String,
    target: Target,
    page: PageSize,
    /// `#RRGGBB` strings, index = colour number.
    palette: Vec<String>,
    ranges: Vec<ProjectRange>,
    grab: Option<(i16, i16)>,
    transparent_color: Option<u16>,
    frames: Vec<FrameEntry>,
    spare: Option<SpareEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PageSize {
    width: u32,
    height: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameEntry {
    layers: Vec<LayerEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerEntry {
    name: String,
    image: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpareEntry {
    image: String,
    width: u32,
    height: u32,
}

fn layer_path(frame: usize, layer: usize) -> String {
    format!("frames/{frame}/layers/{layer}.png")
}

// ---- writer ----

/// Writes the project. The same project always gives the same bytes: entries come in a fixed
/// order with a fixed time stamp and fixed attributes.
pub fn write_project(project: &Project) -> Result<Vec<u8>, ProjectError> {
    let doc = &project.document;
    let (page, palette) = (doc.page(), doc.palette());
    let png = |image: &IndexedImage| {
        write_png(image, palette).map_err(|e| ProjectError::Write(e.to_string()))
    };
    let page_png = png(page)?;
    let spare_png = project.spare.as_ref().map(png).transpose()?;
    let manifest = Manifest {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        target: doc.target(),
        page: PageSize {
            width: page.width(),
            height: page.height(),
        },
        palette: palette.colors().iter().map(ToString::to_string).collect(),
        ranges: project.extras.ranges.clone(),
        grab: project.extras.grab,
        transparent_color: project.extras.transparent_color,
        frames: vec![FrameEntry {
            layers: vec![LayerEntry {
                name: LAYER_NAME.into(),
                image: layer_path(0, 0),
            }],
        }],
        spare: project.spare.as_ref().map(|s| SpareEntry {
            image: SPARE_PATH.into(),
            width: s.width(),
            height: s.height(),
        }),
    };
    let mut json = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| ProjectError::Write(format!("manifest: {e}")))?;
    json.push(b'\n');

    let mut entries: Vec<(String, Vec<u8>)> =
        vec![(MANIFEST.into(), json), (layer_path(0, 0), page_png)];
    if let Some(spare) = spare_png {
        entries.push((SPARE_PATH.into(), spare));
    }
    zip_entries(&entries)
}

fn zip_entries(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, ProjectError> {
    let werr = |e: &dyn fmt::Display| ProjectError::Write(format!("ZIP: {e}"));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        zip.start_file(name.as_str(), options)
            .map_err(|e| werr(&e))?;
        zip.write_all(data).map_err(|e| werr(&e))?;
    }
    Ok(zip.finish().map_err(|e| werr(&e))?.into_inner())
}

// ---- reader ----

/// True if `bytes` start like a ZIP archive (local file header, or an empty archive).
pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

/// Reads and validates a project with the default [`Limits`].
pub fn read_project(bytes: &[u8]) -> Result<Project, ProjectError> {
    read_project_with(bytes, Limits::default())
}

/// Entry names must be plain relative paths of `/`-separated components.
fn check_name(name: &str) -> Result<(), ProjectError> {
    let bad = name.is_empty()
        || name.len() > 255
        || name.starts_with('/')
        || name.contains(['\\', '\0', ':'])
        || name
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..");
    if bad {
        return Err(invalid(format!("unsafe entry name {name:?}")));
    }
    Ok(())
}

pub fn read_project_with(bytes: &[u8], limits: Limits) -> Result<Project, ProjectError> {
    if !is_zip(bytes) {
        return Err(ProjectError::NotAProject(
            "The file is not a ZIP archive.".into(),
        ));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| ProjectError::NotAProject(format!("The ZIP archive cannot be read: {e}.")))?;
    if archive.len() > limits.max_entries {
        return Err(ProjectError::TooLarge(format!(
            "The archive has {} entries; at most {} are accepted.",
            archive.len(),
            limits.max_entries
        )));
    }
    // Names and declared sizes first, before anything is inflated.
    let mut names: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let f = archive
            .by_index_raw(i)
            .map_err(|e| invalid(format!("entry {i}: {e}")))?;
        let name = f
            .name()
            .map_err(|e| invalid(format!("entry {i}: bad name: {e}")))?
            .to_string();
        if f.is_dir() || name.ends_with('/') {
            return Err(invalid(format!("unexpected directory entry {name:?}")));
        }
        check_name(&name)?;
        if !seen.insert(name.clone()) {
            return Err(invalid(format!("duplicate entry {name:?}")));
        }
        if !matches!(
            f.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(invalid(format!(
                "entry {name:?} uses {:?} compression; only stored and deflate are accepted",
                f.compression()
            )));
        }
        total = total.saturating_add(f.size());
        names.push(name);
    }
    if total > limits.max_total_bytes {
        return Err(ProjectError::TooLarge(format!(
            "The entries add up to {total} bytes uncompressed; at most {} are accepted.",
            limits.max_total_bytes
        )));
    }

    let mut read_entry = |name: &str, cap: u64| -> Result<Vec<u8>, ProjectError> {
        let mut f = archive
            .by_name(name)
            .map_err(|_| invalid(format!("missing entry {name:?}")))?;
        let mut out = Vec::new();
        // Declared sizes can lie: the real number of bytes is capped as well.
        (&mut f)
            .take(cap + 1)
            .read_to_end(&mut out)
            .map_err(|e| invalid(format!("entry {name:?}: {e}")))?;
        if out.len() as u64 > cap {
            return Err(ProjectError::TooLarge(format!(
                "Entry {name:?} is larger than the {cap} bytes accepted."
            )));
        }
        Ok(out)
    };

    if !names.iter().any(|n| n == MANIFEST) {
        return Err(invalid(format!("missing entry {MANIFEST:?}")));
    }
    let raw = read_entry(MANIFEST, limits.max_manifest_bytes)?;
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| invalid(format!("{MANIFEST}: {e}")))?;
    match value.get("format").and_then(|f| f.as_str()) {
        Some(FORMAT) => {}
        _ => {
            return Err(ProjectError::NotAProject(format!(
                "{MANIFEST} does not say \"format\": \"{FORMAT}\"."
            )));
        }
    }
    match value.get("format_version").and_then(|v| v.as_u64()) {
        Some(v) if v > u64::from(FORMAT_VERSION) => return Err(ProjectError::NewerVersion(v)),
        Some(v) if v >= 1 => {}
        _ => return Err(invalid("format_version must be a whole number from 1")),
    }
    let m: Manifest =
        serde_json::from_value(value).map_err(|e| invalid(format!("{MANIFEST}: {e}")))?;

    // Palette.
    let colors = m
        .palette
        .iter()
        .map(|s| parse_hex(s))
        .collect::<Result<Vec<_>, _>>()?;
    let palette = Palette::new(colors).map_err(|e| invalid(format!("palette: {e}")))?;

    // Layers: exactly one frame with one layer in version 1.
    let [frame] = m.frames.as_slice() else {
        return Err(invalid(format!(
            "version 1 holds exactly one frame, the manifest lists {}",
            m.frames.len()
        )));
    };
    let [layer] = frame.layers.as_slice() else {
        return Err(invalid(format!(
            "version 1 holds exactly one layer, the manifest lists {}",
            frame.layers.len()
        )));
    };
    if layer.name.chars().count() > 255 {
        return Err(invalid("layer name is longer than 255 characters"));
    }
    check_name(&layer.image)?;
    if let Some(s) = &m.spare {
        check_name(&s.image)?;
    }

    // The entry set must be exactly the manifest plus the images it names.
    let mut expected: Vec<&str> = vec![MANIFEST, &layer.image];
    if let Some(s) = &m.spare {
        expected.push(&s.image);
    }
    let expected_set: HashSet<&str> = expected.iter().copied().collect();
    if expected_set.len() != expected.len() {
        return Err(invalid("the manifest names an image twice"));
    }
    for e in &expected {
        if !names.iter().any(|n| n == e) {
            return Err(invalid(format!("missing entry {e:?}")));
        }
    }
    if let Some(extra) = names.iter().find(|n| !expected_set.contains(n.as_str())) {
        return Err(invalid(format!("unexpected entry {extra:?}")));
    }

    let budget = limits.max_total_bytes;
    let mut image = |path: &str, w: u32, h: u32| -> Result<IndexedImage, ProjectError> {
        let data = read_entry(path, budget)?;
        decode_layer(path, &data, w, h, &palette, limits)
    };
    let page = image(&layer.image, m.page.width, m.page.height)?;
    let spare = match &m.spare {
        Some(s) => Some(image(&s.image, s.width, s.height)?),
        None => None,
    };

    let document = Document::from_picture(page, palette, m.target);
    let extras = ProjectExtras {
        ranges: m.ranges,
        grab: m.grab,
        transparent_color: m.transparent_color,
    };
    Ok(Project {
        document,
        extras,
        spare,
    })
}

fn parse_hex(s: &str) -> Result<Rgb8, ProjectError> {
    let b = s.as_bytes();
    if b.len() != 7 || b[0] != b'#' || !b[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(invalid(format!("palette entry {s:?} is not #RRGGBB")));
    }
    let v = u32::from_str_radix(&s[1..], 16).expect("six hex digits");
    Ok(Rgb8::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Checks the PNG header, then decodes and compares size and palette with the manifest.
fn decode_layer(
    path: &str,
    data: &[u8],
    width: u32,
    height: u32,
    palette: &Palette,
    limits: Limits,
) -> Result<IndexedImage, ProjectError> {
    let bad = |what: String| invalid(format!("{path}: {what}"));
    // Signature (8), IHDR length (4), "IHDR" (4), width, height, bit depth, colour type.
    if data.len() < 26 || &data[..8] != b"\x89PNG\r\n\x1a\n" || &data[12..16] != b"IHDR" {
        return Err(bad("not a PNG picture".into()));
    }
    let w = u32::from_be_bytes(data[16..20].try_into().expect("4 bytes"));
    let h = u32::from_be_bytes(data[20..24].try_into().expect("4 bytes"));
    let (depth, color) = (data[24], data[25]);
    if (color, depth) != (3, 8) {
        return Err(bad(format!(
            "must be an 8-bit indexed PNG (colour type {color}, {depth} bits found)"
        )));
    }
    if (w, h) != (width, height) {
        return Err(bad(format!(
            "is {w}x{h} pixels, the manifest says {width}x{height}"
        )));
    }
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > limits.max_pixels {
        return Err(ProjectError::TooLarge(format!(
            "{path}: {width}x{height} pixels is empty or exceeds {} pixels.",
            limits.max_pixels
        )));
    }
    match read_png(data) {
        Ok(PngImage::Indexed(pic)) => {
            if pic.palette != *palette {
                return Err(bad("its PLTE differs from the manifest palette".into()));
            }
            if !pic.transparent_entries.is_empty() {
                return Err(bad("has PNG transparency (tRNS), pages are opaque".into()));
            }
            Ok(pic.image)
        }
        Ok(PngImage::TrueColor(_)) => Err(bad("is not an indexed PNG".into())),
        Err(e) => Err(bad(e.to_string().replace('\n', ": "))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dpx_core::{Chipset, DisplayMode, Resolution, Video};

    fn sample_image(w: u32, h: u32, colors: usize) -> IndexedImage {
        let mut img = IndexedImage::new(w, h, 0);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                img.set(x, y, ((x * 5 + 3 * y) as usize % colors) as u8);
            }
        }
        img
    }

    fn sample_palette(colors: usize) -> Palette {
        Palette::new(
            (0..colors)
                .map(|i| Rgb8::new(i as u8, (i * 7) as u8, 255 - i as u8))
                .collect(),
        )
        .unwrap()
    }

    fn project(w: u32, h: u32, colors: usize) -> Project {
        let doc = Document::from_picture(
            sample_image(w, h, colors),
            sample_palette(colors),
            Target::default(),
        );
        Project::new(doc, ProjectExtras::default())
    }

    fn assert_same(a: &Project, b: &Project) {
        assert_eq!(a.document.page(), b.document.page());
        assert_eq!(a.document.palette(), b.document.palette());
        assert_eq!(a.document.target(), b.document.target());
        assert_eq!(a.extras, b.extras);
        assert_eq!(a.spare, b.spare);
    }

    fn round_trip(p: &Project) -> Project {
        let back = read_project(&write_project(p).unwrap()).unwrap();
        assert_same(p, &back);
        back
    }

    fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        (0..z.len())
            .map(|i| {
                let mut f = z.by_index(i).unwrap();
                let mut d = Vec::new();
                f.read_to_end(&mut d).unwrap();
                (f.name().unwrap().to_string(), d)
            })
            .collect()
    }

    fn zipped(list: &[(&str, &[u8])]) -> Vec<u8> {
        let owned: Vec<_> = list
            .iter()
            .map(|(n, d)| (n.to_string(), d.to_vec()))
            .collect();
        zip_entries(&owned).unwrap()
    }

    fn manifest_of(bytes: &[u8]) -> serde_json::Value {
        let e = entries(bytes);
        serde_json::from_slice(&e[0].1).unwrap()
    }

    /// The default project with its manifest changed by `f`.
    fn with_manifest(f: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let bytes = write_project(&project(8, 4, 4)).unwrap();
        let mut e = entries(&bytes);
        let mut m = manifest_of(&bytes);
        f(&mut m);
        e[0].1 = serde_json::to_vec(&m).unwrap();
        zip_entries(&e).unwrap()
    }

    fn err_of(bytes: &[u8]) -> ProjectError {
        read_project(bytes).unwrap_err()
    }

    #[test]
    fn round_trip_default_and_odd_sizes() {
        let pal = Palette::dpaint_default_32();
        let doc = Document::new(320, 256, pal);
        round_trip(&Project::new(doc, ProjectExtras::default()));
        for (w, h) in [(1, 1), (7, 3), (33, 5), (321, 257)] {
            round_trip(&project(w, h, 4));
        }
    }

    #[test]
    fn round_trip_palette_sizes() {
        for colors in [2, 32, 256] {
            let back = round_trip(&project(40, 20, colors));
            assert_eq!(back.document.palette().len(), colors);
        }
    }

    #[test]
    fn round_trip_aga_eight_planes_and_modes() {
        let mode = DisplayMode::new(Video::Ntsc, Resolution::Hi, true);
        let target = Target::new(Chipset::Aga, mode).with_screen(704, 480);
        let doc = Document::from_picture(sample_image(704, 480, 256), sample_palette(256), target);
        let back = round_trip(&Project::new(doc, ProjectExtras::default()));
        assert_eq!(back.document.target().screen, (704, 480));
        let ocs = Target::new(Chipset::Ocs, DisplayMode::PAL_LORES);
        let doc = Document::from_picture(sample_image(320, 256, 16), sample_palette(16), ocs);
        round_trip(&Project::new(doc, ProjectExtras::default()));
    }

    #[test]
    fn round_trip_extras_and_spare() {
        let mut p = project(30, 20, 32);
        p.extras = ProjectExtras {
            ranges: vec![
                ProjectRange {
                    rate: 4096,
                    flags: 0,
                    low: 20,
                    high: 27,
                },
                ProjectRange {
                    rate: 16384,
                    flags: 0x8003,
                    low: 8,
                    high: 15,
                },
            ],
            grab: Some((-3, 12)),
            transparent_color: Some(5),
        };
        p.spare = Some(sample_image(31, 7, 32));
        round_trip(&p);
        let mut none = p.clone();
        none.extras.grab = None;
        none.extras.transparent_color = None;
        none.spare = None;
        round_trip(&none);
    }

    #[test]
    fn writer_is_deterministic_with_a_fixed_layout() {
        let mut p = project(30, 20, 32);
        p.spare = Some(sample_image(30, 20, 32));
        let a = write_project(&p).unwrap();
        assert_eq!(a, write_project(&p).unwrap());
        // Writing a read project gives the same bytes again.
        assert_eq!(a, write_project(&read_project(&a).unwrap()).unwrap());
        let names: Vec<_> = entries(&a).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            ["manifest.json", "frames/0/layers/0.png", "spare/page.png"]
        );
        assert!(is_zip(&a));
    }

    #[test]
    fn manifest_describes_the_project() {
        let m = manifest_of(&write_project(&project(8, 4, 2)).unwrap());
        assert_eq!(m["format"], "dpxproj");
        assert_eq!(m["format_version"], 1);
        assert_eq!(m["page"]["width"], 8);
        assert_eq!(m["palette"][1], "#0107FE");
        assert_eq!(m["target"]["chipset"], "aga");
        assert_eq!(m["frames"][0]["layers"][0]["name"], "Background");
        assert_eq!(
            m["frames"][0]["layers"][0]["image"],
            "frames/0/layers/0.png"
        );
    }

    #[test]
    fn newer_version_is_named() {
        let e = err_of(&with_manifest(|m| m["format_version"] = 7.into()));
        assert!(matches!(e, ProjectError::NewerVersion(7)), "{e}");
        assert!(e.details().contains("version 7"), "{e}");
        assert!(e.summary().len() <= 34);
        let e = err_of(&with_manifest(|m| m["format_version"] = 0.into()));
        assert!(matches!(e, ProjectError::Invalid(_)), "{e}");
    }

    #[test]
    fn foreign_and_damaged_files_are_refused() {
        assert!(matches!(
            err_of(b"FORM....ILBM"),
            ProjectError::NotAProject(_)
        ));
        assert!(matches!(
            err_of(b"PK\x03\x04junk"),
            ProjectError::NotAProject(_)
        ));
        let e = err_of(&with_manifest(|m| m["format"] = "other".into()));
        assert!(matches!(e, ProjectError::NotAProject(_)), "{e}");
        let e = err_of(&with_manifest(|m| m["surprise"] = 1.into()));
        assert!(e.details().contains("surprise"), "{e}");
        let e = err_of(&with_manifest(|m| m["palette"][0] = "red".into()));
        assert!(e.details().contains("#RRGGBB"), "{e}");
        let e = err_of(&with_manifest(|m| m["palette"] = serde_json::json!([])));
        assert!(e.details().contains("palette"), "{e}");
        for e in [
            err_of(b"FORM"),
            err_of(&with_manifest(|m| m["format_version"] = 9.into())),
        ] {
            assert!(e.summary().len() <= 34, "{e}");
        }
        let good = write_project(&project(8, 4, 4)).unwrap();
        assert!(read_project(&good[..good.len() / 2]).is_err());
    }

    #[test]
    fn traversal_and_unsafe_names_are_refused() {
        let good = write_project(&project(8, 4, 4)).unwrap();
        let e = entries(&good);
        for bad in [
            "../evil.png",
            "/abs.png",
            "frames/../../x.png",
            "a\\b.png",
            "C:evil.png",
            "frames//0.png",
        ] {
            let bytes = zipped(&[(&e[0].0, &e[0].1), (&e[1].0, &e[1].1), (bad, b"x")]);
            let err = err_of(&bytes);
            assert!(err.details().contains("unsafe entry name"), "{bad}: {err}");
        }
        // The same in the manifest's image path.
        let err = err_of(&with_manifest(|m| {
            m["frames"][0]["layers"][0]["image"] = "../0.png".into();
        }));
        assert!(err.details().contains("unsafe entry name"), "{err}");
    }

    #[test]
    fn missing_and_extra_entries_are_refused() {
        let good = write_project(&project(8, 4, 4)).unwrap();
        let e = entries(&good);
        let err = err_of(&zipped(&[(&e[0].0, &e[0].1)]));
        assert!(err.details().contains("missing entry"), "{err}");
        let err = err_of(&zipped(&[(&e[1].0, &e[1].1)]));
        assert!(err.details().contains("manifest.json"), "{err}");
        let err = err_of(&zipped(&[
            (&e[0].0, &e[0].1),
            (&e[1].0, &e[1].1),
            ("notes.txt", b"hi"),
        ]));
        assert!(err.details().contains("unexpected entry"), "{err}");
        let err = err_of(&zipped(&[
            (&e[0].0, &e[0].1),
            (&e[1].0, &e[1].1),
            ("dir/", b""),
        ]));
        assert!(err.details().contains("directory"), "{err}");
        let err = err_of(&with_manifest(|m| {
            m["frames"][0]["layers"] = serde_json::json!([]);
        }));
        assert!(err.details().contains("exactly one layer"), "{err}");
        let err = err_of(&with_manifest(|m| m["frames"] = serde_json::json!([])));
        assert!(err.details().contains("exactly one frame"), "{err}");
    }

    #[test]
    fn limits_are_enforced() {
        let mut p = project(64, 64, 4);
        p.spare = Some(sample_image(8, 8, 4));
        let bytes = write_project(&p).unwrap();
        let tiny_entries = Limits {
            max_entries: 2,
            ..Limits::default()
        };
        assert!(matches!(
            read_project_with(&bytes, tiny_entries),
            Err(ProjectError::TooLarge(_))
        ));
        let tiny_total = Limits {
            max_total_bytes: 100,
            ..Limits::default()
        };
        assert!(matches!(
            read_project_with(&bytes, tiny_total),
            Err(ProjectError::TooLarge(_))
        ));
        let tiny_manifest = Limits {
            max_manifest_bytes: 10,
            ..Limits::default()
        };
        assert!(matches!(
            read_project_with(&bytes, tiny_manifest),
            Err(ProjectError::TooLarge(_))
        ));
        let tiny_pixels = Limits {
            max_pixels: 100,
            ..Limits::default()
        };
        assert!(matches!(
            read_project_with(&bytes, tiny_pixels),
            Err(ProjectError::TooLarge(_))
        ));
        assert!(read_project_with(&bytes, Limits::default()).is_ok());
    }

    #[test]
    fn wrong_pngs_are_refused() {
        let good = write_project(&project(8, 4, 4)).unwrap();
        let e = entries(&good);
        let with_png = |data: &[u8]| zipped(&[(&e[0].0, &e[0].1), (&e[1].0, data)]);
        // Not a PNG at all.
        let err = err_of(&with_png(b"not a png at all, really not a png"));
        assert!(err.details().contains("not a PNG"), "{err}");
        // True-colour PNG.
        let mut rgb = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut rgb, 8, 4);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()
                .unwrap()
                .write_image_data(&[0; 8 * 4 * 3])
                .unwrap();
        }
        let err = err_of(&with_png(&rgb));
        assert!(err.details().contains("8-bit indexed"), "{err}");
        // 2-bit indexed PNG.
        let mut low = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut low, 8, 4);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Two);
            enc.set_palette(vec![0; 12]);
            enc.write_header()
                .unwrap()
                .write_image_data(&[0; 2 * 4])
                .unwrap();
        }
        let err = err_of(&with_png(&low));
        assert!(err.details().contains("8-bit indexed"), "{err}");
        // Right kind, wrong size.
        let other = write_png(&sample_image(9, 4, 4), &sample_palette(4)).unwrap();
        let err = err_of(&with_png(&other));
        assert!(err.details().contains("9x4"), "{err}");
        // Right size, other palette.
        let other = write_png(&sample_image(8, 4, 4), &sample_palette(5)).unwrap();
        let err = err_of(&with_png(&other));
        assert!(err.details().contains("PLTE"), "{err}");
        // Cut off.
        let cut = &e[1].1[..e[1].1.len() - 12];
        assert!(read_project(&with_png(cut)).is_err());
    }

    #[test]
    fn writer_refuses_pixels_outside_the_palette() {
        let doc =
            Document::from_picture(sample_image(8, 4, 4), sample_palette(2), Target::default());
        let e = write_project(&Project::new(doc, ProjectExtras::default())).unwrap_err();
        assert!(matches!(e, ProjectError::Write(_)), "{e}");
    }
}
