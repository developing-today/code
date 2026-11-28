# Discovery v4 — local reputation computation design (#16501)

Design record for issue #16501 ("id(phase-14): Discovery v4 — local
reputation computation"). This is §9 item 3 of the v4 curation design
(PR #16496, merged 2026-11-28T06:22:44Z, merge commit
`0d7612ff5d1c3f1dd4cecdcfedde73ac8f5a9491`; record
`doc/2026-11-28T06-10-00Z_design_id6290_open_discovery_v4/`, §5 and §9).
Parent outline #6290; core topic #6277. Sibling step: #16499 (§9 item 1,
subscription policy budgets, record
`doc/2026-11-28T07-59-00Z_design_id16499_subscription_policy_budgets/`).

Status: design only. No code in this issue — this record names the
inputs, the hosted instance's published method, the weighting-setting
shape, and the traffic-signal exclusion as a testable rule. Nothing here
authorizes implementation beyond a future separately scoped slice.

**Grounding — verified 2026-11-28 (lane 0207-I, firsthand from `main`
at `b3918c1ffe20139a20346fcaa0421425cc355a7f`).**

- The v4 design §5 fixes the semantics this record elaborates and does
  not re-decide: reputation is computed per curator over subscribed
  lists only; it never crosses a mirror boundary (same class as `seq`
  and policy, v3 §3 rule 5); scores are served only inside the
  curator's own views, attributed to the curator; there is no shared
  reputation feed and no merging of scores across registries.
- No reputation scoring exists in `pkgs/id` today: code search for
  `reputation` under `pkgs/id` returns book/research material and
  unrelated federation slices only — no scoring function, no score
  type, no served score surface in `id-sync`. This record is greenfield
  method design, not a change to a running scorer.
- The list substrate the inputs read from is the signed registry list
  as imported by `Registry::import`
  (`pkgs/id/crates/id-sync/src/registry.rs`): entries carry their
  listing curator's key, first-seen and per-fetch presence are
  observable at import/fetch time, and tombstones are first-class list
  objects (`delisted_here` in the import path). Every input in §2 is
  derivable from that substrate and nothing else.

## 1. Governing rules (from v4 §5, restated as constraints)

R1. **Curator-local.** A reputation score is always a triple
    (scoring curator, subject, method version). There is no
    curator-free score, and no view may render a score without naming
    its scoring curator.
R2. **Subscribed lists only.** The inputs for curator C's score are
    computed exclusively over lists C subscribes to. A list C does not
    subscribe to contributes nothing — not even a denominator.
R3. **Never mirrored, never merged.** Scores are policy-class data:
    they are never embedded in mirrored announcements, never propagated
    across a mirror boundary, and never merged across registries.
R4. **Combination is client-side and user-weighted.** A client may
    combine several curators' scores locally, only under explicit
    user-chosen weighting (§4). No combined score is ever published,
    served, or mirrored by our hosted instance.
R5. **Traffic exclusion.** No traffic-derived signal appears anywhere
    in our hosted pipeline's scores (§5, stated as a testable rule).

## 2. Named inputs

For scoring curator C and subject world W, the input vector is computed
over the set L(C) of lists C subscribes to, over a rolling window of
the last 90 days of fetches (default; see §6). All inputs are derived
from public list data — the lists themselves are the evidence, and any
third party holding the same lists can recompute the same vector.

| # | Input | Definition | Range |
|---|-------|------------|-------|
| I1 | `age` | Time since W first appeared in any list in L(C), measured from the earliest first-seen timestamp across L(C) to the scoring instant. Saturating: full credit at 180 days. | [0, 1] after saturation curve §3 |
| I2 | `persistence` | Fraction of completed fetches of lists in L(C), within the window, in which W was present in at least one fetched list. Denominator is fetches actually performed, never scheduled fetches. | [0, 1] |
| I3 | `breadth` | Number of independent lists in L(C) that currently list W. Independent = distinct listing-curator signing keys; two lists signed by the same key count once. Saturating: full credit at 5 independent listings. | [0, 1] after saturation curve §3 |
| I4 | `tombstones_against` | Count of live tombstone-notices against W issued by curators whose lists are in L(C), weighted per §3. A tombstone withdrawn by its issuer stops counting at the next fetch. | [0, ∞), mapped per §3 |
| I5 | `tombstones_for` | Count of live tombstone-notices by curators in L(C) against entries that impersonated or spoofed W (a tombstoned listing whose subject was a false W). Same withdrawal rule as I4. | [0, ∞), mapped per §3 |

The input list is **closed**: an implementation may not add an input
without a new design record amending this one. In particular, no input
may be derived from views, joins, popularity, relay metadata, or any
other traffic signal (R5/§5), and no input may be derived from another
curator's score (no recursion, no score-of-scores).

## 3. The hosted method (published)

Our hosted instance computes, for each world W in the lists it
subscribes to as a curator, the score

```
score(C, W) = clamp01( 0.20·agê + 0.35·persistence + 0.25·breadtĥ
                       − 0.50·tombstones_against̂ + 0.10·tombstones_for̂ )
```

where the saturation/normalization curves are:

- `agê = min(1, ln(1 + age_days) / ln(181))` (log saturation at 180 d).
- `breadtĥ = min(1, independent_listings / 5)`.
- `tombstones_against̂ = min(1, n_against / 3)` — three independent
  live tombstones against W fully zero the positive terms' headroom
  via the −0.50 weight combined with clamp01 in any realistic vector;
  one tombstone is a warning, not a verdict.
- `tombstones_for̂ = min(1, n_for / 3)`.
- `persistence` enters raw (it is already a fraction).

