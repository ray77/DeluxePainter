# HARDWARE_MODES: Amiga display modes (OCS / ECS / AGA)

Phase 0 research for DeluxePainter. Written in English; code identifiers and chunk names stay as in the specs.

Confidence levels: **verified** (stated by at least one primary or near-primary source that was actually read), **likely** (consistent across sources or derived from verified facts, but not stated directly), **unverified** (author background knowledge or a single weak source; must be checked before it goes into code).

Source IDs `[S#]` refer to `docs/research/format-sources.md`. Everything below is summarised in our own words.

Scope: OCS (original chipset) and AGA are in scope. ECS is noted only where it introduces something AGA inherits (SuperHiRes, Productivity); it is not a target.

---

## 1. Chipset x resolution overview

| Chipset | Resolution class | Pixel width (page) | Max bitplanes | Max colours (indexed) | Palette depth | Confidence |
|---|---|---|---|---|---|---|
| OCS | LoRes | 320 | 5 normal (+1 plane for EHB/HAM = 6) | 32 (EHB: 64 incl. derived; HAM6: 4096 on screen) | 12 bit (4 bit/channel, 4096) | verified [S5][S6][S8] |
| OCS | HiRes | 640 | 4 | 16 | 12 bit | verified [S5] (16 colours in HiRes); the "4 planes" count is derived (16 = 2^4), likely |
| OCS | SuperHiRes | not available | - | - | - | likely (SHRES arrived with ECS) [S5][S9] |
| ECS (note only) | SuperHiRes | 1280 | 2 (4 colours) | 4 | 12 bit nominal | [S5] says 4 colours from a 64-colour palette; [S9] says "4 bits" / up to 16 colours. **Contradiction**, see Open Points |
| ECS (note only) | Productivity | 640x480 non-interlaced (multiscan only) | 2 | 4 | 12 bit | verified that it exists [S5][S9]; plane count unverified |
| AGA | LoRes | 320 | 8 | 256 (HAM8: 64 base, ~262k reachable; EHB: 64) | 24 bit (8 bit/channel) | verified [S3][S5][S3][S11] |
| AGA | HiRes | 640 | 8 (needs fetch mode >= 2) | 256 | 24 bit | likely: [S11] gives the fetch-mode table only for SuperHiRes; [S5] says AGA palette is 256 of 16.8M "for all monitors" and HAM8 works "in any display resolution" |
| AGA | SuperHiRes | 1280 | 8 (fetch mode 4) | 256 | 24 bit | verified [S11] (planes 5-8 listed, fetch mode 4); HAM8 also listed |

DeluxePainter implication (likely, not a source claim): the editor can model every mode as "N bitplanes (1-8) + palette of 2^N entries + optional HAM/EHB flag", with palette precision 4 bit (OCS) or 8 bit (AGA).

## 2. Default page sizes (non-overscan and overscan)

| Region | Interlace | LoRes (WxH) | HiRes (WxH) | SuperHiRes (WxH) | Confidence |
|---|---|---|---|---|---|
| NTSC | no | 320x200 | 640x200 | 1280x200 | verified [S5][S8][S9] |
| NTSC | yes (LACE) | 320x400 | 640x400 | 1280x400 | likely (doubling stated in [S5], 400 explicitly named; exact SHRES-lace number derived) |
| PAL | no | 320x256 | 640x256 | 1280x256 | verified [S5][S8][S9] |
| PAL | yes (LACE) | 320x512 | 640x512 | 1280x512 | likely (512 named in [S5]) |
| NTSC max overscan (Workbench "visible max") | no | 362x241 | 724x241 | n/a | verified [S5] (laced variants "taller": not given numerically) |
| PAL max overscan | no | 362x283 | 724x283 | n/a | verified [S5] |
| Overscan, other view (hardware limit text) | | 368 / 736 wide, 241/283 lines (483/567 laced) | | | verified [S8]; slight width mismatch with [S5] (362 vs 368), see Open Points |
| AGA example, high | interlaced PAL, overscan | 1440x580 maximum quoted | | | verified as a quoted figure only [S3] |
| Multiscan Productivity | no / yes | 640x480 / 640x960 | | | verified [S5] |

DPaint preset page sizes (e.g. what DPaint IV proposes in its "Page Size" requester) are **unverified** here. Phase 0 should check the PDF manual in `Manuals, Pics` (not touched by this task).

## 3. PAL/NTSC and the pixel aspect ratio

