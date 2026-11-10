# Test data

Self-made JPEG fixtures for `jpeg_io` tests (public domain / CC0).

- `red-blue-16x8.jpg`: 16x8, left 8x8 block pure red, right 8x8 block pure blue.
  Made with Pillow: quality 95, `subsampling=0` (4:4:4), baseline.
- `grey-8x8.jpg`: 8x8 single-component greyscale, value 128, quality 95.
- `v1-single-frame.dpxproj`: an 8x4, 4-colour project (one colour range, grab point,
  transparent colour, spare page) written by the version-1 project writer before animation
  frames existed (ADR-28); single-frame projects must still be written byte for byte like it.
