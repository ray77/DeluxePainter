# ADR-40: Responsive debug builds and low-latency presentation

- Status: accepted (2026-11-11; decided without the owner after "the UI still feels a bit sluggish. VSync?")
- Context:
  - The owner runs the app with `cargo run`, which used the plain dev profile: every crate, including egui, wgpu, png and zip, at opt-level 0.
  - Measured through `PainterApp::ui` (Waterfall example, 320×200 screen, 300 passes each, CPU only, no GPU):

    | per pass | dev, opt 0 | dev, this ADR | release |
    |---|---|---|---|
    | freehand drag | 6.5 ms | 0.8 ms | 0.26 ms |
    | colour cycling | 2.0 ms | 0.2 ms | 0.08 ms |

    The cost stays the same over a long stroke (no growth with stroke length). The real window adds egui's and wgpu's own work, which was also unoptimized, so a 60 Hz frame (16.7 ms) could be missed while drawing.
  - eframe 0.37 presents with VSync and `SurfaceConfig::HIGH_THROUGHPUT` (up to two queued frames). egui documents `SurfaceConfig::LOW_LATENCY` (VSync, one queued frame) as the good default for GUIs with little GPU work. DeluxePainter draws one textured quad per frame.
- Decision:
  - `[profile.dev] opt-level = 1` for our crates (full debug info kept) and `[profile.dev.package."*"] opt-level = 3` for all dependencies. The first dev build takes longer once; later builds only recompile our crates. Release is unchanged.
  - `NativeOptions.wgpu_options.surface = SurfaceConfig::LOW_LATENCY`: VSync stays on (no tearing in cycling or playback), one frame less input lag.
- Verified:
  - The table above (temporary timing test, removed again).
  - The window starts and runs with the new surface settings (macOS); the gate (fmt, clippy, all tests) passes in the new dev profile.
  - Not measured: end-to-end input latency on screen (needs a camera or an OS-level probe), and Windows/Linux presentation (CI only builds and tests there).