| Item | Value | Confidence |
|---|---|---|
| BMHD stores xAspect:yAspect (UBYTE each); a reader must use the ratio, not assume square pixels | stated in ILBM spec | verified [S2] |
| NTSC LoRes 320x200 (and 640x200 HiRes-non-lace in NTSC conventions) | roughly 10:11 (xAspect=10, yAspect=11) | verified for 320x200 [S2]; the page says "about" |
| Square pixels (Macintosh, many PC images) | 1:1 | verified [S2] |
| PAL LoRes | commonly written as 44:52 in ILBM files | unverified (background knowledge, not found in a read source) |
| HiRes non-interlaced (640x200/256): pixels are tall (about half as wide as lores) | e.g. 10:22 / 22:52 style ratios | unverified |
| HiRes interlaced 640x400/512: close to square | e.g. 10:11 / 44:26 style | unverified |
| SuperHiRes non-interlaced: very tall pixels | ratio derived from clock (35 ns vs 140 ns for LoRes) = 1:4 width relative to LoRes | derived from verified pixel clock facts [S8][S11] (140 ns lores, 70 ns hires, 35 ns SHRES); the xAspect/yAspect numbers written by real software are unverified |

Rule for the editor (design recommendation): store the aspect pair the user's mode implies, write it to BMHD, and fall back to 1:1 if xAspect or yAspect is 0 on import (zero is invalid; handling = likely).

## 4. Interlace

| Fact | Confidence |
|---|---|
| Interlace doubles the vertical resolution (400/512 lines at 15 kHz modes) and flickers on ordinary monitors | verified [S5][S8] |
| LACE flag in CAMG = 0x0004 | verified [S2][S6] |
| AGA "Mode Promotion" (scan-doubling / de-interlacing to ~27 kHz) is an OS feature, off by default, irrelevant to file formats | verified [S5] |

## 5. CAMG and ModeID values

CAMG holds one 32-bit big-endian LONG. Two conventions exist: an old one (only the low 16 bit hold the viewport flags, high word zero) and the Kickstart 2.0+ convention (the whole value is a ModeID). [S2]

### 5.1 Mode-part keys (lower word, default monitor)

| Name | Hex | Confidence |
|---|---|---|
| LORES_KEY | 0x00000000 | verified [S6] |
| LORESLACE_KEY | 0x00000004 | verified [S6] |
| HIRES_KEY | 0x00008000 | verified [S6] |
| HIRESLACE_KEY | 0x00008004 | verified [S6] |
| SUPER_KEY | 0x00008020 (SUPER = HIRES bit + 0x20) | verified [S6] |
| SUPERLACE_KEY | 0x00008024 | verified [S6] |
| HAM_KEY | 0x00000800 | verified [S2][S6] |
| HAMLACE_KEY | 0x00000804 | verified [S6] |
| EXTRAHALFBRITE_KEY (EHB) | 0x00000080 | verified [S2][S6] |
| EXTRAHALFBRITELACE_KEY | 0x00000084 | verified [S6] |
| LORESDPF_KEY | 0x00000400 (dual playfield) | verified [S6]; DPF is out of scope for DeluxePainter |
| HIRESDPF_KEY / SUPERDPF_KEY | 0x00008400 / 0x00008420 | verified [S6] |

HAM8 and EHB-on-AGA use the same HAM / EHB flag bits as OCS; the plane count in BMHD (8 vs 6) distinguishes HAM8 from HAM6. (likely; [S3] says HAM8 is HAM bit + 8 planes in BPLCON0, and no separate CAMG bit was found.)

### 5.2 Monitor part (upper word)

| Name | Hex | Confidence |
|---|---|---|
| DEFAULT_MONITOR_ID | 0x00000000 | verified [S6] |
| NTSC_MONITOR_ID | 0x00011000 | verified [S6] |
| PAL_MONITOR_ID | 0x00021000 | verified [S6][S7] |
| VGA_MONITOR_ID | 0x00031000 | verified [S6] |
| A2024_MONITOR_ID | 0x00041000 | verified [S6] |
| DBLNTSC / DBLPAL monitor IDs | not found in a read source | unverified |

Full ModeID = monitor part OR mode key, e.g. PAL HAM lace = 0x00021000 | 0x00000804. The source stresses that the Amiga OS recommends never decoding ModeIDs bit by bit; for a file importer we nevertheless have to look at a few bits (HAM, EHB, HIRES, LACE, SUPER), which is the de-facto practice. [S6][S2]

### 5.3 Old-style CAMG flags that must be masked off

| Flag | Value | Confidence |
|---|---|---|
| EXTENDED_MODE | 0x1000 | likely (derived: the stated strip mask below equals the sum of the five flags) |
| SPRITES | 0x4000 | likely (same derivation) |
| VP_HIDE | 0x2000 | likely |
| GENLOCK_AUDIO | 0x0100 | likely |
| GENLOCK_VIDEO | 0x0002 | likely |
| Combined strip set | 0x7102 (the "set" the spec says to remove from old-style values); keep mask 0x8EFD | verified [S2] |
| Brush garbage heuristic: high word nonzero while 0x1000 not set in low word => treat as old-style, keep low 16 bit | verified [S2] |
| CAMG missing and nPlanes == 6 => guess HAM (spec statement); EHB cannot be inferred this way and needs CAMG | verified [S2]; our own recommendation to additionally accept EHB when 6 planes and CMAP has exactly 32 entries is a design suggestion (likely, widely used heuristic, not sourced) |

