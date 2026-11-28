# Capability connect verdict

Issue #12921's connect-stage decision is a bounded pure slice of the capability-change notice. The world client currently does not resolve release and instance records, so this change will not wire the decision into a live connection pipeline or claim persistence/UI behavior.

The `id-caps` API will decide from a known baseline, release identity, the current shipped class-table version, and an optional freshly computed message set. First connect returns the classes for the disclosure flow. A matching release and class-table version short-circuits silently. Otherwise, missing computation fails closed with the existing `unknown` diff; a covered set proceeds and reports whether it is strictly different (for narrowing bookkeeping); an uncovered set holds with exactly the diff's new classes.

The class-table version is `CAP_CLASS_TABLE_VERSION` in `cap_class.rs`; consumers do not supply or duplicate it. Actual manifest resolution, connection holding, persistence, and user interaction remain follow-on integration work.
