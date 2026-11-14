# Watch record: OCapN / CapTP stability and the CapTP bridge spike (issue #6014)

- **Date opened:** 2026-11-08
- **Status:** standing watch record; no implementation is included here.
- **Issue:** [#6014](https://github.com/developing-today/code/issues/6014),
  "id: research: OCapN watch + CapTP bridge feasibility spike", part of the
  worlds design program [#5955](https://github.com/developing-today/code/issues/5955)
  (wave-2 follow-up from the prior-art research, Lane B item 45).
- **Parent topic:** [#5989](https://github.com/developing-today/code/issues/5989)
  (capability systems, OCAP, Spritely Goblins/OCapN, CRDT shared worlds).
- **Decision this record watches:** the OCapN deferral recorded in
  [`doc/2026-10-09T09-59-07Z_design_attenuated_capabilities/`](../../2026-10-09T09-59-07Z_design_attenuated_capabilities/2026-10-09T09-59-07Z_design_attenuated_capabilities.md),
  §"Stages": _"OCapN (trigger: programs in different servers must hold
  references to each other's objects). Adds network-transparent references,
  handoffs and distributed GC. It is pre-standard. It is not needed for
  worlds that are hosted on one server and joined by capability."_ The same
  section states the deferral is revisited **on a trigger, not on a date**.
  This record supplies the dated evidence the issue asks for, and the
  trigger test that decides whether the spike is worth running.

## What this record is for

Issue #6014 asks for two things, in order:

1. **Watch** — re-check CapTP/OCapN stability status on a cadence and record
   dated state changes.
2. **Spike, only when the watch says it is worth it** — a minimal CapTP
   interop test between a Goblins vat and an id-side stub, producing
   feasibility evidence and a go/no-go for bridge work.

The watch is the deliverable that can be discharged now. The spike is
explicitly conditional; running it before the trigger fires would contradict
the recorded position, so this record defines the trigger instead of
pre-empting it.

## Trigger test (what makes the spike worth running)

The spike is triggered only when **all three** hold:

| #   | Condition                                                                                                                                                                                        | How it is checked                                                                                       |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------- |
| T1  | Spritely's own documentation no longer carries the "do not rely on the protocol being stable" caution, **or** an OCapN specification release is published with a stability/versioning commitment | `https://files.spritely.institute/docs/guile-goblins/latest/OCapN.html`; releases/tags on `ocapn/ocapn` |
| T2  | A published OCapN protocol version has stopped taking breaking changes across at least one Goblins release cycle                                                                                 | Goblins release notes ("OCapN protocol updates" sections)                                               |
| T3  | An id-side need exists to hold references to objects in another server's vat — i.e. the deferral trigger in the attenuated-capabilities record has fired for real, not hypothetically            | A design or feature issue in the worlds program that requires cross-server object references            |

T1 and T2 are about the protocol; T3 is about us. A stable protocol with no
need is still a no-go, and a real need against an unstable protocol is a
"record the need, keep waiting" — not a spike against a moving target.

## Watch log

### 2026-11-08 — first dated entry (checked by queue task 13154)

**Verdict: no change to the deferral. Spike not triggered (T1 and T2 both
fail; T3 not met).**

Evidence gathered fresh on 2026-11-08:

- **Spritely's docs still warn against relying on CapTP.** The _latest_
  guile-goblins OCapN manual page
  (`https://files.spritely.institute/docs/guile-goblins/latest/OCapN.html`,
  fetched 2026-11-08) still reads: _"CAUTION: CapTP support is still in its
  early days. Please do not rely on the protocol being stable at this time."_
  This is the same caution the 2026-11-04 prior-art research quoted from the
  0.13.0 page — it has survived into the current docs. **T1 fails.**
- **The OCapN specification is still a draft taking breaking changes.**
  Goblins 0.18.0 (2026-04-21, `https://spritely.institute/news/spritely-goblins-v0-18-0-sleepy-actors.html`)
  announces: _"The OCapN draft specification has changed in the time since
  the last Goblins release. The op:deliver-only operation has been dropped in
  favor of a single op:deliver operation. GC operations now accept a list of
  export positions… The protocol version number has thus been bumped, which
  means that applications built with an earlier release of Goblins are
  incompatible with the OCapN shipped in this release."_ A wire-incompatible
  bump between minor releases is the opposite of a stability commitment.
  **T2 fails.**
- **The spec repository has no released version.** `ocapn/ocapn`
  (`https://github.com/ocapn/ocapn`) carries **no tags and no releases**
  (checked 2026-11-08 via the GitHub API), and its most recent commits
  (2026-10-13) are still protocol-shape work — `op:listen` / listen-mode
  delivery semantics, removal of `op:get`/`op:index`/`op:untag`, and a revert
  of a `op:deliver-only` fold. The repository is active (last push
  2026-10-13), not frozen, and not versioned.
- **Development is moving toward features, not stabilisation.** Spritely's
  2026-10-15 post, _End-to-end encrypted relays for OCapN_
  (`https://spritely.institute/news/end-to-end-encrypted-relays-for-ocapn.html`),
  adds a Noise-based E2EE relay netlayer and states that an NLnet grant
  includes _"writing a specification for the OCapN group so that the relay can
  be broadly supported by other OCapN implementations"_ — new spec surface
  being authored now.
- **T3 (id-side need) is not met.** No issue in the worlds program requires
  programs in different servers to hold references to each other's objects;
  the attenuated-capabilities record's trigger for OCapN remains hypothetical.
  Worlds remain hosted on one server and joined by capability.

**Conclusion.** The deferral stands unchanged. Nothing is re-decided here;
the position recorded on 2026-11-04 (adopt the discipline — no ambient
authority, authority by reference passing — keep handle semantics
OCapN-compatible, revisit when CapTP stabilises) remains correct on current
evidence.

## Cadence and where the next entry goes

Per the triage scoping note on #6014 (2026-11-05) and this record's opening,
the re-check is **folded into the worlds-programme research pass on
[#5989](https://github.com/developing-today/code/issues/5989)** rather than
run as a standalone timer for a low-priority watch. Each pass appends a dated
entry above; if T1+T2 have flipped, that pass is also where the spike gets
its go/no-go.

A standalone queue task per re-check is deliberately not used: a watch whose
only output is "still unstable" would return to the queue every cycle and
produce no landed work, which is exactly the churn that put #6014 back in the
queue after triage task 10380.

## What a future entry must contain

- The date, and the exact URLs/versions checked.
- The state of T1, T2, T3, each with the evidence that decides it.
- If the trigger has fired: the spike scope (Goblins vat ↔ id-side stub),
  what "feasible" means for it, and the go/no-go.
- If the trigger has not fired: one line saying so, and nothing else. A watch
  entry that has not changed should be cheap to write.
