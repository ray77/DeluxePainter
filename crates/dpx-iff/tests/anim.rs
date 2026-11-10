//! ANIM codec tests (ADR-29): round trips of generated animations, a hand-built file that
//! does not use the crate's writer, refusals with their exact texts.

use dpx_core::{IndexedImage, Palette, Rgb8};
use dpx_iff::anim::{self, Anhd, EncodeOptions, ErrorKind, Hold};

fn img(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> IndexedImage {
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.push(f(x, y));
        }
    }
    IndexedImage::from_pixels(w, h, px).unwrap()
}

fn pal(n: usize) -> Palette {
    Palette::new(
        (0..n)
            .map(|i| Rgb8 {
                r: (i * 7 % 256) as u8,
                g: (i * 13 % 256) as u8,
                b: (i * 31 % 256) as u8,
            })
            .collect(),
    )
    .unwrap()
}

/// Deterministic pseudo-random animation: frame k moves a few blocks around.
fn frames(w: u32, h: u32, colors: u32, n: usize) -> Vec<IndexedImage> {
    (0..n)
        .map(|k| {
            let k = k as u32;
            img(w, h, move |x, y| {
                let bx = (x + 3 * k) / 5;
                let by = (y + k) / 4;
                ((bx * 7 + by * 3 + k * 11) % colors) as u8
            })
        })
        .collect()
}

fn round_trip(fs: &[IndexedImage], colors: usize, looped: bool) {
    let opts = EncodeOptions {
        loop_frames: looped,
        hold: Hold::Uniform(5),
        ..EncodeOptions::default()
    };
    let bytes = anim::encode(fs, &pal(colors), &opts).unwrap();
    let (a, notes) = anim::read_with_notes(&bytes).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    let extra = if looped && fs.len() >= 2 { 2 } else { 0 };
    assert_eq!(a.frames.len(), fs.len() + extra);
    assert_eq!(a.loop_frames, extra);
    let got: Vec<&IndexedImage> = a.logical_frames().iter().map(|f| &f.image).collect();
    assert_eq!(got, fs.iter().collect::<Vec<_>>());
    if extra == 2 {
        // the closing frames are copies of frames 0 and 1
        assert_eq!(a.frames[fs.len()].image, fs[0]);
        assert_eq!(a.frames[fs.len() + 1].image, fs[1]);
    }
    assert_eq!(a.palette, pal(colors));
    assert_eq!((a.width, a.height), (fs[0].width(), fs[0].height()));
    assert_eq!(a.jiffies_per_frame(), Some(5));
    assert_eq!(a.dpan.unwrap().frames as usize, fs.len());
    assert_eq!(a.dpan.unwrap().version, 4);
}

#[test]
fn round_trip_sizes_colours_and_frame_counts() {
    for &(w, h) in &[(1, 1), (7, 5), (17, 9), (33, 13), (64, 20), (320, 40)] {
        for &colors in &[2u32, 32, 256] {
            for &n in &[1usize, 2, 3, 4, 7, 10] {
                round_trip(&frames(w, h, colors, n), colors as usize, true);
            }
        }
    }
}

#[test]
fn round_trip_without_loop_frames() {
    for &n in &[1usize, 2, 5] {
        round_trip(&frames(21, 11, 16, n), 16, false);
    }
}

#[test]
fn identical_and_full_change_frames() {
    let a = img(19, 8, |x, y| ((x + y) % 4) as u8);
    round_trip(&[a.clone(), a.clone(), a.clone(), a.clone()], 4, true);
    // every pixel changes between frames
    let fs: Vec<_> = (0..5u32)
        .map(|k| img(19, 8, move |x, y| ((x * 3 + y * 5 + k * 7 + k) % 8) as u8))
        .collect();
    round_trip(&fs, 8, true);
    // checkerboards flipping every frame: worst case for op counts
    let fs: Vec<_> = (0..4u32)
        .map(|k| img(16, 700, move |x, y| u8::from((x + y + k) % 2 == 0)))
        .collect();
    round_trip(&fs, 2, true);
}

