# note-submission

The `note-submission` crate defines the typed pipeline contract for agent-note
submissions: the `submissions`, `notes_out`, `upload_attempts`, and
`corpus_controls` records; staging; and the explicit human-promotion gate.

The crate is a substrate, not the placement classifier. Callers provide a
placement decision, and the gate requires a human approval bound to that exact
destination and sensitivity. It fails closed when controls are absent,
disabled, or held, and retains failed attempts for audit. The source/caller
sensitivity ceiling is checked before staging. The crate does not replace the
Turso submission DB: production storage implements `SubmissionStore`; tests
use `MemoryStore` and a simulated destination writer.

Run the package tests with:

```sh
cargo test --manifest-path pkgs/note-submission/Cargo.toml --all-targets
```

Policy-aware classification and the ID-native publication format remain the
responsibility of #7717 and #6062 respectively.
