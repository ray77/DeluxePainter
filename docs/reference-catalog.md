# Reference catalog

Local reference material lives in `Manuals, Pics/` next to the repository root. That folder is **read-only reference input** and is **excluded from the public repository** (see `.gitignore`): the manuals and screenshots show Electronic Arts' copyrighted software and documentation. They may inform our own implementation but must not be redistributed. The only asset from that folder that ships with the project is the app logo, released for public use by the project owner.

Inventory date: 2026-11-09.

## Documents

| ID | File | Content | Notes |
|---|---|---|---|
| DOC-1 | `Deluxe-Paint-4-AGA-Manual-ENG.pdf` | DeluxePaint IV **AGA** (v4.5) manual, English, 321 pages, scanned images (no text layer) | **Primary design reference.** OCR'd locally with tesseract for analysis (text not stored in repo). Findings: `docs/research/manual-ui-notes.md` |
| DOC-2 | `DeluxePaintIV.pdf` | DeluxePaint IV (v4.0, pre-AGA) manual, 304 pages, scanned | Secondary; used to identify 4.0 vs 4.5 AGA differences |
| DOC-3 | `DELUXEPAINTER_CLAUDE_BAUPLAN.md` | Project master plan (German) | Requirements source; summarised in English in `docs/FEATURES.md` / `docs/ROADMAP.md` |
| DOC-4 | `CLAUDE.md` | Short agent brief (German) | Superseded by repo-root `CLAUDE.md` (English) |

## Images

| ID | File | Size | Version shown | What it shows | Use |
|---|---|---|---|---|---|
| IMG-1 | `DeluxePainter.png` | 709×685 | — | **Project logo**: pixel-art golden pharaoh mask in a round gold frame, blue/gold nemes stripes, laurel wreath, small pen nib. Transparent background. | App icon / README logo (owner-approved). Needs icon-size derivatives (16–1024 px) and a check for legibility at 16/32 px. |
| IMG-2 | `Choose_Setup.png` | 560×395 | **4.5 AGA** | "Choose Display Mode" startup requester: mode list (PAL: High Res 640×256, High Res Laced 640×512, Low Res 320×256, Low Res Laced 320×512, Super-High Res 1280×256 …), selected mode line, Display Information panel (Type, Max Colors 256 of 16777216, Max Size 736×290, genlock, double-buffering, "Advanced Information" toggle), Screen Size (Standard + W/H fields), Page Size (Screen + W/H), Palette Size slider (256), Retain Picture checkbox, Credits panel, Use / Cancel | Reference for AGA screen-mode requester |
| IMG-3 | `Inapp_gfx_01.png` | 590×416 | **4.5 AGA** | About requester ("About DeluxePaint IV AGA", version, memory chip/fast, Anim/AnimBrush/Brush size, Ok). Idle menu bar `DeluxePaint  Color`, toolbox, AGA palette with page selector | About dialog, idle screen |
| IMG-4 | `Inapp_gfx_02.png` | 800×600 | **4.5 AGA** | Clean main screen (640×480-ish page inside 800×600 capture): idle menu bar, crosshair cursor, toolbox (2 columns × 10 rows + brush block), FG/BG bar, **multi-column AGA palette** with `◂ a ▸` page selector beneath | Primary layout reference for toolbox + palette |
| IMG-5 | `inapp_gfx_03.png` | 640×512 | 4.x (Topaz-style font; AGA not certain) | Picture menu open: Load.. / Save.. / Delete.. / Print.. with Amiga-key shortcuts (L, S, D, P), Flip ▸, Spare ▸, Page Size.., Show Page (S), Screen Format.., About.., Quit (Q). Menu titles: Picture, Brush, Mode, Anim, Color, Effect, Prefs | Menu bar + dropdown style, shortcut rendering |
| IMG-6 | `Inapp_gfx_04.png` | 640×512 | **4.0** | "About Deluxe Paint IV" v4.0, idle bar `DeluxePaint  Matte` | Version comparison only |
| IMG-7 | `Inapp_gfx_05.png` | 640×512 | 4.0 (probably) | **Spacing** requester: N Total (20), Every Nth dot (8), Airbrush (16), Continuous (selected, drawn highlighted), Cancel / Ok | Requester widget style (buttons, numeric fields, selected state) |

### Known artefacts in the screenshots

- **Rounded window corners** visible in some screenshots come from the capture/preview source. **The original UI has square corners everywhere** — menus, requesters, buttons and the screen itself. DeluxePainter draws only square, 1-pixel-bevelled shapes.
- Some captures are scaled with bilinear filtering (soft edges, colour fringes). Do not sample colours or pixel metrics from them blindly; prefer pixel-exact measurements from 640×512 captures (IMG-5/6/7).
- IMG-1 and IMG-4 are not native Amiga resolutions (letterboxed / rescaled).

## Video

| ID | Source | Notes |
|---|---|---|
| VID-1 | https://www.youtube.com/watch?v=F3csY10bVZI | Saddleback Graphics VHS tutorial for **DPaint IV v4.0** (not AGA). Analysis: `docs/research/video-ui-analysis.md` |