#[test]
fn per_frame_hold_times_and_loop_hold() {
    let fs = frames(16, 8, 4, 3);
    let opts = EncodeOptions {
        hold: Hold::PerFrame(vec![3, 9, 12]),
        ..EncodeOptions::default()
    };
    let a = anim::read(&anim::encode(&fs, &pal(4), &opts).unwrap()).unwrap();
    let holds: Vec<u32> = (0..3).map(|i| a.hold_jiffies(i)).collect();
    assert_eq!(holds, [3, 9, 12]);
    assert_eq!(a.jiffies_per_frame(), None);
    assert_eq!(a.frames[0].rel_time, 0);
    let bad = EncodeOptions {
        hold: Hold::PerFrame(vec![1]),
        ..EncodeOptions::default()
    };
    assert_eq!(
        anim::encode(&fs, &pal(4), &bad).unwrap_err().kind,
        ErrorKind::Invalid
    );
}

#[test]
fn ranges_and_mode_survive_on_the_first_picture() {
    let fs = frames(16, 8, 8, 2);
    let opts = EncodeOptions {
        ranges: vec![dpx_iff::ColorRange {
            rate: 8192,
            flags: 1,
            low: 1,
            high: 4,
        }],
        camg: Some(0x0002_1000),
        ..EncodeOptions::default()
    };
    let a = anim::read(&anim::encode(&fs, &pal(8), &opts).unwrap()).unwrap();
    assert_eq!(a.ranges.len(), 1);
    assert_eq!(a.ranges[0].low, 1);
    assert_eq!(a.camg, Some(0x0002_1000));
}

// ---- a file assembled byte by byte, independent of the crate's writer ------------------

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = id.to_vec();
    v.extend_from_slice(&(data.len() as u32).to_be_bytes());
    v.extend_from_slice(data);
    if data.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn form(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
    let mut body = kind.to_vec();
    for p in parts {
        body.extend_from_slice(p);
    }
    let mut v = b"FORM".to_vec();
    v.extend_from_slice(&(body.len() as u32).to_be_bytes());
    v.extend_from_slice(&body);
    v
}

/// 8x4, 1 plane, uncompressed first frame: row 0 = 0xF0, others 0.
fn first_ilbm() -> Vec<u8> {
    let bmhd = [0, 8, 0, 4, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 8, 0, 4];
    let mut anhd = [0u8; 40];
    anhd[14..18].copy_from_slice(&6u32.to_be_bytes());
    form(
        b"ILBM",
        &[
            chunk(b"BMHD", &bmhd),
            chunk(b"CMAP", &[0, 0, 0, 255, 255, 255]),
            chunk(b"ANHD", &anhd),
            chunk(b"DPAN", &[0, 4, 0, 3, 0, 0, 0, 0]),
            chunk(b"BODY", &[0xF0, 0, 0, 0, 0, 0, 0, 0]),
        ],
    )
}

fn delta_ilbm(op: u8, interleave: u8, bits: u32, rel: u32, dlta: &[u8]) -> Vec<u8> {
    let mut anhd = [0u8; 40];
    anhd[0] = op;
    anhd[14..18].copy_from_slice(&rel.to_be_bytes());
    anhd[18] = interleave;
    anhd[20..24].copy_from_slice(&bits.to_be_bytes());
    form(b"ILBM", &[chunk(b"ANHD", &anhd), chunk(b"DLTA", dlta)])
}

/// Op 5 chunk for the 8x4 plane: column 0 rows `rows` get value `v` (one Uniq op each
/// contiguous run of a single row is skip + uniq 1).
fn dlta_row(row: u8, v: u8) -> Vec<u8> {
    let mut d = vec![0u8; 64];
    d[3] = 64;
    let mut col = vec![];
    let mut ops = 0;
    if row > 0 {
        col.push(row);
        ops += 1;
    }
    col.extend_from_slice(&[0x81, v]);
    ops += 1;
    d.push(ops);
    d.extend_from_slice(&col);
    d.push(0); // column 1: no ops
    if d.len() % 2 == 1 {
        d.push(0);
    }
    d
}

