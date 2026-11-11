//! Area stencils (ADR-39): Effect ▸ Stencil ▸ Paint and Effect ▸ Background ▸ Lock FG protect
//! page pixels by their position, not by their colour (the colour stencil of ADR-20 is the
//! 256-entry table every stroke carries).
//!
//! An [`AreaStencil`] is a page-sized mask. Painting commands honour it when they are wrapped
//! in [`crate::Command::Stenciled`]: strokes and shapes skip protected pixels as they paint
//! (the same test as the colour stencil, [`crate::paint::plot`]), fills, CLR and a restored
//! fixed background put protected pixels back afterwards. While Stencil Paint is on, the
//! drawing tools paint the mask itself: [`mask_command`] turns their command into one that
//! paints a two-colour mask page ([`AreaStencil::to_image`]).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::command::{Command, MAX_PAGE_SIDE};
use crate::gradient::FillType;
use crate::image::IndexedImage;
use crate::paint::{BrushImage, DrawMode, PaintFx};
use crate::palette::{Palette, Rgb8};

/// Colour of free pixels in a mask page ([`AreaStencil::to_image`]).
pub const FREE: u8 = 0;
/// Colour of protected pixels in a mask page.
pub const PROTECTED: u8 = 1;

/// Page pixels protected by position, row by row (`true` = protected). Clones share the mask.
///
/// In JSON the mask is run lengths: `{"width":w,"height":h,"runs":[free, protected, free,
/// …]}`, alternating and starting with free pixels (the first run may be 0); the runs add up
/// to `width * height`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AreaFile", into = "AreaFile")]
pub struct AreaStencil {
    width: u32,
    height: u32,
    mask: Arc<Vec<bool>>,
}

impl fmt::Debug for AreaStencil {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The mask can be a megabyte; its size and count say enough.
        f.debug_struct("AreaStencil")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("protected", &self.count())
            .finish()
    }
}

/// The JSON form of an [`AreaStencil`].
#[derive(Serialize, Deserialize)]
struct AreaFile {
    width: u32,
    height: u32,
    runs: Vec<u32>,
}

impl From<AreaStencil> for AreaFile {
    fn from(a: AreaStencil) -> Self {
        let mut runs = Vec::new();
        let mut current = false;
        let mut n = 0u32;
        for &p in a.mask.iter() {
            if p != current {
                runs.push(n);
                current = p;
                n = 0;
            }
            n += 1;
        }
        runs.push(n);
        AreaFile {
            width: a.width,
            height: a.height,
            runs,
        }
    }
}

impl TryFrom<AreaFile> for AreaStencil {
    type Error = String;

    fn try_from(f: AreaFile) -> Result<Self, String> {
        if !(1..=MAX_PAGE_SIDE).contains(&f.width) || !(1..=MAX_PAGE_SIDE).contains(&f.height) {
            return Err(format!(
                "area stencil {}x{} is outside 1..={MAX_PAGE_SIDE}",
                f.width, f.height
            ));
        }
        let len = f.width as u64 * f.height as u64;
        let total: u64 = f.runs.iter().map(|&r| u64::from(r)).sum();
        if total != len {
            return Err(format!(
                "area stencil runs cover {total} pixels, the {}x{} mask has {len}",
                f.width, f.height
            ));
        }
        let mut mask = Vec::with_capacity(len as usize);
        for (i, &r) in f.runs.iter().enumerate() {
            mask.extend(std::iter::repeat_n(i % 2 == 1, r as usize));
        }
        Ok(AreaStencil {
            width: f.width,
            height: f.height,
            mask: Arc::new(mask),
        })
    }
}

impl AreaStencil {
    /// A mask of `width`×`height` pixels; `None` unless `mask` has one entry per pixel and the
    /// size is a page size (1..=[`MAX_PAGE_SIDE`]).
    pub fn from_mask(width: u32, height: u32, mask: Vec<bool>) -> Option<Self> {
        let ok = (1..=MAX_PAGE_SIDE).contains(&width)
            && (1..=MAX_PAGE_SIDE).contains(&height)
            && mask.len() as u64 == width as u64 * height as u64;
        ok.then(|| Self {
            width,
            height,
            mask: Arc::new(mask),
        })
    }

    /// Nothing protected, `page`'s size.
    pub fn free(page: &IndexedImage) -> Self {
        Self::from_image(&IndexedImage::new(page.width(), page.height(), FREE))
    }

