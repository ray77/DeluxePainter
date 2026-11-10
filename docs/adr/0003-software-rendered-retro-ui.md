# ADR-3: Whole UI software-rendered into a low-res framebuffer; egui is only the host

- Status: accepted (Phase 0)
- Context: the classic look needs pixel-exact bitmap fonts, bevels and icons; egui widgets are anti-aliased and modern.
- Decision: `dpx-ui` renders menu bar, toolbox, palette, requesters and the zoomed canvas viewport into one RGBA8 framebuffer at virtual Amiga-like resolution. It is uploaded as an egui texture with `TextureOptions::NEAREST` and drawn at an integer scale snapped to physical pixels (letterboxed). Input is mapped physical → UI pixel → canvas pixel with integer maths.
- Update (ADR-13): the window snaps to the screen in whole and half pixel-size steps, so there are no bars in a window; only full screen centres the screen with a dark border.
- Consequences: deterministic, headless-testable UI screenshots; no blur at any DPI; we must build our own small widget set and an original bitmap font.
