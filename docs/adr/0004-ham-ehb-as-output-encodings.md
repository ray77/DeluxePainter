# ADR-4: EHB/HAM are output encodings; editing stays indexed or RGBA

- Status: accepted (Phase 0)
- Decision: EHB documents edit 32 base colours with derived half-bright entries 32–63 (read-only, computed). HAM6/HAM8 imports become an RGBA layer plus preserved raw planes for lossless re-export if unchanged; HAM export runs a dedicated encoder with a preview and error report. No "HAM" toggle is offered for plain indexed modes.
- Open: HAM8 control-bit plane position and per-line start colour must be verified with real files (see HARDWARE_MODES.md open points).
