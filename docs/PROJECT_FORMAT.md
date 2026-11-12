# `.dpxproj` project format, versions 1 to 3

The lossless save format of DeluxePainter (CLAUDE.md: Save = `.dpxproj`, Export = IFF ILBM / IFF ANIM / PNG). Code: `crates/dpx-formats/src/project.rs`; decisions: ADR-23, animation frames (version 2): ADR-28, layers (version 3): ADR-44.

A document with one frame and the one visible "Background" layer is written as version 1, byte for byte as before animation frames existed; a document with several frames (or another frame rate) and that one layer as version 2; a document with more than one layer, or one layer that is renamed or hidden, as version 3. The reader takes all three.

## Container

A ZIP archive. Entries use deflate (the reader also accepts stored); no other compression, no encryption, no directory entries. The writer is deterministic: fixed entry order, fixed time stamp (1980-01-01 00:00), fixed Unix permissions (0644), so the same project gives the same bytes. The ZIP magic (`PK\x03\x04`) identifies the file; the extension is `.dpxproj`.

Entries, in writing order:

| Entry | Content |
|---|---|
| `manifest.json` | the manifest below (UTF-8 JSON, pretty-printed) |
| `frames/0/layers/0.png` | the page: 8-bit indexed PNG, `PLTE` = the palette |
| `frames/<n>/layers/<k>.png` | layer `k` (from 0, the bottom layer) of frame `n` (from 0): frame by frame, in each frame bottom layer first. Versions 1 and 2 have layer 0 only |
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

A version-3 manifest adds the version-2 fields and the layer stack, for example two frames of two layers:

```json
{
  "format_version": 3,
  "frames": [
    { "layers": [ { "name": "Background", "image": "frames/0/layers/0.png" }, { "name": "Ink", "image": "frames/0/layers/1.png" } ] },
    { "layers": [ { "name": "Background", "image": "frames/1/layers/0.png" }, { "name": "Ink", "image": "frames/1/layers/1.png" } ] }
  ],
  "current_frame": 1,
  "fps": 12,
  "layers": [ { "name": "Background", "visible": true }, { "name": "Ink", "visible": false } ],
  "active_layer": 1
}
```

- `format` is `"dpxproj"`; `format_version` is a whole number from 1. A newer version is refused with an error naming it.
- `app_version` is informational (the writing build).
- `target`: chipset (`ocs`/`aga`), display mode (video `pal`/`ntsc`, resolution `lo`/`hi`/`super_hi`, `interlace`) and screen size in pixels (the reader refuses a screen the mode does not allow, e.g. zero or huge).
- `page`: page size in pixels. It may differ from the screen (the GUI keeps pages at least screen sized, but files from `convert` keep the picture's size).
- `palette`: 1 to 256 `#RRGGBB` strings; the index is the colour number. 8 bits per channel are kept exactly.
- `ranges`: colour-cycling ranges as ILBM CRNG stores them (`rate` 16384 = 60 steps/s, `flags` bit 0 active, bit 1 reverse, other bits kept). Ranges outside the palette are kept.
- `grab`: brush grab point `[x, y]` or `null`. `transparent_color`: palette entry that is transparent in brushes, or `null`.
- `frames`: the animation frames in order, each with its layers bottom first. Version 1 has exactly one frame; versions 2 and 3 one or more (the writer uses version 2 from two frames on). In versions 1 and 2 every frame has exactly one layer; in version 3 every frame has one entry per layer of `layers`, in the same order and with the same names. `layers[].name` is at most 255 characters; `image` is the relative path of the entry. All frames and layers have the page size.
- `current_frame` (versions 2 and 3, required): the frame shown when the project was saved, counted from 0.
- `fps` (versions 2 and 3, required): the playback rate, 1 to 60 frames per second (Anim ▸ Control ▸ Set Rate).
- `layers` (version 3 only, required): the layer stack shared by every frame, bottom first, 1 to 32 entries. `name`: at most 255 characters; `visible`: whether the layer shows (layers are shown or hidden, nothing in between). On every layer above the bottom one, colour 0 is transparent; the bottom layer is opaque. A version-1 or version-2 file has one visible layer named as its frames' layer (always "Background" from DeluxePainter).
- `active_layer` (version 3 only, required): the layer that was being painted when the project was saved, counted from 0 (the bottom layer).
- `spare`: the spare page or `null`; `width`/`height` must match its PNG.
- Unknown fields are an error (a format that adds fields bumps `format_version`); `current_frame` and `fps` are an error in version 1, `layers` and `active_layer` in versions 1 and 2.
- The undo history and the revision counter are not saved.

## Layer images

8-bit indexed PNG (colour type 3, bit depth 8), size equal to the manifest, `PLTE` equal to the manifest palette, no `tRNS`. Pixel indices are the document's indices exactly; the transparency of colour 0 above the bottom layer is a rule of the document, not stored in the PNG. Written by `dpx_formats::write_png`.

## Reader rules

The reader refuses, with a short summary line and details (ADR-17 style):

- not a ZIP, a damaged ZIP, a wrong `format`, an invalid manifest or palette;
- `format_version` newer than 3, or 0 or not a number;
- entry names that are empty, absolute, contain `\`, `:` or NUL, or have an empty, `.` or `..` component (path traversal), in the archive and in the manifest; directory entries; duplicate names;
- a missing entry, or an entry that the manifest does not name (the entry set is exactly the manifest plus its images);
- a PNG that is not 8-bit indexed, has another size or palette than the manifest, has `tRNS`, or is damaged;
- limits (`Limits::default`): at most 31 970 entries (999 frames of 32 layers, the manifest and the spare page), 256 MiB uncompressed in total (declared sizes are checked first, and the bytes actually read are capped too), 1 MiB manifest, 64 megapixels per image;
- versions 2 and 3: no frames, a missing or out-of-range `current_frame` or `fps`;
- versions 1 and 2: a frame without exactly one layer;
- version 3: a missing `layers` or `active_layer`, no layers or more than 32, an `active_layer` outside them, a frame without exactly one entry per layer, or an entry named otherwise than its layer;
- more frames than the document limits allow: 999 frames, at most 128 MiB of pictures (frames times layers times page pixels: `dpx_core::anim::max_frames_for_layers`); checked before any image is decoded.

## Compatibility

Version 1 files stay readable, and single-frame documents at the default 30 fps with the one visible "Background" layer are still written as version 1; a single frame with another rate is written as version 2 so the rate is not lost (the test `single_frame_projects_stay_version_1_byte_for_byte` compares with a file written by the version-1 writer), so builds from before ADR-28 read them. Animations of one layer are version 2, as since ADR-28, so builds from before ADR-44 read them. Projects with layers are version 3: an older build refuses them with "Project is from a newer version" instead of dropping layers. A later version raises `format_version` again; this reader then reports the version instead of guessing.
