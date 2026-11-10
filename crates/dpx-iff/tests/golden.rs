//! Golden ILBM fixtures (ADR-17): tiny pictures assembled byte by byte from the spec in this
//! file, independent of the crate's writer and ByteRun1 packer. Tests assert exact pixels,
//! palettes and metadata for good files, and the exact notes or error texts for broken ones.
//!
//! A few fixtures are also checked in under `tests/data/` (for the CLI and other tools);
//! `checked_in_fixtures_match_their_builders` keeps them identical to the builders here.
//! After changing a builder on purpose, regenerate them with
//! `DPX_BLESS=1 cargo test -p dpx-iff --test golden`.

use dpx_core::Rgb8;
use dpx_iff::ilbm::{self, ErrorKind, Ilbm, IlbmError};
use dpx_iff::{ColorRange, Compression, Masking};

// ---- byte builders (spec only, no crate code) ----

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend((data.len() as u32).to_be_bytes());
    v.extend(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn form(ty: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
    let body = chunks.concat();
    let mut v = b"FORM".to_vec();
    v.extend(((body.len() + 4) as u32).to_be_bytes());
    v.extend(ty);
    v.extend(body);
    v
}

fn bmhd_data(w: u16, h: u16, planes: u8, masking: u8, compression: u8, tc: u16) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend(w.to_be_bytes());
    v.extend(h.to_be_bytes());
    v.extend([0, 0, 0, 0]);
    v.extend([planes, masking, compression, 0]);
    v.extend(tc.to_be_bytes());
    v.extend([10, 11]);
    v.extend(320i16.to_be_bytes());
    v.extend(200i16.to_be_bytes());
    v
}

fn bmhd(w: u16, h: u16, planes: u8, masking: u8, compression: u8, tc: u16) -> Vec<u8> {
    chunk(b"BMHD", &bmhd_data(w, h, planes, masking, compression, tc))
}

fn cmap(colors: &[Rgb8]) -> Vec<u8> {
    let d: Vec<u8> = colors.iter().flat_map(|c| [c.r, c.g, c.b]).collect();
    chunk(b"CMAP", &d)
}

fn crng(rate: u16, flags: u16, low: u8, high: u8) -> Vec<u8> {
    let mut d = vec![0, 0];
    d.extend(rate.to_be_bytes());
    d.extend(flags.to_be_bytes());
    d.extend([low, high]);
    chunk(b"CRNG", &d)
}

fn camg(mode: u32) -> Vec<u8> {
    chunk(b"CAMG", &mode.to_be_bytes())
}

/// Packs one row: repeats of two or more bytes as runs, everything else as literals.
fn pack(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let flush = |out: &mut Vec<u8>, lit: &mut Vec<u8>| {
        if !lit.is_empty() {
            out.push((lit.len() - 1) as u8);
            out.append(lit);
        }
    };
    let mut i = 0;
    while i < row.len() {
        let mut n = 1;
        while i + n < row.len() && row[i + n] == row[i] && n < 128 {
            n += 1;
        }
        if n >= 2 {
            flush(&mut out, &mut lit);
            out.push((257 - n) as u8);
            out.push(row[i]);
        } else {
            lit.push(row[i]);
            if lit.len() == 128 {
                flush(&mut out, &mut lit);
            }
        }
        i += n;
    }
    flush(&mut out, &mut lit);
    out
}

/// A test picture: pixels, palette and optional mask plane.
#[derive(Clone)]
struct Pic {
    w: usize,
    h: usize,
    planes: u8,
    pixels: Vec<u8>,
    palette: Vec<Rgb8>,
    mask: Option<Vec<bool>>,
}

fn palette(n: usize) -> Vec<Rgb8> {
    (0..n)
        .map(|i| Rgb8::new((i * 37) as u8, (i * 91 + 5) as u8, (255 - i % 256) as u8))
        .collect()
}

fn pic(w: usize, h: usize, planes: u8) -> Pic {
    let colors = 1usize << planes;
    let pixels = (0..h)
        .flat_map(|y| (0..w).map(move |x| ((x * 5 + y * 3 + x * y / 2) % colors) as u8))
        .collect();
    Pic {
        w,
        h,
        planes,
        pixels,
        palette: palette(colors),
        mask: None,
    }
}

