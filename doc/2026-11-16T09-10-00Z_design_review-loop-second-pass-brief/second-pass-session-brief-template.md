# Second-pass session brief — template (design #2025 §5.4)

For issue https://github.com/developing-today/code/issues/2314.
Parent design: https://github.com/developing-today/code/issues/2025
(`files/review-loop-design-2025.md` §5.4, keep-the-four-repos-healthy goal workspace).
Packet schema: `second_pass` kind + `review` block, repo-task/v1
(issue https://github.com/developing-today/code/issues/2310, PR #9402).

Copy this template into the second-pass packet's dispatch notes / commissioning
comment. Fill every field; delete nothing. Field order is the design's priority
order — the verdict note (gap list first) is what the session acts on.

---

## 1. Verdict note (gap list first)

- Verdict note URL: <link to the `## Shadow verdict (PoC)` / verdict section>
- Verdict: <`second-pass`> (this template is only used for second-pass verdicts)
- Gap list (checkable per design §3.1 — copy verbatim, gap list first):
  1. <gap 1: behavior / test / adjacent surface it names>
  2. <gap 2>
  3. ...

## 2. Original issue + acceptance criteria, checked item by item

- Original issue: <owner/repo#n — title>
- Pass-0 ruling on each acceptance item (met / unmet / unknown):
  - [ ] <criterion 1> — <met|unmet|unknown + one-line evidence>
  - [ ] <criterion 2> — <…>

## 3. Goal context refs

- Parent epic: <epic issue link, e.g. code#2056>
- Goal brief: <goal workspace brief path or link>
- Design record: <design doc path, e.g. `files/review-loop-design-2025.md`>

## 4. Pass-0 artifact and its queue result

- Pass-0 artifact: <PR link + merge SHA, or unmerged branch + head SHA>
- Queue result: <task id, outcome — e.g. `fixed_merged`, result summary link>

## 5. Pass-cap facts + fresh-vs-iterating ruling

- Current pass: <n> of cap <cap> (`payload.review.pass` / `pass_cap`;
  the broker rejects `pass > pass_cap` at enqueue — the cap is arithmetic).
- Prior gap lists this pass must shrink: <links to pass-0 … pass-(n-1) gap lists>
- Fresh vs iterating: <`fresh session` (default) | `iterate in place`>
- Exception or explicit none: <named exception with reason, or `none applies —
  fresh session per the default`>

## 6. Route floor from the verdict + pool measurement gate

- Route floor: <minimum pool/route the pass may run on, from the verdict>
- Pool measurement gate: quota read for the floor pool must be no more than
  60 minutes old at dispatch — re-read if stale.

## 7. Explicit non-goals

- No producing-session transcript is attached or reconstructed; the verdict
  note + gap list + pass-0 artifact are the whole input.
- The pass artifact is verdicted by a non-producer (never the session that
  produced the pass under review).

---

## Validation (dispatcher checklist)

- [ ] All seven sections present, in order; gap list copied verbatim, first.
- [ ] Every acceptance item of the original issue has a pass-0 ruling.
- [ ] `pass` ≤ `pass_cap`; prior gap lists linked.
- [ ] Fresh-vs-iterating has a named exception or an explicit `none applies`.
- [ ] Quota read for the route floor is ≤ 60 minutes old.
- [ ] Non-goals acknowledged (no transcript; non-producer verdict).