## 6. EHB (Extra Half-Brite)

| Rule | Confidence |
|---|---|
| 6 bitplanes; planes 0-4 give a palette index 0-31; plane 5 set => displayed colour = half brightness of register (index & 31) | verified [S2][S8] |
| Equivalent formulation: registers 32-63 = registers 0-31 at half brightness | verified (implied by the above; the task brief states it as such) |
| Half brightness arithmetic: each 4-bit (OCS) channel shifted right by 1 (value >> 1); for AGA 8-bit channel >> 1 | likely (standard implementation; not stated numerically in a read source) |
| Hardware flag: EHB is active when HOMOD=0, DBLPF=0 and 6 planes (OCS) | verified [S12] |
| AGA: 64-colour mode that is *not* EHB requires KILLEHB (BPLCON2 bit 9). AGA therefore has EHB and a "true 64 colour" variant. | verified [S11]; DPaint IV AGA behaviour for that variant unverified |
| PBM files cannot hold EHB or HAM | verified [S4] |
| In an ILBM the CMAP normally holds only 32 entries for EHB; some writers store all 64 (extra half-bright already computed) | unverified. Importer should accept 32 or 64 entries: if 64, ignore entries 32-63 or, to be safe, use them as-is only when they differ from the computed halves (design suggestion) |

## 7. HAM6 and HAM8 pixel decoding

### 7.1 HAM6 (OCS and AGA, 6 planes)

Per pixel: bits of planes 0-3 = 4-bit data value `d`, bits of planes 4-5 = control `c` (plane 5 is the high bit).

| c (plane5 plane4) | Meaning | Confidence |
|---|---|---|
| 00 | Set: colour = palette[d] (16 registers) | verified [S2][S13][S14] |
| 01 | Modify blue: keep R,G of left neighbour; B = d | verified [S13][S14]; **[S2] (as summarised) lists the codes differently, see Contradictions** |
| 10 | Modify red: keep G,B; R = d | verified [S13][S14] |
| 11 | Modify green: keep R,B; G = d | verified [S2][S13][S14] |

OCS values are 4 bit per channel. When expanding to 8 bit for display, replicate the nibble (0xN -> 0xNN) (likely; consistent with the CMAP advice in [S2]).

### 7.2 HAM8 (AGA, 8 planes)

| Rule | Confidence |
|---|---|
| Per pixel 2 control bits + 6 data bits | verified [S4][S14] |
| The control bits are the two *top* planes (6 and 7) and the data bits planes 0-5 | verified by [S4] ("top two planes select"); **[S11] says the lower two bits are the command, which contradicts [S4] and the HAM6 analogy.** Real AGA files are expected to follow [S4]; must be validated against a real HAM8 ILBM (open point) |
| Set (00): data selects one of 64 palette registers (24-bit colour, 8 bit/channel) | verified [S14][S11] |
| Modify (01/10/11): the 6 data bits replace the **upper 6 bits** of the chosen 8-bit channel; the lower 2 bits of that channel stay as left by the last Set (or by earlier hold) | verified [S14][S11] |
| Same control-code mapping as HAM6 (01 blue, 10 red, 11 green) | likely (HAM8 inherits HAM6 logic; not stated explicitly in a read source) |
| Colours reachable: 262,144 (18 bit) commonly quoted; true count can be higher because of the low 2 bits | verified [S3][S14] |
| Fringing: changing several components needs up to 2 intermediate pixels | verified [S14] |

### 7.3 Per-scanline state

| Question | Answer | Confidence |
|---|---|---|
| What is the "previous pixel" for pixel 0 of each scanline? | Treat it as colour register 0 (the background/border colour). Every scanline restarts from that state; there is no carry-over from the end of the previous line. | likely. [S2] says a first pixel that modifies changes "border register 0", i.e. the state starts from colour 0. [S13] says every line must begin with a base colour from the first 16 registers and that behaviour otherwise is unpredictable. The Hardware Reference Manual node we could reach [S12] does not state it. The widely used decoder convention (start from palette[0] each line) is therefore **likely, not verified against the HRM**. |
| Does the state reset in the left overscan / non-visible part? | Unknown | unverified |
| HAM8: the starting low 2 bits | taken from palette[0] low bits | likely (same logic) |

