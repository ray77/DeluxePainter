# IFF_SUPPORT_MATRIX: ILBM / ANIM chunk support plan

Phase 0 research for DeluxePainter. All multi-byte values are **big-endian**. Source IDs `[S#]` refer to `docs/research/format-sources.md`. Confidence: **verified** / **likely** / **unverified** (same definitions as in HARDWARE_MODES.md). Spec content is summarised in our own words.

Phases: **early** = ILBM + ByteRun1 + masking + CAMG/CRNG/CCRT; **mid** = EHB / HAM / AGA; **ANIM** stepwise; **later** = PCHG / SHAM / CTBL / DEEP / PBM.

---

## 1. IFF container basics

| Rule | Detail | Confidence |
|---|---|---|
| Chunk header | 4-byte ASCII ID + 4-byte unsigned length (ckSize, excludes header and pad) | verified [S2] (grammar), IFF-85 general knowledge |
| Even padding | Every chunk is padded to an even length with one 0 byte; the pad is *not* counted in ckSize. Applies e.g. to odd-length CMAP and BODY | verified [S2] |
| FORM | `FORM <size> ILBM <chunks...>`; size covers the type ID and all chunks | verified [S2][S4] |
| Property order | BMHD, CMAP, GRAB, DEST, SPRT, CAMG, CRNG*, CCRT*, BODY is the canonical order; property chunks may come in any order but must precede BODY; if a property repeats, the last one before BODY wins | verified [S2] |
| Unknown chunks | must be skipped (ckSize + pad). Chunks after BODY are commonly ignored by readers | verified [S4] |
| Scalability | write only canonical order; preserve unknown chunks on roundtrip where cheap (plan, see section 6) | design decision |

## 2. ILBM chunk matrix

| Chunk | Purpose | Import plan | Export plan | Phase | Confidence |
|---|---|---|---|---|---|
| FORM ILBM | container | parse, validate type | write | early | verified |
| BMHD | bitmap header | mandatory; reject files without it | always written | early | verified |
| CMAP | palette | read; apply 4-bit heuristic; handle odd length | write 2^nPlanes entries (EHB: 32 only, see below) | early | verified |
| BODY | pixel data | decode ByteRun1 / uncompressed, mask plane | write ByteRun1 (default) or uncompressed | early | verified |
| CAMG | Amiga mode | read, mask old-style flags | always write, ModeID style | early | verified |
| CRNG | colour cycling range (DPaint) | read all, keep order | write all active/inactive ranges (DPaint writes 4) | early | verified (layout), likely (DPaint count) |
| CCRT | colour cycling (Graphicraft) | read; convert to internal range | write alongside CRNG (optional) | early | verified |
| GRAB | hotspot | read for brushes | write for brushes | early (brush use) | verified |
| DEST | plane scatter for merging | read and store; apply only when loading a brush into a deeper image | write only when explicitly set; otherwise omit | later | verified (layout) |
| SPRT | sprite precedence | read and keep as metadata | write only if present on import | later | verified |
| TINY | thumbnail | skip | skip (maybe write later) | later | verified that it exists [S4]; layout unverified |
| DPPS | DPaint private page state | keep as opaque chunk | not written initially | later | verified that it exists [S15]; layout unverified |
| PCHG / SHAM / CTBL | per-line palette changes | skip in early phases | later | later | unverified (not read) |
| DEEP / 24-bit ILBM | true-colour | detect, refuse with clear message | later | later | verified as concept [S2][S4]; layout partly in [S2] |
| PBM form | chunky 8-bit variant | detect FORM type PBM | later | later | verified [S4] |

## 3. Byte layouts

### 3.1 BMHD (20 bytes) [S2] (offsets derived from the packed struct; the page gives no table) - verified