fn anim_file(frames: &[Vec<u8>]) -> Vec<u8> {
    form(b"ANIM", frames)
}

#[test]
fn hand_built_file_decodes_with_double_buffering() {
    // Frame 0: row 0 set. Frame 1 = delta on frame 0: row 1 := 0xAA. Frame 2 = delta on
    // frame 0 (two back): row 2 := 0x55 (row 1 must be back to 0, row 0 still set).
    // Frame 3 = delta on frame 1: row 3 := 0xFF (frame 1 has row 1 set, row 2 clear).
    let file = anim_file(&[
        first_ilbm(),
        delta_ilbm(5, 0, 0, 6, &dlta_row(1, 0xAA)),
        delta_ilbm(5, 2, 0, 7, &dlta_row(2, 0x55)),
        delta_ilbm(5, 0, 0, 8, &dlta_row(3, 0xFF)),
    ]);
    let a = anim::read(&file).unwrap();
    assert_eq!(a.frames.len(), 4);
    assert_eq!(a.loop_frames, 0);
    let rows = |i: usize| -> Vec<u8> {
        // first pixel of each row, and the bit pattern of row's byte via pixel bits
        let im = &a.frames[i].image;
        (0..4)
            .map(|y| (0..8).fold(0u8, |b, x| b << 1 | im.get(x, y).unwrap()))
            .collect()
    };
    assert_eq!(rows(0), [0xF0, 0, 0, 0]);
    assert_eq!(rows(1), [0xF0, 0xAA, 0, 0]);
    assert_eq!(rows(2), [0xF0, 0, 0x55, 0]);
    assert_eq!(rows(3), [0xF0, 0xAA, 0, 0xFF]);
    assert_eq!(a.frames[1].rel_time, 6);
    assert_eq!(a.frames[2].rel_time, 7);
    assert_eq!(a.hold_jiffies(0), 6);
    assert_eq!(a.hold_jiffies(2), 8);
    assert_eq!(a.dpan.unwrap().frames, 3);
    assert_eq!(
        a.palette.colors()[1],
        Rgb8 {
            r: 255,
            g: 255,
            b: 255
        }
    );
}

#[test]
fn interleave_one_uses_the_previous_frame() {
    let file = anim_file(&[
        first_ilbm(),
        delta_ilbm(5, 1, 0, 1, &dlta_row(1, 0xAA)),
        delta_ilbm(5, 1, 0, 1, &dlta_row(2, 0x55)),
    ]);
    let a = anim::read(&file).unwrap();
    let byte =
        |i: usize, y: i32| (0..8).fold(0u8, |b, x| b << 1 | a.frames[i].image.get(x, y).unwrap());
    assert_eq!((byte(2, 1), byte(2, 2)), (0xAA, 0x55));
}

#[test]
fn xor_flag_in_either_bit_xors() {
    for bits in [2u32, 4] {
        let file = anim_file(&[first_ilbm(), delta_ilbm(5, 1, bits, 1, &dlta_row(0, 0xFF))]);
        let a = anim::read(&file).unwrap();
        let im = &a.frames[1].image;
        // row 0 was 0xF0, xor 0xFF -> 0x0F
        let b = (0..8).fold(0u8, |b, x| b << 1 | im.get(x, 0).unwrap());
        assert_eq!(b, 0x0F);
    }
}

#[test]
fn method_zero_frames_are_read() {
    let mut second = first_ilbm();
    // flip the BODY row to 0x0F (the last 8 bytes of the form are the BODY data)
    let n = second.len();
    second[n - 8] = 0x0F;
    let file = anim_file(&[first_ilbm(), second]);
    let a = anim::read(&file).unwrap();
    let im = &a.frames[1].image;
    assert_eq!(
        (0..8).fold(0u8, |b, x| b << 1 | im.get(x, 0).unwrap()),
        0x0F
    );
}

