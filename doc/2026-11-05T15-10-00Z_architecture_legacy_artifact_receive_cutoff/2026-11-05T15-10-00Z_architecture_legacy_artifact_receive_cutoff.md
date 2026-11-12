# Legacy artifact receive cutoff

See [the implementation plan](../../.opencode/plans/2026-11-05T15-10-00Z-legacy-artifact-receive-cutoff.md).

Request: implement the accepted ruling on [#4459](https://github.com/developing-today/code/issues/4459) so peer pulls can continue across pre-upgrade `home_declared` artifacts that violate the #3187 world-ID rules.

## Intent and initial plan

The cutoff is trusted artifact/journal format version, not peer provenance. Artifacts before the strict format remain chain history: accept only the scoped legacy `home_declared` case needed for continuity and surface that leniency as a recorded/typed note. Artifacts at or after the cutoff use the full current parser. Invalid signatures, malformed envelopes, chain conflicts, and genuinely corrupt entries remain hard errors at every version.

For artifacts without a version marker, use the author's chain position relative to that author's first strict-version artifact: earlier entries are legacy; the first strict artifact and later entries are strict. Keep the exception local to peer receive; do not reuse local replay leniency wholesale or relax new receipts.

Implementation will keep the public pull path non-halting for the accepted legacy case and add regression coverage that exercises the peer-pull path, while retaining a strict-version rejection and corrupt-artifact rejection oracle. The implementation detail for carrying/versioning the marker will follow the existing signed-artifact and journal contracts discovered in code.

## References

- [#4459 ruling](https://github.com/developing-today/code/issues/4459#issuecomment-6634862794)
- [#3187 strict-receipt PR #4432](https://github.com/developing-today/code/pull/4432)
- `pkgs/id/src/artifact.rs`
- `pkgs/id/src/directory.rs`
- [Implementation plan](../../.opencode/plans/2026-11-05T15-10-00Z-legacy-artifact-receive-cutoff.md)

## Revision 2026-11-11 (task 12788 decision, implemented by task 15019)

The format-version cutoff above was superseded before merge: an accepted
Codex P2 on PR #6407 showed that putting `format_version` inside
`signed_bytes` makes unupgraded peers reject every newly signed artifact
(the push/pull protocols have no version negotiation). The recorded
decision on issue #4459 replaces the marker with a receive-side-only
cutoff derived from per-account chain position:

- The signed format does not change; nothing about what signers emit
  changes, so unupgraded peers keep verifying new artifacts as today.
- The legacy world-ID exception applies only to an artifact that extends
  its author's held chain at a position before the first artifact the
  receiver itself accepted under strict receipt for that author.
- The position comes only from the chain's signed links and the
  receiver's own strict-receipt history — never a sender-asserted field.
  When a position cannot be established, validation stays strict.
- Signature and all other structural checks stay strict at every
  position; the typed legacy note is still recorded during pulls.

Implementation note: the artifact wire layer moved to the `id-sync`
crate in the #4406 crate split, so the receive path lives in
`pkgs/id/crates/id-sync/src/artifact.rs` (`ArtifactLog::insert_received`),
with the pull-path regression tests in the facade tests in
`pkgs/id/src/artifact.rs`.
