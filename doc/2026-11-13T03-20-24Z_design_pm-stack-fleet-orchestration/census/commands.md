# Track B command compatibility inventory

Machine-readable inventory is [`commands.jsonl`](commands.jsonl): one row per command operation,
option, event operation, or scoped contract. It captures every operation named in source 01 lines
105–119, source 03 §§4.1–4.8, and the flattened recovered source 04's concrete command examples,
plus actual current queue CLI verbs that matter for compatibility. Canonical names retain source
spelling; they are not aliases for current commands unless a row explicitly records that.
Each of the 215 inventory keys has exactly one primary-slice row in
[`command-slice-map.jsonl`](command-slice-map.jsonl), validated one-to-one. Baseline, out-of-scope,
and pending-decision rows are explicitly dispositioned rather than hidden in a wildcard.

The actual queue CLI dispatcher is `pkgs/fleet/crates/fleet-queue/src/client.rs:5301–5345`.
`cmd_list` is at :1650; `cmd_claim` at :2230; `cmd_park` at :2914. `repo-queue` is an alias into
this Fleet binary, per `pkgs/fleet/crates/fleet-registry/src/lib.rs:36,220,331`. Thus `park` is
implemented, while proposal `search`, `get`, and `resume` are not exact registered spellings
(`wake` is adjacent, not an alias). Existing lifecycle-like queue verbs do not prove the higher
level `fleet work` workflows. Help/version/health smoke checks are in
`pkgs/fleet/smoke.sh:474–485`; list tests include `tests/chunked_list.rs:65–71`.

`fleet gh` is an existing rotation-aware GitHub wrapper and Metadata-aware implementation path
(#8887/#9016), not authorization for a parallel `fleet github` backend. The original spellings stay
as compatibility candidates, with exact status tied to pinned parser evidence or explicitly
unverified current passthrough details. Creation outbox remains exactly the bounded
`new-issue-creator-code-a` producer (`doc/fleet/creation-outbox.md:54–60`).

Inventory spans discovery; queue; work and notes; every issue/PR/comment/event command; metadata;
dependencies; activity/sync; each slash command and agent session operation; and each source option
as its own row. A `partial` status is only used for an exact command whose implementation covers a
real subset; adjacent command spellings are not called partial. `absent` means the exact spelling
is not registered at pinned source, not that no adjacent capability exists. See `track-b.md` and
`components/9206-work-item-lifecycle-fleet-subcommands.md` for status definitions, evidence limits,
contract design, and implementation issue slices.