| Offset | Field | Type | Meaning |
|---|---|---|---|
| 0 | w | UWORD | image width in pixels |
| 2 | h | UWORD | image height |
| 4 | x | WORD | position within destination page (write 0) |
| 6 | y | WORD | position within destination page (write 0) |
| 8 | nPlanes | UBYTE | number of bitplanes in BODY (0 = palette-only file, no BODY) |
| 9 | masking | UBYTE | 0 none, 1 hasMask (extra mask plane per row), 2 hasTransparentColor, 3 lasso |
| 10 | compression | UBYTE | 0 none, 1 ByteRun1 |
| 11 | pad1 | UBYTE | write 0, ignore on read |
| 12 | transparentColor | UWORD | only meaningful for masking 2 or 3; else 0 |
| 14 | xAspect | UBYTE | pixel aspect, width part |
| 15 | yAspect | UBYTE | pixel aspect, height part |
| 16 | pageWidth | WORD | source page width |
| 18 | pageHeight | WORD | source page height |

Masking semantics: 0 = opaque rectangle; 1 = a mask plane follows the colour planes in every row (bit 1 = opaque, bit 0 = transparent); 2 = pixels equal to transparentColor are transparent; 3 = "lasso": the reader derives the mask by flood-filling inwards from the border with transparentColor (needs a seed-fill implementation). [S2] verified.

Import: validate w,h > 0, nPlanes <= 8 (24/32 => DEEP/true-colour, later), compression in {0,1}; unknown compression => error. Treat xAspect==0 or yAspect==0 as 1:1 (likely, design).
Export: pageWidth/pageHeight = mode default page (see HARDWARE_MODES.md), x=y=0.

### 3.2 CMAP - verified [S2]

| Item | Detail |
|---|---|
| Content | n x (R,G,B) bytes, 0-255 each; n = ckSize / 3, normally 2^nPlanes |
| Odd ckSize | followed by a 0 pad byte |
| Fewer entries than colours | fill rest with reader defaults; extra entries ignored |
| 4-bit pitfall | older OCS writers shift 4-bit values to the high nibble (0xN0) instead of replicating (0xNN). Detect: every byte in the CMAP has its low nibble 0. Fix: v = v | (v >> 4). Writers should always write the replicated form. |
| Possible second variant | low-nibble storage (0x0N). Not discussed in sources => unverified; heuristic: all bytes <= 0x0F => multiply by 17. |
| EHB | write 32 entries (preferred) but accept 64 on import (unverified what DPaint writes) |
| Palette-only file | nPlanes = 0 and no BODY: valid way to store a bare palette (verified [S2]) |

### 3.3 CAMG (4 bytes) - verified [S2][S6]

| Offset | Field | Type |
|---|---|---|
| 0 | viewModes / ModeID | ULONG |

Flag values and ModeID constants: see HARDWARE_MODES.md section 5. Import: if high word nonzero and bit 0x1000 clear in low word => old-style garbage; use low word only, and strip 0x7102 bits. If CAMG missing: 6 planes => guess HAM (spec statement) but prefer EHB if CMAP has exactly 32 entries (design suggestion).

### 3.4 CRNG (8 bytes) - verified [S2]; DPaint specifics see 3.5

| Offset | Field | Type | Meaning |
|---|---|---|---|
| 0 | pad1 | WORD | write 0 |
| 2 | rate | WORD | 16384 = 60 steps/s; linear: steps/s = rate * 60 / 16384 (30 steps/s = 8192) |
| 4 | flags | WORD | bit 0 RNG_ACTIVE = cycling on; bit 1 RNG_REVERSE = opposite direction; other bits reserved |
| 6 | low | UBYTE | first colour register of the range |
| 7 | high | UBYTE | last colour register of the range |

Notes:
- Rate 0 (or a very small rate) must be treated as "do not cycle" to avoid a divide-by-zero; behaviour at rate 0 itself is unverified, so treat as inactive (design).
- [S2] warns about at least one program that sets the active bit but uses a tiny rate (36) to mean inactive. 36 -> 0.13 steps/s, practically frozen. Importer should clamp: rates below about 1 step/s... threshold is a design decision (unverified).
- Which direction "forward" means (colours move to higher indices vs lower) is **not specified in a read source**: unverified. Check against a real DPaint IV file with a known cycle before implementing.
- DPaint usually writes 4 CRNG chunks (verified [S2]); the DPaint IV AGA count for its larger palette is unverified.
- ILBM from a DPaint ANIM first frame carried six CRNG chunks in the DPaint III anim-brush description [S15].

