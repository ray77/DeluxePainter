# `.dpxproj` project format, versions 1 and 2

The lossless save format of DeluxePainter (CLAUDE.md: Save = `.dpxproj`, Export = IFF ILBM / IFF ANIM / PNG). Code: `crates/dpx-formats/src/project.rs`; decisions: ADR-23, animation frames (version 2): ADR-28.

A document with one frame is written as version 1, byte for byte as before animation frames existed; a document with several frames is written as version 2. The reader takes both.

## Container

A ZIP archive. Entries use deflate (the reader also accepts stored); no other compression, no encryption, no directory entries. The writer is deterministic: fixed entry order, fixed time stamp (1980-01-01 00:00), fixed Unix permissions (0644), so the same project gives the same bytes. The ZIP magic (`PK\x03\x04`) identifies the file; the extension is `.dpxproj`.

Entries, in writing order:

| Entry | Content |
|---|---|
| `manifest.json` | the manifest below (UTF-8 JSON, pretty-printed) |
| `frames/0/layers/0.png` | the page: 8-bit indexed PNG, `PLTE` = the palette |
| `frames/<n>/layers/0.png` | version 2: the page of frame `n` (from 0), one entry per frame in frame order |
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
- `target`: chipset (`ocs`/`aga`), display mode (video `pal`/`ntsc`, resolution `lo`/`hi`/`super_hi`, `interlace`) and screen size in pixels (the reader refuses a screen the mode does not allow, e.g. zero or huge).
- `page`: page size in pixels. It may differ from the screen (the GUI keeps pages at least screen sized, but files from `convert` keep the picture's size).
- `palette`: 1 to 256 `#RRGGBB` strings; the index is the colour number. 8 bits per channel are kept exactly.
- `ranges`: colour-cycling ranges as ILBM CRNG stores them (`rate` 16384 = 60 steps/s, `flags` bit 0 active, bit 1 reverse, other bits kept). Ranges outside the palette are kept.
- `grab`: brush grab point `[x, y]` or `null`. `transparent_color`: palette entry that is transparent in brushes, or `null`.
- `frames`: the animation frames in order, each with its layers. Version 1 has exactly one frame; version 2 one or more (the writer uses it from two frames on). Every frame has exactly one layer so far. `layers[].name` is at most 255 characters; `image` is the relative path of the entry. All frames have the page size.
- `current_frame` (version 2 only, required): the frame shown when the project was saved, counted from 0.
- `fps` (version 2 only, required): the playback rate, 1 to 60 frames per second (Anim ▸ Control ▸ Set Rate).
- `spare`: the spare page or `null`; `width`/`height` must match its PNG.
- Unknown fields are an error (a format that adds fields bumps `format_version`); `current_frame` and `fps` are an error in version 1.
- The undo history and the revision counter are not saved.

## Layer images

8-bit indexed PNG (colour type 3, bit depth 8), size equal to the manifest, `PLTE` equal to the manifest palette, no `tRNS`. Pixel indices are the document's indices exactly. Written by `dpx_formats::write_png`.

## Reader rules

The reader refuses, with a short summary line and details (ADR-17 style):

- not a ZIP, a damaged ZIP, a wrong `format`, an invalid manifest or palette;
- `format_version` newer than 2, or 0 or not a number;
- entry names that are empty, absolute, contain `\`, `:` or NUL, or have an empty, `.` or `..` component (path traversal), in the archive and in the manifest; directory entries; duplicate names;
- a missing entry, or an entry that the manifest does not name (the entry set is exactly the manifest plus its images);
- a PNG that is not 8-bit indexed, has another size or palette than the manifest, has `tRNS`, or is damaged;
- limits (`Limits::default`): at most 1024 entries, 256 MiB uncompressed in total (declared sizes are checked first, and the bytes actually read are capped too), 1 MiB manifest, 64 megapixels per image;
- version 2: no frames, a frame without exactly one layer, a missing or out-of-range `current_frame` or `fps`, or more frames than the document limits allow (999 frames, at most 128 MiB of frame pixels: `dpx_core::anim::max_frames_for`; checked before any image is decoded).

## Compatibility

Version 1 files stay readable, and single-frame documents at the default 30 fps are still written as version 1; a single frame with another rate is written as version 2 so the rate is not lost (the test `single_frame_projects_stay_version_1_byte_for_byte` compares with a file written by the version-1 writer), so builds from before ADR-28 read them. Animations are version 2: an older build refuses them with "Project is from a newer version" instead of dropping frames. A later version (e.g. layers) raises `format_version` again; this reader then reports the version instead of guessing.
