# Worked example — second-pass brief for code PR #4175

Renders the Stage A shadow second-pass verdict on
https://github.com/developing-today/code/pull/4175
(quoted in full in the Stage A exit report,
https://github.com/developing-today/code/issues/2313 —
pr-review.log, 2026-11-13 05:24 CST) through
`second-pass-session-brief-template.md`.

> NOTE: this is a worked example of the template shape, not a live
> commission. The verdict below was a *shadow* verdict (PoC): no packet was
> enqueued and PR #4175's gaps were later fixed on main.

---

## 1. Verdict note (gap list first)

- Verdict note URL: https://github.com/developing-today/code/issues/2313
  (Stage A exit report; source line `shadow verdict (PoC):` in pr-review.log,
  2026-11-13 05:24 CST)
- Verdict: `second-pass`
- Gap list (verbatim from the shadow verdict):
  1. `test_owner_changed_*` defined after `unittest.main()` in the `__main__`
     guard — the branch-switch revalidation shipped with zero running tests.
  2. `queue_claims` single `limit=1000` read silently drops claims beyond
     the page.
  3. Ring 2: `open_pr_branches --limit 500` silently drops open-PR branches
     beyond 500.

## 2. Original issue + acceptance criteria, checked item by item

- Original issue: developing-today/code#4175's tracked issue (the PR's
  `fixes` target — dispatcher resolves at commission time; unknown here
  because the shadow verdict records the as-merged artifact only).
- Pass-0 rulings (from the gap list, per-gap):
  - [ ] Branch-switch revalidation is exercised by running tests — **unmet**
    (gap 1: test functions defined but never executed).
  - [ ] Claim reads page through the full claim set — **unmet**
    (gap 2: silent truncation past 1000).
  - [ ] Open-PR branch reads page through the full set — **unmet**
    (gap 3: silent truncation past 500).

## 3. Goal context refs

- Parent epic: https://github.com/developing-today/code/issues/2056
  (ID program — Ring 3 goal anchor; adjust to the verdict's goal at
  commission time)
- Goal brief: keep-the-four-repos-healthy goal brief
- Design record: `files/review-loop-design-2025.md` (§§3, 5–6, 8)

## 4. Pass-0 artifact and its queue result

- Pass-0 artifact: https://github.com/developing-today/code/pull/4175
  (as-merged revision at verdict time)
- Queue result: shadow only — no queue task; the lane log notes all three
  gaps were later fixed on main (evidence the gaps were real and checkable).

## 5. Pass-cap facts + fresh-vs-iterating ruling

- Current pass: 1 of cap 2 (first genuine second pass; cap from the
  commissioning packet's `pass_cap`).
- Prior gap lists this pass must shrink: the three gaps in §1 above
  (pass-0 list; no earlier passes).
- Fresh vs iterating: `fresh session` (default).
- Exception or explicit none: `none applies — fresh session per the default;
  the gaps span test layout and two fleet-queue read paths, i.e. wider than
  the producing diff`.

## 6. Route floor from the verdict + pool measurement gate

- Route floor: <from the verdict — the shadow verdict names no floor, so the
  dispatcher sets the program default floor at commission time and records it
  here>.
- Pool measurement gate: quota read for the floor pool must be no more than
  60 minutes old at dispatch — re-read if stale.

## 7. Explicit non-goals

- No producing-session transcript is attached or reconstructed; the verdict
  note + gap list + pass-0 artifact are the whole input.
- The pass artifact is verdicted by a non-producer (never the session that
  produced the pass under review).

---

## Why this verdict passes the §3.1 checkable bar

Each gap names a behavior plus the test or surface that decides it:
(1) run the test file and count executed tests; (2)(3) read past the page
limit and observe the dropped rows. No vibes — a session can fail or close
each gap independently, which is exactly what the later fixes on main did.
