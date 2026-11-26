# Parser conformance corpus

Each case is a pair of files: `<name>.id` is semantic source and
`<name>.golden` is its normalized parser IR. `cargo test -p id-render` checks
the snapshots in CI. Add cases rather than changing existing ones; re-bless a
golden only alongside an intentional parser-contract change and its rationale.

Parser snapshots are supplemented by `../coverage/`, which gates a case for
every registered element and every settled #5850 assertion role. Cross-target
lowering snapshots live in `../target-goldens/`; the role corpus exercises
each web/native mapping and explicit platform-default marker.