### 3.5 CCRT (14 bytes) - verified [S2][S16]

| Offset | Field | Type | Meaning |
|---|---|---|---|
| 0 | direction | WORD | 0 off, 1 forward, -1 backward |
| 2 | start | UBYTE | first register |
| 3 | end | UBYTE | last register |
| 4 | seconds | LONG | delay between steps, seconds part |
| 8 | microseconds | LONG | delay, microseconds part |
| 12 | pad | WORD | write 0 |

Both sources say CCRT behaves like CRNG; new programs should prefer CRNG, and a program may write both so DPaint and Graphicraft each find data [S16]. Import: if CRNG is present ignore CCRT; otherwise convert (rate = 16384 * 60 / ... i.e. steps/s = 1 / (seconds + microseconds/1e6), then rate = steps/s * 16384 / 60). Export: write CRNG; CCRT optional.

### 3.6 GRAB / DEST / SPRT - verified [S2]

| Chunk | Offset | Field | Type | Notes |
|---|---|---|---|---|
| GRAB | 0 / 2 | x / y | WORD | hotspot relative to upper-left (4 bytes) |
| DEST | 0 | depth | UBYTE | number of bitplanes in the destination |
| DEST | 1 | pad1 | UBYTE | 0 |
| DEST | 2 | planePick | UWORD | bit=1: next source plane goes to this destination plane; 0: use planeOnOff |
| DEST | 4 | planeOnOff | UWORD | value for non-picked planes |
| DEST | 6 | planeMask | UWORD | bit=1: destination plane is written |
| SPRT | 0 | precedence | UWORD | 0 = foremost; readers may ignore |

DEST default: planePick = planeMask = 2^nPlanes - 1. Many readers ignore DEST (verified [S2]).

### 3.7 BODY

Per scanline, in order: plane 0, plane 1, ..., plane nPlanes-1, then (if masking == 1) the mask row. Plane 0 is the least-significant bit of the colour index. Within a row the leftmost pixel is the most significant bit of the first byte. [S2] verified.

| Quantity | Formula |
|---|---|
| words per row | (w + 15) / 16 (integer division) |
| row_bytes | 2 * ((w + 15) / 16) |
| rows per scanline | nPlanes (+1 if masking == 1) |
| uncompressed BODY size | row_bytes * (nPlanes + (masking == 1 ? 1 : 0)) * h |
| compressed | each row compressed independently; compression never spans rows or planes |

The mask row is compressed the same way as colour rows. Readers may drop or add planes (e.g. 8 -> 5 planes) (verified [S2]). When the image is narrower than expected, writers often skip compression because it would grow the data (verified [S2]).

### 3.8 ByteRun1 - verified [S2], widely known as PackBits

Decoder, per row, until row_bytes bytes were produced:
1. read control byte n as **signed** int8.
2. 0 <= n <= 127: copy the next n + 1 bytes literally.
3. -127 <= n <= -1: read one byte and repeat it (-n + 1) times.
4. n == -128: no operation (skip, consume nothing more).

Robustness requirements (design): bound all reads by chunk end, stop a row exactly at row_bytes (error or clamp if a run overshoots), tolerate a trailing pad byte.
Encoder guidance from [S2]: emit a 2-byte repeat as a run only when it is not between two literal runs; always encode 3 or more equal bytes as a run; a run is at most 128 bytes; literal at most 128 bytes. Never emit -128.

## 4. Mode-dependent import/export plan

