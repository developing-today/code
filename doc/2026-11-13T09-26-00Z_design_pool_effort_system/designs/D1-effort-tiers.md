# D1 — Effort tiers, overlays, aliases, cross-pool equivalence, per-tier limits

**Program:** code#9399 (design mill, queue item 15864) · **Component:** D1 · **Status:** draft for the mill; nothing committed, nothing filed.
**Route/head:** opencode-route/workhorse · **Directory:** /home/user/git/wt-mill-pools (branch design/pool-effort-system).
**Binding inputs:** `main.md` AD1–AD4 and AD6 (AD5, AD7–AD10 are read as context only), `brief.md`, `brief-decisions-supplement.md`, `questions.md`, `baseline/v1-pools-efforts.md` (cited "V1a §x"), `baseline/v1-pools.json` (cited "JSON"), `research/R1-routing-effort-sota.md` (cited "R1 §x").
**Evidence not in the repo:** AA v5 rankings from `~/.local/state/pm-aggregator/aa-last-good.json` (batch `fe02783b`, generated 2026-11-13T09:42Z). The path named in the brief (`~/workspace/model-review-2026-11-12/aa-results.json`) does not exist (`baseline/v1-data-sources.md` S5b). All AA numbers below are "score / cost-per-task (USD)" from the **AA|v5** rankings unless tagged v4.3.2.

---

## 1. Scope

**Themes:** T3 (context limits), T4 (effort tiers), T7 (testing plan, as it applies to tiers: brief §8 item 6, "trial many/most major models at 2+ effort levels"). T7 is also owned by D4 for trial mechanics; D1 owns only the tier-level part.

**Statement ids.** `inventory/statement-map.json` does not exist in this program, so statements are cited as brief sections: `B<section>.<bullet>` counts bullets top to bottom inside that section of `brief.md`. `S<section>.<bullet>` is the same for `brief-decisions-supplement.md`. `AD<n>` is `main.md` §2.

