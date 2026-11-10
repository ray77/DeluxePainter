# ADR-25: EHB and HAM6/HAM8 codecs (library only)

- Status: accepted (Phase 5 groundwork; not wired into the app)
- Context: ADR-4 makes EHB/HAM output encodings. The app still refuses HAM/EHB files (unchanged); this ADR covers the pure codecs in `dpx-formats`.
- Decisions:
  - `dpx_formats::ehb`: `ehb_palette`, `half_bright`, `decode_ehb`. Entries 32-63 = base >> 1 per channel; on OCS precision the shift is applied to the 4-bit value and re-expanded (0xN -> 0xNN), on AGA to the 8-bit value.
  - `dpx_formats::ham`: `decode_ham`, `encode_ham`, `choose_base_palette`, `convert_to_ham`. Input/output is the chunky plane value (top two planes = control, rest = data) as the ILBM reader produces it. Control 01 = blue, 10 = red, 11 = green. HAM6 modify: 4 data bits, nibble replication. HAM8 modify: 6 data bits replace the upper 6 bits, low 2 bits kept. Each row starts from palette[0].
  - Encoder: greedy per pixel, squared RGB error, candidates Set (nearest palette entry) and modify R/G/B (best data value); a modify wins only if strictly better (ties prefer Set, then R, G, B). Alpha is ignored. A report gives mean/max Euclidean error and a count above a caller threshold. Base palette: existing median-cut `quantize::reduce` (OCS precision for HAM6), padded with black to 16/64.
- Verified: hand-computed unit tests (set, modify R/G/B, line reset, HAM8 low-bit hold, EHB values), HAM-legal round trip with zero error, determinism. Encode time 320x256 (release, synthetic gradient, quantiser included): HAM6 about 11 ms, HAM8 about 21 ms. Example gradient quality: HAM6 mean error about 8, HAM8 about 2 (Euclidean RGB).
- Not verified: HAM8 control bits against a real HAM8 file (top planes per [S4] assumed; [S11] disagrees); control-code mapping and per-line start colour against real files/HRM; behaviour of AGA "64 colour without EHB"; no real HAM/EHB file was available (the ILBM reader still refuses them, so no raw plane access). The greedy encoder does not look ahead (no fringing optimisation).
- Next: the ILBM reader/writer must expose raw plane values for HAM/EHB, then app wiring.

- Addendum (ADR-26): the ILBM reader now hands over raw plane values and Load opens EHB/HAM; the "still refuses" remarks above are superseded.