| Feature | Import | Export | Phase | Notes |
|---|---|---|---|---|
| 1-5 planes, indexed | full | full | early | OCS LoRes/HiRes |
| masking 1 (mask plane) | to alpha | from alpha | early | verified |
| masking 2 (transparent colour) | to alpha | when brush marks a colour | early | verified |
| masking 3 (lasso) | flood-fill from edge on import | not written | mid | verified; low value |
| EHB (6 planes, CAMG 0x80) | expand with half-brightness lookup | export keep 6 planes + 32-entry CMAP | mid | see HARDWARE_MODES.md section 6; Load opens it as a 64-colour indexed picture (ADR-26), saves 64-colour non-EHB; real EHB export later |
| HAM6 | decode with rules from section 7 there | encoder later | mid | decoder and greedy encoder exist in `dpx_formats::ham` (ADR-25); Load decodes and reduces like true colour (ADR-26), HAM export later |
| HAM8 / AGA 8 planes | same, 8-bit CMAP | same | mid | codec exists (ADR-25), Load wired (ADR-26); control-bit layout needs validation against a real file |
| 7-8 plane indexed | full | full | mid | |
| CMAP depth conversion | 4 -> 8 bit heuristic | 8-bit replicated | early | |
| PBM / DEEP | detect + message | none | later | |
| PCHG, SHAM, CTBL | skip | none | later | not read in Phase 0 |

## 5. ANIM

### 5.1 Structure (summary) - verified [S15]

An ANIM file is a sequence of ILBM frames: the first is a full ILBM (BMHD, CMAP, CRNG, CAMG, BODY ...), each following frame carries an ANHD (frame header) and a DLTA (delta data). The last delta can return the animation to the first frame to allow looping (DPaint convention). Frames are applied to buffers with `interleave` controlling how many frames back the delta refers to (default 0 means 2, i.e. double buffering).

### 5.2 ANHD (40 bytes, offsets derived from the field sizes) - verified [S15]

| Offset | Field | Type | Meaning |
|---|---|---|---|
| 0 | operation | UBYTE | DLTA compression method (see 5.3) |
| 1 | mask | UBYTE | XOR mode only: plane mask |
| 2 | w | UWORD | XOR mode only: area width |
| 4 | h | UWORD | XOR mode only: area height |
| 6 | x | WORD | XOR mode only: area X |
| 8 | y | WORD | XOR mode only: area Y |
| 10 | abstime | ULONG | unused |
| 14 | reltime | ULONG | delay since previous frame, in jiffies (1/60 s) |
| 18 | interleave | UBYTE | frames back to modify; 0 means 2 |
| 19 | pad0 | UBYTE | unused |
| 20 | bits | ULONG | option flags for operations 4 and 5 (see below) |
| 24 | pad[16] | UBYTE[16] | reserved |

`bits` (per the spec table): bit 0 short/long data; bit 1 set/XOR; bit 2 separate/shared info per plane; bit 3 RLC; bit 4 horizontal/vertical; bit 5 short/long info offsets. **Contradiction in the same source:** the DPaint anim-brush description marks bit 2 (value 4) as the XOR indicator while the core table puts it at bit 1. Treat as an open point.

### 5.3 DLTA operations [S15]

| Op | Name | Summary | Written by DPaint? | Priority |
|---|---|---|---|---|
| 0 | Set directly | normal ILBM BODY, no delta | first frame, yes | early |
| 1 | XOR ILBM | historical XOR of changed area | no evidence | skip |
| 2 | Long Delta | changed longwords with offsets | no evidence | skip |
| 3 | Short Delta | same with words | no evidence | skip |
| 4 | Generalized delta | short/long, horizontal/vertical, XOR, RLC selectable via bits | no evidence | skip |
| 5 | Byte Vertical Delta (Jim Kent) | per plane, per byte column, run-length coded; the spec's recommended method | **yes: DPaint III anim brushes use op 5** (verified [S15]); DPaint IV full ANIMs: likely op 5 (unverified, not found in a read source) | **ANIM step 1** |
| 6 | Stereo op 5 | third-party stereo variant | no | skip |
| 7 | Short/long vertical delta | faster playback, less compression than 5 (Anim7, 1992) | unverified | ANIM step 3 (optional) |
| 8 | Vertical delta (Anim8) | word/long columns, ops inline | unverified | ANIM step 3 |
| 74 ('J') | Eric Graham method | reserved, details never published | no | never |