    /// The pixels of a mask page that are not [`FREE`].
    pub fn from_image(img: &IndexedImage) -> Self {
        Self {
            width: img.width(),
            height: img.height(),
            mask: Arc::new(img.pixels().iter().map(|&p| p != FREE).collect()),
        }
    }

    /// The mask as a page: [`PROTECTED`] and [`FREE`] pixels.
    pub fn to_image(&self) -> IndexedImage {
        let pixels = self
            .mask
            .iter()
            .map(|&p| if p { PROTECTED } else { FREE })
            .collect();
        IndexedImage::from_pixels(self.width, self.height, pixels).expect("one entry per pixel")
    }

    /// The pixels of `page` whose colour the colour stencil `table` protects (Stencil ▸ Paint
    /// starts from the colours of the Make Stencil requester, AGA p.120).
    pub fn from_colors(page: &IndexedImage, table: &[bool]) -> Self {
        Self {
            width: page.width(),
            height: page.height(),
            mask: Arc::new(
                page.pixels()
                    .iter()
                    .map(|&c| table.get(usize::from(c)).copied().unwrap_or(false))
                    .collect(),
            ),
        }
    }

    /// Background ▸ Lock FG: the pixels of `page` that differ from the fixed `background`,
    /// whatever their colour (AGA p.275). `None` if the sizes differ.
    pub fn from_changes(page: &IndexedImage, background: &IndexedImage) -> Option<Self> {
        if (page.width(), page.height()) != (background.width(), background.height()) {
            return None;
        }
        Some(Self {
            width: page.width(),
            height: page.height(),
            mask: Arc::new(
                page.pixels()
                    .iter()
                    .zip(background.pixels())
                    .map(|(a, b)| a != b)
                    .collect(),
            ),
        })
    }

    /// Stencil ▸ Reverse: every free pixel protected and the other way round.
    pub fn reversed(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            mask: Arc::new(self.mask.iter().map(|&p| !p).collect()),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether the mask has `page`'s size.
    pub fn fits(&self, page: &IndexedImage) -> bool {
        (self.width, self.height) == (page.width(), page.height())
    }

    /// Whether page pixel (`x`, `y`) is protected; pixels outside the mask are not.
    pub fn protects(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return false;
        }
        self.mask[y as usize * self.width as usize + x as usize]
    }

    /// Number of protected pixels.
    pub fn count(&self) -> usize {
        self.mask.iter().filter(|&&p| p).count()
    }

    /// Nothing is protected.
    pub fn is_empty(&self) -> bool {
        !self.mask.iter().any(|&p| p)
    }

    /// Puts `before`'s colour back into every protected pixel of `img` (both of the mask's
    /// size; other sizes are left alone).
    pub fn restore(&self, img: &mut IndexedImage, before: &IndexedImage) {
        if !self.fits(img) || !self.fits(before) {
            return;
        }
        let pixels: Vec<u8> = img
            .pixels()
            .iter()
            .zip(before.pixels())
            .zip(self.mask.iter())
            .map(|((&now, &old), &p)| if p { old } else { now })
            .collect();
        *img = IndexedImage::from_pixels(self.width, self.height, pixels).expect("same size");
    }
}

/// The colour that shows `palette` entry `c` at half brightness: the nearest palette entry.
/// Entries outside the palette map to themselves.
pub fn half_bright_table(palette: &Palette) -> Vec<u8> {
    (0..=255u8)
        .map(|c| match palette.get(c) {
            Some(col) => palette.nearest(Rgb8::new(col.r / 2, col.g / 2, col.b / 2)),
            None => c,
        })
        .collect()
}

/// How an area stencil is shown (Stencil ▸ Paint and ▸ Show, AGA p.120, p.271): the picture
/// dimmed, the protected pixels at full brightness. With one palette for the whole screen,
/// dimming takes the nearest palette colour at half brightness ([`half_bright_table`]).
/// Display only.
pub fn view(page: &IndexedImage, area: &AreaStencil, palette: &Palette) -> IndexedImage {
    let dim = half_bright_table(palette);
    let mut out = page.clone();
    for y in 0..page.height() as i32 {
        for x in 0..page.width() as i32 {
            if !area.protects(x, y)
                && let Some(c) = page.get(x, y)
            {
                out.set(x, y, dim[usize::from(c)]);
            }
        }
    }
    out
}

/// `b` as one colour: opaque pixels `value`, transparent ones the other mask colour.
fn silhouette(b: &BrushImage, value: u8) -> BrushImage {
    let other = if value == PROTECTED { FREE } else { PROTECTED };
    BrushImage {
        pixels: b
            .pixels
            .iter()
            .map(|&p| {
                if Some(p) == b.transparent {
                    other
                } else {
                    value
                }
            })
            .collect(),
        transparent: b.transparent.map(|_| other),
        ..b.clone()
    }
}

