# ADR-17: Diagnostics for broken and foreign picture files

- Status: accepted (Phase 2, 2026-11-10)
- Context: Until now a broken file gave one generic line ("corrupt BODY: row 3, plane 1: …", "IFF file is truncated"), container quirks (wrong FORM size, missing final pad byte) were clamped silently, and a few harmless writer quirks were refused outright (a BODY one row short, a 6-byte CRNG, lasso masking). DeluxePaint IV itself only says "Can't Load…" and blames memory (AGA manual, troubleshooting note on PDF page 12). Real Amiga files arrive cut off by old transfers, padded by XMODEM, crunched with PowerPacker, or written by tools that forget pad bytes or the FORM size. The file requester has one message line (252 pixels of the UI font); the full story belongs on stderr and in `deluxepainter info`.
- Decision:
  - **Error text contract.** Every read error is `<first line>\n<details>`:
    - The first line names the problem in plain words, at most 34 characters and 252 pixels of the UI font (for example `No pixel data (BODY missing)`, `BODY: bad ByteRun1 run, row 12`, `HAM pictures arrive in Phase 5`).
    - The details name the chunk, its file offset in decimal and Amiga hex (`offset 60 ($3C)`), the values found and what was expected, and for missing chunks the chunks that were found and whether the file is truncated.
    - `dpx_iff::ilbm::IlbmError` is now a struct with `kind` (`ErrorKind`), `summary` and `details`; `IffError`, `PngError` and `JpegError` offer `summary()`/`details()`. PNG and JPEG errors keep the decoder's own reason in the details.
    - `files::load` returns `<path>: <first line>` plus the details indented by two spaces. Files that are none of ILBM/PNG/JPEG are named: empty, GIF, BMP, TIFF, WebP, PowerPacker (`PP20`), XPK, LhA/LZX/ZIP archives, Amiga programs, IFF `LIST`/`CAT`. Read failures say `File not found`, `No permission to read the file` or `That is a drawer`.
    - Writer validation errors (`cannot write ILBM: …`) stay one line.
  - **Refused (error)**: not IFF, shorter than a FORM header, a chunk header cut off before the BODY, FORM types other than ILBM (PBM, ACBM, true-colour DEEP/RGBN/RGB8/TVPP: "not supported yet"; ANIM: "arrive in Phase 6"; 8SVX and other non-pictures named), no BMHD before the BODY, no BODY with planes > 0, BMHD shorter than 20 bytes or cut off, zero width or height, more than 64 megapixels (the old reader would have tried to allocate up to 4 GB), plane counts other than 0–8 (24/32 "not supported yet", anything else "bad"), masking > 3, compression > 1, HAM or EHB in CAMG and 6 planes without CAMG ("arrive in Phase 5"), a DEEP chunk, a ByteRun1 run or literal longer than the rest of its row.
  - **Tolerated with a note** (notes flow into `Loaded::notes` and appear in the "Picture loaded" requester of ADR-12, on stderr and in `info`):
    - container: data after the FORM (or a second FORM), a FORM size larger than the file while every chunk is complete, a FORM size too small for the chunks that follow (including 0, never patched), a missing pad byte after an odd chunk at the end or in the middle (detected by the next chunk id), a cut-off chunk header after the BODY;
    - a BODY that ends early (cut by the file end or a chunk too short): every plane row that is not complete stays 0, so the rest of the picture is colour 0 and the note says the row, plane and, for uncompressed data, the byte counts;
    - CMAP with fewer entries than `2^planes` (rest black), more entries (dropped), a length that is not a multiple of 3, no complete colour or no CMAP at all (grey ramp);
    - CAMG, CRNG, CCRT or GRAB shorter than their layout (ignored), chunks with invalid ids (skipped, not re-written), chunks after the BODY (not read);
    - lasso masking (3) is read as transparent colour; the outline fill is not done.
  - `ilbm::read` keeps its signature and simply drops the notes; `ilbm::read_with_notes` returns them. `iff::parse_form` keeps its signature; `iff::read_form` returns the chunks with offsets and sizes, the container notes, the cut-off header and the missing byte count. `Chunk` gained `offset` and `size`.
  - A palette-only file (0 planes, no BODY) is not checked for HAM/EHB: there are no pixels to interpret.
- Deviation from DeluxePaint IV: much more detail than "Can't Load…", and quirks that DPaint IV may refuse or show as garbage are repaired with a note. How DPaint IV treats each broken case is not known (not tested on an Amiga or emulator).
- Verified:
  - `crates/dpx-iff/tests/golden.rs`: fixtures assembled byte by byte from the spec (own builder and ByteRun1 packer, independent of the crate's writer): 1–8 planes with widths 1–65 (none a multiple of 16), uncompressed and ByteRun1, mask plane, CRNG, 4-bit CMAP, palette-only file; exact pixels, palettes, ranges and masks; exact note texts for every tolerated quirk; exact error texts and kinds for 22 refused files, first lines at most 34 characters. Seven fixtures are checked in under `crates/dpx-iff/tests/data/` and compared with their builders.
  - Unit tests for the container quirks, ByteRun1 error positions, PNG cut off / CRC damage, JPEG cut off, and in `files.rs` the full `load` texts for empty, GIF, PowerPacker, unknown, LIST, HAM, bad-run, cut PNG and cut JPEG files, including that the requester line is at most 252 pixels wide in the UI font.
  - The six DeluxePaint-era sample pictures used for §8 of IFF_SUPPORT_MATRIX.md still load with no notes (checked locally, not committed).
  - CLI: `deluxepainter info crates/dpx-iff/tests/data/broken-body-cut.iff` prints the picture and `note: BODY ends in row 2 of 4, plane 0 (the file ends inside the BODY, 9 of 16 bytes): the rest of the picture is colour 0`; `--open` of that file with `--notes` shows the note in the "Picture loaded" requester (headless snapshot).
  - Not verified: the file requester showing only the first line in the running app (see consequences); behaviour on files from tools we have no samples of.
- Consequences:
  - **Open integration step:** the app's Load handler (`main.rs`, not part of this change) passes the whole error text after `<path>: ` to the file requester, which draws the newline as `?` and clips the rest. It must pass only the first line (`e.lines().next()`); until then the requester shows the first line followed by `?` and the start of the details.
  - Other readers (ANIM, PBM, palette files) can reuse `iff::read_form`, the note conventions and the first-line contract.

- Addendum (ADR-26): HAM and EHB pictures are no longer refused; the "arrive in Phase 5" messages are gone (HAM with a plane count other than 6/8 is still refused).

Note: all file writes (save and CLI convert) go through `write_atomic`: short temp name, fsync before rename.