Which ops a given DPaint IV version writes is **unverified**; check by hex-dumping ANHD.operation of real files in `Manuals, Pics`.

### 5.4 DLTA op 5 layout (verified [S15])

- Chunk starts with 16 LONG offsets (from chunk start): the first 8 point to per-plane data, the last 8 are unused. Offset 0 means the plane is unchanged. (DPaint: only the first 6 used; the first is 0 or 64, as stated for the anim-brush case.)
- Each plane is coded separately; each *byte column* of a plane is coded separately (320x200: 40 columns of 200 bytes). Column: one op-count byte, 0 = no change; then that many ops:
  - **Skip**: byte with high bit clear and non-zero: advance destination by that many rows.
  - **Uniq**: high bit set: low 7 bits = N literal bytes follow, copied to N rows.
  - **Same**: byte 0, then a count byte, then one value repeated count times.
- The destination stride between rows is the bytes-per-row of the bitmap (not 1).

### 5.5 Op 7 and 8 summary (verified [S15])

- Op 7: header of 16 LONGs: 8 pointers to opcode lists, 8 pointers to data lists; unused planes 0. Opcode list (bytes) and data list (words or longs, selected by bit 0) are stored separately. Columns are word-wide (20 columns of 200 words for 320x200, or 10 long columns). Skip ops carry no data; Uniq copies N items from the data list; Same repeats one data item N times.
- Op 8: header of 16 LONGs, first 8 point to op lists, last 8 unused; data are inline with the ops, as words or longs. Skip max 0x7fff (word) or 0x7fffffff (long); same-count up to 0xffff / 0xffffffff. If the width does not divide into an even number of long columns, the last column is word-coded (336 px = 10 long columns + 1 word column).
- First frame may be a DLTA against a cleared "black" frame instead of a BODY; ANHD.operation must match.

### 5.6 DPaint anim-brush / DPaint ANIM specifics (verified [S15] for DPaint III)

- Chunk DPAN: version (4), nframes, flags (unused); only nframes is read.
- First ILBM: BMHD, CMAP, DPPS, GRAB, six CRNG, DPAN, CAMG, BODY. Later frames: ANHD + DLTA.
- Anim brush ANHD: interleave 1, reltime 1, abstime 0, bits 4, w/h = brush box, rest 0. Deltas use XOR so playback is reversible; because of XOR and interleave 1 normal ANIM players cannot play such brush files.
- Whether DPaint IV AGA changes any of this is unverified.

### 5.7 ANIM implementation plan (stepwise)

| Step | Scope | Phase |
|---|---|---|
| 1 | Read ANIM FORM, first frame as ILBM, op 5 non-XOR, interleave 2 (double buffer) and 1 | ANIM-1 (after ILBM) |
| 2 | Playback timing (reltime jiffies = 1/60 s), looping via the closing delta | ANIM-2 |
| 3 | XOR variant (anim brushes), ops 7 and 8 | ANIM-3 |
| 4 | Writing op 5 (encoder: per column diff, Skip/Uniq/Same), op 0 for frame 1 | ANIM-4 |
| 5 | Ops 1-4, 6, J: not planned unless a test file needs them | not planned |

Roundtrip: write op 5 with interleave 2 (standard ANIM5) unless source was brush type; keep per-frame CRNG only on the first frame; preserve reltime per frame.

## 6. Roundtrip notes

| Topic | Note |
|---|---|
| Unknown chunks | store (id, bytes) with position relative to BODY; re-emit before BODY. Reduces data loss for DPPS, TINY, vendor chunks. Design decision |
| CAMG | preserve the full ModeID; write monitor part from the document (PAL 0x00021000 etc.) |
| CMAP | keep original bytes in addition to expanded palette so that an unmodified file is bit-identical (optional) |
| ByteRun1 | byte-identical output with the original encoder is not guaranteed (encoder heuristics differ); roundtrip test = decode(encode(x)) == x, not file equality |
| Padding | always re-emit even padding |
| Mask | mask bit 1 = opaque; if alpha is multi-level, threshold at export |
| HAM / EHB | keep the indexed planes as the source of truth; decoded RGB is derived (otherwise lossless roundtrip impossible) |
| CRNG | preserve all ranges including inactive ones and their order |

