# Parser conformance corpus

Each case is a pair of files: `<name>.id` is semantic source and
`<name>.golden` is its normalized parser IR. `cargo test -p id-render` checks
the snapshots in CI. Add cases rather than changing existing ones; re-bless a
golden only alongside an intentional parser-contract change and its rationale.

This is the first incremental slice of issue #7919, not the complete suite.
Target lowering goldens and coverage for every vocabulary element and
assertion role remain to be added. In particular, assertion-role coverage is
blocked on the still-unsettled #5850 vocabulary; this corpus does not claim
that acceptance criterion is met.
