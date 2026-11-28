# Decision: mobile-native (app-store) shells — adopt, high priority

- **Date:** 2026-11-23
- **Status:** Decision record; no implementation is included here.
- **Issue:** [#12820](https://github.com/developing-today/code/issues/12820)
  (shells v2 slice 4 — mobile-native decision record), follow-on
  slice 4 of [#6297](https://github.com/developing-today/code/issues/6297)
  (More shells v2 — outline: native & TUI completion; record
  [`doc/2026-11-23T03-17-14Z_design_more_shells_v2/`](../2026-11-23T03-17-14Z_design_more_shells_v2/2026-11-23T03-17-14Z_design_more_shells_v2.md),
  PR #12816).
- **Question settled:** Q2 / D2 of the v2 outline — mobile-native
  app-store shells: adopt in v2, defer to a named later rung, or
  drop? v1 deferred this question to the v2 rung; this record is
  where it is answered.
- **Decision maker:** Drew (owner ruling, 2026-11-22, main chat),
  recorded in full on
  [#6297](https://github.com/developing-today/code/issues/6297#issuecomment-7090738837)
  and pointed to from #12820 (owner comment, 2026-11-23).

## Decision

1. **Mobile-native shells are ADOPTED.** App-store shells are v2
   work, not a later-rung afterthought: the ruling states
   explicitly that mobile native "is not a v3 afterthought".
2. **Mobile native is high priority.** It ranks as a first-class
   platform commitment of the More shells programme, on the same
   footing as the v2 outline's native-GA and TUI scope.
3. **Platform order.** The platform priority order ruled alongside
   this decision (Q1 on #6297) is **Linux → Android → Web → macOS
   → Windows**. Android ranks second overall, behind only Linux;
   macOS is the second _desktop_ native target, not the second
   platform. The mobile platform this decision names is therefore
   **Android**. The ruling does not address iOS; this record does
   not settle an iOS shell either way.
4. **Sequencing is the only open part.** Deferring the _start_ of
   the implementation is acceptable only if the mill judges it
   genuinely best; a deferred start is not the default. The
   adoption itself is not conditional on that judgement.
5. **Implementation is filed from this record.** Per #12820, the
   implementation issues for the adopted mobile-native shell are
   filed from this record as linked issues — not from the v2
   outline.

## Rationale and rejected alternatives

The options are the three the v2 outline framed for Q2; the
ruling's recorded reasoning is quoted or paraphrased from it
without addition.

- **Adopt in v2 (chosen).** "Drew wants mobile native adopted…
  and mobile native is high priority — it is not a v3
  afterthought. (Consistent with Q1: Android ranks second
  overall.)" No further rationale is recorded on the issue, and
  none is invented here.
- **Defer to a named later rung (rejected as the decision).** v1
  already deferred this question once, with the reason recorded
  (the PWA covered phones in v1; the native proof obligation went
  to the Linux desktop shell first). The ruling closes the
  deferral path as a _decision_: what remains permissible is a
  deferred _start_ on a genuine-best judgement (Decision item 4),
  not a deferred answer.
- **Drop (rejected).** Dropping app-store shells would leave the
  PWA as the permanent phone story; the ruling adopts native
  instead. No drop rationale was recorded because the option was
  not taken.
- **Leave the question unrecorded (rejected by construction).**
  The v2 outline forbids silence on this question ("Silence is
  the one outcome this outline forbids"), and #12820 exists to
  produce this record.

## Evidence

- **v1 deferral:** the v1 record
  ([`doc/2026-11-10T20-40-00Z_design_more_shells_v1/`](../2026-11-10T20-40-00Z_design_more_shells_v1/2026-11-10T20-40-00Z_design_more_shells_v1.md),
  PR #8243) states "Mobile-native (Android/iOS) is a v2 question
  (#6297's ladder rung), not a v1 omission: the PWA covers phones
  in v1", and lists mobile-native shells under "deferred above,
  with the reason stated".
- **v2 framing:** the v2 outline, §3 ("The mobile-native
  question — settled here, one way or the other") and open
  question Q2, frames exactly the three options above and names
  the decision as Drew's; its follow-on slice 4 is this record
  ("the decision and its recorded reasoning; implementation, if
  adopted, is filed from that record, not from this outline").
- **The ruling:** owner rulings on the v2 open questions, Drew,
  2026-11-22 (main chat), recorded on #6297 at
  https://github.com/developing-today/code/issues/6297#issuecomment-7090738837
  — Q2: "Mobile-native shells: ADOPT", high priority, deferred
  start only on a genuine-best judgement; Q1: the platform order
  in Decision item 3; Q3: per-platform releases (see
  Consequences). The pointer comment on #12820 (owner,
  2026-11-23) restates the same ruling for this slice.

## Consequences and follow-on filing

- **Follow-on filing (the next step this record creates):** the
  mobile-native implementation — an Android shell on the shared
  frame layer — is filed as linked issue(s) from this record,
  under #6297's slice ladder. Until that filing exists, this
  decision is recorded but unimplemented; the filing is the
  deliverable #12820's completion hands off.
- **The shell contract binds the mobile shell as it binds every
  shell.** Per the v1/v2 records' settled rule, a shell is a
  capability statement against the F13.1 shell contract (frames
  sent, server frames presented, capability providers declared),
  and done = conformance plus behavioral completion checks —
  never a demo. The v1 slice that landed the first native shell's
  capability statement (#10928) is the pattern the mobile shell's
  filing follows. The v2 outline's §4 bar — one conformance suite
  across every shell — is the standard the mobile shell is held
  to when its implementation is filed.
- **Releases:** the Q3 ruling recorded alongside this decision —
  per-platform releases with granular tags per app / platform —
  applies to the mobile shell's releases when it ships; the
  mobile shell does not wait for an all-platforms-green gate.
- **Until the mobile shell lands,** the PWA remains the phone
  story, exactly as v1 left it; adoption changes the commitment,
  not the current shipping surface.
- **Open part, with its default:** sequencing (Decision item 4).
  Default in effect: the start is not deferred. A deferred start
  requires the mill's genuine-best judgement, recorded on the
  implementation filing this record creates.