## 7. Open points

1. ANHD `bits` bit 1 vs bit 2 (XOR) contradiction in [S15].
2. Which DLTA op real DPaint IV versions write for full-screen ANIM: check real files.
3. CRNG direction semantics and exact rate handling in DPaint IV: check real files.
4. CMAP low-nibble variant and EHB palette length: check real files.
5. TINY, DPPS, PCHG, SHAM, CTBL, DEEP layouts not yet read (later phases).
6. The Devices Manual mirror at amigadev.elowar.com could not be fetched securely (expired TLS certificate); all ILBM layout facts come from wiki.amigaos.net [S2] and cross-checks [S4][S16].

## 8. Implementation status (Phase 2 start, 2026-11-10)

Crate `dpx-iff` (`iff`, `byterun1`, `ilbm`), used by `deluxepainter info|convert` and `--open`.

| Item | Status |
|---|---|
| FORM/chunk parsing, even padding, oversized FORM/chunk sizes clamped | done, tested |
| BMHD, CMAP (4-bit fix, padded/truncated to 2^nPlanes, grey ramp without CMAP), CAMG (kept raw), CRNG (kept raw, order kept), CCRT (converted to CRNG when no CRNG), GRAB | done, tested |
| BODY 1–8 planes, uncompressed and ByteRun1, mask plane (masking 1), masking 2 flag + transparent colour kept | done, tested |
| Unknown chunks before BODY (DPPS, DPPV, TINY …) | kept and re-emitted before BODY |
| Writer | canonical order BMHD, CMAP, GRAB, CAMG, CRNG*, other chunks, BODY; CMAP written in replicated 8-bit form; refuses pixels outside the palette, a palette larger than the plane count, and inconsistent masks |
| EHB / HAM6 / HAM8 | read as raw plane values (`Ilbm::special`, CMAP cut to 32 / 16 / 64 entries); the app opens EHB as 64 free colours and HAM as decoded RGB reduced like true colour (ADR-26) |
| Refused with a clear message | HAM with other plane counts, >8 planes / DEEP, other FORM types (PBM, ACBM, ANIM …); broken files with chunk and offset, see §9 |
| Tolerated with a note (§9) | lasso masking (read as transparent colour), BODY ending early, odd CMAP sizes, short CAMG/CRNG/CCRT/GRAB, missing pad bytes, wrong FORM sizes, trailing data |
| GUI Picture ▸ Save (ADR-8) | writes CAMG from the document target (AGA full mode id, OCS 16-bit), BMHD aspect and page size from the display mode, CRNG/GRAB/transparent colour kept from the loaded file; private chunks (DPPS, DPPV, TINY) and mask planes are not carried over after editing, with a note |
| True-colour PNG/JPEG → ILBM (ADR-9) | reduced to the mode's colour count, saved with that mode's CAMG; transparent pixels → masking 2 with transparent colour 0 (pink); `convert photo.jpg out.iff` uses the default screen (AGA PAL LoRes) |
| Not done | EHB/HAM editing and export (Phase 5; the writer refuses HAM/EHB), CMAP low-nibble variant, chunks after BODY, ANIM |

Verification on six real DeluxePaint-era pictures (320×200, 5 and 8 planes, ByteRun1, masking 0/2, 4-bit CMAPs, DPPS/DPPV/TINY, up to 16 CRNG): pixels and palettes of our PNG export, our ILBM rewrite and a PNG → ILBM round trip all match an independent Python ILBM decoder read with Pillow. Rewritten files are not byte-identical to the originals: the CMAP is written in replicated form (0xNN instead of 0xN0) and chunks are written in canonical order.

## 9. Diagnostics for broken and foreign files (ADR-17)

