# Documentation Protocol for Design & Architecture Decisions

When making a significant design or architecture decision—whether modifying an existing component or introducing a new feature that changes how things operate—**document first, then implement**.

## When to Document

- New features or components that affect system behavior
- Architectural changes or refactors
- Design decisions with trade-offs worth recording
- Changes to existing components that alter their interface or semantics

## Initial Documentation

1. **Create a docs folder** for the change:

   ```
   docs/<UTC_RFC_DATETIME>_<kind>_<name>/
   ```

   - `<UTC_RFC_DATETIME>`: e.g., `2026-03-16T14-30-00Z`
   - `<kind>`: `feature`, `architecture`, `refactor`, `design`, `component`, etc.
   - `<name>`: descriptive snake_case name

2. **Create the initial document** with the same naming:

   ```
   docs/2026-03-16T14-30-00Z_feature_blob_streaming/2026-03-16T14-30-00Z_feature_blob_streaming.md
   ```

3. **Document the request/intent and initial plan** before implementing:
   - What was requested or identified as needed
   - Initial design approach
   - Key decisions and their rationale

4. **Append updates during rollout** as new sections:
   - Add a page break (`---`) before each appended section
   - Start with a header: `## <UTC_RFC_DATETIME> <type>: <brief description>`
   - Types: `Update`, `Clarification`, `Deviation`, `Discovery`, `Decision`
   - Example: `## 2026-03-17T10-00-00Z Update: Switched to async streaming`
   - Content: modifications, clarifications, edge cases, deviations from plan

## Post-Rollout Updates

After initial rollout is complete:

- **If significantly different or many updates**: Create a new file in the same folder with a new datetime and clarifying suffix:

  ```
  2026-03-18T09-00-00Z_feature_blob_streaming_final_design.md
  ```

- **Returning in a new session with major changes planned**: Create a new datetime document with a suffix explaining the revision type:

  ```
  2026-03-25T11-00-00Z_feature_blob_streaming_v2_proposal.md
  ```

- **Short updates**: Files can be brief notes, updates to specific parts, or complete re-summarization of current/proposed design

## File Immutability

- **Do not modify files** after initial creation (except appending during active rollout)
- **After some time**, stop appending—create new files instead
- **Preserve historical record**: Files represent the state of understanding at that point in time

## Handling Superseded Features

If a new feature replaces or subsumes an old documented feature:

1. Create a new folder based on the new feature's creation date
2. Reference the old feature's folder in the new documentation
3. Add a note file in the old feature's folder backlinking to the new one
   - This may not be a 1:1 replacement—could indicate a shift in direction
   - Old feature may be deprioritized over time, or not

## Noticed Discrepancies

If the latest summary + subsequent notes are out of date with the actual implementation:

- Create a TODO to provide an update datetime file
- Cover at minimum: noticed differences, understanding of intent/implications, timeline if known

## Example Structure

```
docs/
├── 2026-03-10T08-00-00Z_feature_meta_protocol/
│   ├── 2026-03-10T08-00-00Z_feature_meta_protocol.md          # Initial design
│   ├── 2026-03-12T14-00-00Z_feature_meta_protocol_revised.md  # Post-rollout summary
│   └── 2026-03-20T10-00-00Z_note_superseded_by_v2.md          # Backlink to replacement
├── 2026-03-20T09-00-00Z_feature_meta_protocol_v2/
│   └── 2026-03-20T09-00-00Z_feature_meta_protocol_v2.md       # References old folder
```