| Statement | Text (short) | Where in D1 |
| --- | --- | --- |
| B3.1 | Strictest in / out / cache / total limit per model | §4.8, §4.9 (E7) |
| B3.2 | Each tier applies the strictest limits that fit it, going up the pool | §4.8 |
| B4.1 | low, medium, high work for every pool | §4.6 (every pool row), §4.9 (E2) |
| B4.2 | Align current seats as medium; medium is the default | §4.2 (medium = as written), §4.6 |
| B4.3 | Use AA scores and other factors to find a relative equivalent at low and at high | §4.6, §4.7, §2.3 |
| B4.4 | Tiers may have separate member lists, or overlay the defaults | §4.3 (cell types) |
| B4.5 | Until implemented, xhigh / max / ultra / fast point to high | §4.4 |
| B4.6 | Overlays set any effort, remove a model-effort, replace a model or a model-effort | §4.3 |
| B4.7 | No good low/high: set it to medium, or do not overlay | §4.3 (`same-as-medium`) |
| B4.8 | Cross-pool equivalences are real (summit low ≈ workhorse high; speed or cost brackets) | §4.7 |
| B4.9 | Present and implement low / medium / high for every pool | §4.6, §4.9 |
| B5.1 | Effort data: AA + real fleet data (provider, model, effort, pool, tokens, success) | §4.10 (recorded fields), §5 V3 |
| B7.1 | In low, prefer DeepSeek off-peak, Luna 100k, OpenAI 388k-class, free, lower effort | §4.6 (limited, background, scout) |
| B7.2 | Extrapolate from AA: OpenAI punishes effort above high; off-peak / low context valuable; Claude costs more per %-dollar; speed matters only in small/speed/scout | §4.6 (OpenAI high cap), §4.9 (E6) |
| B7.3 | Cost efficiency is king in workhorse | §4.6 (workhorse rows) |
| B8.6 | Trial many major models at 2+ effort levels where price efficiency aligns | §4.7 (EQ rows), D1-S20 |
| B9.2 | `limited` pool with per-target caps (Luna 100k, Terra 388k, DeepSeek 1M, Haiku 388k) | §4.6, §4.8 |
| B9.3 | DeepSeek `#medium` is not a real rung; it silently remaps to `#high` | §2 (V1 fact 2), §4.2 (no silent remap) |
| B9.6 | Luna round-robin effort trial (PR #9018) is the standing trial pattern | §4.7, D1-S20 |
| S6.1 | Tie-break lean uses low/medium/high classes | §5 (tier classes handed to D5) |
| S3 (decision effort) | Decision effort defaults to no impact | §4.4 (aliases do not touch decision routes) |
| AD1 | One canonical tier, resolved late, per leaf; unsupported = error recorded as `unsupported-tier` | §4.2, §4.5 |
| AD2 | Medium is the pool as written; low/high are keyed overlays; equal-priority conflicts are errors | §4.3 |
| AD3 | Cross-pool equivalence is data with provenance and band | §4.7 |
| AD4 | Limits per tier, strictest wins; fork schema extension for input/cache | §4.8 |
| AD6 | One visited set per expansion; cycle-skipped is distinct from failed | §4.5 (cells with `from`) |

---

## 2. V1 — what exists today (cited, not redesigned)

- **No tier exists.** `#variant` pins a leaf; any pin in a route's expansion makes the whole route advertise no variants (`model-routes.ts:1269`, V1a §b.3). The pin also overrides the caller: in the repo proxy, `if (t.variant) v = t.variant` (`bin/opencode-route-proxy.mjs:800`) runs after the caller's variant is applied, so a pinned target never sees the caller's tier.
- **Medium is a fiction for DeepSeek.** The Go and ClinePass doors remap `medium→high` (`.opencode/opencode.jsonc:2285, 2289, 2364, 2368`); a remap is kept only when the door declares the target (`model-routes.ts:1017-1021`), otherwise it is dropped without a record (V1a §b.4).
- **Silent drops.** A pinned variant the door cannot produce prunes the leaf with `continue` and no decision record (`model-routes.ts:978`). The proxy runs a bare call when `modelDef.variants[leaf.variant]` is missing (`bin/opencode-route-proxy.mjs:1134-1135`, `eff` null), recorded with the requested `variant` string, so the ledger says the tier ran when it did not.
- **Ledger is native-only.** `recordAttempt` writes `variant: leaf.variant` (`bin/opencode-route-proxy.mjs:503-530`, call sites 1254-1392). The requested token and alias are not recorded. Fork table `route_attempt` has one `variant text()` column (`~/git/opencode` `packages/core/src/model-route-log/sql.ts:43`).
- **No overlays, no aliases, no per-tier caps.** V1a §f lists each gap. `xhigh/max/ultra/fast` are real rungs on Claude, Muse and DeepSeek and are remapped per leaf (V1a §b.1, b.4).
- **Context caps exist only per target** (`limit.context`, `.opencode/opencode.jsonc:3597-3671`, schema `model-routes.ts:61-70`) and per route (`:494-503`, "limits the route presents as its own"). Output is the minimum over leaves (`model-routes.ts:338`). Input is derived only when every leaf declares it (`:339-357`). No cache cap exists (V1a §c.3).
- **OpenChamber sends a route-level variant.** `~/.config/openchamber/routing.json` categories carry `"model": opencode-route/workhorse, "variant": "max"` for `research` (and `speed` carries `high`). Today a route-level `max` reaches only unpinned targets; every workhorse class leaf is pinned (V1a §b.3, Q6 in `questions.md`).
- **Two expanders.** The fork expands in `packages/core/src/config/plugin/model-routes.ts` (`expand()` at 796-1050, `routeVariants()` at 1268). The fleet's repo proxy expands its own copy in `bin/opencode-route-proxy.mjs` (`resolveRoute()` 781-808). Both write `route_attempt`. Which one serves a given request is not settled in the baseline (open question Q-D1-1).

---

## 3. Options considered

Each major choice has at least four options. Costs are judged on the brief's criteria: architecture, scalability (pools × tiers × legs), performance (expansion time, cache), reliability (silent-failure surface), extensibility. Prior art is cited to R1 sections.

### 3.1 How a tier is represented (the rung)

| Option | Description | Pros | Cons | Prior art |
| --- | --- | --- | --- | --- |
| **O-a. Per-pool explicit member lists per tier** | Each pool has three full lists. | Zero magic, auditable. | 3× lists, drift between tiers, no shared equivalence. | R1 §7 O1 |
| **O-b. Per-model vocabulary mapped to canonical tiers (rung table)** | Table keyed door × model × tier → native string. | One mapping per model; aliases become data; `unsupported` is a row. | Needs a table kept current with vendor enums. | R1 §3.1, §7 O2; OpenRouter `supported_efforts` (R1 §2.1) |
| **O-c. Token budget per tier** | Rung = thinking budget (4096/8192/16384 as Claude). | Provider-independent. | Gemini 3 maps budgets to levels; OpenAI has no budget knob. | R1 §7 O9 |
| **O-d. Ask the vendor for supported efforts at runtime** | Discover `supported_efforts` from the catalog on each sync. | Never stale. | Catalog snapshot is not live for catalog-only doors (V1a §b.2); not a decision input alone. | OpenRouter `GET /models` (R1 §2.1) |
| **O-e. Single effort word everywhere** | Caller `low/medium/high` passed straight to each door. | Simplest. | Wrong for OpenAI `none/minimal` and Gemini `LOW..HIGH`; silent remaps move to the wire. | Rejected by R1 §3.1 (a) |

**Chosen: O-b**, with O-d as a V3 drift check (D1-S23). Medium is resolved as-written (see §4.2, Challenge C2), so the table holds low and high rows.

### 3.2 How overlays are expressed

| Option | Description | Pros | Cons | Prior art |
| --- | --- | --- | --- | --- |
| **O-f. Keyed overlay ops with priority** (`set/add/remove/replace/replace-effort/members`, key = ref + door + window) | Ops apply to medium; conflicts loud. | Brief B4.6 exactly; NixOS priorities; removal is a tombstone. | Second source of truth; needs a validator. | NixOS `mkOverride` (R1 §5.1), Kustomize SMP keyed merge (R1 §5.2) |
| **O-g. Index-addressed patches** (`/legs/3`) | JSON 6902 style. | Precise. | Breaks on reorder; not keyed. | R1 §5.2 (rejected as default) |
| **O-h. Full replacement per tier** (= O-a) | No overlay at all. | Simple. | Forces copy of medium into low/high. | — |
| **O-i. Tier as a separate route** (`summit-low`) | Route ids per tier. | Works today with no code. | Callers name the tier in the id; no `same-as-medium`; tier pins still pin. | — |
| **O-j. Last-wins merge** | Later overlay replaces earlier. | Familiar (Helm/--set). | Read-order sensitive; silent. | R1 §5.3 warns against it |

**Chosen: O-f.** O-i is the V2 bridge where the fork schema is not yet extended (§4.11).

### 3.3 How aliases are handled

| Option | Description | Pros | Cons |
| --- | --- | --- | --- |
| **O-k. Alias table as data** (`aliases: {xhigh: high, max: high, ultra: high, fast: high}`) | Resolver maps caller token → tier; record keeps token and alias. | Reversible; visible; one line to implement a real rung later. | Loses Claude `max` (65k thinking) until a rung exists. |
| **O-l. Hard-coded in resolver** | Same behaviour, in code. | None over O-k. | Not auditable; not reversible without a deploy. |
| **O-m. Keep xhigh/max as native rungs** | No alias; each leaf keeps its own names. | Max quality on Claude. | Brief B4.5 says no; `#max` is per-leaf today, so `research` cannot be tiered. |
| **O-n. Alias applied to pins as well** | Medium pins `xhigh` become `high`. | One rule. | Changes today's medium for speed, summit, specialist (B4.2 breaks). |

**Chosen: O-k**, applied to caller tokens only (medium pins keep their native string).

### 3.4 Cross-pool equivalence

| Option | Description | Pros | Cons | Prior art |
| --- | --- | --- | --- | --- |
| **O-o. Equivalence rows with provenance and a quality band** | `a ≈ b` within δ points at cost ratio r, tagged `XAA`. | Matches brief B4.8; explicit about extrapolation. | Needs maintenance. | Azure Balanced 1–2 % and Cost 5–6 % bands; Bedrock IPR threshold (R1 §2.10) |
| **O-p. Computed Pareto frontier** | Tiers as frontier points from measured data. | Honest over time. | No measured points yet; needs D3 store. | RouterBench convex hull (R1 §2.6) |
| **O-q. Pairwise judge (Bradley–Terry)** | Fit strengths from pairwise outcomes. | Principled CIs. | Needs judge and labelled data (V3/V4). | R1 §4.2 |
| **O-r. Implicit (no rows)** | Trust AA numbers in comments. | None. | Reproduces V1's "decided in comments" (V1a §b.5). | — |

**Chosen: O-o now, O-p in V3** (D1-S21, D1-S22).

### 3.5 Per-tier limits

| Option | Description | Pros | Cons |
| --- | --- | --- | --- |
| **O-s. Four ceilings per tier** (`input, output, cache, total`) with strictest-wins | Brief B3.1/B3.2 exactly. | Expresses plan ceilings and reasoning reserve. | Needs fork schema (cache and tier-level output do not exist). |
| **O-t. Context cap per target only** (today's `limit.context`) | Use V1's only cap. | Works now. | Cannot express output or cache; cannot differ by tier. |
| **O-u. Limit as a pool-level policy object** | Pool-wide caps in a separate section. | One place. | Splits the cap from the tier that needs it; conflicts with per-target caps. |
| **O-v. Model-declared ceilings only** | Trust native limits. | No config. | Ignores plan ceilings (`haiku 388k`, `terra 388k`) that drive B9.2. |

**Chosen: O-s**, with V2 expressing only `total` (via target `limit.context`) and `output` (via model `limit.output`, min over leaves) (§4.8).

### 3.6 Where the rung table lives

| Option | Description | Pros | Cons |
| --- | --- | --- | --- |
| **O-w. Fork config schema `experimental.routes.effort`** | Validated at load by the fork. | Single validator; live in both expander and resolver. | Fork change needed before any V2 value lands. |
| **O-x. Sidecar `.opencode/route-effort.jsonc`** | Read by repo proxy and guard; fork ignores it until the schema slice lands. | Lands now; no fork dependency; diff-reviewable. | Second file; fork must later read it or migrate. |
| **O-y. Inline in `.opencode/opencode.jsonc`** | Next to pools. | One file. | The strict fork schema may reject unknown keys (not verified in this pass); 3,731 lines already. |
| **O-z. Code table in `bin/`** | Hard-coded natives. | Testable. | Not operator-editable; repeats the V1 "decided in code" problem. |

**Chosen: O-x for V2, migrating to O-w in D1-S13.** Default answer for the unknown-key question (Q-D1-3): sidecar, so `opencode.jsonc` is not touched until the schema slice lands.

### 3.7 Recording

| Option | Description | Pros | Cons |
| --- | --- | --- | --- |
| **O-α. Add `effort_requested`, `effort_tier`, `effort_alias`, `effort_source`, `effort_declared`, `rung_provenance` beside `variant`** | `variant` keeps meaning "native sent". | Additive; back-compatible; the record says what ran and what was asked. | Six nullable columns. |
| **O-β. Overwrite `variant` with the tier** | One column. | Simple. | Breaks every existing reader of `variant` (V1 fact 2 shows trials already misread it). |
| **O-γ. A separate `effort_record` table** | Joined by attempt id. | No change to `route_attempt`. | Join on every read; ledger already the source (main.md AD8). |

**Chosen: O-α.** Recorded in §4.10.

---

## 4. V2 design (chosen)

### 4.1 Terms

- **Door** — a provider id (`_claude`, `_openai.main`, `opencode-go.developing-today-0`, `cline-pass`, `opendesign`, `mistral`, `_muse`, `_antigravity`, `_vibe`).
- **Leaf** — a `(door, model)` pair with an optional native variant string (the existing `Model.Ref`).
- **Member** — one entry in a route's target list (a leaf reference or a route reference), with optional window gate and as-written variant.
- **Tier** — canonical `low | medium | high`. Only these three are real. Callers may send an alias (§4.4).
- **Rung** — the row `(door, model, tier) → native`. The native string is what is sent to the door.
- **Cell** — one tier's definition for one route: `rungs`, `same-as-medium`, `propagate`, `unsupported`, `members` or `overlay`.

### 4.2 Resolution rule for medium and for the other tiers

- **medium** resolves **as written**: each member keeps its own variant (today's pin, or default when unpinned). Nothing about medium changes at V2. This is what B4.2 asks for and what AD2 states.
- **low** and **high** resolve through the rung table, or through an overlay that names members explicitly. A leaf with no rung row in a tier whose cell is not `same-as-medium` is **not run**: it is skipped and recorded as `unsupported-tier` (AD1). Nothing is remapped silently.
- A rung row may be the literal `"=medium"`: the leaf's low (or high) native is its as-written medium native. This is the per-leaf form of `same-as-medium`, used where a door has no tier data (Muse Spark, MiMo, Beam). It is declared by its `prov` (`XAA` where no tier data exists), so E3 is satisfied without a silent fallback.
- Rung rows are generated, not hand-written, where a config remap already declares the target (`variants` maps at `opencode.jsonc:2285-2305, 2364-2372, 2470, 2544, 2572-2596, 2683`). The generator turns each remap into a row with `prov: CFG`, `declared: true`, and the existing line.

Challenge C2 (to AD1/AD2 wording, not a deviation): medium is resolved as written, not from the rung table, so the rung table's `medium` column is an audit copy only. The guard checks the two agree (E4).

### 4.3 Overlay grammar and cells

#### 4.3.1 Cell types (every route × tier must have one; E2)

| Cell | Meaning | Use |
| --- | --- | --- |
| `"rungs"` | Members resolve via rung rows; no list change. | Pools whose legs all have low and high rows (e.g. `deepseek-flash-plan` low/high). |
| `"same-as-medium"` | Members run exactly as written. Declared, not silent (B4.7). | No good option at this tier, or no data. |
| `"propagate"` | A route member receives the same tier request. | Class routes (default for route members). |
| `{ "unsupported": "reason" }` | The whole route has no defined tier; every call is recorded as `unsupported-tier`. | GLM pools at medium (no medium rung). Requires a reason. |
| `{ "members": [ ... ] }` | Replaces the member list for this tier (AD2 "separate list"). | Trial routes at a tier (`deepseek-effort-rr` low/high). |
| `{ "from": "pool#tier", "overlay": [ ... ] }` | Starts from another route's tier (AD3 composition edge), then applies overlays. | Equivalence-derived tiers; visited-set guarded (AD6). |
| `{ "overlay": [ ... ] }` | Starts from medium; applies ops. | Most pools. |

#### 4.3.2 Operations

| Op | Keys | Effect | Error if |
| --- | --- | --- | --- |
| `set` | `ref`, optional `door`, `window`; `effort` | This member runs at canonical `effort` in this tier (rung lookup). | matches 0 or >1 members (unless `all`); no rung for `effort`. |
| `add` | `ref`, `door`, `effort`, `order` | Inserts a member not in medium. | a member with the same key already exists after prior ops. |
| `remove` | `ref`, optional `door`, `window`; `reason` | Tombstone: member is absent in this tier. The tombstone is kept in the resolved view. | matches 0 members. |
| `replace` | `ref`, `door`, `window`; `with: {ref, door, effort}` | Swaps one member for another. | matches 0 members. |
| `replace-effort` | `ref`, `door`, `native`, `reason` | Forced native string for one leaf in this tier, bypassing the rung row. `declared` is false and `reason` is mandatory. | `reason` missing. |
| `members` | (cell type only) | Replaces the whole list. | combined with other ops in the same tier. |

Common keys: `priority` (default `1000`; lower wins, NixOS style; `50` is "forced"), `reason` (mandatory for `remove`, `replace-effort`, and any op whose `effort` is not in the rung table), `prov` (provenance code, §4.6.1), `all` (default `false`).

**Keying.** An op's key is `(ref, door?, window?)`. It matches members by exact `ref`, and by `door` / `window` when given. A key matching more than one member is an error unless `all: true`. This matters for `deepseek-flash-plan` inside `background`, which appears twice with different windows (`opencode.jsonc:2957, 2961`).

**Route members.** When the key names a route member (`opencode-route/...`), `set effort E` sets the tier that child receives (E is then passed down, §4.5); `remove` drops the child from this tier. A leaf member's `set effort E` looks up the rung for E on that door and model.

**Priority and conflicts.**
1. Ops on the same member at different priorities: the lower number wins; the loser is recorded as `overridden` in the resolved view.
2. Ops on the same member at the **same** priority: `overlay-conflict` error. Never last-wins (R1 §5.1, the copy-worthy property).
3. A `remove` at priority p and a `set`/`add` at q < p restores the member. At q > p it stays removed.

#### 4.3.3 JSONC examples (the V2 sidecar, `.opencode/route-effort.jsonc`)

```jsonc
// .opencode/route-effort.jsonc — V2 sidecar (D1-S1). Read by bin/route-effort.mjs and
// the guard. Moves under experimental.routes.effort when the fork schema slice lands (D1-S13).
{
  "version": 1,
  "tiers": ["low", "medium", "high"],
  "aliases": { "xhigh": "high", "max": "high", "ultra": "high", "fast": "high" },
  "pools": {
    // summit: quality tier. Low keeps Sol and the cheap Opus leg; high drops the dominated Claude legs.
    "summit": {
      "low": {
        "overlay": [
          { "op": "remove", "ref": "opencode-route/fable-plan", "priority": 1000, "prov": "AA5",
            "reason": "fable low 38 @ $4.47 is dominated by sol low 35 @ $0.16 within the 5-point band" },
          { "op": "remove", "ref": "opencode-route/argon-plan", "priority": 1000, "prov": "AA5",
            "reason": "argon low 45 @ $9.25 is dominated by argon medium 49 @ $8.45" },
          { "op": "remove", "ref": "_muse/muse-wonder-1.0-contributor", "priority": 1000, "prov": "FLT",
            "reason": "no low rung measured; 100% failed in 72h (JSON door_health)" },
          { "op": "remove", "ref": "opencode-route/opus-plan", "priority": 1000, "prov": "AA5",
            "reason": "opus low 34.8 @ $0.98 is dominated by sol low 35 @ $0.16 in the same tier (EQ5)" },
          { "op": "set", "ref": "opencode-route/sol-plan", "effort": "low", "priority": 1000, "prov": "AA5" },
          { "op": "set", "ref": "_antigravity/gemini-4-hydrogen", "effort": "low", "priority": 1000, "prov": "AA5" }
        ]
      },
      "high": {
        "overlay": [
          { "op": "set", "ref": "opencode-route/sol-plan", "effort": "high", "priority": 1000, "prov": "AA5",
            "reason": "sol high 50 @ $0.59 replaces medium 46 @ $0.36; it is 17% of opus high's cost for 2 points less" },
          { "op": "remove", "ref": "opencode-route/fable-plan", "priority": 1000, "prov": "AA5",
            "reason": "fable high 47 @ $7.54 is dominated by opus high and sol high" },
          { "op": "remove", "ref": "opencode-route/argon-plan", "priority": 1000, "prov": "AA5",
            "reason": "argon high 50.4 @ $9.95 is dominated by hydrogen high 51 @ $1.87" },
          { "op": "remove", "ref": "_muse/muse-wonder-1.0-contributor", "priority": 1000, "prov": "AA5",
            "reason": "muse wonder max 43 @ $6.95 is dominated by opus high 51.8 @ $3.46" }
        ]
      }
    },
    // workhorse-weighted: a set that replaces a pinned leg needs a rung, not a remap.
    "workhorse-weighted": {
      "low": {
        "overlay": [
          { "op": "remove", "ref": "opencode-route/luna-effort-rr", "priority": 1000, "prov": "AA5",
            "reason": "luna low 10 is below the workhorse floor; cost saving is $0.01 per task" },
          { "op": "replace", "ref": "_antigravity/claude-sonnet-5-5", "with": { "ref": "_claude/claude-haiku-5-5", "effort": "low" },
            "priority": 1000, "prov": "AA5", "reason": "Claude premium: haiku low 19 @ $0.04 instead of sonnet medium 30.8 @ $0.78" }
        ]
      },
      "high": {
        "overlay": [
          { "op": "set", "ref": "_antigravity/gemini-4-hydrogen", "effort": "high", "priority": 1000, "prov": "AA5" }
        ]
      }
    },
    // ClinePass low was silently remapped to high in V1 (opencode.jsonc:2368). Its low leaf is removed.
    // OpenDesign low is catalog-only (CAT, unverified): forced to its native `low` with a reason.
    "deepseek-pro-plan": {
      "low": {
        "overlay": [
          { "op": "remove", "ref": "cline-pass/deepseek-v4-pro", "priority": 1000, "prov": "CFG",
            "reason": "no verified low rung on ClinePass; V1 remapped low to high silently (opencode.jsonc:2368)" },
          { "op": "replace-effort", "ref": "opendesign/deepseek-v4-pro", "native": "low", "priority": 1000, "prov": "CAT",
            "reason": "OpenDesign low is catalog-only; forced to native low until D1-S3 confirms the variant" }
        ]
      }
    }
  }
}
```

The `replace-effort` op is the only escape from a rung row, and it always carries a reason. E5 rejects a `replace-effort` whose `native` equals the rung row it bypasses (a no-op) and rejects two ops on one member at one priority.

#### 4.3.4 Keyed merge algorithm (pseudocode)

```
applyOverlays(members, ops):
  // members: ordered list of { ref, door, window, native, order, tombstone? }
  groups = groupBy(ops, op -> key(op.ref, op.door ?? "*", op.window ?? "*"))   // E5
  for (key, group) in groups:
    targets = members.filter(m -> matches(m, key))
    if op.op != "add" and targets.len == 0:      error("overlay-no-target", key)
    if targets.len > 1 and !group.all:           error("overlay-ambiguous", key)
    best = group.minPriority()
    if group.filter(priority == best).len > 1:   error("overlay-conflict", key)
    apply(best, targets)   // set/remove/replace/replace-effort/add
  return members
  // tombstones: members with tombstone = true are removed from the list but kept in `overridden` / `removed` for the record
```

Terminates in O(|ops| × |members|). No op can add a member that another op removes at the same priority (E5 catches it).

### 4.4 Aliases (B4.5)

`aliases` maps a caller token to a canonical tier. Rules:
- The alias table is data (`route-effort.jsonc` `aliases`), not code.
- Keys must not be canonical tiers; values must be canonical tiers; no chains (an alias value cannot be an alias key). E1.
- `xhigh`, `max`, `ultra`, `fast` are the four required keys (B4.5). The guard fails if one is missing.
- The record keeps the **caller's token** (`effort_requested`), the **alias applied** (`effort_alias`, e.g. `max->high`) and the **canonical tier** (`effort_tier`). A caller who sends `high` gets `effort_alias = null`.
- **OpenChamber `research → opencode-route/workhorse (max)`.** The category sends the route variant `max`. The resolver aliases it to `high` before expansion. Workhorse high therefore runs: deepseek flash high (via `workhorse-ordered`), Luna high and Hydrogen high (via `workhorse-weighted`), Haiku high and no fable (via `workhorse-last-resort`). Today the same call ran the workhorse pins (mostly medium-equivalent) because every class leaf is pinned. This is a behaviour change for `research`: it is intended by B4.5 but is more expensive per %-dollar than the pins it replaces. D1 does not edit the category (Q6 default; the category is D2's). Recommendation for D2: set the category to `high`, which is the same effect with an honest token.
- Decision routes (S3, D5) do not use aliases. Decision effort defaults to no impact (S3); the alias table is not consulted for `decision` pools.

### 4.5 Resolution algorithm

```
resolveRoute(route, requestToken, ctx):          // ctx.visited is the per-call set (AD6)
  token = requestToken ?? "medium"
  tier  = aliases[token] ?? token
  alias = (tier != token) ? token : null
  if tier not in tiers: return error("invalid-tier", token)
  if route.id in ctx.visited: return { skipped: "cycle-skipped", route: route.id }
  ctx.visited.add(route.id)
  cell = route.pools[tier]                       // defaults: medium -> "as-written"
  base = route.members                           // as written (medium)
  if tier == "medium" or cell == "same-as-medium": members = base
  elif cell == "propagate": members = base      // route members get `tier` below
  elif cell.unsupported:    return { skipped: "unsupported-tier", reason: cell.unsupported }
  elif cell.members:        members = cell.members
  else:
    source = cell.from ? resolveRoute(lookup(cell.from), cell.from.tier, ctx).members : base
    members = applyOverlays(source, cell.overlay)
  out = []
  for m in members:
    if m.isRoute: out += resolveRoute(m.route, tier, ctx)           // propagation
    else:
      native = (tier == "medium") ? m.asWrittenNative : rung(m.door, m.model, tier)
      if native is undefined and cell != "same-as-medium":
        out += skipped(m, "unsupported-tier")
      else:
        out += leaf(m.door, m.model, native, source = tier == "medium" ? "as-written" : (cell == "same-as-medium" ? "same-as-medium" : "rung"))
  return out
```

Order of operations: caller token → tier → cell → members → leaves. A leaf reached by two paths keeps the smallest per-target limit (`model-routes.ts:847-850`, unchanged).

### 4.6 The proposed low / medium / high table for every pool

**Read this table with three rules.**
1. **Medium** is the pool as written in V1 (JSON `routes`), unchanged.
2. **Cells** are the §4.3.1 types. "same as medium" = `same-as-medium` (declared). "rungs" = no list change, rung rows decide.
3. **Provenance codes** in brackets: `CFG` declared in config (line cited); `CAT` declared in the live catalog snapshot, unverified against the door; `AA5` AA v5 rankings (score / $ per task); `XAA` extrapolated (cross-version, cross-door, or from a non-reasoning proxy); `FLT` fleet observation (`route_usage` 72 h, V1a §e); `RULED` brief or ruling. No row is `MEAS`: no measured effort trial exists yet (V1a §b.4 "deepseek-effort-rr" is not reviewed).

#### 4.6.1 AA v5 point data used (score / $ per task; sources: `aa-last-good.json` rankings, AA|v5)

| Model | low | medium | high | xhigh | max | Note |
| --- | --- | --- | --- | --- | --- | --- |
| Claude Haiku 5.5 | 19 / 0.04 | 25 / 0.09 | 30 / 0.16 | 34 / 0.25 | 36 / 0.41 | Cheapest Claude |
| Claude Sonnet 5.5 | 25.6 / 0.51 | 30.8 / 0.78 | 41.5 / 1.64 | 49.9 / 4.09 | 54.5 / 10.02 | |
| Claude Opus 5.5 | 34.8 / 0.98 | 48.2 / 2.45 | 51.8 / 3.46 | 57.4 / 6.05 | 59.1 / 10.04 | |
| Claude Fable 5.1 (config `claude-fable-5-2`, XAA) | 38 / 4.47 | 43 / 5.56 | 47 / 7.54 | 50 / 10.85 | 49 / 12.99 | Version name differs (Q-D1-8) |
| GPT-6 Luna | 10 / 0.01 | 18 / 0.04 | 21 / 0.07 | 23 / 0.10 | 28 / 0.15 | max banned (OpenAI cap) |
| GPT-6.1 Terra | 29 / 0.07 | 38 / 0.17 | 42 / 0.26 | 45 / 0.40 | 51 / 0.96 | max banned |
| GPT-6.1 Sol | 35 / 0.16 | 46 / 0.36 | 50 / 0.59 | 52 / 0.76 | 55 / 1.27 | |
| Gemini 4 Hydrogen | 27 / 0.29 | 41 / 0.80 | 51 / 1.87 | 54 / 3.16 | — | |
| Gemini 4 Argon | 45 / 9.25 | 49 / 8.45 | 50.4 / 9.95 | 51 / 17 | — | Most expensive door |
| DeepSeek V4.1 Flash | non-reasoning 19 / 0.21–0.43 (1486 tps); 1017 high 29; max 32 / 0.60 | — | — | — | — | No low/medium rows. Low proxy is non-reasoning (XAA). |
| DeepSeek V4 Pro 0813 | non-reasoning 11; max 23 | — | — | — | — | Weaker than Flash on AA5 |
| Muse Spark 1.3 | — | — | — | 34 (no $) | max 36 / 2.39. AA v4.3.2 has xhigh 45 / 1.37, max 48 / 1.60 |
| Muse Wonder | — | — | — | — | 43 / 6.95 | |
| GLM-5.3 | low 23 | — | — | — | 32 / 5.01 | Flash: reasoning 28 / 0.58 |
| Beam 1.0 | reasoning 17.7 / 0.28 (one point) | | | | | Free tier ended (JSON) |
| MiMo-V2.6-Flash | reasoning 27 (no $) | | | | | Cline cost 0.06 (catalog, XAA) |

Catalog column `aaScore` in `aa-last-good.json` `catalog[]` is v4.3.2 for Muse (48 vs v5 36). The V2 rows use AA5 only (Q-D1-7).

#### 4.6.2 Pool rows

Columns: **Pool** (route id, JSON line) · **Medium (as written)** · **Low cell** · **High cell** · **Provenance and notes**. Class routes list their children; children apply the same tier request.

| Pool (line) | Medium (as written) | Low | High | Provenance and notes |
| --- | --- | --- | --- | --- |
| **small** (3672) | `limited`, `speed` | propagate | same-as-medium | [RULED B7.1, B7.3] Speed matters in small; high is not worth the speed cost for these light calls. |
| **speed** (3544) | Muse xhigh; DS flash high; Hydrogen high; Haiku high; then workhorse, summit, specialist | overlay: `set` DS flash low; `set` Haiku low; `set` Hydrogen low. Muse unchanged (same-as-medium leaf). | propagate to chain | [AA5] Hydrogen low 78 s vs high 360 s (JSON time). Haiku low 102 s. DS flash non-reasoning 44 s at 1486 tps. Speed is the one place low is chosen for time, not cost (B7.2). |
| **workhorse** (3176) | `workhorse-ordered`, `workhorse-primary` (empty), `workhorse-weighted`, `workhorse-last-resort`, then scout, specialist | propagate | propagate | [RULED B7.3] Cost efficiency is king: the class decides by its children (see below). |
| **summit** (3490) | fable medium; opus high; argon high; Hydrogen high; Muse Wonder xhigh; sol xhigh; then specialist | overlay (§4.3.3): remove fable, argon, Muse Wonder; set sol low; keeps opus low and Hydrogen low (Hydrogen low: set) | overlay: set sol high; remove fable, argon, Muse Wonder | [AA5] Claude-high is dominated: fable high 47 @ 7.54 vs opus high 51.8 @ 3.46; argon high 50.4 @ 9.95 vs hydrogen high 51 @ 1.87. Summit low ≈ workhorse high (§4.7 EQ1). |
| **specialist** (sink, 3445) | fable low; sonnet high; argon high; spark xhigh; sol medium | same-as-medium (sink must not weaken) | overlay: set sol high; remove fable | [RULED sink] Sol high 50 @ 0.59 replaces sol medium 46 @ 0.36. Fable high dominated (§4.6.1). |
| **scout** (2859) | beam; DS flash low; Haiku low; workhorse | same-as-medium (already the cheapest rungs; beam has no low row) | overlay: set DS flash high; set Haiku high | [AA5 + RULED B7.1] DS flash high 29 @ $0.43–0.60 gives a real option when explore is hard. Explore children are 716 of 747 on workhorse (`opencode.jsonc:1734-1752`), so scout high must not be the default. |
| **background** (2898) | MiMo (peak-early, peak-late), DS flash high (not early), DS flash high (early), MiMo, then workhorse | overlay: `set` DS flash low for both windows (`all: true` on the `ref` key) | same-as-medium | [RULED B7.1 off-peak, B7.3] DS low (19 AA5, non-reasoning proxy; XAA) is the cheapest good option for batch work. Off-peak windows stay as gates. |
| **workhorse-ordered** (3007) | `deepseek-effort-rr` (trial: high w3 / low w1) | propagate to `deepseek-effort-rr` low | propagate to `deepseek-effort-rr` high | [CFG 2992-3002] Trial route; per-tier cells replace the trial split at tier requests. |
| **deepseek-effort-rr** (2970) | DS flash high w3, DS flash low w1 (trial) | `members`: DS flash low (rung) | `members`: DS flash high (rung) | [CFG] The trial's 5,000-draw review (2992-2997) still applies to the medium split. |
| **workhorse-primary** (3030) | — (empty since 2026-11-07) | same-as-medium | same-as-medium | [CFG 3167] Contributes nothing. |
| **workhorse-weighted** (3240) | luna-effort-rr w10; Muse high w10; Hydrogen low w10; Sonnet (antigravity) medium w10 | overlay: remove luna-effort-rr; replace antigravity Sonnet → Haiku low | overlay: set Hydrogen high; set luna-effort-rr → `members`: Luna high | [AA5] Luna low 10 is too weak for the default pool. Claude premium: Haiku low 19 @ $0.04 vs sonnet medium 30.8 @ $0.78 (B7.2). Muse: same-as-medium (no low/high data). |
| **workhorse-last-resort** (3400) | Haiku high; fable low | overlay: set Haiku low; remove fable | overlay: remove fable | [AA5] Fable is the expensive leg at either end: low 4.47, high 7.54. Haiku is the only door in this pool that is not dominated. |
| **luna-effort-rr** (2481) | openai Luna medium, Luna high (round robin) | `members`: openai-plan Luna low | `members`: openai-plan Luna high | [CFG 2481-2501] Trial route. Low 10 is kept for the limited-tier path only. |
| **limited** (3597) | Luna medium (100k), Terra low (388k), MiMo peak-early / peak-late (1,048,576), DS flash low ×2 (1M), Haiku medium (388k) | overlay: set luna-plan low | overlay: set terra-plan high; set haiku-plan high | [AA5 + RULED B7.1] Luna low 10 @ $0.01 is acceptable only at the 100k cap (light work). Terra high 42 @ $0.26 is the 388k-class upgrade. Caps are per target and do not change with tier (B3.2: tier caps add, not replace). |
| **deepseek-flash** (2325) | wrapper → deepseek-flash-plan | propagate | propagate | [CFG] |
| **deepseek-flash-plan** (2165) | Go DS flash (w2, medium→high), ClinePass DS flash (w1, medium→high), OpenDesign DS flash (w1, xhigh/max→high), Mistral 1017 (w1, medium→high) | rungs (Go low CFG 2186-2187; ClinePass low CAT; OpenDesign low CAT; Mistral low CFG `_vibe` 602-623) | rungs (all doors high) | [CFG/CAT] Medium on Go, ClinePass and Mistral is high in fact (silent in V1; now declared rows, E4). OpenDesign medium is the door default (unverified). |
| **deepseek-pro-plan** (2348) | Go Pro (w3, medium→high), ClinePass Pro (w1, low→high, medium→high), OpenDesign Pro (w1) | overlay: remove ClinePass Pro (no verified low; V1 remapped silently) | rungs | [CFG 2363-2373] ClinePass low→high is the silent remap called out in V1 fact 2. Pro is dominated by Flash on AA5 (Pro max 23 vs Flash max 32): D4 candidate. |
| **fable-plan** (2381) | `_claude` fable (bare = default) | rungs (declared low) | rungs (declared high) | [CFG 441-484; AA5 Fable 5.1 XAA] Fable is dominated at both ends (§4.6.1); single door, so the pool itself carries no removal. |
| **haiku-plan** (2397) | `_claude` Haiku (bare = default) | rungs (declared low) | rungs (declared high) | [CFG 284-326; AA5] Haiku is the cheapest Claude door. Live door at 34% success (FLT); D4 owns its health. |
| **sonnet-plan** (2550) | `_claude` Sonnet (round robin); antigravity Sonnet (xhigh/max→high, dead: 100% fail) | rungs (Claude low; antigravity low CAT) | rungs | [CFG 333-376; AA5] Sonnet high is what `specialist` already pays (sonnet#high), so the high cell is an existing seat, not a new cost. |
| **opus-plan** (2578) | `_claude` Opus (ordered); antigravity Opus (xhigh/max→high) | rungs | rungs | [CFG 382-425; AA5] Opus low 34.8 @ 0.98 is dominated by Sol low 35 @ 0.16 across pools (EQ6); D4 candidate for replacement at low. |
| **gemini-hydrogen-plan** (2512) | antigravity Hydrogen (max→xhigh remap; bare) | rungs (CAT low) | rungs (CAT high) | [CAT] Door is 100% failed (FLT). Medium is bare = default; see Q-D1-2. |
| **luna-plan** (2430) | openai Luna (bare), opendesign Luna (xhigh/max→high), Go Luna (walled until 2026-11-15) | rungs (openai low CAT) | rungs | [AA5 + B7.2] OpenAI cap: high is the top rung; xhigh/max banned on OpenAI doors (E6). |
| **terra-plan** (2692) | openai Terra (bare) | rungs (Terra low 29 @ 0.07) | rungs (Terra high 42 @ 0.26) | [AA5 + B7.1] The 388k-class OpenAI seat. Door health: `_openai.main` 98% fail, `_openai.alt` 82% fail; retired `_openai` id carries 17,858 attempts at 4% fail (FLT; Q-D1-9). |
| **sol-plan** (2647) | openai Sol (w3, bare); opendesign Sol (xhigh/max→high) | rungs (Sol low 35 @ 0.16) | rungs (Sol high 50 @ 0.59) | [AA5] Sol is the best cheap OpenAI point at both ends; `_openai.main` 8% fail (FLT). Summit uses sol xhigh as written (medium); the high cell becomes sol high. |
| **spark-plan** (2602) | Go Muse, ClinePass Muse, `_muse` Muse (ordered) | same-as-medium | same-as-medium | [AA5 + XAA] Muse Spark 1.3 has no low/high in AA v5. Config declares low/high (`opencode.jsonc:1082`); candidate for a D4 trial. Door is 100% fail (FLT). |
| **argon-plan** (2732) | Go Argon (w3), antigravity Argon (bare) | same-as-medium (low 45 @ 9.25 is dominated by medium 49 @ 8.45) | same-as-medium (high 50.4 @ 9.95: +1.4 for +$1.50 per task, too expensive for the gain; B4.7) | [AA5] Most expensive door. D4 candidate. |
| **beam-plan** (2769) | Go beam-free (w3); walled until 2026-11-15 | same-as-medium | same-as-medium | [AA5 one point, FLT] No effort split in AA. |
| **mimo-flash-plan** (2792) | Go MiMo (w3), ClinePass MiMo, OpenDesign MiMo | same-as-medium | same-as-medium | [AA5 one point; XAA cost] No low/high data. Used in `background` and `limited` at peak windows. |
| **glm-5-3-plan** (2819, defined only) | `_vibe` GLM 5.3 (xhigh→max) | rungs (GLM low, AA5 23) | rungs (GLM high, declared) | [CFG `_vibe` 638-659] No medium rung: medium cell = `{ "unsupported": "GLM declares none/low/high/max; no medium" }` (E2 accepts with a reason). No route references it (2831). |
| **glm-5-3-flash-plan** (2841, defined only) | `_vibe` GLM 5.3 Flash | rungs | rungs | [CFG `_vibe` 669-690] As glm-5-3-plan; medium unsupported. |
| **openai-plan** (provider route, 2097) | members `_openai.alt`, `_openai.main` | n/a: a door selector; tiers resolve on its leaves | n/a | [CFG 2097-2114] |
| **mistral-plan** (provider route, 2126) | members `_vibe`, `mistral` | n/a | n/a | [CFG 2126-2128] |
| **opencode-go-plans** (provider route, 2149) | members `drewrypope`, `popesdrew`, `developing-today-0` | n/a | n/a | [CFG 2149-2152] Every Go leaf is 100% failed (FLT), walled until 2026-11-15 (CFG 2774-2775, 3324-3326). |

**Coverage check.** Every route in JSON `routes[]` has a row (38 rows including the three provider routes). Each row has a low cell and a high cell, which is the B4.9 requirement. Medium is as written by definition.

**Door health in the rows.** Cells are chosen on AA and cost; door health is a runtime input (walled-skipped, D4). The table does not pick doors by health. A walled leg still carries its rung row, so when the door recovers the tier works without a config edit (AD9 boundary).

#### 4.6.3 Principles applied (for review)

- **Cost is the tie-breaker in workhorse and background** (B7.3). A leg is kept at high only when its AA gain over medium exceeds the cost ratio, or when the tier is the one that owns quality (summit, specialist).
- **OpenAI caps at high** (B7.2). No OpenAI rung is xhigh or max. This is enforced by E6.
- **Claude is treated as dearer than AA says** (B7.2). Where a Claude low or high is dominated by a non-Claude point at the same tier, it is removed from the tier (summit, workhorse-last-resort), not kept for "quality".
- **Speed only in small, speed, scout** (B7.2). Elsewhere a faster door is not a reason to change a tier.
- **Off-peak is a window gate, not a tier.** DS off-peak wins are expressed by windows already in config; tiers do not duplicate them.

### 4.7 Cross-pool equivalence rows (AD3, B4.8)

Equivalence is data (`equivalences` in `route-effort.jsonc`). A row names two `route#tier` or `leaf#tier` references, a quality band in AA points, a cost ratio, and provenance. Equivalence is *claimed* only inside the band; outside, the row is a dominance or a bracket.

**Band definitions (V2):** `a ≈ b` iff |score(a) − score(b)| ≤ 5 AA points. `a dominates b` iff score(a) ≥ score(b) and cost(a) ≤ cost(b), strict in one. Cost ratio = cost(a) / cost(b).

| Row | a | b | Score a / b | Cost a / b | Relation | Provenance | Claim |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **EQ1** | `summit#low` (sol low) | `workhorse#high` (Haiku high) | 35 / 30 | $0.16 / $0.16 | a ≈ b and a ≥ b at equal cost | XAA (cross-door, cross-pool) | This is the brief's example (B4.8): summit low is at least workhorse high for the same money. Trial candidate (D1-S20). |
| **EQ2** | `scout#low` (DS flash low) | `speed#low` (Haiku low) | 19 / 19 | $0.21–0.43 / $0.04 | a ≈ b; b cheaper by 5–10× | XAA (non-reasoning proxy) | Speed bracket: DS is 1486 tps vs Haiku 158 tps (B7.2). Cost bracket: Haiku wins. Choose by role (scout = speed, background = cost). |
| **EQ3** | `terra-plan#high` (Terra high) | `sol-plan#medium` (Sol medium) | 42 / 46 | $0.26 / $0.36 | a ≈ b (within 4); a cheaper | AA5 | Middle point: Terra high is not dominated by Sol medium; both stay on the frontier (B4.8, "the middle"). |
| **EQ4** | `specialist#high` (Sol high) | `summit#high` (opus high) | 50 / 51.8 | $0.59 / $3.46 | a ≈ b; a is 17% of cost | AA5 | Sol high dominates opus high in the summit high tier. Drives D4 replacement. |
| **EQ5** | `summit#low` (sol low) | `opus-plan#low` (opus low) | 35 / 34.8 | $0.16 / $0.98 | a ≈ b; a is 16% of cost | AA5 | Sol low dominates opus low. Opus low is the first D4 candidate. |
| **EQ6** | `workhorse-last-resort` (haiku high) | `fable-plan#high` | 30 / 47 | $0.16 / $7.54 | b scores +17 at 47× cost | AA5 | Fable high is never worth it in a workhorse chain. Band is wide; the row says "not claimed equivalent". |
| **EQ7** | `limited#low` (Luna low, 100k) | `limited#medium` (Luna medium, 100k) | 10 / 18 | $0.01 / $0.04 | b dominates on score; a is ¼ cost | AA5 | Light work accepts Luna low only at the cap. Trial candidate. |

**Fleet use.** Equivalence rows are not used by the expander. They are read by the guard (E8: both sides resolve, bands are finite) and by D4 (dominated-model proposals). A V3 frontier (D1-S22) replaces the hand-set rows with measured ones; the schema keeps the same fields so the swap is a data change.

### 4.8 Per-tier limits (AD4, B3.1, B3.2)

#### 4.8.1 Shape

```jsonc
// tier overlay with limits (fork schema, D1-S13; V2 sidecar may carry only `total`)
"limited": {
  "high": {
    "overlay": [ { "op": "set", "ref": "opencode-route/terra-plan", "effort": "high", "priority": 1000, "prov": "AA5" } ],
    "limits": { "total": 388000, "output": 64000 }
  }
}
```

Four fields: `input`, `output`, `cache`, `total`. `total` is the context window (the existing `limit.context`). Units are tokens.

#### 4.8.2 What V2 can express with today's schema

| Field | Today | V2 expression | Fork slice |
| --- | --- | --- | --- |
| `total` | Target `limit.context` (`model-routes.ts:61-70`), route `limit.context` (`:494-503`) | Target `limit.context` per tier via the sidecar (`luna 100000`, `terra 388000`) | D1-S16 for tier-level |
| `output` | Model `limit.output` min over leaves (`model-routes.ts:338`); route-level output cap does not exist | Model-declared only | D1-S16 |
| `input` | Only when every leaf declares `limit.input` (`model-routes.ts:339-357`) | Model-declared only | D1-S16 |
| `cache` | `cache_format` exists (`model-routes.ts:71, 461`); no cap | Not expressible | D1-S16, and a provider-side cache cap is a V3 question |

#### 4.8.3 Strictest-wins

For a leaf `L` reached through tier `t`, each dimension `d ∈ {input, output, cache, total}`:

```
effective[d](L, t) = min over sources s in { model(L), door(L), target(L), tier(t) } where s[d] is defined
                     (if no source defines d: effective[d] = undefined and E7 records "limit-unknown")
winner[d]          = the source that supplied the min (recorded)
```

Walk order follows AD4: model native → door policy cap (none today) → leaf cap in the route → tier cap. A cap on a nested route composes by min along the reach path (`model-routes.ts:847-850`).

**Reasoning reserve.** A tier's output must be at least the reserve for that tier (R1 §3.2, §6.1), mirroring the Claude thinking budgets in config (`variant_map`: 4096 / 8192 / 16384; V1a §b.2):

| Tier | Reserve (output tokens) |
| --- | --- |
| low | 4,096 |
| medium | 8,192 |
| high | 16,384 |

A leaf whose effective output is below its tier's reserve is skipped as `output-too-small` (E7), not run. This is what stops a high rung with a small output cap from returning empty (R1 §3.2).

#### 4.8.4 Proposed tier limits (V2)

| Pool | Tier | Proposed | Why |
| --- | --- | --- | --- |
| limited | low | total 100,000 on the Luna leg (target cap, already in config), 388,000 on Terra | B9.2; Luna plan price cliff (V1a §c.2) |
| limited | high | total 388,000 (Terra), 100,000 (Luna leg stays at its cap) | Plan ceiling; does not change with tier |
| all OpenAI leaves | any | output ≥ 16,384 at high (reserve), otherwise skipped | R1 §6.1 |
| all others | — | no tier cap | Native limits govern; no evidence of a plan ceiling |

### 4.9 Guard extensions (`bin/check-opencode-route-rules.mjs`)

New export `inspectEffortTiers(modelRoutes, effortSidecar, catalog)` returning issue strings, added to `checkRouteRules()` next to `inspectFallbackLayout` (existing pattern at `:67`, `:257`, `:473`). Existing checks stay as they are.

| Id | Invariant | Failure text (shape) | Oracle |
| --- | --- | --- | --- |
| E1 | Tiers are exactly `low, medium, high`. Aliases map only to tiers; required keys `xhigh, max, ultra, fast` are present; no alias chains. | `effort: alias "X" must map to a tier` | Declared list (B4.5) |
| E2 | Every route in `model_routes` that has targets defines a cell for `low` and `high` (medium is as written). An absent cell is an error. `unsupported` requires a reason. | `effort: route "R" has no low cell` | Route list in config (JSON `routes`) |
| E3 | Every reachable leaf at low/high has a rung row, or its tier cell is `same-as-medium` (declared). Otherwise it is `unsupported-tier` and must be listed. | `effort: leaf "D/M" at tier "T" has no rung and no same-as-medium` | Expansion (§4.5), no silent drop |
| E4 | No silent remap: every `variants` remap in `opencode.jsonc` has a rung row with the same native, `declared` true, and a reason when native ≠ tier name. Medium rows equal the as-written pin. | `effort: remap "X→Y" on "D/M" has no rung row` | Config remap entries (`model-routes.ts:50-58`) |
| E5 | Overlay keys are unique per (tier, ref, door, window, priority). Two ops on one member at the same priority are a conflict. An op matching zero members (non-`add`) or more than one without `all` is an error. `members` is exclusive with other ops in the tier. | `effort: overlay conflict at "R"/"T" key K priority P` | Keyed merge (§4.3.4) |
| E6 | OpenAI doors (`openai-plan`, `_openai.*`, `openai.*`) have no rung row whose native is `xhigh`, `max`, or `ultra`. | `effort: OpenAI rung "M" at high is "max" (cap is high)` | B7.2, property P6 |
| E7 | For every leaf and tier, effective output ≥ tier reserve (§4.8.3); `limits.total` ≥ `limits.output`; `limits` only on tiers and only with known fields. | `effort: leaf "D/M" tier "T" output "n" below reserve` | R1 §3.2 |
| E8 | Equivalence rows reference existing `route#tier` cells; band is finite; `from` edges form a DAG under the visited-set walk (AD6). | `effort: equivalence "X≈Y" references unknown tier` | §4.7 |
| E9 | Every rung row has a `prov` code from §4.6 and `declared` is `true` iff its `prov` is `CFG`. `CAT`-only rows are warnings, not errors, until D1-S3 lands. | `effort: rung "D/M" tier "T" declared without CFG` | §4.6 |

**Pass/fail policy.** E1–E8 are errors (the guard exits non-zero). E9 is a warning in V2 and an error in V3 (D1-S23). Unknown or unsupported tiers are always *surfaced* in the guard output (count per route), never hidden: the guard prints `effort: N leaf/tier pairs unsupported (route R)` even when it passes.

**Property tests** (`bin/check-opencode-route-rules.effort.test.mjs`, `node --test`; §4.9.1). Oracle for each: the reference resolver in `bin/route-effort.mjs`, which is the executable form of §4.5. The generator is a seeded PRNG (no new dependency); if `fast-check` is already a dev dependency of the repo, use it instead. Each property names its oracle as the rules below require.

#### 4.9.1 PBT invariants

| Id | Property | Oracle | Generator |
| --- | --- | --- | --- |
| P1 | Totality: for any config G, any route R, any tier t, `resolve(R, t)` returns either leaves each with a defined native, or skips each with a reason. No leaf has an undefined native. | Reference resolver | random pools (1–6 members, 0–3 routes deep), random rung rows |
| P2 | Overlay order invariance: permuting the list of overlay ops (with distinct keys/priorities) gives the same members in the same order. | Reference resolver | random op lists |
| P3 | Equal-priority conflicts are always reported as `overlay-conflict`, never resolved last-wins. | Invariant check (the error is present) | two ops on one key at the same priority |
| P4 | Remove/restore: `remove` at 1000 plus `add` at 50 restores the member; `remove` at 50 plus `add` at 1000 keeps it removed. | Model: priority order | random priorities |
| P5 | Empty overlay ≡ medium: a tier cell `same-as-medium` yields identical leaves and natives to medium (with source = `same-as-medium`). | Medium resolution | random pools |
| P6 | OpenAI cap: for every OpenAI leaf and tier, the native is in {low, medium, high, minimal, none}; never xhigh/max/ultra. | Rule list (B7.2) | random OpenAI leaves with overlays |
| P7 | No silent drop: for every resolution, `|members in| = |leaves out| + |skips out|`, and every skip has a reason. | Counting identity | random configs |
| P8 | Record fidelity: from an attempt record `(effort_requested, effort_alias, effort_tier, variant, effort_source)`, re-resolving gives the same native and source. | Re-resolution | random requests |
| P9 | Limits: `effective[d] = min` over defined sources; adding a tier cap never increases an effective limit (monotone). | `Math.min` over defined sources | random caps |
| P10 | Termination: `from` cycles are reported (`cycle-skipped`) and the walk visits each route at most once per call (AD6). | Visited-set walk | random `from` graphs with cycles |

### 4.10 Recording (route_attempt)

**Additive columns** (all nullable, legacy rows = NULL). Written by both writers (§4.11).

| Column | Type | Meaning | Example |
| --- | --- | --- | --- |
| `effort_requested` | TEXT | Caller's token exactly as sent | `max` |
| `effort_alias` | TEXT | Alias applied, `token->tier`, or NULL | `max->high` |
| `effort_tier` | TEXT | Canonical tier used (`low`/`medium`/`high`), or NULL for legacy and bare calls | `high` |
| `effort_source` | TEXT | `as-written`, `rung`, `same-as-medium`, `overlay`, `members`, `unsupported` | `rung` |
| `effort_declared` | INTEGER (0/1) | 1 when the native is declared (CFG), 0 otherwise | `1` |
| `rung_provenance` | TEXT | `prov` code of the rung row used (or `as-written`) | `CFG` |

**Existing `variant` keeps its meaning**: the native string actually sent (after remap or rung). Test oracle (P8): `variant` equals the resolver's native for `(door, model, effort_tier)` whenever `effort_tier` is set. Legacy readers keep working; the trial readers in D4 switch to `effort_tier`.

**Route decisions.** The fork route decision candidate list gains `unsupported-tier` as an exclusion reason (`model-routes.ts:978` currently `continue`s silently). The repo proxy's `eff` null path (`bin/opencode-route-proxy.mjs:1134-1135`) records `effort_source = unsupported` instead of a bare run.

### 4.11 Where the code lands

| Layer | Path | Change | Slice |
| --- | --- | --- | --- |
| Sidecar (V2) | `.opencode/route-effort.jsonc` (new) | Tiers, aliases, rung rows, pool cells, equivalences | D1-S1, S5–S8, S19 |
| Pure module (V2) | `bin/route-effort.mjs` (new) | `parseSidecar`, `resolveAlias`, `applyOverlays`, `resolveRoute`, `effectiveLimits` (the reference oracle) | D1-S1, S17 |
| Guard (V2) | `bin/check-opencode-route-rules.mjs` | `inspectEffortTiers` (E1–E9) | D1-S2, D1-S23 (E9 error) |
| Tests | `bin/check-opencode-route-rules.effort.test.mjs` (new) | P1–P10 | D1-S2 |
| Repo proxy (V2) | `bin/opencode-route-proxy.mjs` | Read sidecar; alias at `resolveRoute` (781-808); tier request in place of `variantIn`; record columns (503-530); bare-run path (1134) | D1-S9, D1-S10 |
| Fork schema (V2) | `~/git/opencode` `packages/schema/src/config/model-routes.ts` | `experimental.routes.effort` (tiers, aliases, rungs, cells, overlays, limits) with decode checks mirroring E1–E5 | D1-S13 |
| Fork overlay (V2) | `~/git/opencode` `packages/core/src/config/plugin/model-route-overlay.ts` (new) | Pure keyed merge (§4.3.4) | D1-S14 |
| Fork expander (V2) | `~/git/opencode` `packages/core/src/config/plugin/model-routes.ts` | Tier cell per route (`expand()` 796-1050); leaf native via rung (`:958-978`); remap drop recorded (`:1017-1021`); `unsupported-tier` recorded (`:978`); `routeVariants` (`:1268`) becomes tiers | D1-S12, D1-S15 |
| Fork resolver (V2) | `~/git/opencode` `packages/core/src/model-resolver.ts` | Dispatch native by tier (`MRes:1065-1073`) | D1-S15 |
| Fork ledger (V2) | `~/git/opencode` `packages/core/src/model-route-log/sql.ts` (+ writer, migration) | Six columns (§4.10) | D1-S11 |
| Docs (V2) | `.opencode/effort-tiers.md` (new) | Operator guide: tiers, aliases, cells, provenance, adding a rung | D1-S18 |

---

## 5. V3 and V4 layers

**V3 — measured and automatic.** Entry gate: V2 records exist for ≥ 14 days per tier decision (main.md §0). Adds:
- **Frontier tiers** (O-p): each (door, model, effort) point gets cost-per-success with CI from `route_attempt` plus D3's store; the frontier is computed; low = cheapest frontier point above a score floor, high = best point below a cost ceiling, medium = the cost midpoint (R1 §7 O4). Replaces the hand-set EQ rows (D1-S21, D1-S22). Learns from: D3 stats store, D4 trials.
- **Band tiers** (R1 §7 O5): tiers as cheapest-within-δ of a class reference (δ_low 5–6, δ_high 1–2 AA points), calibrated from measured spreads. Entry: frontier data for ≥ 3 leaves per class.
- **Rung drift guard** (E9 becomes an error; D1-S23): rung rows compared with the live catalog's declared variants on each sync; a removed variant fails the guard rather than becoming `unsupported-tier` at run time.
- **Measured per-tier limits:** effective output observed from `route_attempt.tokens_output` distribution per tier (p90, p99), to set reserves from data (R1 §6.1), not from Claude's budgets.

**V4 — learned and adaptive.** Entry gate: V3 shows the hand-set table is the limiting factor (main.md §0). Adds:
- **Per-request tier choice and cascade** (R1 §7 O10, O15): start at low, escalate on verifier failure or tool-loop detection, budget-gated (D1-S24). Learns from: attempt outcomes with quality labels (D6).
- **Mid-conversation effort switching** (R1 §7 O12) only where the harness can emit a configuration-update item that keeps the prefix cache (OpenAI `configuration_update`, R1 §3.1). Not in scope until the harness supports it.
- **Bandit allocation inside a tier's safe set** (R1 §4.4.1, O11) — D4 owns the mechanism; D1 provides the safe set (the tier's rung rows and E-invariants).

**Hand-offs.** D5 uses tier classes for its effort lean (S6.1) and reads `effort_tier`. D2 consumes D1 for category effort (the `research → max` question, §4.4). D4 consumes the EQ rows (dominance) and the trial pattern (B9.6). D6 reads `effort_source` and `effort_declared` for review.

---

## 6. Worked examples

### Example A — `summit#low` (the brief's "summit low is workhorse high")

Caller: `opencode-route/summit` with variant `low`.

1. **Caller token.** `low` (canonical, no alias). `effort_requested = low`, `effort_alias = null`, `effort_tier = low`.
2. **Route cell.** `summit.pools.low` is an overlay (§4.3.3). Base = summit medium as written: `fable-plan#medium`, `opus-plan#high`, `argon-plan#high`, `_antigravity/gemini-4-hydrogen#high`, `_muse/muse-wonder-1.0-contributor#xhigh`, `sol-plan#xhigh`, `specialist`.
3. **Overlays, keyed.** `remove fable-plan` (priority 1000) → tombstone. `remove argon-plan` → tombstone. `remove muse-wonder` → tombstone. `set sol-plan low` → member exists, rung lookup for sol-plan is a route member, so the op applies to the route's own tier (propagates into `sol-plan`'s low cell, §4.5). `opus-plan` and `hydrogen` keep medium unless their own low cell is applied: hydrogen gets `set low` in the table; opus stays at `#high` **as written** because `opus-plan` is a route member and its own low cell is `rungs` (Opus low 34.8 @ 0.98).

   *Correction to the table row:* `summit` low lists `opus-plan` at low. The overlay is `set opus-plan low` (not "keeps"). This is the one place the §4.6.2 summary is shorthand; the JSON in §4.3.3 is the source of truth and should include `set opus-plan low` at D1-S5.
4. **Resolution.** `sol-plan` at low → its low cell `rungs` → `openai-plan/gpt-6.1-sol` rung low → native `low` (declared, CAT). `opus-plan` at low → `rungs` → `_claude/claude-opus-5-5` rung low → native `low`. `hydrogen` at low → `_antigravity/gemini-4-hydrogen` rung low → native `low` (CAT).
5. **Leaves out.** `sol low` (35 / $0.16), `opus low` (34.8 / $0.98), `hydrogen low` (27 / $0.29). Specialist at low is `same-as-medium`: its members run as written.
6. **Record.** One row per leaf: `effort_requested=low`, `effort_tier=low`, `effort_source=rung`, `variant=low`, `effort_declared=1` for Claude (CFG) and `0` for OpenAI (CAT), `rung_provenance=CAT` for OpenAI rows.
7. **Equivalence check (EQ1).** `summit#low` = `sol low` 35 @ $0.16. `workhorse#high` = `haiku-plan high` 30 @ $0.16. |35 − 30| = 5 ≤ band: equivalent; summit low is at least as good at equal cost. The row carries `XAA`.

Failure modes: the Claude door fails (JSON 2,609 / 2,975 fable failures; opus 106 / 108) — runtime walled-skip, not a tier error (AD9 boundary). Hydrogen is 100% failed: skip recorded as `walled-skipped`, not `unsupported-tier`.

### Example B — `research` (OpenChamber `variant: max`) on `workhorse`

Caller: `opencode-route/workhorse` with variant `max` (`~/.config/openchamber/routing.json` category `research`).

1. **Token.** `max`. `aliases.max = high`. Record: `effort_requested = max`, `effort_alias = max->high`, `effort_tier = high`.
2. **Route cells.** `workhorse.high` = propagate. `workhorse-ordered.high` = propagate → `deepseek-effort-rr.high` = members [DS flash high] → rung high on `opencode-go-plans/deepseek-v4.1-flash` (CFG `opencode.jsonc:2186-2187`) → native `high`. `workhorse-weighted.high` = overlay: `set Hydrogen high`, `luna-effort-rr` → members [Luna high] → rung high on `openai-plan/gpt-6-luna` (CAT) → native `high`. Muse: no cell change → as written `xhigh` is medium; under the tier it is `same-as-medium` → native `xhigh` (declared, `source=same-as-medium`). `workhorse-last-resort.high` = overlay: remove fable → Haiku high, native `high` (CFG 284-326).
3. **Behaviour change.** Before: `max` was ignored by the pinned seats (V1a §b.3); the category ran the pins. After: the seats run at high. Net cost change is the Hydrogen high leg (27 → 51 AA points for $0.29 → $1.87 per task) and the Luna high leg (cost 0.04 → 0.07). This is the B4.5 intent, and it is the reason §4.4 recommends that D2 set the category to `high`.
4. **Record.** Every attempt row for this call: `effort_requested = max`, `effort_alias = max->high`, `effort_tier = high`. A reader that wanted "what did research ask for" sees `max`; a reader that wanted "what ran" sees the native column (`variant`).

### Example C — `background` at low (cost king, DeepSeek windows)

Caller: `opencode-route/background` with variant `low` (a batch job).

1. **Token.** `low`. Cell `background.low` = overlay: `set deepseek-flash-plan low` with `all: true` (both window entries, `opencode.jsonc:2957, 2961`), `prov: AA5`, `reason: non-reasoning proxy at 19 for $0.21–0.43`.
2. **Window gates.** Both DS entries keep their window gates (`deepseek-peak-early` inverted and not). The mimo entries (`2949`, `2953`, `2964`) are `same-as-medium` and keep running.
3. **Resolution.** `deepseek-flash-plan` at low → legs: Go DS flash low (CFG 2186-2187), ClinePass DS flash low (CAT, unverified), OpenDesign DS flash low (CAT), Mistral 1017 low (CFG `_vibe` 602-623). Each is a rung row; none is `unsupported`.
4. **Why this is cheap.** DS flash at low is the AA non-reasoning point (19 AA points) at $0.21–0.43 per task, versus Haiku at 19 for $0.04 (EQ2). Background chooses cost, so the rule (B7.3) keeps DS for its off-peak window and the Haiku leg stays on `workhorse-last-resort` for quality.
5. **Off-peak.** A background call at peak still gets DS low; the peak-early gate removes the high entry, which is the existing behaviour (V1a §a.1). Tier does not change the window logic.
6. **Recording.** `effort_source = overlay`, `effort_declared = 1` for Go and Mistral, `0` for ClinePass and OpenDesign (CAT). An operator sees which legs ran on unverified rungs.

---

## 7. Risks, open questions, and challenges

### 7.1 Open questions (each with the default chosen)

Format: `Q-D1-n` · status · question · default · why · where applied. Q-items already in `questions.md` (Q1–Q6) are not repeated, except where D1 changes their default.

- **Q-D1-1 — Which expander serves a request?** The fork (`model-routes.ts`) or the repo proxy (`bin/opencode-route-proxy.mjs`). *Default:* both read the same sidecar until the fork schema lands (D1-S13); a test (D1-S9) asserts they produce identical natives for every row. *Why:* the baseline does not settle the live path. *Applied:* D1-S9, D1-S10.
- **Q-D1-2 — Pin explicit medium where the default is bare?** Several pools (`haiku-plan`, `opus-plan`, `sol-plan`, `gemini-hydrogen-plan`, `luna-plan`, `terra-plan`, `fable-plan`) run unpinned, so medium is "the door's default effort". Pinning changes behaviour. *Default:* no pin at V2 (medium stays as written, B4.2); the default is recorded per door in the rung table as `medium: "default"` (declared false) so it is visible. *Why:* B4.2 says "align current seats as medium", and today's seat is the default. *Applied:* §4.2; E4 accepts `default`.
- **Q-D1-3 — Does `opencode.jsonc`'s strict fork schema accept an unknown `experimental.routes.effort` key?** Not verified in this pass. *Default:* sidecar (`.opencode/route-effort.jsonc`) until D1-S13 lands; O-y (inline) rejected for now. *Applied:* §3.6, §4.11.
- **Q-D1-4 — Is `research → max` meant as high?** *Default:* the alias is honoured as B4.5 requires; the category is not edited (Q6). Recommendation to D2: set the category to `high`. *Applied:* §4.4, Example B.
- **Q-D1-5 — Keep rung rows for walled doors?** (Hydrogen, Muse, Go, Sonnet antigravity, most OpenAI-retired ids.) *Default:* yes. The row describes the door, not its health; health is a runtime input (D4). *Applied:* §4.6.2 final paragraph.
- **Q-D1-6 — Catalog-only doors (ClinePass, OpenDesign, `opencode-go*`, `_openai.*` variants, Antigravity, Mistral).** Their variant sets are not in config (V1a caveats). *Default:* `prov: CAT`, `declared: false`, warning E9; resolved by D1-S3 against the live catalog. *Applied:* §4.6.2 coverage; D1-S3.
- **Q-D1-7 — Catalog `aaScore` vs AA v5 rankings.** `aa-last-good.json` `catalog[].aaScore` for Muse is 48 (v4.3.2), v5 is 36. *Default:* the AA5 rankings are authoritative; the catalog column is stale. *Applied:* §4.6.1; D1-S4.
- **Q-D1-8 — Fable naming.** AA lists "Claude Fable 5.1"; config is `claude-fable-5-2`. *Default:* treat as the same model with `prov: XAA`. *Applied:* §4.6.1, fable rows.
- **Q-D1-9 — Terra through the retired `_openai` id.** 17,858 attempts at 4% failure, while `_openai.main`/`_openai.alt` fail at 98% and 82%. *Default:* record as an anomaly; no tier decision depends on it until D1-S4 explains it (is the retired id still serving real traffic, or is it a counting artefact?). *Applied:* D1-S4.
- **Q-D1-10 — Overlay ops that also appear in `route-fallbacks.md`.** The doc is stale on three points (V1a §a.7). *Default:* D1 does not regenerate it; D2 owns it. *Applied:* none in D1.
- **Q-D1-11 — `same-as-medium` on a leaf whose medium pin is `xhigh` (speed/muse).** *Default:* `same-as-medium` runs it at native `xhigh` (declared), and the record says `effort_source = same-as-medium`. That is honest and is what B4.7 asks for.

### 7.2 Risks

- **Risk R-D1-1 — Health masks tiers.** Most leaves fail (V1a §e). A tier with only failing legs will run walled-skipped and look like "the tier is down". *Mitigation:* the record keeps `effort_tier` and the skip reason; D4 reads them separately.
- **Risk R-D1-2 — Silent drop returns through a new path.** *Mitigation:* E3, E4, P7; the proxy path's `eff` null branch is changed to record (D1-S10).
- **Risk R-D1-3 — Overlay sprawl.** Many keyed ops per pool. *Mitigation:* E5 keeps one op per key per tier; the sidecar is generated from §4.6 rows where possible; reviewers see one diff per pool.
- **Risk R-D1-4 — Cell and dominance rules age.** AA data is a single snapshot (2026-10-31 batch, generated 2026-11-13). *Mitigation:* E9 warnings; V3 frontier replaces the hand-set rows (D1-S21).
- **Risk R-D1-5 — Claude premium is a judgement.** B7.2 says Claude costs more per %-dollar than AA suggests; the table uses AA cost as given and removes dominated Claude legs. *Mitigation:* the rule is written down (§4.6.3); D4 measures real per-%-dollar cost.
- **Risk R-D1-6 — Unknown tier on a new door.** *Mitigation:* E3 fails the guard; the leaf is never run at an undeclared native.

### 7.3 Challenges to `main.md` (deviations or sharpenings — none silent)

- **C1 (AD1, sharpening).** AD1 says the tier is resolved to a native "per (door, model) at expansion time from a single rung table". Medium is resolved **as written** instead (§4.2) so that B4.2 holds exactly and V2 changes no medium. The rung table holds low and high rows, and a medium audit column that E4 checks. *Default applied:* as written.
- **C2 (AD2, sharpening).** AD2 says "medium is the pool as written". Several pools are unpinned, so "as written" means "the door's default" (Q-D1-2). The design records it as `default`, declared false, rather than pinning. *Default applied:* no pin at V2.
- **C3 (AD1 and B4.5, consequence).** Aliasing caller `max` to `high` changes what OpenChamber `research` runs (Example B). Intended by B4.5, but main.md should record that the alias is caller-side only and that categories are D2's to fix.
- **C4 (AD3, provenance).** AD3 allows provenance `measured`. There are no measured equivalence rows in V2; every row is `XAA` or `AA5`. The `measured` value is reserved for V3 (D1-S21).
- **C5 (AD4, feasibility).** AD4 says the effective limit is the minimum of "the door's policy cap". No door policy cap exists in the schema either. V2 drops that term (§4.8.2); the fork slice may add it later.
- **C6 (AD1 vs the code, a real gap).** AD1 assumes one expander. The repo has two (§2, Q-D1-1). Recommend main.md name the fork expander authoritative and require the proxy to match it by test (D1-S9).
- **C7 (AD6, scope).** AD6's visited set is specified for expansion. The tier resolver is a second walk (`from` cells and propagation). D1 applies the same visited set to it (§4.5), so AD6 holds without change.

### 7.4 Things this design does not decide

- The decision-pool (`decision`) effort lean (S6.1, D5).
- Which Claude `max` cases deserve a native rung (B4.5 says none until implemented).
- Whether Terra's retired-id traffic is real (Q-D1-9).

---

## 8. Statement coverage table

| Statement | Section(s) of this design | Slice(s) |
| --- | --- | --- |
| B3.1 | §4.8.1, §4.8.2, §4.8.3 | D1-S16, D1-S17 |
| B3.2 | §4.8.3, §4.8.4 | D1-S16 |
| B4.1 | §4.6.2 (every row), §4.9 E2 | D1-S1, D1-S5, D1-S6, D1-S7, D1-S8 |
| B4.2 | §4.2, §7.3 C1, C2 | D1-S1, D1-S5 |
| B4.3 | §4.6.1, §4.7 | D1-S4, D1-S19 |
| B4.4 | §4.3.1 | D1-S1, D1-S5 |
| B4.5 | §4.4, §4.9 E1, Example B | D1-S1, D1-S9 |
| B4.6 | §4.3.2, §4.3.3, §4.3.4 | D1-S1, D1-S14 |
| B4.7 | §4.3.1 (`same-as-medium`), §4.6.2 (argon, spark, scout) | D1-S5 |
| B4.8 | §4.7 (EQ1–EQ7) | D1-S19, D1-S20 |
| B4.9 | §4.6.2 coverage check, §4.9 E2 | D1-S2, D1-S5–S8 |
| B5.1 | §4.10 (fields recorded), §5 V3 | D1-S10, D1-S11, D1-S21 |
| B7.1 | §4.6.2 (limited, background, scout), Example C | D1-S5, D1-S7 |
| B7.2 | §4.6.2, §4.6.3, §4.9 E6, P6 | D1-S2, D1-S5, D1-S7 |
| B7.3 | §4.6.2 (workhorse rows), §4.6.3 | D1-S6 |
| B8.6 | §4.7, §5 | D1-S20 |
| B9.2 | §4.6.2 (limited), §4.8.4 | D1-S7, D1-S16 |
| B9.3 | §2, §4.2, E4 | D1-S2, D1-S7 |
| B9.6 | §4.7 (EQ7), §4.6.2 (luna-effort-rr) | D1-S6, D1-S20 |
| S6.1 | §5 hand-offs | D1-S15 (tier classes exposed to D5) |
| S3 (decision effort) | §4.4 last bullet | (no D1 slice; D5 owns) |
| AD1 | §4.2, §4.5, §7.3 C1 | D1-S1, D1-S15 |
| AD2 | §4.3, §7.3 C2 | D1-S1, D1-S14 |
| AD3 | §4.7, §7.3 C4 | D1-S19 |
| AD4 | §4.8, §7.3 C5 | D1-S16 |
| AD6 | §4.5, §7.3 C7 | D1-S1, D1-S15 |
