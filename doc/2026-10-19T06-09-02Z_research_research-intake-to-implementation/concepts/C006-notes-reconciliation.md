# C006: Asynchronous Notes Reconciliation and Receipts

[Main reference](../main.md#c006)

## Relevant transcript excerpts

<!-- workflow:excerpt C006 33 33 -->

[Transcript lines 33-33](../transcript.txt#L33-L33)

```text
After this skill there should be another which first goes through all of the meta files (like this-to-that files and such) and ensures that all comments from other agents were appropriately transferred into the document or at least acknowledged. The other skill should be doing this as it runs but once the document's done, it doesn't need to wait around and it will end. This skill should fully sync and ensure that all notes in this or for this file are incorporated.
```

## Normalized purpose

Process all directed notes sent to a concept, recording atomic resolution receipts and incorporating findings before handoff.

## Initial thoughts

- Every note must be explicitly incorporated, acknowledged, declined, or superseded with rationale.
- Reconciliation must scan remote branches to catch notes arriving from parallel workers.

## Inferred suggestions

- Inference: Use atomic file creation (O_EXCL) for receipt generation to prevent duplicate dispositions.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
