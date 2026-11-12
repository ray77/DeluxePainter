//! Palette arrangement and remapping (ADR-31): Color ▸ Palette ▸ Arrange and Color ▸ Remap.
//!
//! Pure functions on palettes and index buffers; [`crate::Command::ArrangePalette`] applies
//! [`arranged_palette`] and [`reindex`] as one undoable step, [`crate::Command::RemapColors`]
//! applies [`remap_to_palette`] (every frame and layer, ADR-44).

use crate::image::IndexedImage;
use crate::palette::{Palette, Rgb8};

/// Whether `order` is a permutation of `0..n` (and `n` fits the colour indices).
pub fn is_permutation(order: &[u8], n: usize) -> bool {
    if order.len() != n || n > 256 {
        return false;
    }
    let mut seen = [false; 256];
    for &o in order {
        let o = usize::from(o);
        if o >= n || std::mem::replace(&mut seen[o], true) {
            return false;
        }
    }
    true
}

/// The identity order `0..n`.
pub fn identity(n: usize) -> Vec<u8> {
    (0..n.min(256)).map(|i| i as u8).collect()
}

/// The palette rearranged: entry `i` of the result is entry `order[i]` of `palette`.
pub fn arranged_palette(palette: &[Rgb8], order: &[u8]) -> Vec<Rgb8> {
    order.iter().map(|&o| palette[usize::from(o)]).collect()
}

/// `lut[old] = new`: where the colour that was at `old` sits after arranging with `order`.
pub fn inverse(order: &[u8]) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = i as u8;
    }
    for (new, &old) in order.iter().enumerate() {
        lut[usize::from(old)] = new as u8;
    }
    lut
}

/// Pixels renumbered for a palette arranged with `order`: every pixel keeps its colour.
pub fn reindex(pixels: &[u8], order: &[u8]) -> Vec<u8> {
    let lut = inverse(order);
    pixels.iter().map(|&p| lut[usize::from(p)]).collect()
}

/// Moves the entry at position `from` to position `to`; the entries in between shift by one.
pub fn move_entry(order: &mut [u8], from: usize, to: usize) {
    if from >= order.len() || to >= order.len() || from == to {
        return;
    }
    if from < to {
        order[from..=to].rotate_left(1);
    } else {
        order[to..=from].rotate_right(1);
    }
}

/// Exchanges the entries at positions `a` and `b` (the EX button).
pub fn swap_entries(order: &mut [u8], a: usize, b: usize) {
    if a < order.len() && b < order.len() {
        order.swap(a, b);
    }
}

/// `lut[i]` = the index in `to` nearest to colour `i` of `from` (an identical colour wins;
/// of several equally near entries the lowest). Entries beyond `from` map to themselves.
pub fn nearest_lut(from: &[Rgb8], to: &Palette) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        *v = from.get(i).map_or(i as u8, |&c| to.nearest(c));
    }
    lut
}

/// [`nearest_lut`] for a layer above the bottom one (ADR-44), where colour 0 is
/// transparent: 0 stays 0, every other colour goes to the nearest entry but 0 (so nothing
/// opaque turns transparent).
pub fn nearest_lut_upper(from: &[Rgb8], to: &Palette) -> [u8; 256] {
    let mut lut = [0u8; 256];
    for (i, v) in lut.iter_mut().enumerate().skip(1) {
        *v = from
            .get(i)
            .map_or(i as u8, |&c| to.nearest_excluding(c, &[0]));
    }
    lut
}

/// Color ▸ Remap: the page repainted with the colours of `from` (the palette its pixels were
/// drawn for) looked up in `to` (the current palette), nearest colour.
pub fn remap_to_palette(page: &IndexedImage, from: &[Rgb8], to: &Palette) -> IndexedImage {
    remap_with(page, &nearest_lut(from, to))
}