/// The BODY rows in file order: per line plane 0..n-1, then the mask row.
fn rows(p: &Pic) -> Vec<Vec<u8>> {
    let rb = p.w.div_ceil(16) * 2;
    let mut out = Vec::new();
    for y in 0..p.h {
        for plane in 0..p.planes {
            let mut row = vec![0u8; rb];
            for x in 0..p.w {
                if p.pixels[y * p.w + x] & (1 << plane) != 0 {
                    row[x / 8] |= 0x80 >> (x % 8);
                }
            }
            out.push(row);
        }
        if let Some(m) = &p.mask {
            let mut row = vec![0u8; rb];
            for x in 0..p.w {
                if m[y * p.w + x] {
                    row[x / 8] |= 0x80 >> (x % 8);
                }
            }
            out.push(row);
        }
    }
    out
}

fn body_data(p: &Pic, compression: u8) -> Vec<u8> {
    rows(p)
        .iter()
        .flat_map(|r| if compression == 1 { pack(r) } else { r.clone() })
        .collect()
}

/// BMHD, CMAP, `extra` chunks, BODY.
fn file(p: &Pic, compression: u8, extra: &[Vec<u8>]) -> Vec<u8> {
    let masking = if p.mask.is_some() { 1 } else { 0 };
    let mut chunks = vec![
        bmhd(p.w as u16, p.h as u16, p.planes, masking, compression, 0),
        cmap(&p.palette),
    ];
    chunks.extend_from_slice(extra);
    chunks.push(chunk(b"BODY", &body_data(p, compression)));
    form(b"ILBM", &chunks)
}