Recommendation: implement the decoder as `prev = palette[0]` at x=0 of every row; encoder (editor "convert to HAM") must therefore output a Set pixel or use a modify that makes sense relative to palette[0].

## 8. Palette depth: OCS (4 bit) vs AGA (8 bit) in CMAP

| Item | Value | Confidence |
|---|---|---|
| OCS registers: 12 bit (4 bit/channel), 4096 colours | verified [S5][S8] |
| AGA registers: 24 bit, 256 entries | verified [S3][S5] |
| AGA writes colour in two steps: high nibbles, then low nibbles (LOCT bit 9 of BPLCON3); 8 banks of 32 registers via BPLCON3 bits 13-15 | verified [S11] (hardware detail; irrelevant for files) |
| ILBM CMAP stores 8 bit per channel. Old OCS writers left-justify 4-bit values (0x0F -> 0xF0) | verified [S2] |
| Recommended 4->8 bit conversion: replicate the nibble (0xN -> 0xNN) | verified [S2] |
| Detection on import (as hinted by [S2]): if every CMAP byte has all four low bits zero, treat as a 4-bit palette stored left-justified and expand with nibble replication (v | v>>4) | verified as a hint in [S2]; threshold behaviour ("all entries") likely. Edge case: a genuine 8-bit AGA palette that happens to use only multiples of 16 would be mis-detected; mitigated by also looking at CAMG (AGA-only bits/8 planes) |
| Writers that store the value in the *low* nibble (0x0N) instead of 0xNN | unverified (not discussed in read sources; recommend importer heuristic: all bytes <= 0x0F => scale by 17) |

## 9. Summary table for the editor's mode list (proposal)

| Mode | Planes | Colours | Palette | CAMG (PAL/lores example) | Phase |
|---|---|---|---|---|---|
| LoRes 32 | 5 | 32 | 4 bit | 0x00021000 | early |
| LoRes EHB | 6 | 64 | 4 bit | 0x00021080 | mid |
| LoRes HAM6 | 6 | 4096 | 4 bit | 0x00021800 | mid |
| HiRes 16 | 4 | 16 | 4 bit | 0x00028000 \| 0x00021000 | early |
| AGA LoRes 256 | 8 | 256 | 8 bit | 0x00021000 | mid |
| AGA HAM8 | 8 | 262144 | 8 bit | 0x00021800 | mid |
| AGA SuperHiRes | up to 8 | up to 256 | 8 bit | 0x00021000 \| 0x00008020 | later |

(CAMG values are combinations of verified constants; combinations themselves: likely.)

## 10. Open points

1. **HAM scanline start** not confirmed in the Hardware Reference Manual. Obtain the HRM HAM section (Commodore PDF is linked in [S12]'s search results; not read). Until then: palette[0] per line (likely).
2. **HAM8 control-bit location** conflict ([S4] top planes vs [S11] low planes). Validate with a real HAM8 ILBM file from the `Pics` folder or a reference decoder source.
3. **HAM control-code mapping** differs between the summary of the ILBM wiki page [S2] and three other sources [S13][S14][S13][S14]. Re-read [S2] directly or test with a real HAM6 file.
4. **ECS SuperHiRes colours**: [S5] says 4 colours (palette 64?), [S9] says 4 bit / 16. Out of scope but affects AGA SHRES plane limit on OCS-compatible fallback. Check the Hardware Reference Manual.
5. **AGA HiRes plane limit** (8 planes needs fetch mode 2/4 and bandwidth): no read table; [S11] covers only SuperHiRes.
6. **PAL / HiRes / SuperHiRes pixel aspect pairs** as written by DPaint IV: unverified. Inspect BMHD of real DPaint IV AGA files.
7. **DBLNTSC / DBLPAL ModeIDs** and which AGA scan-doubled modes DPaint IV offers: unverified.
8. **DPaint IV page-size presets**: NTSC LoRes read from the manual figure (Standard 320×200, Full Page 320×340, Overscan 368×241; ADR-10). PAL and other modes are derived, not verified.
9. **Overscan width**: 362 [S5] vs 368/736 [S8]; DeluxePainter uses 368/736/1472 (also in DeluxePaint's Set Page Size figure) and even laced heights 482/566 (ADR-11). Still to reconcile with the display database (DisplayInfo clip rectangles).
10. **CMAP nibble variants**: confirm with real OCS-era files that the left-justified (0xN0) form really occurs in DPaint-written files, and how DPaint IV AGA writes its palette.
11. **EHB CMAP length** (32 vs 64 entries written by DPaint IV): check real files.
12. **elowar.com mirror**: the Devices Manual nodes at amigadev.elowar.com have an expired TLS certificate; the content of the ILBM chapter was therefore read from wiki.amigaos.net instead (see sources file).