/// Stencil ▸ Paint (ADR-39): `cmd` as a command that paints a mask page (two colours,
/// [`AreaStencil::to_image`]) instead of the picture. With `add` the pixels it paints become
/// protected, otherwise free. Strokes and shapes keep their path, brush shape, filled flag
/// and symmetry and paint plain (no mode, range, stencil or effect); a custom brush paints
/// its shape. A fill fills the mask's own region at its point, CLR the whole mask (with
/// `add` = false: everything free). A [`Command::Stenciled`] paints its command. `None` for
/// commands that do not paint (they do not belong to the stencil).
pub fn mask_command(cmd: &Command, add: bool) -> Option<Command> {
    let value = if add { PROTECTED } else { FREE };
    Some(match cmd {
        Command::Stroke {
            brush,
            points,
            continuous,
            custom,
            ..
        } => Command::Stroke {
            brush: brush.clone(),
            color: value,
            points: points.clone(),
            continuous: *continuous,
            custom: custom.as_ref().map(|b| silhouette(b, value)),
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: PaintFx::default(),
        },
        Command::Shape {
            shape,
            brush,
            filled,
            symmetry,
            custom,
            ..
        } => Command::Shape {
            shape: shape.clone(),
            brush: brush.clone(),
            color: value,
            filled: *filled,
            symmetry: *symmetry,
            custom: custom.as_ref().map(|b| silhouette(b, value)),
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: PaintFx::default(),
        },
        Command::FloodFill { x, y, .. } => Command::FloodFill {
            x: *x,
            y: *y,
            color: value,
            fill: FillType::Solid,
        },
        Command::Clear { .. } => Command::Clear { color: value },
        Command::Stenciled { command, .. } => return mask_command(command, add),
        _ => return None,
    })
}