Method rules:

M1. **Versioned.** The method is identified as `hosted-reputation/1`.
    Every score the hosted instance serves is stamped with the method
    id and the input vector it was computed from — a served score is
    always explainable down to I1–I5. A change to any weight, curve,
    or input definition is a new method version (`/2`, …), announced
    in this record's successor note before it serves; scores are
    recomputed at fetch time under the current version only (no
    retroactive restatement of previously served views).
M2. **Deterministic.** The same list snapshot + same scoring instant +
    same method version yields the same score. Fetch-time computation
    only; no hidden state, no randomness, no machine-learned component.
M3. **Served shape.** The hosted instance serves, inside its own views
    only (R1/R3): subject, scoring curator (itself), method id, score,
    the five input values, and the window end. It serves no ranking
    across curators, no combined score (R4), and no global ordering of
    worlds by score as a discovery surface — ordering a curator's own
    view by its own score is the curator's view choice, not a feed.
M4. **Absence is not a score.** A world absent from L(C) has no score
    (not zero). A world present but never tombstoned simply has
    I4 = I5 = 0.

## 4. Client-side combination — the weighting-setting shape

Combination happens only in the client, only over scores the client
has actually received from curators it subscribes to, and only under a
user setting. The setting lives beside the subscription set (same
class as the v4 §3 budget settings: a local policy input, never
mirrored, never published).

```json
{
  "reputation": {
    "combine": "weighted-mean",
    "weights": { "<curator-key>": 1.0 },
    "min_curators": 1
  }
}
```

Semantics:

W1. `weights` maps a scoring curator's key to a non-negative weight.
    A curator absent from the map has weight 1.0 (equal by default);
    weight 0 excludes that curator's scores from combination. Negative
    weights are invalid and the setting fails validation whole (no
    partial application).
W2. The combined value is the weighted mean over curators that (a)
    have weight > 0 and (b) actually serve a score for W. A curator
    with no score for W is excluded from numerator and denominator —
    absence is never averaged in as zero (M4 applied client-side).
W3. If fewer than `min_curators` curators qualify, the client shows
    the individual attributed scores and no combined value.
W4. A combined value is always displayed with its contributing
    curators and their weights — an unattributed combined number is a
    spec violation, because it would be indistinguishable from a
    global score (forbidden by R3/R4 and the v4 out-of-scope list).
W5. Our hosted instance ships no privileged default: in a fresh
    client, the hosted curator appears in `weights` only if the user
    subscribes to it, at the same default 1.0 as every other curator.

## 5. The traffic-signal exclusion, as a testable rule

**Rule R-EXCL.** No value derived from views, joins, popularity,
relay metadata, fetch counts of other parties, or any other traffic
signal may influence any score our hosted instance computes or serves.

This is testable, and the implementation slice must carry these named
tests (oracles stated here, code in that slice):

T-EXCL-1 (closed inputs). The scoring function's input type enumerates
exactly I1–I5 plus the scoring instant; a compile-time/schema test
fails if a field is added without amending this record. Oracle: the
§2 table.
T-EXCL-2 (traffic invariance). Two fixture worlds identical on I1–I5
but differing arbitrarily in recorded views/joins/relay counts score
identically under `hosted-reputation/1`. Oracle: score equality on
the paired fixture.
T-EXCL-3 (determinism). The same list snapshot scored twice at the
same instant yields byte-identical served records (M2).
T-EXCL-4 (mirror boundary). A mirrored announcement fixture contains
no score, input vector, or method id (R3); the mirror of a scored
view is score-free.
T-EXCL-5 (combination honesty). With one curator weighted 0 and one
weighted 1, the combined value equals the weight-1 curator's score;
with a qualifying-curator count below `min_curators`, no combined
value is produced (W2/W3).
T-EXCL-6 (tombstone sign). Adding one live tombstone against W never
increases score(C, W); withdrawing it restores the prior score at the
next fetch (I4).

## 6. Open choices — decided here with defaults

- **Window length:** 90 days for `persistence` (I2). Shorter windows
  make persistence a uptime contest; longer windows forgive abandonment.
  A curator may publish a different window only as a new method id —
  the window is part of the method, not a served-score footnote.
- **Presentation:** scores are shown as the number plus its input
  vector (M1/M3), never as a leaderboard and never as a badge that
  hides the vector. A "trusted/untrusted" binary badge derived from a
  threshold is refused: it would hide exactly the disagreement the
  curation layer exists to show (v4 §7 rule: shown, never judged).
- **Cold start:** a newly listed world scores low on I1/I2 by
  construction. That is intended — age and persistence are the
  anti-spam inputs (v4 §3) — and no newcomer boost is added, because
  any boost keyed on arrival is a Sybil subsidy.

## 7. Out of scope (explicit)

- Shared reputation feeds, global scores, mirrored or merged scores
  (v4 §5/§10, upheld).
- Traffic-derived signals anywhere in our hosted pipeline (R5).
- Any code: the scoring implementation, the served surface, and the
  client combination UI are future separately scoped slices; the
  tests in §5 are named here, written there.
- Adjudicating disputes between curators, or a curator's appeal
  against another curator's tombstone (takedown/appeals practice is
  v4 §9 item 4, under #1067).
- Key recovery / identity handover (accepted v1 limitation, unchanged).

---

**Verified when:** this record is merged with the `doc/README.md`
index line; the outcome (record link + merge SHA) is posted on #16501;
and the four Done-check elements are all present above — the input
list (§2), the hosted method (§3), the weighting-setting shape (§4),
and the exclusion rule stated testably (§5).