/// `page` with every pixel `p` replaced by `lut[p]`.
pub fn remap_with(page: &IndexedImage, lut: &[u8; 256]) -> IndexedImage {
    let pixels = page.pixels().iter().map(|&p| lut[usize::from(p)]).collect();
    IndexedImage::from_pixels(page.width(), page.height(), pixels).expect("same size")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Vec<Rgb8> {
        vec![
            Rgb8::new(0, 0, 0),
            Rgb8::new(255, 0, 0),
            Rgb8::new(0, 255, 0),
            Rgb8::new(0, 0, 255),
        ]
    }

    #[test]
    fn permutation_check() {
        assert!(is_permutation(&[2, 0, 1], 3));
        assert!(!is_permutation(&[2, 0, 0], 3));
        assert!(!is_permutation(&[0, 1], 3));
        assert!(!is_permutation(&[0, 1, 3], 3));
        assert!(is_permutation(&identity(256), 256));
    }

    #[test]
    fn arranging_keeps_the_picture_looking_the_same() {
        let order = [3u8, 1, 0, 2];
        let new = arranged_palette(&pal(), &order);
        assert_eq!(new[0], Rgb8::new(0, 0, 255));
        let pixels = [0u8, 1, 2, 3, 3, 2];
        let moved = reindex(&pixels, &order);
        for (old, new_i) in pixels.iter().zip(&moved) {
            assert_eq!(pal()[usize::from(*old)], new[usize::from(*new_i)]);
        }
    }

    #[test]
    fn move_and_swap() {
        let mut o = identity(5);
        move_entry(&mut o, 1, 3);
        assert_eq!(o, [0, 2, 3, 1, 4]);
        move_entry(&mut o, 4, 0);
        assert_eq!(o, [4, 0, 2, 3, 1]);
        swap_entries(&mut o, 0, 4);
        assert_eq!(o, [1, 0, 2, 3, 4]);
        move_entry(&mut o, 9, 0);
        assert_eq!(o, [1, 0, 2, 3, 4], "out of range does nothing");
    }

    #[test]
    fn remap_uses_the_nearest_current_colour() {
        let old = pal();
        // The current palette is the old one with red and blue exchanged and green dimmed.
        let cur = Palette::new(vec![
            Rgb8::new(0, 0, 0),
            Rgb8::new(0, 0, 255),
            Rgb8::new(0, 200, 0),
            Rgb8::new(255, 0, 0),
        ])
        .unwrap();
        let page = IndexedImage::from_pixels(4, 1, vec![0, 1, 2, 3]).unwrap();
        let out = remap_to_palette(&page, &old, &cur);
        assert_eq!(out.pixels(), &[0, 3, 2, 1]);
    }

    #[test]
    fn remap_is_the_identity_for_an_unchanged_palette() {
        let p = Palette::new(pal()).unwrap();
        let page = IndexedImage::from_pixels(3, 1, vec![3, 1, 0]).unwrap();
        assert_eq!(remap_to_palette(&page, &pal(), &p), page);
    }

    #[test]
    fn arrange_command_is_one_undoable_step_that_keeps_the_picture() {
        use crate::{Command, Document, Engine, Target};
        let mut colors = Palette::dpaint_default_32().colors().to_vec();
        colors[1] = Rgb8::new(10, 20, 30);
        let mut page = IndexedImage::new(8, 4, 0);
        page.set(1, 1, 1);
        page.set(2, 2, 7);
        page.set(3, 3, 31);
        let doc = Document::from_picture(page, Palette::new(colors).unwrap(), Target::default());
        let before = doc.clone();
        let mut e = Engine::new(doc);
        let mut order = identity(32);
        move_entry(&mut order, 1, 20);
        swap_entries(&mut order, 0, 31);
        let out = e
            .execute(Command::ArrangePalette {
                order: order.clone(),
            })
            .unwrap();
        assert!(out.palette_changed);
        let d = e.document();
        // The picture looks the same: every pixel's RGB is unchanged.
        for y in 0..4 {
            for x in 0..8 {
                let old = before.palette().get(before.page().get(x, y).unwrap());
                let new = d.palette().get(d.page().get(x, y).unwrap());
                assert_eq!(old, new, "pixel {x},{y}");
            }
        }
        assert_eq!(d.palette().get(20), Some(Rgb8::new(10, 20, 30)));
        assert_eq!(d.page().get(1, 1), Some(20));
        e.undo().unwrap();
        assert_eq!(e.document().page(), before.page());
        assert_eq!(e.document().palette(), before.palette());
        e.redo().unwrap();
        assert_eq!(e.document().page().get(1, 1), Some(20));
    }

    #[test]
    fn arrange_command_rejects_a_bad_order() {
        use crate::{Command, Document, Engine, ErrorCode};
        let mut e = Engine::new(Document::new(4, 4, Palette::dpaint_default_32()));
        for order in [vec![0u8; 32], identity(31), vec![32u8; 32]] {
            let err = e.execute(Command::ArrangePalette { order }).unwrap_err();
            assert_eq!(err.code, ErrorCode::PaletteRangeInvalid);
        }
        assert!(!e.can_undo());
    }
}
