# Test data

Self-made JPEG fixtures for `jpeg_io` tests (public domain / CC0).

- `red-blue-16x8.jpg`: 16x8, left 8x8 block pure red, right 8x8 block pure blue.
  Made with Pillow: quality 95, `subsampling=0` (4:4:4), baseline.
- `grey-8x8.jpg`: 8x8 single-component greyscale, value 128, quality 95.
- `v1-single-frame.dpxproj`: an 8x4, 4-colour project (one colour range, grab point,
  transparent colour, spare page) written by the version-1 project writer before animation
  frames existed (ADR-28); single-frame projects must still be written byte for byte like it.
- `v1-ocs-not-12-bit.dpxproj`: a 320x256 OCS PAL:Low Res, 32-colour version-1 project as a
  build before ADR-46 could save it: colours 0-7 are DeluxePaint's 12-bit start-up colours,
  8-31 the old 24-bit grey ramp of Screen Format (`v = i * 255 / 23`), so entries 9-27, 29
  and 30 are not 12-bit. Grey bands (colours 8-31), a red hill (1) and a white moon (3).
  Written by a stdlib-only Python script (zipfile, zlib; an indexed 8-bit PNG with PLTE),
  independent of the Rust writer, for the wave-8 integration of ADR-46.
