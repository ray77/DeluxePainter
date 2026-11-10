# Test data

Self-made ILBM fixtures (public domain / CC0), written byte by byte by the builders in
`../golden.rs` from the IFF/ILBM spec, not by DeluxePainter's own writer. They contain no
artwork: the pixels are a formula, the palettes are made up.
`checked_in_fixtures_match_their_builders` keeps them identical to the builders; after a
deliberate builder change run `DPX_BLESS=1 cargo test -p dpx-iff --test golden`.

| File | What it is | Expected result |
|---|---|---|
| `golden-8planes-byterun1-65x3.iff` | 65x3 (odd width), 8 planes, ByteRun1 | reads exactly, no notes |
| `golden-5planes-crng-17x3.iff` | 17x3, 5 planes, uncompressed, one CRNG, CAMG 0 | reads exactly, no notes |
| `golden-mask-20x3.iff` | 20x3, 3 planes + mask plane, ByteRun1 | reads exactly with mask, no notes |
| `broken-body-cut.iff` | 16x4, 2 planes, file ends 9 bytes into the BODY | loads; rows 2-3 colour 0; note |
| `broken-bad-run.iff` | ByteRun1 repeat run longer than its row | error `BODY: bad ByteRun1 run, row 1` |
| `broken-no-body.iff` | file ends inside the CMAP | error `No pixel data (BODY missing)` |
| `broken-ham.iff` | 6 planes with CAMG HAM | error `HAM pictures arrive in Phase 5` |

Try them with `deluxepainter info <file>`.