#[test]
fn frame_cmap_changes_the_palette_from_that_frame() {
    let mut anhd = [0u8; 40];
    anhd[0] = 5;
    let f = form(
        b"ILBM",
        &[
            chunk(b"ANHD", &anhd),
            chunk(b"CMAP", &[9, 9, 9]),
            chunk(b"DLTA", &[0u8; 64]),
        ],
    );
    let a = anim::read(&anim_file(&[first_ilbm(), f])).unwrap();
    assert_eq!(
        a.frames[1].palette.as_ref().unwrap().colors()[0],
        Rgb8 { r: 9, g: 9, b: 9 }
    );
    // entry 1 is kept from the first palette
    assert_eq!(
        a.palette_at(1).colors()[1],
        Rgb8 {
            r: 255,
            g: 255,
            b: 255
        }
    );
    assert_eq!(a.palette_at(0).colors()[0], Rgb8 { r: 0, g: 0, b: 0 });
}

#[test]
fn cut_file_keeps_the_frames_before_the_cut() {
    let mut file = anim_file(&[
        first_ilbm(),
        delta_ilbm(5, 0, 0, 6, &dlta_row(1, 0xAA)),
        delta_ilbm(5, 0, 0, 6, &dlta_row(2, 0x55)),
    ]);
    file.truncate(file.len() - 10);
    let (a, notes) = anim::read_with_notes(&file).unwrap();
    assert_eq!(a.frames.len(), 2);
    assert!(
        notes.iter().any(|n| n.contains("ends inside frame 2")),
        "{notes:?}"
    );
}

// ---- refusals ---------------------------------------------------------------------------

fn refused(file: &[u8], kind: ErrorKind, summary: &str, detail_part: &str) {
    let e = anim::read(file).unwrap_err();
    assert_eq!(e.kind, kind, "{e}");
    assert_eq!(e.summary, summary);
    assert!(e.summary.chars().count() <= 34, "{}", e.summary);
    assert!(e.details.contains(detail_part), "{}", e.details);
}

#[test]
fn refusals() {
    // a plain ILBM
    refused(
        &first_ilbm(),
        ErrorKind::NotAnim,
        "Not an ANIM file (single picture)",
        "FORM ILBM",
    );
    // not IFF at all
    let e = anim::read(b"GIF89a000000000000").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Iff);
    // empty ANIM
    refused(
        &anim_file(&[]),
        ErrorKind::MissingFrame,
        "ANIM has no frames",
        "no FORM ILBM",
    );
    // first frame without BODY
    let no_body = form(b"ILBM", &[chunk(b"ANHD", &[0u8; 40])]);
    refused(
        &anim_file(&[no_body]),
        ErrorKind::Unsupported,
        "First ANIM frame has no BODY",
        "black screen",
    );
    // methods that are not supported
    for (op, summary) in [
        (7u8, "ANIM method 7: not supported"),
        (8, "ANIM method 8: not supported"),
        (74, "ANIM method J: not supported"),
        (4, "ANIM method 4: not supported"),
    ] {
        let f = anim_file(&[first_ilbm(), delta_ilbm(op, 0, 0, 1, &[0u8; 64])]);
        refused(&f, ErrorKind::Unsupported, summary, "frame 1");
    }
    // method 5 without DLTA
    let mut anhd = [0u8; 40];
    anhd[0] = 5;
    let nodlta = form(b"ILBM", &[chunk(b"ANHD", &anhd)]);
    refused(
        &anim_file(&[first_ilbm(), nodlta]),
        ErrorKind::BadFrame,
        "Frame has no DLTA chunk",
        "no DLTA",
    );
    // DLTA shorter than the offset table
    let f = anim_file(&[first_ilbm(), delta_ilbm(5, 0, 0, 1, &[0u8; 10])]);
    refused(
        &f,
        ErrorKind::BadDelta,
        "DLTA too short for method 5",
        "needs 64 bytes",
    );
    // offset outside the chunk
    let mut d = vec![0u8; 64];
    d[3] = 200;
    let f = anim_file(&[first_ilbm(), delta_ilbm(5, 0, 0, 1, &d)]);
    refused(
        &f,
        ErrorKind::BadDelta,
        "DLTA: bad offset, plane 0",
        "starts at byte 200",
    );
    // column cut off (op count says 2, data end)
    let mut d = vec![0u8; 64];
    d[3] = 64;
    d.extend_from_slice(&[2, 1]);
    let f = anim_file(&[first_ilbm(), delta_ilbm(5, 0, 0, 1, &d)]);
    refused(
        &f,
        ErrorKind::BadDelta,
        "DLTA cut off, plane 0",
        "plane 0, column 0",
    );
    // skip past the bottom (4 rows)
    let mut d = vec![0u8; 64];
    d[3] = 64;
    d.extend_from_slice(&[1, 9, 0, 0]);
    let f = anim_file(&[first_ilbm(), delta_ilbm(5, 0, 0, 1, &d)]);
    refused(
        &f,
        ErrorKind::BadDelta,
        "DLTA runs past row 4, plane 0",
        "reaches row 9 of 4",
    );
    // ANHD without method and BODY
    let f = anim_file(&[first_ilbm(), form(b"ILBM", &[chunk(b"CMAP", &[1, 2, 3])])]);
    refused(
        &f,
        ErrorKind::BadFrame,
        "Frame has no ANHD chunk",
        "method is unknown",
    );
}

