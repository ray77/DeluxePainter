# ADR-27: Save writes the project, Export writes the picture

- Status: accepted (Phase 4 start, 2026-11-10; decided while the owner was away, following CLAUDE.md "Save = .dpxproj, Export = IFF ILBM / IFF ANIM / PNG")
- Context:
  - Picture ▸ Save.. wrote IFF ILBM (or PNG by name). With the lossless `.dpxproj` format (ADR-23) in place, saving a picture as ILBM would make the Amiga file the working file, which CLAUDE.md rules out.
- Decision:
  - **Picture ▸ Save..** (Amiga-S) opens the "Save Project" requester and always writes `.dpxproj`. A picture extension (`.iff`, `.ilbm`, `.lbm`, `.png`, `.jpg`, `.jpeg`) becomes `.dpxproj`; any other name gets `.dpxproj` appended, because Amiga names often have no extension (`Venus` → `Venus.dpxproj`). The suggested name is the current picture's name as a project. After saving, the project is the picture's name.
  - **Picture ▸ Export..** (new item below Save.., no shortcut) opens the "Export Picture" requester with an "Export" button. It writes IFF ILBM, or PNG for a `.png` name, with the same notes as before (what the format cannot store) and the same overwrite question. A `.dpxproj` name is refused ("Use Save.. for projects"). The suggested name is the picture's own name, or for a project the same name with `.iff`. Exporting does not change the picture's name.
  - **Load..** opens projects, ILBM, PNG and JPEG as before.
  - The spare page, undo history and tool settings are not saved, like DeluxePaint's Save. The format can hold a spare page (ADR-23); wiring it waits for an owner decision.
- Deviations from DeluxePaint IV: DeluxePaint has no Export item; its Save writes ILBM.
- Verified: app test (Save suggests `Castle.dpxproj`, writes it and the project loads back pixel-identical; Export suggests `Castle.iff`, asks before replacing, writes ILBM and PNG, refuses `.dpxproj` and `.jpg`; the picture keeps the project name), `project_path` cases; CI gate. Not tried by hand in the window.
