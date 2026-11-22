# ID walking-skeleton CLI path

## Request and intent

Expose the #12232 walking-skeleton path from the `id` CLI so a person can
run the five-step signed-record, durable-store, transport, conflicting-write,
and conflict-report scenario. The README must show how to run it, and the
existing ID nightly must execute the same path.

## Initial design

Add one narrowly scoped `id substrate-path` command rather than exposing the
federation's low-level envelope and store APIs as a general-purpose CLI. The
command runs the whole deterministic demonstration in an isolated directory:
two disk-backed replicas, signed sample records, an actual local iroh exchange,
and reopen checks for versions, heads, and conflict reports. It prints the five
steps and the resulting `id://` address and conflict identity. The sample
signing keys and records are demonstration fixtures; the command is not a
general node server or a way to write to a user's normal ID store.

The network protocol currently serves an in-memory `Replica`; persistence is
provided separately by `DiskReplica`. The command will keep that boundary
explicit: exchange through the existing protocol, then replay newly admitted
envelopes through each `DiskReplica`'s public receive path before verifying
the durable state. It adds no federation protocol and no dependency.

Use the existing CLI integration-test suite and its current flake/nightly
coverage. The README walkthrough will include the runnable command and list
all five path steps; no new schedule or just recipe is needed.

## References

- #12232 — walking-skeleton acceptance and five steps.
- #12492 / PR #12497 — durable versions and reports.
- #12621 — two-process transport path.
- `doc/2026-11-22T16-13-20Z_design_id-walking-skeleton/note.md` — path and
  shared interface map.
