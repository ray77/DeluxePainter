# `.dpxproj` project format, version 1

The lossless save format of DeluxePainter (CLAUDE.md: Save = `.dpxproj`, Export = IFF ILBM / IFF ANIM / PNG). Code: `crates/dpx-formats/src/project.rs`; decisions: ADR-23.

## Container

A ZIP archive. Entries use deflate (the reader also accepts stored); no other compression, no encryption, no directory entries. The writer is deterministic: fixed entry order, fixed time stamp (1980-01-01 00:00), fixed Unix permissions (0644), so the same project gives the same bytes. The ZIP magic (`PK\x03\x04`) identifies the file; the extension is `.dpxproj`.

Entries of version 1, in writing order:

| Entry | Content |
|---|---|
| `manifest.json` | the manifest below (UTF-8 JSON, pretty-printed) |
| `frames/0/layers/0.png` | the page: 8-bit indexed PNG, `PLTE` = the palette |
| `spare/page.png` | optional spare page, same kind, same palette |

## Manifest

```json
{
  "format": "dpxproj",
  "format_version": 1,
  "app_version": "0.1.0",
  "target": { "chipset": "aga", "mode": { "video": "pal", "resolution": "lo", "interlace": false }, "screen": [320, 256] },
  "page": { "width": 320, "height": 256 },
  "palette": ["#000000", "#AAAAAA"],
  "ranges": [ { "rate": 4096, "flags": 0, "low": 20, "high": 27 } ],
  "grab": [5, -6],
  "transparent_color": 3,
  "frames": [ { "layers": [ { "name": "Background", "image": "frames/0/layers/0.png" } ] } ],
  "spare": { "image": "spare/page.png", "width": 320, "height": 256 }
}
```

- `format` is `"dpxproj"`; `format_version` is a whole number from 1. A newer version is refused with an error naming it.
- `app_version` is informational (the writing build).
- `target`: chipset (`ocs`/`aga`), display mode (video `pal`/`ntsc`, resolution `lo`/`hi`/`super_hi`, `interlace`) and screen size in pixels.
- `page`: page size in pixels. It may differ from the screen (the GUI keeps pages at least screen sized, but files from `convert` keep the picture's size).
- `palette`: 1 to 256 `#RRGGBB` strings; the index is the colour number. 8 bits per channel are kept exactly.
- `ranges`: colour-cycling ranges as ILBM CRNG stores them (`rate` 16384 = 60 steps/s, `flags` bit 0 active, bit 1 reverse, other bits kept). Ranges outside the palette are kept.
- `grab`: brush grab point `[x, y]` or `null`. `transparent_color`: palette entry that is transparent in brushes, or `null`.
- `frames`: prepared for later phases (frames, layers). Version 1 has exactly one frame with exactly one layer. `layers[].name` is at most 255 characters; `image` is the relative path of the entry.
- `spare`: the spare page or `null`; `width`/`height` must match its PNG.
- Unknown fields are an error in version 1 (a format that adds fields bumps `format_version`).
- The undo history and the revision counter are not saved.

## Layer images

8-bit indexed PNG (colour type 3, bit depth 8), size equal to the manifest, `PLTE` equal to the manifest palette, no `tRNS`. Pixel indices are the document's indices exactly. Written by `dpx_formats::write_png`.

## Reader rules

The reader refuses, with a short summary line and details (ADR-17 style):

- not a ZIP, a damaged ZIP, a wrong `format`, an invalid manifest or palette;
- `format_version` newer than 1, or 0 or not a number;
- entry names that are empty, absolute, contain `\`, `:` or NUL, or have an empty, `.` or `..` component (path traversal), in the archive and in the manifest; directory entries; duplicate names;
- a missing entry, or an entry that the manifest does not name (the entry set is exactly the manifest plus its images);
- a PNG that is not 8-bit indexed, has another size or palette than the manifest, has `tRNS`, or is damaged;
- limits (`Limits::default`): at most 1024 entries, 256 MiB uncompressed in total (declared sizes are checked first, and the bytes actually read are capped too), 1 MiB manifest, 64 megapixels per image.

## Compatibility

Version 1 files stay readable. A later version that adds layers, frames or animation raises `format_version`; this reader then reports the version instead of guessing.
