---
name: request-intake
description: Store raw transcripts byte-for-byte, verify SHA-256 integrity, segment ideas, and scaffold initial research collections. Use when capturing a new request, prompt, or noisy voice transcription.
---

# Request Intake (Stage 0)

Scaffold a sealed, evidence-preserving research collection from raw user input. Specifications live in [/planning/workflow/stages.md](/planning/workflow/stages.md) and [/planning/workflow/records.md](/planning/workflow/records.md).

## Procedure

1. **Save Raw Input**:
   Save the exact input bytes without normalization or whitespace changes to a temporary file `/tmp/raw-input.txt`.

2. **Compute Checksum**:
   Compute `sha256sum /tmp/raw-input.txt` to verify byte preservation.

3. **Draft Concept Outline**:
   Inspect the input and map every nonblank section to concepts (`C001`, `C002`, ...), recording source line ranges. Include a "Garbled Input Reading Register" if phonetic or speech-to-text artifacts are present.

4. **Scaffold Package**:

   ```sh
   python3 scripts/research_workflow.py init --input /tmp/raw-input.txt --slug <kebab-slug> --outline <outline.json> --expect-sha256 <hash>
   ```

5. **Verify Package**:
   ```sh
   python3 scripts/research_workflow.py check --package doc/<package-dir>
   ```

## Turn Report

End every turn with:

- Done: <intake summary and package path>
- Verified: <sha256 and check command output>
- Remaining: <handoff to research-prepare>
- Questions (non-blocking): <question -> default>
- Blocked on user: <blocker or none>