fn set_u32(bytes: &mut [u8], at: usize, v: u32) {
    bytes[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

fn read(bytes: &[u8]) -> (Ilbm, Vec<String>) {
    ilbm::read_with_notes(bytes).unwrap_or_else(|e| panic!("{e}"))
}

fn error(bytes: &[u8]) -> IlbmError {
    match ilbm::read_with_notes(bytes) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

fn assert_picture(got: &Ilbm, want: &Pic) {
    assert_eq!(
        (got.image.width() as usize, got.image.height() as usize),
        (want.w, want.h)
    );
    assert_eq!(got.image.pixels(), &want.pixels[..], "pixels");
    assert_eq!(got.palette.colors(), &want.palette[..], "palette");
    assert_eq!(got.planes, want.planes);
    assert_eq!(got.mask, want.mask, "mask");
}

// ---- good files ----

#[test]
fn every_depth_with_odd_widths_both_compressions() {
    let widths = [1, 7, 13, 17, 31, 33, 47, 65];
    for planes in 1..=8u8 {
        let w = widths[usize::from(planes) - 1];
        let p = pic(w, 3, planes);
        // A 6-plane file needs a CAMG without HAM/EHB flags to mean 64 colours.
        let extra = if planes == 6 { vec![camg(0)] } else { vec![] };
        for compression in [0, 1] {
            let (got, notes) = read(&file(&p, compression, &extra));
            assert_picture(&got, &p);
            assert!(notes.is_empty(), "{planes} planes: {notes:?}");
            let want = [Compression::None, Compression::ByteRun1][usize::from(compression)];
            assert_eq!(got.compression, want);
            assert_eq!((got.x_aspect, got.y_aspect), (10, 11));
            assert_eq!((got.page_width, got.page_height), (320, 200));
        }
    }
}

#[test]
fn mask_plane_is_read_exactly() {
    let mut p = pic(20, 3, 3);
    p.mask = Some((0..60).map(|i| (i % 20 + i / 20) % 3 != 0).collect());
    for compression in [0, 1] {
        let (got, notes) = read(&file(&p, compression, &[]));
        assert_picture(&got, &p);
        assert_eq!(got.masking, Masking::HasMask);
        assert!(notes.is_empty());
    }
}

#[test]
fn colour_ranges_and_4bit_cmap() {
    let mut p = pic(16, 2, 5);
    // 4-bit values in the high nibble, as old OCS writers stored them.
    let stored: Vec<Rgb8> = (0..32u8)
        .map(|i| Rgb8::new((i % 16) << 4, (15 - i % 16) << 4, 0xA0))
        .collect();
    let bytes = {
        let mut f = file(
            &p,
            1,
            &[
                crng(8192, 1, 4, 11),
                crng(2560, 3, 20, 31),
                crng(0, 0, 0, 0),
            ],
        );
        // Replace the CMAP data (offset 40 + 8) with the 4-bit values.
        let d: Vec<u8> = stored.iter().flat_map(|c| [c.r, c.g, c.b]).collect();
        f[48..48 + 96].copy_from_slice(&d);
        f
    };
    p.palette = stored
        .iter()
        .map(|c| Rgb8::new(c.r | c.r >> 4, c.g | c.g >> 4, c.b | c.b >> 4))
        .collect();
    let (got, notes) = read(&bytes);
    assert!(notes.is_empty(), "{notes:?}");
    assert_picture(&got, &p);
    assert!(got.cmap_4bit_fixed);
    assert_eq!(got.palette.get(1), Some(Rgb8::new(0x11, 0xEE, 0xAA)));
    let r = |rate, flags, low, high| ColorRange {
        rate,
        flags,
        low,
        high,
    };
    assert_eq!(
        got.ranges,
        [r(8192, 1, 4, 11), r(2560, 3, 20, 31), r(0, 0, 0, 0)]
    );
    assert!((got.ranges[0].steps_per_second() - 30.0).abs() < 1e-9);
    assert!(got.ranges[1].active() && got.ranges[1].reverse());
}

#[test]
fn palette_only_file_needs_no_body() {
    // nPlanes 0 and no BODY: a bare palette. Its CAMG (HAM here) does not matter.
    let colors = palette(16);
    let bytes = form(
        b"ILBM",
        &[bmhd(320, 200, 0, 0, 0, 0), cmap(&colors), camg(0x800)],
    );
    let (got, notes) = read(&bytes);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(got.planes, 0);
    assert_eq!(got.palette.colors(), &colors[..]);
    assert!(got.image.pixels().iter().all(|&c| c == 0));

    // More than 256 entries cannot be one palette.
    let bytes = form(b"ILBM", &[bmhd(8, 8, 0, 0, 0, 0), cmap(&palette(300))]);
    let (got, notes) = read(&bytes);
    assert_eq!(got.palette.len(), 256);
    assert_eq!(
        notes,
        ["CMAP has 300 colours but a palette holds at most 256: colours 256-299 are not kept"]
    );
}

// ---- tolerated quirks: exact notes ----

#[test]
fn body_cut_by_the_end_of_the_file_is_filled_with_colour_0() {
    let p = pic(16, 4, 2);
    // BMHD at 12, CMAP at 40 (12 bytes), BODY at 60, data at 68 (16 bytes).
    let full = file(&p, 0, &[]);
    assert_eq!(full.len(), 84);
    let (got, notes) = read(&full[..68 + 9]);
    let mut want = p.clone();
    want.pixels[32..].fill(0);
    assert_picture(&got, &want);
    assert_eq!(
        notes,
        [
            "BODY ends in row 2 of 4, plane 0 (the file ends inside the BODY, 9 of 16 bytes): \
          the rest of the picture is colour 0"
        ]
    );

    // Nothing left of the BODY but its header.
    let (got, notes) = read(&full[..68]);
    assert!(got.image.pixels().iter().all(|&c| c == 0));
    assert_eq!(
        notes,
        [
            "BODY at offset 60 ($3C) holds no pixel data (the file ends inside the BODY, 0 of \
          16 bytes): the picture is colour 0"
        ]
    );
}

#[test]
fn short_byterun1_body_chunk_is_filled_with_colour_0() {
    let p = pic(16, 4, 2);
    let packed: Vec<u8> = rows(&p).iter().take(5).flat_map(|r| pack(r)).collect();
    let bytes = form(
        b"ILBM",
        &[
            bmhd(16, 4, 2, 0, 1, 0),
            cmap(&p.palette),
            chunk(b"BODY", &packed),
        ],
    );
    let (got, notes) = read(&bytes);
    // Rows 0 and 1 complete, row 2 has plane 0 only.
    let mut want = p.clone();
    for x in 0..16 {
        want.pixels[32 + x] &= 1;
    }
    want.pixels[48..].fill(0);
    assert_picture(&got, &want);
    assert_eq!(
        notes,
        [
            "BODY ends in row 2 of 4, plane 1 (the BODY chunk is too short): the rest of the \
          picture is colour 0"
        ]
    );
}

#[test]
fn container_quirks_are_notes() {
    // Odd BODY (one literal: 01 12 34) whose pad byte is missing at the end of the file.
    let mut p = pic(16, 1, 1);
    p.pixels = [0x12u8, 0x34]
        .iter()
        .flat_map(|b| (0..8).map(move |i| (b >> (7 - i)) & 1))
        .collect();
    let mut bytes = file(&p, 1, &[]);
    assert_eq!(body_data(&p, 1), [0x01, 0x12, 0x34]);
    bytes.pop();
    let (got, notes) = read(&bytes);
    assert_picture(&got, &p);
    assert_eq!(
        notes,
        ["the pad byte after the last chunk (BODY) is missing at the end of the file"]
    );

    // XMODEM-style padding after the FORM.
    let mut bytes = file(&p, 0, &[]);
    let end = bytes.len();
    bytes.extend([0x1A; 6]);
    let (_, notes) = read(&bytes);
    assert_eq!(
        notes,
        [format!(
            "6 bytes after the end of the FORM ignored (at offset {end} (${end:X}))"
        )]
    );

    // A FORM size that was never filled in.
    let mut bytes = file(&p, 0, &[]);
    set_u32(&mut bytes, 4, 0);
    let (got, notes) = read(&bytes);
    assert_picture(&got, &p);
    assert_eq!(
        notes,
        ["the FORM size (0 bytes) is too small for its chunks; the file is read to its end"]
    );
}

#[test]
fn cmap_quirks_are_notes() {
    let p = pic(8, 1, 5);
    let with_cmap = |colors: &[u8]| {
        form(
            b"ILBM",
            &[
                bmhd(8, 1, p.planes, 0, 0, 0),
                chunk(b"CMAP", colors),
                chunk(b"BODY", &body_data(&p, 0)),
            ],
        )
    };
    let bytes_of = |c: &[Rgb8]| -> Vec<u8> { c.iter().flat_map(|c| [c.r, c.g, c.b]).collect() };

    // 16 entries for 32 colours.
    let (got, notes) = read(&with_cmap(&bytes_of(&p.palette[..16])));
    let mut want = p.palette[..16].to_vec();
    want.resize(32, Rgb8::BLACK);
    assert_eq!(got.palette.colors(), &want[..]);
    assert_eq!(
        notes,
        ["CMAP has 16 colours for 32 (5 bitplanes): colours 16-31 are black"]
    );

    // 256 entries for 32 colours.
    let (got, notes) = read(&with_cmap(&bytes_of(&palette(256))));
    assert_eq!(got.palette.colors(), &palette(256)[..32]);
    assert_eq!(
        notes,
        ["CMAP has 256 colours but 5 bitplanes show 32: colours 32-255 are not kept"]
    );

    // 97 bytes: 32 colours and one stray byte.
    let mut d = bytes_of(&p.palette);
    d.push(0x77);
    let (got, notes) = read(&with_cmap(&d));
    assert_eq!(got.palette.colors(), &p.palette[..]);
    assert_eq!(
        notes,
        ["CMAP at offset 40 ($28) has 97 bytes, not a multiple of 3: the last byte is ignored"]
    );

    // An empty CMAP and none at all: grey ramps.
    let grey: Vec<Rgb8> = (0..32)
        .map(|i| {
            let v = (i * 255 / 31) as u8;
            Rgb8::new(v, v, v)
        })
        .collect();
    let (got, notes) = read(&with_cmap(&[]));
    assert_eq!(got.palette.colors(), &grey[..]);
    assert_eq!(
        notes,
        ["CMAP at offset 40 ($28) holds no complete colour: a grey palette is used"]
    );
    let no_cmap = form(
        b"ILBM",
        &[bmhd(8, 1, 5, 0, 0, 0), chunk(b"BODY", &body_data(&p, 0))],
    );
    let (got, notes) = read(&no_cmap);
    assert_eq!(got.palette.colors(), &grey[..]);
    assert_eq!(notes, ["no CMAP chunk: a grey palette is used"]);
}

#[test]
fn odd_cmap_without_its_pad_byte() {
    // 2 planes, 3 colours = 9 CMAP bytes, written without the pad byte.
    let p = pic(16, 1, 2);
    let mut cm = chunk(
        b"CMAP",
        &palette(3)
            .iter()
            .flat_map(|c| [c.r, c.g, c.b])
            .collect::<Vec<_>>(),
    );
    cm.pop();
    let bytes = form(
        b"ILBM",
        &[
            bmhd(16, 1, 2, 0, 0, 0),
            cm,
            chunk(b"BODY", &body_data(&p, 0)),
        ],
    );
    let (got, notes) = read(&bytes);
    assert_eq!(got.image.pixels(), &p.pixels[..]);
    let mut want = palette(3);
    want.push(Rgb8::BLACK);
    assert_eq!(got.palette.colors(), &want[..]);
    assert_eq!(
        notes,
        [
            "the pad byte after the odd-sized CMAP chunk at offset 40 ($28) is missing",
            "CMAP has 3 colours for 4 (2 bitplanes): colour 3 is black",
        ]
    );
}

#[test]
fn short_property_chunks_odd_ids_lasso_and_chunks_after_body() {
    let p = pic(16, 2, 2);
    // BMHD 12, CMAP 40 (12 bytes) → 60.
    let extra = [
        chunk(b"CAMG", &[0, 0]),                // 60, 10 bytes
        chunk(b"CRNG", &[0, 0, 0x20, 0, 0, 1]), // 70, 14 bytes
        chunk(b"GRAB", &[0, 5]),                // 84, 10 bytes
        chunk(&[0, 1, 2, 3], &[9, 9]),          // 94, 10 bytes
    ];
    let mut chunks = vec![bmhd(16, 2, 2, 3, 0, 3), cmap(&p.palette)];
    chunks.extend(extra);
    chunks.push(chunk(b"BODY", &body_data(&p, 0)));
    chunks.push(chunk(b"TINY", &[0; 4]));
    let (got, notes) = read(&form(b"ILBM", &chunks));
    assert_eq!(got.image.pixels(), &p.pixels[..]);
    assert_eq!(
        (got.camg, got.ranges.len(), got.grab),
        (None, 0, None),
        "short chunks are ignored"
    );
    assert!(got.other_chunks.is_empty(), "the odd id is not kept");
    assert_eq!(
        (got.masking, got.transparent_color),
        (Masking::TransparentColor, 3)
    );
    assert_eq!(
        notes,
        [
            "CAMG at offset 60 ($3C) has 2 bytes, 4 needed: ignored",
            "CRNG at offset 70 ($46) has 6 bytes, 8 needed: ignored",
            "GRAB at offset 84 ($54) has 2 bytes, 4 needed: ignored",
            "chunk with the invalid id 00 01 02 03 (\"????\") at offset 94 ($5E) skipped (2 bytes)",
            "chunk after the BODY not read: TINY",
            "lasso masking is read as transparent colour 3; the outline fill is not done",
        ]
    );
}

// ---- refused files: exact error texts ----

/// Every refused fixture with its exact text (first line, newline, details).
fn refused() -> Vec<(&'static str, Vec<u8>, ErrorKind, String)> {
    let p2 = pic(16, 2, 2);
    let mut v = Vec::new();
    let mut add = |name, bytes, kind, text: &str| v.push((name, bytes, kind, text.to_string()));

    add(
        "gif",
        b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec(),
        ErrorKind::Iff,
        "Not an IFF file\nIt starts with 47 49 46 38 (\"GIF8\") instead of FORM.",
    );
    add(
        "short",
        b"FORM\0\0".to_vec(),
        ErrorKind::Iff,
        "File too short for IFF\nThe file has 6 bytes; an IFF FORM header alone needs 12.",
    );
    add(
        "list",
        b"LIST\0\0\0\x04ILBM".to_vec(),
        ErrorKind::Iff,
        "IFF LIST files: not supported yet\nThe file is an IFF LIST (several FORMs in one \
         file). DeluxePainter reads a single FORM ILBM.",
    );
    add(
        "pbm",
        form(b"PBM ", &[bmhd(16, 2, 8, 0, 0, 0)]),
        ErrorKind::NotIlbm,
        "PBM pictures: not supported yet\nThis is a FORM PBM picture (chunky pixels, as \
         written by DeluxePaint on the PC). DeluxePainter reads FORM ILBM for now.",
    );
    add(
        "acbm",
        form(b"ACBM", &[bmhd(16, 2, 2, 0, 0, 0)]),
        ErrorKind::NotIlbm,
        "ACBM pictures: not supported yet\nThis is a FORM ACBM picture (contiguous \
         bitplanes, as written by AmigaBASIC programs). DeluxePainter reads FORM ILBM for now.",
    );
    add(
        "anim",
        form(b"ANIM", &[form(b"ILBM", &[bmhd(16, 2, 2, 0, 0, 0)])]),
        ErrorKind::NotIlbm,
        "ANIM files arrive in Phase 6\nThis is an IFF ANIM animation. DeluxePainter loads \
         animations from Phase 6 on; until then it reads single FORM ILBM pictures.",
    );
    add(
        "8svx",
        form(b"8SVX", &[chunk(b"VHDR", &[0; 20])]),
        ErrorKind::NotIlbm,
        "IFF FORM 8SVX: not a picture\nDeluxePainter loads FORM ILBM pictures; this file is \
         an IFF FORM 8SVX (a sound).",
    );
    add(
        "no-bmhd",
        form(
            b"ILBM",
            &[cmap(&p2.palette), chunk(b"BODY", &body_data(&p2, 0))],
        ),
        ErrorKind::MissingChunk,
        "No picture header (BMHD missing)\nThere is no BMHD chunk before the BODY at offset 32 \
         ($20). Chunks found: CMAP at 12, BODY at 32.",
    );
    let cut_in_cmap = {
        let f = file(&pic(8, 1, 5), 0, &[]);
        f[..48 + 50].to_vec()
    };
    add(
        "no-body",
        cut_in_cmap,
        ErrorKind::MissingChunk,
        "No pixel data (BODY missing)\nBMHD says 5 bitplanes, but there is no BODY chunk. The \
         file ends inside the CMAP chunk at offset 40 ($28) (50 of 96 bytes): it is truncated. \
         Chunks found: BMHD at 12, CMAP at 40.",
    );
    add(
        "bmhd-cut",
        file(&p2, 0, &[])[..30].to_vec(),
        ErrorKind::ShortChunk,
        "Picture header (BMHD) is cut off\nBMHD at offset 12 ($C) is cut off: the file ends \
         after 10 of its 20 bytes.",
    );
    add(
        "bmhd-short",
        form(
            b"ILBM",
            &[chunk(b"BMHD", &bmhd_data(16, 2, 2, 0, 0, 0)[..16])],
        ),
        ErrorKind::ShortChunk,
        "Picture header (BMHD) too short\nBMHD at offset 12 ($C) has 16 bytes, 20 needed.",
    );
    let with_bmhd = |b: Vec<u8>, extra: &[Vec<u8>]| {
        let mut chunks = vec![b, cmap(&p2.palette)];
        chunks.extend_from_slice(extra);
        chunks.push(chunk(b"BODY", &body_data(&p2, 0)));
        form(b"ILBM", &chunks)
    };
    add(
        "zero-width",
        with_bmhd(bmhd(0, 200, 2, 0, 0, 0), &[]),
        ErrorKind::BadHeader,
        "Bad picture size 0x200\nBMHD at offset 12 ($C) says 0x200 pixels; a picture needs at \
         least 1x1.",
    );
    add(
        "huge",
        with_bmhd(bmhd(65535, 65535, 2, 0, 0, 0), &[]),
        ErrorKind::BadHeader,
        "Picture too big: 65535x65535\nBMHD at offset 12 ($C) says 65535x65535 pixels; \
         DeluxePainter reads pictures up to 64 megapixels. The header may be corrupt.",
    );
    add(
        "12-planes",
        with_bmhd(bmhd(16, 2, 12, 0, 0, 0), &[]),
        ErrorKind::BadHeader,
        "Bad BMHD: 12 bitplanes\nBMHD at offset 12 ($C) says 12 bitplanes; ILBM pictures have 1 \
         to 8 (or 24 and 32 for true colour).",
    );
    add(
        "24-bit",
        with_bmhd(bmhd(16, 2, 24, 0, 0, 0), &[]),
        ErrorKind::Unsupported,
        "24-bit ILBM: not supported yet\nBMHD at offset 12 ($C) says 24 bitplanes: a \
         true-colour ILBM. DeluxePainter reads 1 to 8 bitplanes for now.",
    );
    add(
        "masking-7",
        with_bmhd(bmhd(16, 2, 2, 7, 0, 0), &[]),
        ErrorKind::BadHeader,
        "Unknown masking type 7\nBMHD at offset 12 ($C) says masking 7; ILBM knows 0 (none), 1 \
         (mask plane), 2 (transparent colour) and 3 (lasso).",
    );
    add(
        "compression-2",
        with_bmhd(bmhd(16, 2, 2, 0, 2, 0), &[]),
        ErrorKind::Unsupported,
        "Unknown compression type 2\nBMHD at offset 12 ($C) says compression 2; ILBM knows 0 \
         (none) and 1 (ByteRun1).",
    );
    add(
        "deep",
        with_bmhd(bmhd(16, 2, 2, 0, 0, 0), &[chunk(b"DEEP", &[0; 8])]),
        ErrorKind::Unsupported,
        "DEEP ILBM: not supported yet\nThe DEEP chunk at offset 60 ($3C) holds \
         true-colour pixels. DeluxePainter reads 1 to 8 bitplanes for now.",
    );
    add(
        "bad-run",
        bad_run(),
        ErrorKind::BadBody,
        "BODY: bad ByteRun1 run, row 1\nAt offset 65 ($41) a ByteRun1 repeat of 4 bytes \
         does not fit: plane 0 of row 1 has 2 of its 2 bytes left. The BODY is corrupt (or \
         packed across rows, which ILBM does not allow).",
    );
    v
}

/// HAM6 pixels (6 planes, CMAP of 16): control in bits 4-5. Row 0 sets colour 1, then modifies
/// blue, red, green; row 1 starts with a red modify from colour 0. The rest sets colour x.
fn ham6_pic() -> Pic {
    let mut p = pic(16, 2, 6);
    for (i, v) in p.pixels.iter_mut().enumerate() {
        *v = (i % 16) as u8;
    }
    p.pixels[..4].copy_from_slice(&[0x01, 0x1F, 0x2A, 0x30]);
    p.pixels[16] = 0x2F;
    p.palette = palette(16);
    p
}

/// HAM8 pixels (8 planes, CMAP of 64): control in bits 6-7, data in bits 0-5.
fn ham8_pic() -> Pic {
    let mut p = pic(16, 2, 8);
    for (i, v) in p.pixels.iter_mut().enumerate() {
        *v = (i % 64) as u8;
    }
    p.pixels[..4].copy_from_slice(&[0x02, 0x50, 0xA0, 0xC0]);
    p.pixels[16] = 0xBF;
    p.palette = palette(64);
    p
}

/// EHB: 6 planes, indices up to 63, CMAP of 32.
fn ehb_pic() -> Pic {
    let mut p = pic(16, 2, 6);
    p.pixels[0] = 63;
    p.pixels[1] = 32;
    p.palette = palette(32);
    p
}

/// 6-plane / HAM8 files for the modes the reader hands over as raw plane values.
fn special_files() -> Vec<(&'static str, Vec<u8>)> {
    let ehb = ehb_pic();
    vec![
        ("ham6", file(&ham6_pic(), 0, &[camg(0x800)])),
        ("ham8", file(&ham8_pic(), 0, &[camg(0x800)])),
        ("ehb", file(&ehb, 0, &[camg(0x80)])),
        // Without CAMG: 32 CMAP entries mean EHB, anything else HAM6.
        ("ehb-no-camg", file(&ehb, 0, &[])),
        ("ham6-no-camg", file(&ham6_pic(), 0, &[])),
    ]
}

#[test]
fn ham_and_ehb_files_are_read_as_raw_plane_values() {
    use dpx_iff::SpecialMode::{Ehb, Ham6, Ham8};
    let all = special_files();
    let get = |n: &str| all.iter().find(|f| f.0 == n).unwrap().1.clone();
    for (name, pic, mode, entries, note) in [
        ("ham6", ham6_pic(), Ham6, 16, None),
        ("ham8", ham8_pic(), Ham8, 64, None),
        ("ehb", ehb_pic(), Ehb, 32, None),
        (
            "ehb-no-camg",
            ehb_pic(),
            Ehb,
            32,
            Some("no CAMG chunk, 6 bitplanes and a 32-colour CMAP: read as Extra-Half-Brite"),
        ),
        (
            "ham6-no-camg",
            ham6_pic(),
            Ham6,
            16,
            Some(
                "no CAMG chunk and 6 bitplanes: read as HAM6, as the IFF spec says (it could \
                 also be a plain 64-colour picture)",
            ),
        ),
    ] {
        let (got, notes) = read(&get(name));
        assert_eq!(got.special, Some(mode), "{name}");
        assert_eq!(
            got.image.pixels(),
            &pic.pixels[..],
            "{name}: raw plane values"
        );
        assert_eq!(got.palette.colors(), &pic.palette[..entries], "{name}");
        assert_eq!(got.planes, pic.planes, "{name}");
        assert_eq!(
            notes,
            note.map(String::from).into_iter().collect::<Vec<_>>()
        );
    }
}

/// 16×2, 1 plane, ByteRun1: row 0 is fine, row 1 repeats 4 bytes into a 2-byte row.
/// BMHD 12, CMAP 40 (6 bytes), BODY 54, data at 62; the bad control byte is data byte 3.
fn bad_run() -> Vec<u8> {
    form(
        b"ILBM",
        &[
            bmhd(16, 2, 1, 0, 1, 0),
            cmap(&palette(2)),
            chunk(b"BODY", &[0x01, 0xAA, 0x55, 0xFD, 0x00]),
        ],
    )
}

#[test]
fn refused_files_have_exact_short_and_long_texts() {
    let all = refused();
    assert_eq!(all.len(), 19);
    for (name, bytes, kind, text) in all {
        let e = error(&bytes);
        assert_eq!(e.to_string(), text, "{name}");
        assert_eq!(e.kind, kind, "{name}");
        assert!(
            e.summary.chars().count() <= 34,
            "{name}: first line too long for the requester: {}",
            e.summary
        );
        assert!(!e.summary.contains('\n') && !e.details.is_empty(), "{name}");
    }
}

// ---- checked-in fixtures ----

/// The fixtures under `tests/data/`, each with its builder.
fn checked_in() -> Vec<(&'static str, Vec<u8>)> {
    let mut masked = pic(20, 3, 3);
    masked.mask = Some((0..60).map(|i| (i % 20 + i / 20) % 3 != 0).collect());
    let cut = file(&pic(16, 4, 2), 0, &[]);
    let refused = refused();
    let get = |name: &str| {
        refused
            .iter()
            .find(|r| r.0 == name)
            .map(|r| r.1.clone())
            .expect("fixture")
    };
    let special_files = special_files();
    let special = |name: &str| {
        special_files
            .iter()
            .find(|f| f.0 == name)
            .map(|f| f.1.clone())
            .expect("fixture")
    };
    vec![
        (
            "golden-8planes-byterun1-65x3.iff",
            file(&pic(65, 3, 8), 1, &[]),
        ),
        (
            "golden-5planes-crng-17x3.iff",
            file(&pic(17, 3, 5), 0, &[crng(8192, 1, 4, 11), camg(0)]),
        ),
        ("golden-mask-20x3.iff", file(&masked, 1, &[])),
        ("broken-body-cut.iff", cut[..68 + 9].to_vec()),
        ("broken-bad-run.iff", bad_run()),
        ("broken-no-body.iff", get("no-body")),
        ("ham6-16x2.iff", special("ham6")),
        ("ham8-16x2.iff", special("ham8")),
        ("ehb-16x2.iff", special("ehb")),
    ]
}

#[test]
fn checked_in_fixtures_match_their_builders() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let bless = std::env::var_os("DPX_BLESS").is_some();
    for (name, bytes) in checked_in() {
        let path = dir.join(name);
        if bless {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        }
        let on_disk = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(on_disk, bytes, "{name} differs from its builder");
    }
}