#[test]
fn encoder_refusals() {
    let p = pal(4);
    let e = anim::encode(&[], &p, &EncodeOptions::default()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    let a = img(8, 8, |_, _| 0);
    let b = img(9, 8, |_, _| 0);
    let e = anim::encode(&[a.clone(), b], &p, &EncodeOptions::default()).unwrap_err();
    assert_eq!(e.summary, "cannot write ANIM: frame sizes");
    assert!(e.details.contains("frame 1 is 9x8"));
}

#[test]
fn pixel_outside_palette_in_first_frame_is_refused() {
    let c = img(8, 8, |_, _| 7);
    let e = anim::encode(&[c], &pal(4), &EncodeOptions::default()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
}

#[test]
fn anhd_bytes_in_a_written_file() {
    let fs = frames(16, 4, 4, 3);
    let bytes = anim::encode(&fs, &pal(4), &EncodeOptions::default()).unwrap();
    // locate the second ANHD (first delta frame) and check its fields
    let pos = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| w == b"ANHD")
        .map(|(i, _)| i)
        .nth(1)
        .unwrap();
    assert_eq!(&bytes[pos + 4..pos + 8], &40u32.to_be_bytes());
    let a = Anhd::parse(&bytes[pos + 8..pos + 48]).unwrap();
    assert_eq!(a.operation, 5);
    assert_eq!(a.interleave, 0);
    assert_eq!(a.bits, 0);
    assert_eq!(a.rel_time, 6);
    assert_eq!((a.w, a.h), (16, 4));
}

/// Real files are local only (never committed): decode every ANIM found in `Examplepics/`
/// of the main checkout, if there is one, and print what was seen.
#[test]
fn local_example_animations_if_present() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Examplepics");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let Ok(bytes) = std::fs::read(e.path()) else {
            continue;
        };
        if bytes.len() > 12 && &bytes[8..12] == b"ANIM" {
            match anim::read_with_notes(&bytes) {
                Ok((a, notes)) => eprintln!(
                    "{}: {} frames ({} loop), {}x{}, {} planes, {notes:?}",
                    e.path().display(),
                    a.frames.len(),
                    a.loop_frames,
                    a.width,
                    a.height,
                    a.planes
                ),
                Err(err) => panic!("{}: {err}", e.path().display()),
            }
        }
    }
}
