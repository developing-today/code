# note-submission

The `note-submission` crate defines the typed pipeline contract for agent-note
submissions: the `submissions`, `notes_out`, `upload_attempts`, and
`corpus_controls` records; staging; and the explicit human-promotion gate.

Placement classification is built in (#7717): `classify` applies the #6022
decision table to a submission's declared class, a caller-supplied routing
context (registry-selected Zettel corpus, owning repo, explicit tier request,
§5 ID-native eligibility evidence), and the submission's sensitivity ceiling,
and fails closed — the submission stays unpublished — whenever classification
or sensitivity is uncertain. Callers may still provide a placement decision
directly, and either way the gate requires a human approval bound to that
exact destination and sensitivity. It fails closed when controls are absent,
disabled, or held, and retains failed attempts for audit. The source/caller
sensitivity ceiling is checked before staging. The crate does not replace the
Turso submission DB: production storage implements `SubmissionStore`; tests
use `MemoryStore` and a simulated destination writer.

Run the package tests with:

```sh
cargo test --manifest-path pkgs/note-submission/Cargo.toml --all-targets
```

A submission may also declare its placement class and intended Atlas tier
from the §2 decision table of the agent-note placement design (#8204):
`submit_with_declaration` records the declaration on the envelope, the
decision table supplies the class's home and default tier when no tier is
intended, and the staged record shows the effective declaration to the
promoter. The declaration also drives the #7717 classifier: `classify`
combines it with the routing context and the sensitivity ceiling to produce
the placement decision (or a fail-closed refusal), and `stage_classified`
stages a routed decision for the same human approval as before. Undeclared
submissions keep the caller-supplied-decision behaviour exactly.

The ID-native publication format remains #6062's responsibility; the
classifier consumes its §5 eligibility boundary and the #5545 writer-set
authority without redefining either.
