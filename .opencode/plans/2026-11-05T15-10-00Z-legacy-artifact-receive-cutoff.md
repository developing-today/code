# Implementation Plan: Legacy artifact receive cutoff (issue #4459)

See the dated design record: [legacy artifact receive cutoff](../../doc/2026-11-05T15-10-00Z_architecture_legacy_artifact_receive_cutoff/2026-11-05T15-10-00Z_architecture_legacy_artifact_receive_cutoff.md).

## Task
Implement the accepted ruling on [#4459](https://github.com/developing-today/code/issues/4459) so peer pulls can continue across pre-upgrade `home_declared` artifacts that violate the #3187 world-ID rules.

## Accepted ruling summary
- Cutoff: artifact/journal format version
- Legacy entries (before cutoff): scoped leniency for over-bound `home_declared` only, recorded with a legacy note; pull continues
- At/after cutoff: strict `parsed()` applies; over-bound declarations are hard errors
- Unmarked artifacts: per-author chain position fallback (entries before first strict artifact are legacy)
- Corrupt entries: hard errors at any version

## Implementation scope
- The signed artifact format and `Directory::receive_artifact` as needed to make the format cutoff trustworthy
- Add regression coverage exercising a pre-upgrade `home_declared` entry through the peer-pull path
- Do not reuse local replay leniency wholesale; do not relax new receipts

## Steps

1. Add a signed artifact format marker
   - Preserve legacy hash/signature bytes when the marker is absent
   - Mark new artifacts at the strict format version
   - Treat unmarked artifacts as legacy only before the author's first strict-version artifact

2. Implement version-scoped leniency in `Directory::receive_artifact`
   - If artifact version < cutoff: allow `home_declared` parsing to succeed with a legacy note; continue pull
   - If artifact version >= cutoff: call strict `artifact.parsed()?`; hard error on violation
   - If no version marker: fallback to per-author chain position (first strict artifact in author's chain)
   - Corrupt entries: hard errors always

3. Add a typed legacy receive outcome
   - Surface the scoped leniency to the pull caller without treating the accepted artifact as an error

4. Add regression coverage
   - Create a test that constructs a pre-upgrade `home_declared` artifact violating #3187 rules
   - Exercise the peer pull path through `Directory::receive_artifact`
   - Assert the pull completes with a legacy outcome and continues to later artifacts
   - Assert strict-version invalid entries and genuinely corrupt entries remain hard errors

5. Verify the targeted artifact/directory suites pass on the laptop
   - Run the narrow `pkgs/id` tests first, then the broader library tests if time permits

## Acceptance criteria
- A peer pull over a journal containing pre-upgrade `home_declared` entries completes (entries applied with a recorded legacy note, or surfaced as a typed non-halting outcome)
- Regression coverage exercises a pre-upgrade `home_declared` entry through the pull path
- Behavior for genuinely corrupt entries remains a hard error; strictness for at/after-cutoff artifacts is unchanged
- `pkgs/id` targeted suites pass

## References
- [#4459 ruling](https://github.com/developing-today/code/issues/4459#issuecomment-6634862794)
- [#3187 PR #4432](https://github.com/developing-today/code/pull/4432)
- `pkgs/id/src/artifact.rs`
- `pkgs/id/src/directory.rs`

## Change log
- 2026-11-05: Initial plan created; implementation detail narrowed to signed format marker and typed receive outcome.

## Change log
- 2026-11-05: Initial plan created
