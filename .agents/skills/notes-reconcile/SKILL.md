---
name: notes-reconcile
description: "Close out the directed notes for one concept of a workflow research package at stage S3: gather every note addressed to it on the current branch and on remote refs (check --remote-notes), write one receipt per note (incorporated, acknowledged, declined, deferred or superseded, each with a target and a reason), bring accepted changes into the concept document, record which refs and SHAs were scanned, and finish without waiting for late notes, handing the concept to issue-handoff. Use after concept-research finishes a concept, when notes to a concept are pending, or to make sure every comment from other agents reached its document. For sending or answering a note while research is still running use concept-research; for the GitHub handoff issue use issue-handoff."
---

# Notes reconcile (S3)

You close out one concept's notes. Every note any agent sent to this concept ends with exactly one receipt, the concept document reflects what was accepted, and the scan you made is on record so a later note is provably late, not lost. Note and receipt formats are in [records.md](../../../planning/workflow/records.md); the stage contract is in [stages.md](../../../planning/workflow/stages.md).

Commands below are `python3 -I scripts/research_workflow.py <command>`, run from the repository root; `<pkg>` is the package folder `doc/<UTC>_research_<slug>`. Run `<command> --help` for argument names.

## Load or not

Load for:

- reconciling the notes sent to a concept after its research PR merged;
- "are all the notes to C002 resolved?" or "make sure every comment from other agents reached C002";
- a router report that a concept has pending notes, including notes found only on remote branches.

Do not load for:

- writing a note, or answering one while research is still running: `concept-research`;
- the research itself: `concept-research`;
- the parent handoff issue that follows: `issue-handoff`;
- review comments on a pull request: answer them in the PR;
- a decision between two concepts' conflicting proposals: `design-orchestrate`.

## Ownership and parallelism

One agent per concept runs this stage, and the concepts reconcile in parallel. You are the concept's current owner: the only writer of its concept file and of the receipts for notes addressed to it. Notes are immutable and belong to their senders; you never edit, move or delete one. A change another concept must make goes back to it as a new note. `main.md` changes only inside this concept's section and only through your PR. Boot, lease, heartbeat and final-report rules are in [collaboration.md](../../../planning/workflow/collaboration.md).

## Steps

1. **Check the entry condition and boot.** The concept's S2 research PR is merged into `main` (`status --live --package <pkg>` shows it). If it is still open, stop: report "blocked: S2 PR <url> not merged" and leave the concept to the router. Otherwise boot per collaboration.md on your own branch (the brief's name, else `research/<slug>/s3-c001`), push it empty, take the concept lease with `lease acquire`, and append `start` to your live log.
   Done when the S2 PR is merged, your branch is pushed and `lease list` shows your lease.
2. **Gather every note.** `git fetch origin`, then run `check --package <pkg>` for unresolved notes on your branch and `check --package <pkg> --remote-notes` for notes addressed to this concept that sit only on `origin/*` refs. Record the scan in the concept file's `## Reconciliation` section: the UTC time and every scanned ref with its SHA (`git for-each-ref refs/remotes/origin --format='%(refname:short) %(objectname)'`).
   Done when you have one list of every note addressed to the concept, each marked local or remote with its ref, and the scan is recorded.
3. **Bring remote notes in.** A note that exists only on another branch cannot be resolved here. Import it byte for byte with `git checkout <ref> -- <pkg>/notes/<note-file>`; the identical file merges cleanly when the sender's branch lands. Run `check --package <pkg>` to confirm its body hash, and record "imported from `<ref>` at `<sha>`" in the Reconciliation section.
   Done when every note from step 2 is on your branch and `check` accepts each one.
4. **Decide each note.** Read the note, re-check its claims (a note is a report, not a source), and choose one disposition: `incorporated`, `acknowledged`, `declined`, `deferred` or `superseded`. Read `references/receipts.md` for when each applies and what its target and reason must say. Incorporate accepted changes the way concept-research does: the refinement goes directly below what it refines (`item add`, then `item set-status` on the earlier item), cited, and nothing earlier is deleted. A note asking for a change you do not own gets `acknowledged` or `deferred`, plus a reply note or a PR, as `receipts.md` says.
   Done when every note has a chosen disposition and every `incorporated` change is in the document.
5. **Write the receipts.** One per note: `resolve-note --package <pkg> --note <pkg>/notes/<note-file> --disposition <disposition> --target <link inside the package> --reason "<reason>"`. Creation is exclusive, so a note that already has a receipt (for example from S2) keeps it: skip it and list it. Receipts are never edited; if your view changes later, record it as a new item in the document and, if the sender must act, send a new note.
   Done when `check --package <pkg> --require-resolved` exits 0.
6. **Sync the document.** In `## Cross-concept notes`, list every received note with its sender, disposition and a link to its receipt. Set the state block's `stage` to `S3` and append a `history` entry; add a change-log line in the concept file and one line to `changes.md`.
   Done when every note in `notes/` addressed to the concept appears in the document with a receipt link.
7. **Final scan, then finish.** Fetch and run `check --package <pkg> --remote-notes` once more just before pushing. Handle any new note with steps 3 to 6. Then stop: do not wait or poll for notes that may come later. A note that arrives after your recorded scan is post-reconciliation, and the router sends it to a new notes-reconcile run.
   Done when the final scan is recorded and lists nothing pending.
8. **Publish and hand off.** Format your files (`prettier --write`), run `check --package <pkg>` and `check --changed-since origin/main`, merge `origin/main` into your branch, push, open a ready PR into `main`, record it in the state block, `lease release`, and append `end`.
   Done when `git ls-remote origin <branch>` equals the PR head and the reported SHA, and the turn report names `issue-handoff` mode `parent` as next.

## References

- `references/receipts.md`: the five dispositions, target and reason rules, the deferred revisit trigger, the scan record, imported notes and late notes. Read before step 4.

## Turn report

End every turn with:

```text
Done: <notes found, receipts written by disposition, notes imported from remote refs>
Verified: <check --require-resolved -> exit 0>, <check --remote-notes -> none pending>, ...
Remaining: <"issue-handoff mode parent for C001 after PR <url> merges", or what is left>
Questions (non-blocking): <question -> default being used>, or "none"
Blocked on user: <only what nothing else can proceed without>, or "none"
```

The final turn also gives the branch, the PR URL, the last pushed SHA and the scan time with its ref count.
