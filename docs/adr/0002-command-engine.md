# ADR-2: Single serialisable Command API for GUI, CLI and MCP

- Status: accepted (Phase 0)
- Decision: all mutations go through `Engine::execute(Request{document_id, expected_revision, dry_run, command})`. Commands are serde + schemars types, validated before mutation, recorded for undo (tile snapshots), grouped into all-or-nothing transactions. Interactive strokes are previewed in an overlay and committed as one command on mouse-up.
- Consequences: MCP tool parameters are the command structs; no duplicate pixel logic; revision checks prevent lost updates between GUI and agents.
