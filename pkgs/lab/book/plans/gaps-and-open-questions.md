# Gaps and open questions

This chapter is the lab census's gap list (§8 of census C,
2026-11-23/24), carried into the book verbatim in substance. It
is the honest to-do record: where the record is thin, what is
unverified, and which contradictions stand unreconciled. A gap
listed here is not softened anywhere else in the book — chapters
cite these numbers.

1. **No per-machine inventory exists for any basement machine.**
   Only the laptop `amd` has a collected record. CPU model,
   board-per-machine, RAM-per-machine, drive-per-machine: all
   ungathered
   ([#1095](https://github.com/developing-today/code/issues/1095) /
   [#1096](https://github.com/developing-today/code/issues/1096) /
   [#1810](https://github.com/developing-today/code/issues/1810)
   open). Every "identical core boxes" claim rests on Drew's
   2026-10-20 dictation.
2. **NIC specification unverified.** "At least 200 Gb ports"
   (dictated) versus a 100GbE-max switch fabric of record;
   [#1097](https://github.com/developing-today/code/issues/1097)
   open. The LINSTOR/DRBD design cannot be sanity-checked until
   this lands.
3. **The three build-out Dell switches have no model, hostname,
   or state anywhere in the record**
   ([#6073](https://github.com/developing-today/code/issues/6073)–[#6075](https://github.com/developing-today/code/issues/6075)
   will create it). Whether they are the inventoried
   PowerConnect 5448s is unstated. **Location/state (racked, in
   use, boxed) is unstated for every one of the 21 inventoried
   network devices** —
   [#13920](https://github.com/developing-today/code/issues/13920)'s
   first slice owns exactly this.
4. **DX010 count tension:** the inventory says 4× owned (1
   missing fans/PSU);
   [#6071](https://github.com/developing-today/code/issues/6071)
   says install 3 _more_. Install-of-owned versus buy-more is not
   distinguished in the record.
5. **Racking state unknown.** No record of racks present, rails
   installed, or layout executed;
   [#6070](https://github.com/developing-today/code/issues/6070)'s
   very subject ("another 4 each of racking") is unresolved
   dictation.
6. **Power unverified beyond the brief:** no installed-circuit
   record, no UPS record, no measured draw (600 W per machine is
   a planning assumption in the brief itself).
7. **Nothing cluster-shaped has run since the v1.9 test cluster
   was torn down.** Cozystack, LINSTOR/DRBD, CNPG, RustFS, NATS,
   FluxCD on-cluster: zero deployments. TALM is proven only
   against local containers.
8. **#6078's verdict not extracted.** The SBC/Pi research issue
   ([#6078](https://github.com/developing-today/code/issues/6078))
   is closed, but its body checkboxes are unchecked and the
   census did not read its closing comments; the per-role
   verdicts (quorum witness / web serving / ID free tier) must be
   pulled from the issue before the book asserts them. This book
   asserts none.
9. **OCI contradiction — unreconciled, not resolved.** Epic
   [#1091](https://github.com/developing-today/code/issues/1091)
   and child
   [#4904](https://github.com/developing-today/code/issues/4904)
   remain **open** on GitHub, while the memory of record
   (2026-10-31) says OCI is stood down and is not a backend. Both
   facts stand in this book, flagged as unreconciled. Neither
   this chapter nor any other picks a winner; reconciliation is
   programme work on #1091, not an editorial act.
10. **GPU framing tension:** the register lists 3× Intel B50 +
    an RTX 2060 as owned-but-not-a-resource;
    [#6072](https://github.com/developing-today/code/issues/6072)'s
    title records "local (none owned)" for the GPU-capacity
    question. The book uses the register's list and notes the
    #6072 framing difference.
11. **ESP32 ownership is nowhere established.** Hardware-doc is
    research, not ownership; no board is recorded as bought,
    flashed, or run. The AliExpress aggregation
    ([#13910](https://github.com/developing-today/code/issues/13910))
    is the planned fix and is blocked on Drew granting access.
12. **"Which ID app first" is still open** (register open
    question #1, 2026-10-20; repeated in the P9 draft,
    2026-11-12) — it gates epic
    [#1092](https://github.com/developing-today/code/issues/1092)'s
    acceptance.
13. **P9 lab-integration design is unratified**
    ([#8873](https://github.com/developing-today/code/issues/8873))
    — its phased landing plan (Atlas serve, queue workers, PM
    surfaces, build overflow on the cluster) must not be cited as
    decided. This book cites it only as a draft.
14. **Switches research is dated 2026-04-02/03** — the newest
    fact in it is seven months older than the rest of the
    programme; firmware versions, device survival, and the
    DX010's missing fans/PSU may have changed. A physical check
    list is #13920's stated output.
15. **Mini PCs, Dell machines, spare PCs: counts without specs.**
    "2–3 mini PCs", "some Dell machines", "several spare PCs" —
    no models or specifications in any source read.
16. **No timing exists for any lab item.** No record read in the
    census carries a projected or planned date for any physical
    or cluster milestone; the roadmap's Now/Next/Later/Ideas
    horizons are the only sequencing of record. The
    [High-level roadmap](../roadmap/high-level-roadmap.md) keeps
    every lab timing entry as "no timing in record" for exactly
    this reason.

## Closing this list

Each gap names its owning issue. A gap closes when the record
changes — an inventory lands, a physical check is recorded, an
issue is reconciled — and the chapter that carried the gap is
updated in the same pass, per
[Keeping this book current](../development/keeping-this-book-current.md).
Gaps do not close by being reworded.