/// The palette of a mask page: free black, protected white.
pub fn mask_palette() -> Palette {
    Palette::new(vec![Rgb8::BLACK, Rgb8::new(255, 255, 255)]).expect("two colours")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Shape;
    use crate::{Document, Engine, Target};

    fn mask(rows: &[&str]) -> AreaStencil {
        let w = rows[0].len() as u32;
        let bits = rows
            .iter()
            .flat_map(|r| r.chars().map(|c| c == '#'))
            .collect();
        AreaStencil::from_mask(w, rows.len() as u32, bits).unwrap()
    }

    fn rows(img: &IndexedImage) -> Vec<String> {
        (0..img.height() as i32)
            .map(|y| {
                (0..img.width() as i32)
                    .map(|x| char::from_digit(u32::from(img.get(x, y).unwrap()), 36).unwrap())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn json_is_run_lengths_and_round_trips() {
        let a = mask(&["..##", "#..."]);
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(json, r#"{"width":4,"height":2,"runs":[2,3,3]}"#);
        let back: AreaStencil = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        // A mask starting with a protected pixel has an empty first run.
        let b = mask(&["#."]);
        assert_eq!(
            serde_json::to_string(&b).unwrap(),
            r#"{"width":2,"height":1,"runs":[0,1,1]}"#
        );
        for bad in [
            r#"{"width":4,"height":2,"runs":[2,3]}"#,
            r#"{"width":4,"height":2,"runs":[8,1]}"#,
            r#"{"width":0,"height":2,"runs":[]}"#,
            r#"{"width":5000,"height":1,"runs":[5000]}"#,
        ] {
            assert!(serde_json::from_str::<AreaStencil>(bad).is_err(), "{bad}");
        }
        assert!(
            format!("{a:?}").contains("protected: 3"),
            "short debug form"
        );
    }

    #[test]
    fn masks_from_colours_changes_and_images() {
        let page = IndexedImage::from_pixels(3, 2, vec![1, 2, 3, 3, 2, 1]).unwrap();
        let mut table = vec![false; 256];
        table[3] = true;
        assert_eq!(
            AreaStencil::from_colors(&page, &table),
            mask(&["..#", "#.."])
        );
        let bg = IndexedImage::from_pixels(3, 2, vec![1, 0, 3, 3, 0, 0]).unwrap();
        let fg = AreaStencil::from_changes(&page, &bg).unwrap();
        assert_eq!(
            fg,
            mask(&[".#.", ".##"]),
            "changed pixels, whatever their colour"
        );
        assert!(AreaStencil::from_changes(&page, &IndexedImage::new(2, 2, 0)).is_none());
        assert_eq!(fg.reversed(), mask(&["#.#", "#.."]));
        assert_eq!(AreaStencil::from_image(&fg.to_image()), fg);
        assert_eq!(rows(&fg.to_image()), ["010", "011"]);
        assert_eq!(fg.count(), 3);
        assert!(!fg.is_empty() && AreaStencil::free(&page).is_empty());
        assert!(fg.protects(1, 0) && !fg.protects(0, 0) && !fg.protects(9, 0));
        assert!(!fg.protects(-1, 1));
    }

    #[test]
    fn restore_puts_protected_pixels_back() {
        let a = mask(&["#.", ".#"]);
        let before = IndexedImage::new(2, 2, 7);
        let mut img = IndexedImage::new(2, 2, 3);
        a.restore(&mut img, &before);
        assert_eq!(rows(&img), ["73", "37"]);
    }

    #[test]
    fn view_dims_free_pixels_only() {
        let pal = Palette::new(vec![
            Rgb8::BLACK,
            Rgb8::new(200, 200, 200),
            Rgb8::new(100, 100, 100),
            Rgb8::new(255, 0, 0),
            Rgb8::new(50, 50, 50),
        ])
        .unwrap();
        let table = half_bright_table(&pal);
        assert_eq!(&table[..4], &[0, 2, 4, 4]);
        assert_eq!(table[200], 200, "outside the palette");
        let page = IndexedImage::from_pixels(2, 1, vec![1, 1]).unwrap();
        let shown = view(&page, &mask(&["#."]), &pal);
        assert_eq!(rows(&shown), ["12"], "protected bright, free dimmed");
    }

    fn stroke(color: u8, right: bool) -> Command {
        Command::Stroke {
            brush: vec![(0, 0), (1, 0)],
            color,
            points: vec![(0, 0), (0, 2)],
            continuous: true,
            custom: None,
            mode: DrawMode::Blend,
            range: Some((1, 4)),
            stencil: Some(vec![true; 256]),
            right,
            fx: PaintFx {
                translucency: Some(50),
                ..Default::default()
            },
        }
    }

    /// A two-pixel-wide vertical stroke in Matte, without effects.
    fn plain(color: u8) -> Command {
        Command::Stroke {
            brush: vec![(0, 0), (1, 0)],
            color,
            points: vec![(0, 0), (0, 2)],
            continuous: true,
            custom: None,
            mode: DrawMode::Matte,
            range: None,
            stencil: None,
            right: false,
            fx: PaintFx::default(),
        }
    }

    #[test]
    fn mask_commands_paint_plain_on_the_mask() {
        let mut page = IndexedImage::new(4, 3, FREE);
        let pal = mask_palette();
        let add = mask_command(&stroke(17, true), true).unwrap();
        let before = page.clone();
        assert!(add.check_paint(&page, &pal).is_ok());
        add.paint(&mut page, &before, &pal);
        assert_eq!(rows(&page), ["1100", "1100", "1100"]);
        // A custom brush paints its shape; its transparent pixels stay as they are.
        let custom = BrushImage {
            width: 2,
            height: 1,
            pixels: vec![5, 9],
            handle: (0, 0),
            transparent: Some(9),
        };
        let mut cmd = stroke(3, false);
        if let Command::Stroke { custom: c, .. } = &mut cmd {
            *c = Some(custom);
        }
        let free = mask_command(&cmd, false).unwrap();
        let before = page.clone();
        free.paint(&mut page, &before, &pal);
        assert_eq!(rows(&page), ["0100", "0100", "0100"]);
        // Fill: the mask's region; CLR: the whole mask.
        let fill = Command::FloodFill {
            x: 3,
            y: 0,
            color: 6,
            fill: FillType::Solid,
        };
        let before = page.clone();
        mask_command(&fill, true)
            .unwrap()
            .paint(&mut page, &before, &pal);
        assert_eq!(rows(&page), ["0111", "0111", "0111"]);
        assert_eq!(
            mask_command(&Command::Clear { color: 4 }, false),
            Some(Command::Clear { color: FREE })
        );
        let shape = Command::Shape {
            shape: Shape::Rect {
                from: (0, 0),
                to: (1, 1),
            },
            brush: Vec::new(),
            color: 9,
            filled: true,
            symmetry: None,
            custom: None,
            mode: DrawMode::Smear,
            range: None,
            stencil: None,
            right: false,
            fx: PaintFx::default(),
        };
        let wrapped = Command::Stenciled {
            area: AreaStencil::free(&page),
            command: Box::new(shape),
        };
        match mask_command(&wrapped, true) {
            Some(Command::Shape {
                color,
                mode,
                filled,
                ..
            }) => assert_eq!((color, mode, filled), (PROTECTED, DrawMode::Matte, true)),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            mask_command(
                &Command::Flip {
                    axis: crate::Axis::Horizontal
                },
                true
            ),
            None
        );
    }

    fn engine(w: u32, h: u32, fill: u8) -> Engine {
        Engine::new(Document::from_picture(
            IndexedImage::new(w, h, fill),
            Palette::dpaint_default_32(),
            Target::default(),
        ))
    }

    fn stenciled(area: &AreaStencil, command: Command) -> Command {
        Command::Stenciled {
            area: area.clone(),
            command: Box::new(command),
        }
    }

    #[test]
    fn stenciled_commands_spare_protected_pixels_and_undo_in_one_step() {
        let area = mask(&["##..", "....", "...."]);
        let mut e = engine(4, 3, 0);
        e.execute(stenciled(&area, plain(5))).unwrap();
        assert_eq!(rows(e.document().page()), ["0000", "5500", "5500"]);
        // Matches the preview the tools paint with the same command.
        let mut preview = IndexedImage::new(4, 3, 0);
        let before = preview.clone();
        stenciled(&area, plain(5)).paint(&mut preview, &before, e.document().palette());
        assert_eq!(&preview, e.document().page());
        // CLR and fills keep the protected pixels too.
        e.execute(Command::Clear { color: 2 }).unwrap();
        e.execute(stenciled(&area, Command::Clear { color: 9 }))
            .unwrap();
        assert_eq!(rows(e.document().page()), ["2299", "9999", "9999"]);
        e.execute(stenciled(
            &area,
            Command::FloodFill {
                x: 3,
                y: 2,
                color: 2,
                fill: FillType::Solid,
            },
        ))
        .unwrap();
        assert_eq!(rows(e.document().page()), ["2222", "2222", "2222"]);
        e.undo().unwrap();
        assert_eq!(rows(e.document().page()), ["2299", "9999", "9999"]);
        // A fixed background restored by CLR (a SetPage of the page's size).
        let page = IndexedImage::new(4, 3, 7);
        e.execute(stenciled(
            &area,
            Command::SetPage {
                width: 4,
                height: 3,
                pixels: page.pixels().to_vec(),
            },
        ))
        .unwrap();
        assert_eq!(rows(e.document().page()), ["2277", "7777", "7777"]);
        // Redo replays the stencil exactly.
        e.undo().unwrap();
        e.redo().unwrap();
        assert_eq!(rows(e.document().page()), ["2277", "7777", "7777"]);
    }

    #[test]
    fn stenciled_commands_are_validated() {
        let mut e = engine(4, 3, 0);
        let small = mask(&["#."]);
        let err = e
            .execute(stenciled(&small, Command::Clear { color: 1 }))
            .unwrap_err();
        assert!(err.message.contains("2x1"), "{}", err.message);
        let area = AreaStencil::free(e.document().page());
        for bad in [
            Command::Flip {
                axis: crate::Axis::Vertical,
            },
            Command::SetPalette {
                colors: Palette::dpaint_default_32().colors().to_vec(),
            },
            Command::SetPage {
                width: 320,
                height: 256,
                pixels: vec![0; 320 * 256],
            },
            stenciled(&area, Command::Clear { color: 1 }),
            Command::Clear { color: 40 },
        ] {
            let before = e.document().clone();
            assert!(e.execute(stenciled(&area, bad.clone())).is_err(), "{bad:?}");
            assert_eq!(e.document(), &before);
        }
        // The tools' check: like the inner command's, plus the size.
        let pal = Palette::dpaint_default_32();
        let page = IndexedImage::new(4, 3, 0);
        assert!(stenciled(&area, plain(5)).check_paint(&page, &pal).is_ok());
        assert!(
            stenciled(&small, plain(5))
                .check_paint(&page, &pal)
                .is_err()
        );
        assert!(
            stenciled(&area, plain(50))
                .check_paint(&page, &pal)
                .is_err()
        );
    }

    #[test]
    fn stenciled_command_json_round_trips() {
        let cmd = stenciled(&mask(&["#.#."]), Command::Clear { color: 3 });
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"op":"stenciled","area":{"width":4,"height":1,"runs":[0,1,1,1,1]},"command":{"op":"clear","color":3}}"#
        );
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), cmd);
    }
}
