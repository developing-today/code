# Receipts, scans and late notes (S3)

A receipt says what happened to one note. There is exactly one per note, written by the receiver (or by the reconciler on its behalf, who then records its own name), created exclusively, and never edited. `resolve-note` writes it; the receipt template is in `planning/workflow/templates/`.

## The five dispositions

Pick the one that says what happened. When two seem to fit, pick the one that asks least of the next reader.

- **`incorporated`**: you changed the concept document because of the note. Target: the item or heading that now holds the change. Reason: what changed, in one sentence, and the evidence you checked.
  Example reason: "Added C002.6.1 under C002.6: the lease lifetime is 20 minutes, verified in collaboration.md at 4063f248."
- **`acknowledged`**: the note is informational, or it is already reflected, so the document needs no change. Target: where the document already covers it, or the `Cross-concept notes` section. Reason: why no change is needed.
  Example: "Already covered by C002.3; the note confirms it with a second source."
- **`declined`**: you considered the suggestion and will not adopt it. Target: the section holding the reasoning. Reason: why, with evidence. Send a reply note when the sender's own work depends on the answer.
  Example: "Declined: per-note files avoid the append conflicts that a shared log causes (review-parallel-ops scenario 9); see C002.4."
- **`deferred`**: valid, but not for now. Target: the open question or item where it waits. Reason: why it waits, and a revisit trigger written as "Revisit when: <event>", such as a decision, a merge or a stage.
  Example: "Needs the S6 storage decision. Revisit when: the decision record for storage is confirmed."
- **`superseded`**: a later note, a later finding, or a change in the sender's own concept made this note moot. Target: the superseding note or item. Reason: what replaced it.

A note asking for a change in a file you do not own (another concept, `design.md`, `AGENTS.md`) cannot be `incorporated` by you. Use `acknowledged` when you only record it, or `deferred` with a trigger, and then either send a note to that file's owner or open a PR for a wide file, and link it from the reason.

## Target rules

- The target is a link inside the package: `concepts/C001-<slug>.md#<heading>`, `main.md#<heading>`, a note or a decision record.
- It must resolve. `check` validates links and heading anchors; run it after writing.
- Point at the item or heading that changed, not the top of the file.

## Reason rules

- One or two sentences. Say what you did and why, with the evidence: a source number, a `path:line` at a SHA, or a command and its result.
- Never restate the note; link it.
- A note's claim is a report. Re-measure it before you incorporate it, and say in the reason that you did.

## Scan record

The concept file's `## Reconciliation` section holds one entry per scan, add-only:

```markdown
- Scan 2026-10-21T12:00:00Z by notes-reconcile-C002: 14 refs on origin.
  - `origin/main` 4063f248
  - `origin/wf/pkg/C001-s2` 1a2b3c4d
  - Pending found: 1 (imported from `origin/wf/pkg/C001-s2` at 1a2b3c4d: `notes/C001__to__C002__<id>.md`)
```

List every ref scanned, not only the ones with notes, so a later reader can prove what was and was not visible. The final scan of a run says "Pending found: 0".

## Imported notes

A note on an unmerged branch is imported byte for byte with `git checkout <ref> -- <path>`. The file stays the sender's: you neither edit nor reformat it (do not run a formatter over `notes/`). When the sender's branch merges, git sees the same file added on both sides and merges it cleanly. If `check` reports a body-hash mismatch, the note was edited after it was written: do not import it; record the problem in the Reconciliation section and send the sender a note.

## Late notes

You do not wait for notes. A note committed after your final scan is late: its ref was either not on origin at the recorded time or pointed at a different SHA. The router finds it with `check --remote-notes` and starts a new notes-reconcile run on a new branch, because the earlier branch is dead once merged. That run adds a new scan entry and new receipts; it changes nothing the first run wrote.

## What never happens

- Editing, moving, reformatting or deleting a note.
- Editing or replacing a receipt.
- Writing a receipt for a note addressed to another concept.
- Declaring the concept reconciled while `check --require-resolved` fails or the last scan found pending notes.