Every read error is a short first line (at most 34 characters / 252 UI-font pixels, shown in the file requester) followed by details for stderr and `deluxepainter info`: which chunk, its file offset in decimal and hex (`offset 60 ($3C)`), the values found and what was expected. Tolerated quirks become notes (lower case, shown in the "Picture loaded" requester, ADR-12). Fixtures: `crates/dpx-iff/tests/golden.rs` and `crates/dpx-iff/tests/data/`.

### 9.1 Refused

| Case | First line | Details name |
|---|---|---|
| empty file / fewer than 12 bytes | `File is empty` / `File too short for IFF` | byte count |
| does not start with FORM | `Not an IFF file` (file level: GIF, BMP, TIFF, WebP, PowerPacker, XPK, archives, Amiga programs are named) | first four bytes in hex and ASCII |
| IFF `LIST` / `CAT` | `IFF LIST files: not supported yet` | |
| FORM PBM / ACBM | `PBM pictures: not supported yet` | what the format is |
| FORM ANIM | `ANIM files arrive in Phase 6` | |
| FORM DEEP / RGBN / RGB8 / TVPP | `RGBN pictures: not supported yet` | true colour |
| other FORM (8SVX, SMUS, FTXT …) | `IFF FORM 8SVX: not a picture` | |
| no BMHD before the BODY | `No picture header (BMHD missing)` | chunks found with offsets, truncation |
| no BODY, planes > 0 (also: file cut inside an earlier chunk or chunk header) | `No pixel data (BODY missing)` | planes, chunks found, where the file ends |
| BMHD < 20 bytes / cut off | `Picture header (BMHD) too short` / `… is cut off` | offset, bytes present / needed |
| width or height 0 | `Bad picture size 0x200` | offset |
| more than 64 megapixels | `Picture too big: 65535x65535` | offset, limit |
| planes 9–23, 25–31, > 32 | `Bad BMHD: 12 bitplanes` | allowed counts |
| planes 24 / 32 | `24-bit ILBM: not supported yet` | |
| masking > 3 / compression > 1 | `Unknown masking type 7` / `Unknown compression type 2` | known values |
| CAMG HAM with a plane count other than 6 or 8 | `HAM with 5 bitplanes` | CAMG offset and value |
| DEEP chunk inside an ILBM | `DEEP ILBM: not supported yet` | offset |
| ByteRun1 run or literal longer than the rest of its row | `BODY: bad ByteRun1 run, row 12` | file offset of the control byte, run length, plane, bytes left |
| PNG / JPEG decoder errors | `PNG file is cut off`, `PNG file is damaged`, `JPEG file is cut off`, `JPEG cannot be decoded`, `JPEG variant not supported` … | the decoder's own reason |

### 9.2 Tolerated with a note

| Quirk | Handling |
|---|---|
| data after the end of the FORM (XMODEM padding, a second FORM) | ignored, note with byte count and offset |
| FORM size larger than the file, all chunks complete | ignored, note |
| FORM size too small for the chunks that follow (e.g. 0, never patched) | file read to its end, note |
| pad byte missing after an odd chunk, at the end or mid-file (next id checked) | read without it, note |
| chunk header cut off after the BODY | ignored, note |
| BODY ends early (file cut or chunk too short) | incomplete plane rows stay 0 (rest of picture colour 0), note with row, plane, byte counts for uncompressed data |
| CMAP fewer / more entries than 2^planes | missing entries black / surplus dropped, note |
| CMAP length not a multiple of 3; no complete colour; no CMAP | stray bytes ignored; grey ramp; grey ramp; note |
| CAMG < 4, CRNG < 8, CCRT < 14, GRAB < 4 bytes | chunk ignored, note |
| chunk id not printable ASCII | skipped (not re-written), note |
| chunks after the BODY | not read, note naming them |
| lasso masking (3) | read as transparent colour, note (no outline fill yet) |

Not diagnosed: BODY data longer than needed (ignored silently, harmless); repeated BMHD/CMAP (last one wins, per spec); pixel values that are fine but meaningless (e.g. transparent colour outside the palette, which saving already reports).
